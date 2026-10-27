//! Timeline buckets kept up to date round by round, so a snapshot of the main screen costs the
//! number of buckets and hops, not the number of rounds behind them.
//!
//! Buckets sit on a fixed grid: bucket `k` of width `b` covers `[k * b, (k + 1) * b)`. A new round
//! changes only the bucket of its start time, every other bucket stays as it was. A round counts in
//! the bucket of its start time. Rules (docs/ui-contract.md):
//! - destination: a bucket is in `loss` when a round in it did not reach the destination;
//! - hop: `loss` when the hop did not answer in a round where the destination was lost too,
//!   `partial` when the hop did not answer while the destination answered;
//! - the latency line goes through answers only; a bucket without answers is `None`.
//!
//! A run of buckets is a half-open range of indices (`a..z`, `z` not included), like the window
//! and the picked period of the interface.

use std::collections::VecDeque;
use std::net::IpAddr;
use std::ops::Range;
use whilo_trace_engine::stats::LOSS_NOTABLE_PCT;
use whilo_trace_engine::{HopStatus, RoundRecord};

/// One line (the destination or one TTL) in one bucket.
#[derive(Debug, Clone, Default)]
pub struct Cell {
    /// Probes with a sample here: rounds that reached this TTL (for the destination, all rounds).
    pub sent: u32,
    pub recv: u32,
    /// No answer while the destination was lost in the same round.
    pub lost_path: u32,
    n: u32,
    sum: f64,
    min: f64,
    max: f64,
    /// First and last RTT in arrival order, to join jitter across buckets.
    first: f64,
    last: f64,
    /// Sum of |RTT difference| between consecutive answers inside the bucket, and their count.
    jsum: f64,
    jn: u32,
    /// The latest probe here got no answer.
    last_lost: bool,
    /// Who answered last.
    addr: Option<IpAddr>,
}

impl Cell {
    fn add(&mut self, answered: bool, dest_lost: bool, rtt_ms: Option<f64>, addr: Option<IpAddr>) {
        self.sent += 1;
        self.last_lost = !answered;
        if !answered {
            if dest_lost {
                self.lost_path += 1;
            }
            return;
        }
        self.recv += 1;
        if addr.is_some() {
            self.addr = addr;
        }
        if let Some(ms) = rtt_ms {
            if self.n == 0 {
                self.min = ms;
                self.max = ms;
                self.first = ms;
            } else {
                self.min = self.min.min(ms);
                self.max = self.max.max(ms);
                self.jsum += (ms - self.last).abs();
                self.jn += 1;
            }
            self.n += 1;
            self.sum += ms;
            self.last = ms;
        }
    }

    /// No answer while the destination answered: the hop limits its own replies.
    pub fn lost_partial(&self) -> u32 {
        self.sent - self.recv - self.lost_path
    }

    pub fn avg(&self) -> Option<f64> {
        (self.n > 0).then(|| self.sum / self.n as f64)
    }

    pub fn min(&self) -> Option<f64> {
        (self.n > 0).then_some(self.min)
    }

    pub fn max(&self) -> Option<f64> {
        (self.n > 0).then_some(self.max)
    }
}

/// Everything the rounds of one bucket found.
#[derive(Debug, Clone, Default)]
pub struct Slot {
    pub rounds: u32,
    /// The destination: sent is every round, recv the rounds that reached it.
    pub dest: Cell,
    /// One cell per TTL, `hops[ttl - 1]`.
    pub hops: Vec<Cell>,
    /// How often each route length was seen.
    path_lens: Vec<(u8, u32)>,
}

