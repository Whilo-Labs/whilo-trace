//! Per-hop statistics over a window of rounds, and the timeline series.

use crate::monitor::RoundRecord;
use serde::Serialize;
use std::collections::HashMap;
use std::net::IpAddr;

/// Loss below this share (in %) is treated as noise.
pub const LOSS_NOTABLE_PCT: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HopStatus {
    /// Answers, no notable loss.
    Ok,
    /// Never answers itself, while hops further down do: it only drops ICMP aimed at it.
    Silent,
    /// Loses answers, but later hops do not: the router limits its own ICMP replies.
    RateLimited,
    /// Loss that carries on down the path.
    Lossy,
    /// Neither this hop nor anything after it answers.
    NoReply,
}

#[derive(Debug, Clone, Serialize)]
pub struct HopStats {
    pub ttl: u8,
    /// Most frequent responder at this TTL.
    pub addr: Option<IpAddr>,
    /// How many other addresses answered at this TTL (route changes, load balancing).
    pub other_addrs: usize,
    pub sent: u32,
    pub recv: u32,
    pub loss_pct: f64,
    pub last_ms: Option<f64>,
    pub avg_ms: Option<f64>,
    pub min_ms: Option<f64>,
    pub max_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub status: HopStatus,
    pub is_destination: bool,
    /// First hop of the loss that reaches the destination.
    pub loss_origin: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub rounds: usize,
    pub dest_ttl: Option<u8>,
    pub dest_loss_pct: Option<f64>,
    pub dest_avg_ms: Option<f64>,
    pub origin_ttl: Option<u8>,
    pub origin_addr: Option<IpAddr>,
    pub silent_hops: Vec<u8>,
    pub rate_limited_hops: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Bucket {
    pub t_ms: i64,
    pub sent: u32,
    pub lost: u32,
    pub min_ms: Option<f64>,
    pub avg_ms: Option<f64>,
    pub max_ms: Option<f64>,
}

#[derive(Default)]
struct Acc {
    sent: u32,
    recv: u32,
    rtts: Vec<f64>,
    addrs: HashMap<IpAddr, u32>,
    last: Option<Option<f64>>,
}

/// Statistics per TTL over `rounds` (oldest first).
pub fn hop_stats(rounds: &[RoundRecord]) -> Vec<HopStats> {
    let Some(max_ttl) = rounds.iter().map(|r| r.path_len).max() else {
        return Vec::new();
    };
    let dest_ttl = most_common(rounds.iter().map(|r| r.path_len));

    let mut rows: Vec<HopStats> = (1..=max_ttl)
        .map(|ttl| {
            let mut acc = Acc::default();
            for r in rounds {
                let Some(h) = r.hop(ttl) else { continue };
                acc.sent += 1;
                if h.kind.answered() {
                    acc.recv += 1;
                    if let Some(a) = h.addr {
                        *acc.addrs.entry(a).or_default() += 1;
                    }
                    if let Some(ms) = h.rtt_ms() {
                        acc.rtts.push(ms);
                    }
                    acc.last = Some(h.rtt_ms());
                } else {
                    acc.last = Some(None);
                }
            }
            finish(ttl, acc, Some(ttl) == dest_ttl)
        })
        .collect();
    classify(&mut rows);
    rows
}

fn finish(ttl: u8, acc: Acc, is_destination: bool) -> HopStats {
    let mut addrs: Vec<(IpAddr, u32)> = acc.addrs.into_iter().collect();
    addrs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let n = acc.rtts.len();
    let avg = (n > 0).then(|| acc.rtts.iter().sum::<f64>() / n as f64);
    let jitter = (n > 1).then(|| {
        acc.rtts
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .sum::<f64>()
            / (n - 1) as f64
    });
    HopStats {
        ttl,
        addr: addrs.first().map(|a| a.0),
        other_addrs: addrs.len().saturating_sub(1),
        sent: acc.sent,
        recv: acc.recv,
        loss_pct: if acc.sent == 0 {
            0.0
        } else {
            100.0 * (acc.sent - acc.recv) as f64 / acc.sent as f64
        },
        last_ms: acc.last.flatten(),
        avg_ms: avg,
        min_ms: acc.rtts.iter().copied().reduce(f64::min),
        max_ms: acc.rtts.iter().copied().reduce(f64::max),
        jitter_ms: jitter,
        status: HopStatus::Ok,
        is_destination,
        loss_origin: false,
    }
}

/// Tells hops that only drop ICMP aimed at themselves apart from real loss on the path.
/// Loss at a hop matters only if it carries on to the hops after it.
fn classify(rows: &mut [HopStats]) {
    let statuses: Vec<HopStatus> = (0..rows.len())
        .map(|i| {
            let row = &rows[i];
            let later = &rows[i + 1..];
            let later_alive = later.iter().any(|r| r.recv > 0);
            let later_min_loss = later
                .iter()
                .filter(|r| r.recv > 0)
                .map(|r| r.loss_pct)
                .reduce(f64::min);
            if row.recv == 0 {
                if later_alive {
                    HopStatus::Silent
                } else {
                    HopStatus::NoReply
                }
            } else if row.loss_pct < LOSS_NOTABLE_PCT {
                HopStatus::Ok
            } else {
                match later_min_loss {
                    Some(down) if down + margin(row.loss_pct) < row.loss_pct => {
                        HopStatus::RateLimited
                    }
                    _ => HopStatus::Lossy,
                }
            }
        })
        .collect();
    for (row, status) in rows.iter_mut().zip(statuses) {
        row.status = status;
    }

    // Walk back from the destination while the loss continues; silent hops tell nothing.
    let Some(dest) = rows.iter().rposition(|r| r.is_destination) else {
        return;
    };
    if rows[dest].status != HopStatus::Lossy {
        return;
    }
    let mut origin = dest;
    for j in (0..dest).rev() {
        match rows[j].status {
            HopStatus::Lossy => origin = j,
            HopStatus::Silent => {}
            _ => break,
        }
    }
    rows[origin].loss_origin = true;
}

fn margin(loss_pct: f64) -> f64 {
    (loss_pct * 0.5).max(LOSS_NOTABLE_PCT)
}

pub fn verdict(rounds: &[RoundRecord], hops: &[HopStats]) -> Verdict {
    let dest = hops.iter().find(|h| h.is_destination);
    let origin = hops.iter().find(|h| h.loss_origin);
    Verdict {
        rounds: rounds.len(),
        dest_ttl: dest.map(|d| d.ttl),
        dest_loss_pct: dest.map(|d| d.loss_pct),
        dest_avg_ms: dest.and_then(|d| d.avg_ms),
        origin_ttl: origin.map(|o| o.ttl),
        origin_addr: origin.and_then(|o| o.addr),
        silent_hops: hops
            .iter()
            .filter(|h| h.status == HopStatus::Silent)
            .map(|h| h.ttl)
            .collect(),
        rate_limited_hops: hops
            .iter()
            .filter(|h| h.status == HopStatus::RateLimited)
            .map(|h| h.ttl)
            .collect(),
    }
}

/// Latency and loss in `buckets` equal slices of `[from_ms, to_ms)`.
/// `ttl == None` follows the destination, whatever its TTL in each round.
pub fn timeline(
    rounds: &[RoundRecord],
    ttl: Option<u8>,
    from_ms: i64,
    to_ms: i64,
    buckets: usize,
) -> Vec<Bucket> {
    let buckets = buckets.max(1);
    let span = (to_ms - from_ms).max(1);
    let mut acc: Vec<(u32, u32, Vec<f64>)> = vec![(0, 0, Vec::new()); buckets];
    for r in rounds {
        if r.ts_ms < from_ms || r.ts_ms >= to_ms {
            continue;
        }
        let i = (((r.ts_ms - from_ms) as i128 * buckets as i128) / span as i128) as usize;
        let slot = &mut acc[i.min(buckets - 1)];
        let sample = match ttl {
            None => Some(r.destination()),
            Some(t) => r.hop(t).map(|h| h.kind.answered().then_some(h)),
        };
        match sample {
            None => {}
            Some(None) => {
                slot.0 += 1;
                slot.1 += 1;
            }
            Some(Some(h)) => {
                slot.0 += 1;
                if let Some(ms) = h.rtt_ms() {
                    slot.2.push(ms);
                }
            }
        }
    }
    acc.into_iter()
        .enumerate()
        .map(|(i, (sent, lost, rtts))| Bucket {
            t_ms: from_ms + (span as i128 * i as i128 / buckets as i128) as i64,
            sent,
            lost,
            min_ms: rtts.iter().copied().reduce(f64::min),
            avg_ms: (!rtts.is_empty()).then(|| rtts.iter().sum::<f64>() / rtts.len() as f64),
            max_ms: rtts.iter().copied().reduce(f64::max),
        })
        .collect()
}

/// What one round says about one row of the interface: a target (`ttl == None`: its
/// destination, whatever its TTL in that round) or one hop of it. The rules are the
/// "Bucket rules" of `docs/ui-contract.md`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    /// Answered; the RTT in ms when known.
    Reply(Option<f64>),
    /// No answer, and the destination was lost in that round too: loss on the path.
    Lost,
    /// A hop did not answer while the destination did. Never for a target.
    NoReply,
}

