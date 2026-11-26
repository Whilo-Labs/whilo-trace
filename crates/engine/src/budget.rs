//! One budget of probes per second shared by every monitor of an app, so many targets together
//! cannot flood the network they measure: on 2026-10-28 about 137 probes a second made a home
//! router stop answering, and the loss it then showed was made by the program itself.
//!
//! Every monitor takes its rounds from a share of its own: the budget split evenly between the
//! monitors that sent in the last seconds, so no target starves another. A round goes while its
//! share holds at least one probe, and a large round may leave the share in debt that the next
//! rounds pay off: a full round of 30 TTLs is never stuck behind small ones, and over time no more
//! than the budget goes out. A round that finds its share empty is not sent: it is a gap on the
//! timeline, never loss.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Probes a second for all targets together when nobody chose another value.
pub const DEFAULT_PROBES_PER_S: u32 = 50;
/// A monitor that took nothing for this long (paused, or a long interval) leaves its part of the
/// budget to the others until it sends again.
const ACTIVE_SPAN: Duration = Duration::from_secs(5);
/// How far back `sent_per_s` counts.
const RATE_SPAN: Duration = Duration::from_secs(10);
/// How far back `skipped_recently` counts.
const SKIP_SPAN: Duration = Duration::from_secs(60);

pub struct ProbeBudget {
    state: Mutex<State>,
}

struct State {
    per_s: u32,
    since: Instant,
    shares: HashMap<u64, Share>,
    next_id: u64,
    /// Probes taken, with when.
    sent: VecDeque<(Instant, u32)>,
    /// Rounds that found their share empty.
    skipped: VecDeque<Instant>,
}

struct Share {
    tokens: f64,
    at: Instant,
    /// Last time this share took or asked; `None` before its first round.
    seen: Option<Instant>,
}

impl State {
    /// Shares that sent in the last seconds, counting the one asking now.
    fn active(&self, asking: u64, now: Instant) -> usize {
        self.shares
            .iter()
            .filter(|(&id, s)| {
                id == asking
                    || s.seen
                        .is_some_and(|t| now.saturating_duration_since(t) <= ACTIVE_SPAN)
            })
            .count()
            .max(1)
    }

    fn forget(&mut self, now: Instant) {
        while self
            .sent
            .front()
            .is_some_and(|(t, _)| now.saturating_duration_since(*t) > RATE_SPAN)
        {
            self.sent.pop_front();
        }
        while self
            .skipped
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) > SKIP_SPAN)
        {
            self.skipped.pop_front();
        }
    }
}