impl Slot {
    fn add(&mut self, r: &RoundRecord) {
        self.rounds += 1;
        let dest_lost = !r.reached;
        let dest_rtt = r.destination().and_then(|h| h.rtt_ms());
        self.dest.add(r.reached, dest_lost, dest_rtt, Some(r.dst));
        for h in &r.hops {
            if h.ttl == 0 {
                continue;
            }
            let i = h.ttl as usize - 1;
            if self.hops.len() <= i {
                self.hops.resize(i + 1, Cell::default());
            }
            self.hops[i].add(h.kind.answered(), dest_lost, h.rtt_ms(), h.addr);
        }
        match self
            .path_lens
            .iter_mut()
            .find(|(len, _)| *len == r.path_len)
        {
            Some(e) => e.1 += 1,
            None => self.path_lens.push((r.path_len, 1)),
        }
    }

    pub fn hop(&self, ttl: u8) -> Option<&Cell> {
        self.hops.get((ttl as usize).checked_sub(1)?)
    }
}

/// Buckets of one width over the latest rounds of a target.
#[derive(Debug, Clone)]
pub struct Grid {
    /// Bucket width, ms.
    pub b: i64,
    /// Bucket index of `slots[0]`.
    base: i64,
    slots: VecDeque<Slot>,
    /// Buckets kept, counted back from the newest round.
    keep: usize,
    /// Every round from this bucket on is in the grid.
    covered_from: i64,
    /// Last time a snapshot used the grid, for eviction.
    used_ms: i64,
}

impl Grid {
    /// Buckets of width `b` from bucket `from_k` on, filled from `rounds` (any order).
    pub fn build<'a>(
        rounds: impl IntoIterator<Item = &'a RoundRecord>,
        b: i64,
        from_k: i64,
        keep: usize,
    ) -> Self {
        let mut g = Grid {
            b: b.max(1),
            base: from_k,
            slots: VecDeque::new(),
            keep: keep.max(1),
            covered_from: from_k,
            used_ms: 0,
        };
        for r in rounds {
            g.add(r);
        }
        g
    }

    pub fn add(&mut self, r: &RoundRecord) {
        let k = r.ts_ms.div_euclid(self.b);
        if k < self.base {
            return;
        }
        // Drop what falls out of the kept range; a long pause must not fill the gap with empty slots.
        let new_base = k - (self.keep as i64 - 1);
        if new_base > self.base {
            let drop = (new_base - self.base) as usize;
            if drop >= self.slots.len() {
                self.slots.clear();
            } else {
                self.slots.drain(..drop);
            }
            self.base = new_base;
            self.covered_from = self.covered_from.max(new_base);
        }
        let i = (k - self.base) as usize;
        while self.slots.len() <= i {
            self.slots.push_back(Slot::default());
        }
        self.slots[i].add(r);
    }

    pub fn slot(&self, k: i64) -> Option<&Slot> {
        if k < self.base {
            return None;
        }
        self.slots.get((k - self.base) as usize)
    }

    /// The grid holds every round of buckets `from_k..from_k + n`.
    pub fn covers(&self, from_k: i64, n: usize) -> bool {
        from_k >= self.covered_from && n <= self.keep
    }

    /// Buckets `from_k..from_k + n` as a window.
    pub fn window(&self, from_k: i64, n: usize) -> Window<'_> {
        Window {
            slots: (0..n as i64).map(|i| self.slot(from_k + i)).collect(),
        }
    }
}

/// A line drawn on a timeline: one value per bucket and the marked buckets.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Line {
    pub avg: Vec<Option<f64>>,
    pub lo: Vec<Option<f64>>,
    pub hi: Vec<Option<f64>>,
    pub loss: Vec<u32>,
    pub partial: Vec<u32>,
    /// Probes per bucket: the rounds that probed this line in it (0 for a bucket without rounds).
    pub sent: Vec<u32>,
    /// Probes per bucket that got no answer, on the path or not.
    pub lost: Vec<u32>,
}

/// Statistics of one line over a run of buckets, from the raw answers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LineStats {
    pub sent: u32,
    pub recv: u32,
    pub lost_path: u32,
    pub avg: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// RTT of the latest answer.
    pub last: Option<f64>,
    /// The latest probe got no answer.
    pub last_lost: bool,
    /// Mean |difference| of consecutive RTTs.
    pub jitter: Option<f64>,
}