/// `None` when the round did not probe that hop (the path was shorter then).
pub fn outcome(round: &RoundRecord, ttl: Option<u8>) -> Option<Outcome> {
    let Some(ttl) = ttl else {
        return Some(match round.destination() {
            Some(h) => Outcome::Reply(h.rtt_ms()),
            None => Outcome::Lost,
        });
    };
    let h = round.hop(ttl)?;
    Some(if h.kind.answered() {
        Outcome::Reply(h.rtt_ms())
    } else if round.reached {
        Outcome::NoReply
    } else {
        Outcome::Lost
    })
}

/// One bucket of a row: what the rounds that started in it say.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Slot {
    /// Rounds that probed the row.
    pub sent: u32,
    /// Rounds with loss on the path.
    pub lost: u32,
    /// Rounds where the hop did not answer while the destination did.
    pub no_reply: u32,
    pub min_ms: Option<f64>,
    pub avg_ms: Option<f64>,
    pub max_ms: Option<f64>,
}

/// Rounds of `[from_ms, to_ms)` in `n` equal buckets, each in the bucket of its start.
/// Buckets without rounds (a pause, the time before the first round) stay empty: no
/// latency and no loss.
pub fn slots(
    rounds: &[RoundRecord],
    ttl: Option<u8>,
    from_ms: i64,
    to_ms: i64,
    n: usize,
) -> Vec<Slot> {
    let n = n.max(1);
    let span = (to_ms - from_ms).max(1);
    let mut acc: Vec<(Slot, f64, u32)> = vec![(Slot::default(), 0.0, 0); n];
    for r in rounds {
        if r.ts_ms < from_ms || r.ts_ms >= to_ms {
            continue;
        }
        let Some(outcome) = outcome(r, ttl) else {
            continue;
        };
        let i = (((r.ts_ms - from_ms) as i128 * n as i128) / span as i128) as usize;
        let (slot, sum, count) = &mut acc[i.min(n - 1)];
        slot.sent += 1;
        match outcome {
            Outcome::Reply(Some(ms)) => {
                *sum += ms;
                *count += 1;
                slot.min_ms = Some(slot.min_ms.map_or(ms, |m| m.min(ms)));
                slot.max_ms = Some(slot.max_ms.map_or(ms, |m| m.max(ms)));
            }
            Outcome::Reply(None) => {}
            Outcome::Lost => slot.lost += 1,
            Outcome::NoReply => slot.no_reply += 1,
        }
    }
    acc.into_iter()
        .map(|(mut slot, sum, count)| {
            slot.avg_ms = (count > 0).then(|| sum / count as f64);
            slot
        })
        .collect()
}