impl ProbeBudget {
    /// A budget of `per_s` probes a second (at least 1) for all its shares together.
    pub fn new(per_s: u32) -> Arc<Self> {
        let now = Instant::now();
        Arc::new(Self {
            state: Mutex::new(State {
                per_s: per_s.max(1),
                since: now,
                shares: HashMap::new(),
                next_id: 0,
                sent: VecDeque::new(),
                skipped: VecDeque::new(),
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A share for one monitor; it gives its part back when dropped.
    pub fn share(self: &Arc<Self>) -> BudgetShare {
        let now = Instant::now();
        let mut s = self.lock();
        let id = s.next_id;
        s.next_id += 1;
        // A new share starts with a second of its part, so its first round goes at once.
        let others = s
            .shares
            .values()
            .filter(|x| {
                x.seen
                    .is_some_and(|t| now.saturating_duration_since(t) <= ACTIVE_SPAN)
            })
            .count();
        let part = f64::from(s.per_s) / (others as f64 + 1.0);
        s.shares.insert(
            id,
            Share {
                tokens: part.max(1.0),
                at: now,
                seen: None,
            },
        );
        BudgetShare {
            budget: self.clone(),
            id,
        }
    }

    pub fn per_s(&self) -> u32 {
        self.lock().per_s
    }

    /// Changes the probes a second from now on.
    pub fn set_per_s(&self, per_s: u32) {
        self.lock().per_s = per_s.max(1);
    }

    fn take(&self, id: u64, n: u32) -> bool {
        let now = Instant::now();
        let mut s = self.lock();
        s.forget(now);
        let rate = f64::from(s.per_s) / s.active(id, now) as f64;
        let Some(share) = s.shares.get_mut(&id) else {
            return true;
        };
        let elapsed = now.saturating_duration_since(share.at).as_secs_f64();
        share.tokens = (share.tokens + elapsed * rate).min(rate.max(1.0));
        share.at = now;
        share.seen = Some(now);
        if share.tokens + 1e-9 >= 1.0 {
            share.tokens -= f64::from(n);
            s.sent.push_back((now, n));
            true
        } else {
            s.skipped.push_back(now);
            false
        }
    }

    /// Probes a second actually sent by all shares over the last ten seconds.
    pub fn sent_per_s(&self) -> f64 {
        let now = Instant::now();
        let mut s = self.lock();
        s.forget(now);
        let span = now
            .saturating_duration_since(s.since)
            .min(RATE_SPAN)
            .as_secs_f64()
            .max(1.0);
        s.sent.iter().map(|(_, n)| f64::from(*n)).sum::<f64>() / span
    }

    /// Rounds not sent for an empty share over the last minute.
    pub fn skipped_recently(&self) -> u32 {
        let mut s = self.lock();
        s.forget(Instant::now());
        s.skipped.len() as u32
    }
}

/// One monitor's part of a `ProbeBudget`.
pub struct BudgetShare {
    budget: Arc<ProbeBudget>,
    id: u64,
}

impl BudgetShare {
    /// Takes `n` probes for a round. `false`: the share is empty, the round is not sent.
    pub fn take(&self, n: u32) -> bool {
        self.budget.take(self.id, n)
    }
}

impl Drop for BudgetShare {
    fn drop(&mut self) {
        self.budget.lock().shares.remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_share_gives_a_round_and_then_refuses() {
        let b = ProbeBudget::new(50);
        let s = b.share();
        assert!(s.take(30), "the first round goes at once");
        assert!(s.take(20));
        assert!(!s.take(30), "nothing left");
        assert_eq!(b.skipped_recently(), 1);
        assert_eq!(b.per_s(), 50);
    }

    #[test]
    fn a_share_fills_again_at_its_rate() {
        let b = ProbeBudget::new(1000);
        let s = b.share();
        assert!(s.take(1000));
        assert!(!s.take(10));
        std::thread::sleep(Duration::from_millis(60));
        assert!(
            s.take(10),
            "60 ms at 1000 a second bring back about 60 probes"
        );
    }

    #[test]
    fn a_large_round_goes_and_leaves_a_debt() {
        let b = ProbeBudget::new(5);
        let s = b.share();
        assert!(
            s.take(30),
            "a full round goes while the share holds a probe"
        );
        assert!(!s.take(1), "and leaves it in debt");
        std::thread::sleep(Duration::from_millis(300));
        assert!(!s.take(1), "the debt of 25 takes 5 s to pay off");
    }

    #[test]
    fn busy_monitors_split_the_budget_evenly() {
        // Two monitors that ask far more than the budget: each gets about half of it.
        let b = ProbeBudget::new(200);
        let (x, y) = (b.share(), b.share());
        let (mut nx, mut ny) = (0u32, 0u32);
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_millis(500) {
            nx += 4 * u32::from(x.take(4));
            ny += 4 * u32::from(y.take(4));
            std::thread::sleep(Duration::from_millis(2));
        }
        let (fx, fy) = (f64::from(nx), f64::from(ny));
        assert!((fx - fy).abs() <= 0.3 * fx.max(fy), "{nx} and {ny}");
        assert!(
            fx + fy <= 200.0 * 0.6 + 220.0,
            "{nx} + {ny}: half a second of 200/s plus the start"
        );
    }

    #[test]
    fn a_quiet_share_leaves_its_part_to_the_others() {
        let b = ProbeBudget::new(100);
        let busy = b.share();
        let _quiet = b.share();
        assert!(busy.take(1));
        assert!(busy.take(99), "drained");
        // The other share never sent: the busy one gets the whole budget, 100 a second, so
        // 100 ms bring back 10 probes, not 5.
        std::thread::sleep(Duration::from_millis(100));
        assert!(busy.take(8));
        assert!(busy.take(1), "2 of the 10 are left");
    }

    #[test]
    fn sent_probes_give_the_rate() {
        let b = ProbeBudget::new(100);
        let s = b.share();
        assert!(s.take(12));
        assert!(s.take(12));
        let rate = b.sent_per_s();
        assert!(
            (rate - 24.0).abs() < 1e-6,
            "24 probes within the first second: {rate}"
        );
    }

    #[test]
    fn a_new_rate_applies_at_once() {
        let b = ProbeBudget::new(100);
        let s = b.share();
        b.set_per_s(10);
        assert_eq!(b.per_s(), 10);
        assert!(s.take(32), "the share holds what it had");
        assert!(!s.take(5), "and is in debt now");
        drop(s);
        assert!(b.lock().shares.is_empty(), "a dropped share is gone");
    }
}