impl LineStats {
    /// Share lost on the path, %.
    pub fn loss_pct(&self) -> f64 {
        pct(self.lost_path, self.sent)
    }

    /// Share not answered while the destination answered, %.
    pub fn no_reply_pct(&self) -> f64 {
        pct(self.sent - self.recv - self.lost_path, self.sent)
    }
}

fn pct(part: u32, all: u32) -> f64 {
    if all == 0 {
        0.0
    } else {
        100.0 * part as f64 / all as f64
    }
}

/// One TTL over a whole window, classified like `whilo_trace_engine::hop_stats` does.
#[derive(Debug, Clone, PartialEq)]
pub struct HopInfo {
    pub ttl: u8,
    /// Most frequent responder.
    pub addr: Option<IpAddr>,
    pub sent: u32,
    pub recv: u32,
    pub status: HopStatus,
    pub is_destination: bool,
    /// First hop of the loss that reaches the destination.
    pub loss_origin: bool,
}

/// Which line of a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Dest,
    Hop(u8),
}

impl Pick {
    fn cell(self, s: &Slot) -> Option<&Cell> {
        match self {
            Pick::Dest => Some(&s.dest),
            Pick::Hop(ttl) => s.hop(ttl),
        }
    }
}

/// N consecutive buckets of a grid; `None` where the grid has nothing.
pub struct Window<'a> {
    slots: Vec<Option<&'a Slot>>,
}

impl<'a> Window<'a> {
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Every bucket of the window.
    pub fn all(&self) -> Range<usize> {
        0..self.slots.len()
    }

    /// The buckets of `r` that hold rounds; `r` is cut to the window.
    fn slots_in(&self, r: Range<usize>) -> impl Iterator<Item = &'a Slot> + '_ {
        let end = r.end.min(self.slots.len());
        let start = r.start.min(end);
        self.slots[start..end].iter().flatten().copied()
    }

    pub fn rounds(&self) -> u32 {
        self.rounds_in(self.all())
    }

    /// Rounds that started in the buckets `r`.
    pub fn rounds_in(&self, r: Range<usize>) -> u32 {
        self.slots_in(r).map(|s| s.rounds).sum()
    }

    /// Deepest TTL with a sample.
    pub fn max_ttl(&self) -> u8 {
        self.max_ttl_in(self.all())
    }

    fn max_ttl_in(&self, r: Range<usize>) -> u8 {
        self.slots_in(r)
            .map(|s| s.hops.len())
            .max()
            .unwrap_or(0)
            .min(u8::MAX as usize) as u8
    }

    /// Route length seen most often; on a tie the shorter one.
    pub fn dest_ttl(&self) -> Option<u8> {
        self.dest_ttl_in(self.all())
    }

    fn dest_ttl_in(&self, r: Range<usize>) -> Option<u8> {
        let mut counts: Vec<(u8, u32)> = Vec::new();
        for s in self.slots_in(r) {
            for &(len, n) in &s.path_lens {
                match counts.iter_mut().find(|(l, _)| *l == len) {
                    Some(e) => e.1 += n,
                    None => counts.push((len, n)),
                }
            }
        }
        counts
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .map(|(len, _)| len)
    }

    pub fn line(&self, pick: Pick) -> Line {
        let n = self.slots.len();
        let mut out = Line {
            avg: Vec::with_capacity(n),
            lo: Vec::with_capacity(n),
            hi: Vec::with_capacity(n),
            sent: Vec::with_capacity(n),
            lost: Vec::with_capacity(n),
            ..Line::default()
        };
        for (i, s) in self.slots.iter().enumerate() {
            let c = s.and_then(|s| pick.cell(s));
            out.avg.push(c.and_then(Cell::avg));
            out.lo.push(c.and_then(Cell::min));
            out.hi.push(c.and_then(Cell::max));
            out.sent.push(c.map_or(0, |c| c.sent));
            out.lost.push(c.map_or(0, |c| c.sent - c.recv));
            if let Some(c) = c {
                if c.lost_path > 0 {
                    out.loss.push(i as u32);
                } else if c.lost_partial() > 0 {
                    out.partial.push(i as u32);
                }
            }
        }
        out
    }