/// A row as the timeline draws it: `series`, `lo`, `hi`, `loss` and `partial` of a row in
/// the interface contract.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Lane {
    /// Average RTT per bucket; `None` where nothing answered, which breaks the line.
    pub series: Vec<Option<f64>>,
    pub lo: Vec<Option<f64>>,
    pub hi: Vec<Option<f64>>,
    /// Buckets with loss on the path (red).
    pub loss: Vec<u32>,
    /// Buckets where a hop missed answers while the destination answered (amber). A bucket
    /// with loss on the path is only in `loss`. The browser mock draws a silent hop without
    /// either mark.
    pub partial: Vec<u32>,
}

pub fn lane(slots: &[Slot]) -> Lane {
    let mut lane = Lane::default();
    for (i, s) in slots.iter().enumerate() {
        lane.series.push(s.avg_ms);
        lane.lo.push(s.min_ms);
        lane.hi.push(s.max_ms);
        if s.lost > 0 {
            lane.loss.push(i as u32);
        } else if s.no_reply > 0 {
            lane.partial.push(i as u32);
        }
    }
    lane
}

/// Numbers of a row over a period: `stats` and `rangeStats` of a row in the interface contract.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct RowStats {
    /// Rounds with loss on the path, % of `sent`.
    pub loss_pct: f64,
    /// Rounds where the hop did not answer while the destination did, % of `sent`.
    pub no_reply_pct: f64,
    pub avg: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// RTT of the last reply.
    pub last: Option<f64>,
    /// The last round got no answer.
    pub last_lost: bool,
    /// Mean change between consecutive replies, ms.
    pub jitter: Option<f64>,
    /// Rounds that probed the row.
    pub sent: u32,
}