    /// Statistics over the buckets `r` (cut to the window); an empty run gives empty statistics.
    pub fn stats(&self, pick: Pick, r: Range<usize>) -> LineStats {
        let mut st = LineStats::default();
        let (mut n, mut sum, mut jsum, mut jn) = (0u32, 0.0f64, 0.0f64, 0u32);
        let mut prev: Option<f64> = None;
        for s in self.slots_in(r) {
            let Some(c) = pick.cell(s) else { continue };
            if c.sent == 0 {
                continue;
            }
            st.sent += c.sent;
            st.recv += c.recv;
            st.lost_path += c.lost_path;
            st.last_lost = c.last_lost;
            if c.n > 0 {
                n += c.n;
                sum += c.sum;
                st.min = Some(st.min.map_or(c.min, |m| m.min(c.min)));
                st.max = Some(st.max.map_or(c.max, |m| m.max(c.max)));
                jsum += c.jsum;
                jn += c.jn;
                if let Some(p) = prev {
                    jsum += (c.first - p).abs();
                    jn += 1;
                }
                prev = Some(c.last);
                st.last = Some(c.last);
            }
        }
        st.avg = (n > 0).then(|| sum / n as f64);
        st.jitter = (jn > 0).then(|| jsum / jn as f64);
        st
    }

    /// Every TTL of the window with its status, the same rules as the engine's `hop_stats`.
    pub fn hops(&self) -> Vec<HopInfo> {
        self.hops_in(self.all())
    }

    /// Every TTL probed in the buckets `r`, classified over those buckets only: the statuses of a
    /// picked period. The same as `hop_stats` over the rounds of `r`.
    pub fn hops_in(&self, r: Range<usize>) -> Vec<HopInfo> {
        let max_ttl = self.max_ttl_in(r.clone());
        let dest_ttl = self.dest_ttl_in(r.clone());
        let mut rows: Vec<(HopInfo, f64)> = (1..=max_ttl)
            .map(|ttl| {
                let (mut sent, mut recv) = (0u32, 0u32);
                let mut addrs: Vec<(IpAddr, u32)> = Vec::new();
                for c in self.slots_in(r.clone()).filter_map(|s| s.hop(ttl)) {
                    sent += c.sent;
                    recv += c.recv;
                    if let Some(a) = c.addr {
                        match addrs.iter_mut().find(|(x, _)| *x == a) {
                            Some(e) => e.1 += c.recv,
                            None => addrs.push((a, c.recv)),
                        }
                    }
                }
                // On a tie the latest responder wins: max_by keeps the last maximum.
                let addr = addrs.iter().max_by_key(|(_, n)| *n).map(|(a, _)| *a);
                let loss_pct = pct(sent - recv, sent);
                (
                    HopInfo {
                        ttl,
                        addr,
                        sent,
                        recv,
                        status: HopStatus::Ok,
                        is_destination: Some(ttl) == dest_ttl,
                        loss_origin: false,
                    },
                    loss_pct,
                )
            })
            .collect();
        classify(&mut rows);
        rows.into_iter().map(|(h, _)| h).collect()
    }
}

/// Mirrors the classification of `whilo_trace_engine::stats` on window totals: loss at a hop
/// counts only when it carries on to the hops after it. Kept in step by a test against `hop_stats`.
fn classify(rows: &mut [(HopInfo, f64)]) {
    let statuses: Vec<HopStatus> = (0..rows.len())
        .map(|i| {
            let (row, loss) = &rows[i];
            let later = &rows[i + 1..];
            let later_alive = later.iter().any(|(r, _)| r.recv > 0);
            let later_min_loss = later
                .iter()
                .filter(|(r, _)| r.recv > 0)
                .map(|(_, l)| *l)
                .reduce(f64::min);
            if row.recv == 0 {
                if later_alive {
                    HopStatus::Silent
                } else {
                    HopStatus::NoReply
                }
            } else if *loss < LOSS_NOTABLE_PCT {
                HopStatus::Ok
            } else {
                match later_min_loss {
                    Some(down) if down + (loss * 0.5).max(LOSS_NOTABLE_PCT) < *loss => {
                        HopStatus::RateLimited
                    }
                    _ => HopStatus::Lossy,
                }
            }
        })
        .collect();
    for ((row, _), status) in rows.iter_mut().zip(statuses) {
        row.status = status;
    }
    let Some(dest) = rows.iter().rposition(|(r, _)| r.is_destination) else {
        return;
    };
    if rows[dest].0.status != HopStatus::Lossy {
        return;
    }
    let mut origin = dest;
    for j in (0..dest).rev() {
        match rows[j].0.status {
            HopStatus::Lossy => origin = j,
            HopStatus::Silent => {}
            _ => break,
        }
    }
    rows[origin].0.loss_origin = true;
}

/// Grids kept per target: one per bucket width in use, the oldest unused one goes first.
const MAX_GRIDS: usize = 4;

/// What a target found over the last day, in memory: the rounds, the start times of rounds that
/// missed the destination (for the day strip) and the grids built from them.
pub struct History {
    rounds: VecDeque<RoundRecord>,
    lost: VecDeque<i64>,
    grids: Vec<Grid>,
    /// A window of the past, built once and not moved by new rounds.
    past: Option<(i64, i64, usize, Grid)>,
    keep_ms: i64,
    /// Every round from here on is in memory.
    complete_from: i64,
}

impl History {
    /// `complete_from`: rounds from this time on will all come through `push`.
    pub fn new(keep_ms: i64, complete_from: i64) -> Self {
        Self {
            rounds: VecDeque::new(),
            lost: VecDeque::new(),
            grids: Vec::new(),
            past: None,
            keep_ms,
            complete_from,
        }
    }

    pub fn complete_from(&self) -> i64 {
        self.complete_from
    }

    pub fn last(&self) -> Option<&RoundRecord> {
        self.rounds.back()
    }

    pub fn len(&self) -> usize {
        self.rounds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rounds.is_empty()
    }

    /// Adds a new round and drops what is older than the kept span.
    pub fn push(&mut self, r: RoundRecord) {
        for g in &mut self.grids {
            g.add(&r);
        }
        if !r.reached {
            let i = self.lost.partition_point(|&t| t <= r.ts_ms);
            self.lost.insert(i, r.ts_ms);
        }
        let cutoff = r.ts_ms - self.keep_ms;
        // Rounds finish out of order when they overlap; keep them sorted by start time.
        let i = self.rounds.len()
            - self
                .rounds
                .iter()
                .rev()
                .take_while(|x| x.ts_ms > r.ts_ms)
                .count();
        self.rounds.insert(i, r);
        while self.rounds.front().is_some_and(|x| x.ts_ms < cutoff) {
            self.rounds.pop_front();
        }
        while self.lost.front().is_some_and(|&t| t < cutoff) {
            self.lost.pop_front();
        }
        self.complete_from = self.complete_from.max(cutoff);
    }

    /// Adds rounds older than everything in memory (history loaded from disk at start).
    pub fn prepend(&mut self, older: Vec<RoundRecord>, complete_from: i64) {
        let first = self.rounds.front().map_or(i64::MAX, |r| r.ts_ms);
        let older: Vec<RoundRecord> = older.into_iter().filter(|r| r.ts_ms < first).collect();
        let first_lost = self.lost.front().copied().unwrap_or(i64::MAX);
        let lost: Vec<i64> = older
            .iter()
            .filter(|r| !r.reached && r.ts_ms < first_lost)
            .map(|r| r.ts_ms)
            .collect();
        for t in lost.into_iter().rev() {
            self.lost.push_front(t);
        }
        for r in older.into_iter().rev() {
            self.rounds.push_front(r);
        }
        self.complete_from = self.complete_from.min(complete_from);
        self.grids.clear();
        self.past = None;
    }