/// Over the rounds that started in `[from_ms, to_ms)`; `rounds` oldest first.
pub fn row_stats(rounds: &[RoundRecord], ttl: Option<u8>, from_ms: i64, to_ms: i64) -> RowStats {
    let mut s = RowStats::default();
    let (mut lost, mut no_reply) = (0u32, 0u32);
    let mut rtts: Vec<f64> = Vec::new();
    for r in rounds {
        if r.ts_ms < from_ms || r.ts_ms >= to_ms {
            continue;
        }
        let Some(outcome) = outcome(r, ttl) else {
            continue;
        };
        s.sent += 1;
        s.last_lost = !matches!(outcome, Outcome::Reply(_));
        match outcome {
            Outcome::Reply(Some(ms)) => {
                rtts.push(ms);
                s.last = Some(ms);
            }
            Outcome::Reply(None) => {}
            Outcome::Lost => lost += 1,
            Outcome::NoReply => no_reply += 1,
        }
    }
    if s.sent > 0 {
        s.loss_pct = 100.0 * lost as f64 / s.sent as f64;
        s.no_reply_pct = 100.0 * no_reply as f64 / s.sent as f64;
    }
    let n = rtts.len();
    s.avg = (n > 0).then(|| rtts.iter().sum::<f64>() / n as f64);
    s.min = rtts.iter().copied().reduce(f64::min);
    s.max = rtts.iter().copied().reduce(f64::max);
    s.jitter =
        (n > 1).then(|| rtts.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (n - 1) as f64);
    s
}