    /// Rounds with `from_ms <= ts_ms < to_ms`.
    pub fn rounds_between(&mut self, from_ms: i64, to_ms: i64) -> &[RoundRecord] {
        let all = self.rounds.make_contiguous();
        let a = all.partition_point(|r| r.ts_ms < from_ms);
        let b = all.partition_point(|r| r.ts_ms < to_ms);
        &all[a..b.max(a)]
    }

    /// Start times of rounds that missed the destination, `from_ms <= t < to_ms`.
    pub fn lost_between(&self, from_ms: i64, to_ms: i64) -> impl Iterator<Item = i64> + '_ {
        let a = self.lost.partition_point(|&t| t < from_ms);
        self.lost
            .range(a..)
            .copied()
            .take_while(move |&t| t < to_ms)
    }

    /// Who answered at `ttl` most recently, looking back at most `look_back` rounds.
    pub fn addr_at(&self, ttl: u8, look_back: usize) -> Option<IpAddr> {
        self.rounds.iter().rev().take(look_back).find_map(|r| {
            r.hop(ttl)
                .filter(|h| h.kind.answered())
                .and_then(|h| h.addr)
        })
    }

    /// The latest buckets, kept up to date by new rounds: `n` buckets of width `b` from bucket
    /// `from_k`. Built from what is in memory, so right after start, while the last day is still
    /// being read from disk, it may miss older rounds; `prepend` then drops it.
    pub fn live_window(&mut self, b: i64, from_k: i64, n: usize, now: i64) -> Window<'_> {
        let i = match self
            .grids
            .iter()
            .position(|g| g.b == b && g.covers(from_k, n))
        {
            Some(i) => i,
            None => {
                self.grids.retain(|g| g.b != b);
                if self.grids.len() >= MAX_GRIDS {
                    let oldest = (0..self.grids.len())
                        .min_by_key(|&i| self.grids[i].used_ms)
                        .unwrap_or(0);
                    self.grids.remove(oldest);
                }
                let from_ms = from_k * b;
                let start = self.rounds.partition_point(|r| r.ts_ms < from_ms);
                let grid = Grid::build(self.rounds.range(start..), b, from_k, n + 2);
                self.grids.push(grid);
                self.grids.len() - 1
            }
        };
        let g = &mut self.grids[i];
        g.used_ms = now;
        g.window(from_k, n)
    }

    /// Buckets of the past: built from memory once per window and kept until another one is
    /// asked for. `None` when the window starts before the rounds in memory.
    pub fn past_window(&mut self, b: i64, from_k: i64, n: usize) -> Option<Window<'_>> {
        if from_k.saturating_mul(b) < self.complete_from {
            return None;
        }
        let fresh =
            !matches!(&self.past, Some((pb, pk, pn, _)) if *pb == b && *pk == from_k && *pn == n);
        if fresh {
            let from_ms = from_k * b;
            let to_ms = (from_k + n as i64) * b;
            let all = self.rounds.make_contiguous();
            let a = all.partition_point(|r| r.ts_ms < from_ms);
            let z = all.partition_point(|r| r.ts_ms < to_ms);
            let grid = Grid::build(&all[a..z.max(a)], b, from_k, n);
            self.past = Some((b, from_k, n, grid));
        }
        self.past.as_ref().map(|(_, _, _, g)| g.window(from_k, n))
    }

    /// Keeps a past window read from disk, so the next snapshot of it is quick.
    pub fn keep_past(&mut self, b: i64, from_k: i64, n: usize, grid: Grid) {
        self.past = Some((b, from_k, n, grid));
    }

    /// A past window kept by `keep_past` or `past_window`, without building one.
    pub fn kept_past(&self, b: i64, from_k: i64, n: usize) -> Option<Window<'_>> {
        match &self.past {
            Some((pb, pk, pn, g)) if *pb == b && *pk == from_k && *pn == n => {
                Some(g.window(from_k, n))
            }
            _ => None,
        }
    }
}