fn most_common(values: impl Iterator<Item = u8>) -> Option<u8> {
    let mut counts: HashMap<u8, usize> = HashMap::new();
    for v in values {
        *counts.entry(v).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(v, _)| v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::ProbeKind;
    use crate::probe::{HopSample, ReplyKind};
    use std::net::Ipv4Addr;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, last))
    }

    /// Builds rounds from a pattern per hop: '+' answered, '.' lost.
    fn rounds(pattern: &[&str]) -> Vec<RoundRecord> {
        let n = pattern[0].len();
        let path_len = pattern.len() as u8;
        (0..n)
            .map(|k| {
                let hops: Vec<HopSample> = pattern
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let ttl = i as u8 + 1;
                        if p.as_bytes()[k] == b'+' {
                            let kind = if ttl == path_len {
                                ReplyKind::Echo
                            } else {
                                ReplyKind::TimeExceeded
                            };
                            HopSample {
                                ttl,
                                kind,
                                addr: Some(ip(ttl)),
                                rtt_us: Some(1000 * ttl as u32),
                            }
                        } else {
                            HopSample::lost(ttl, ReplyKind::Timeout)
                        }
                    })
                    .collect();
                let reached = hops.last().unwrap().kind == ReplyKind::Echo;
                RoundRecord {
                    ts_ms: k as i64 * 1000,
                    probe: ProbeKind::Icmp,
                    dst: ip(path_len),
                    path_len,
                    reached,
                    hops,
                }
            })
            .collect()
    }

    #[test]
    fn silent_and_rate_limited_hops_are_not_loss() {
        // Like a typical home line: hop 1 answers now and then, 2-3 never, the rest fine.
        let r = rounds(&[
            "+..+..+..+",
            "..........",
            "..........",
            "++++++++++",
            "++++++++++",
        ]);
        let h = hop_stats(&r);
        assert_eq!(h[0].status, HopStatus::RateLimited);
        assert_eq!(h[1].status, HopStatus::Silent);
        assert_eq!(h[2].status, HopStatus::Silent);
        assert_eq!(h[3].status, HopStatus::Ok);
        assert!(h[4].is_destination);
        assert!(h.iter().all(|x| !x.loss_origin));
    }

    #[test]
    fn loss_that_carries_on_points_at_its_first_hop() {
        let r = rounds(&[
            "++++++++++",
            "..........",
            "+++.+++.++",
            "++.+++.+++",
            "+++.+++.++",
        ]);
        let h = hop_stats(&r);
        assert_eq!(h[2].status, HopStatus::Lossy);
        assert_eq!(h[4].status, HopStatus::Lossy);
        assert!(h[2].loss_origin, "{h:?}");
        let v = verdict(&r, &h);
        assert_eq!(v.origin_ttl, Some(3));
        assert_eq!(v.silent_hops, vec![2]);
    }

    #[test]
    fn timeline_counts_destination_loss() {
        let r = rounds(&["++++", "+.+."]);
        let t = timeline(&r, None, 0, 4000, 2);
        assert_eq!((t[0].sent, t[0].lost), (2, 1));
        assert_eq!((t[1].sent, t[1].lost), (2, 1));
        assert_eq!(t[0].avg_ms, Some(2.0));
    }

    /// Six rounds, one second apart, three hops; the destination is hop 3.
    /// Round 1: hop 1 misses. Round 2: hop 2 misses. Round 3: the destination is lost.
    /// Round 4: hop 2 and the destination are lost.
    fn three_hops() -> Vec<RoundRecord> {
        rounds(&["+.++++", "++.+.+", "+++..+"])
    }

    #[test]
    fn target_bucket_is_loss_when_a_round_missed_the_destination() {
        let r = three_hops();
        let s = slots(&r, None, 0, 6000, 3);
        let sent_lost: Vec<(u32, u32, u32)> =
            s.iter().map(|s| (s.sent, s.lost, s.no_reply)).collect();
        assert_eq!(sent_lost, vec![(2, 0, 0), (2, 1, 0), (2, 1, 0)]);
        let l = lane(&s);
        assert_eq!(l.loss, vec![1, 2]);
        assert!(l.partial.is_empty());
        assert_eq!(l.series, vec![Some(3.0); 3]);
    }

    #[test]
    fn hop_bucket_is_red_only_for_loss_on_the_path() {
        let r = three_hops();
        // Hop 1 misses while the destination answers: amber.
        let l1 = lane(&slots(&r, Some(1), 0, 6000, 3));
        assert_eq!((l1.loss.clone(), l1.partial.clone()), (vec![], vec![0]));
        // Hop 2 misses once with the destination up (amber), once with it lost (red).
        let l2 = lane(&slots(&r, Some(2), 0, 6000, 3));
        assert_eq!((l2.loss.clone(), l2.partial.clone()), (vec![2], vec![1]));
        assert_eq!(l2.series, vec![Some(2.0); 3]);
        // The destination as a hop: red where it was lost.
        let l3 = lane(&slots(&r, Some(3), 0, 6000, 3));
        assert_eq!((l3.loss, l3.partial), (vec![1, 2], vec![]));
        // Red wins over amber in one bucket.
        let s = vec![
            Slot {
                sent: 2,
                lost: 1,
                no_reply: 1,
                ..Slot::default()
            },
            Slot::default(),
        ];
        let l = lane(&s);
        assert_eq!((l.loss, l.partial), (vec![0], vec![]));
    }

    #[test]
    fn rounds_fall_into_the_bucket_of_their_start() {
        let r = three_hops();
        // Rounds at 0..5 s; the window [1 s, 4 s) holds rounds 1, 2 and 3.
        let s = slots(&r, None, 1000, 4000, 3);
        assert_eq!(s.iter().map(|s| s.sent).collect::<Vec<_>>(), vec![1, 1, 1]);
        assert_eq!(lane(&s).loss, vec![2]);
        // A pause: no rounds after 2 s, so the later buckets are empty, not loss.
        let s = slots(&r[..2], None, 0, 6000, 3);
        assert_eq!(s.iter().map(|s| s.sent).collect::<Vec<_>>(), vec![2, 0, 0]);
        let l = lane(&s);
        assert_eq!(l.series, vec![Some(3.0), None, None]);
        assert!(l.loss.is_empty() && l.partial.is_empty());
        // A hop deeper than the path of every round was never probed.
        assert!(slots(&r, Some(9), 0, 6000, 3).iter().all(|s| s.sent == 0));
    }

    #[test]
    fn row_stats_split_loss_on_the_path_from_missed_answers() {
        let mut r = three_hops();
        r[1].hops[2].rtt_us = Some(5000);
        let t = row_stats(&r, None, 0, 6000);
        assert_eq!(t.sent, 6);
        assert!((t.loss_pct - 100.0 / 3.0).abs() < 1e-9, "{t:?}");
        assert_eq!(t.no_reply_pct, 0.0);
        assert_eq!(
            (t.min, t.max, t.last, t.last_lost),
            (Some(3.0), Some(5.0), Some(3.0), false)
        );
        // Replies 3, 5, 3, 3: changes 2, 2, 0.
        assert!((t.jitter.unwrap() - 4.0 / 3.0).abs() < 1e-9, "{t:?}");
        assert!((t.avg.unwrap() - 3.5).abs() < 1e-9);

        let h = row_stats(&r, Some(2), 0, 6000);
        assert!((h.loss_pct - 100.0 / 6.0).abs() < 1e-9, "{h:?}");
        assert!((h.no_reply_pct - 100.0 / 6.0).abs() < 1e-9, "{h:?}");
        assert_eq!(h.avg, Some(2.0));

        // A period that ends with a lost round.
        let p = row_stats(&r, None, 0, 5000);
        assert_eq!((p.sent, p.last, p.last_lost), (5, Some(3.0), true));
        // Nothing in the period.
        let e = row_stats(&r, None, 10_000, 20_000);
        assert_eq!(e, RowStats::default());
    }
}
