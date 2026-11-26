//! "Before and after": the same target over two periods, hop by hop, as the trace counts them.
//! The history keeps every round, so there is nothing to save first: any two periods compare.

use crate::contract::{CompareHop, CompareRes, CompareSide};
use whilo_trace_engine::stats::LOSS_NOTABLE_PCT;
use whilo_trace_engine::{
    hop_stats_toward, reached_dest_ttl, verdict, HopStats, HopStatus, RoundRecord,
};

/// Latency is worse or better by at least this much, ms, and by this share of the first value.
const LATENCY_MS: f64 = 10.0;
const LATENCY_SHARE: f64 = 0.2;

/// Loss that counts on a hop: real loss on the path or at the destination, not a router that
/// limits its own answers.
fn path_loss(h: &HopStats) -> f64 {
    match h.status {
        HopStatus::Lossy | HopStatus::NoReply => h.loss_pct,
        _ if h.is_destination => h.loss_pct,
        _ => 0.0,
    }
}

fn side(rounds: &[RoundRecord]) -> (CompareSide, Vec<HopStats>) {
    let hops = hop_stats_toward(rounds, reached_dest_ttl(rounds));
    let v = verdict(rounds, &hops);
    (
        CompareSide {
            rounds: v.rounds as u32,
            loss_pct: v.dest_loss_pct,
            avg_ms: v.dest_avg_ms,
        },
        hops,
    )
}

/// `before` and `after`: the rounds of the two periods, oldest first.
pub fn compare(before: &[RoundRecord], after: &[RoundRecord]) -> CompareRes {
    let (a, ha) = side(before);
    let (b, hb) = side(after);
    let deepest = ha.len().max(hb.len());
    let hops = (0..deepest)
        .map(|i| {
            let (x, y) = (ha.get(i), hb.get(i));
            let ttl = x.or(y).map_or(i as u8 + 1, |h| h.ttl);
            let addr = |h: Option<&HopStats>| h.and_then(|h| h.addr).map(|a| a.to_string());
            let (la, lb) = (x.map(path_loss), y.map(path_loss));
            let (va, vb) = (x.and_then(|h| h.avg_ms), y.and_then(|h| h.avg_ms));
            let verdict = match (addr(x), addr(y)) {
                (None, Some(_)) if x.is_none_or(|h| h.recv == 0) => "new",
                (Some(_), None) if y.is_none_or(|h| h.recv == 0) => "gone",
                (Some(p), Some(q)) if p != q => "rerouted",
                _ => {
                    let dl = lb.unwrap_or(0.0) - la.unwrap_or(0.0);
                    let worse_loss = dl >= LOSS_NOTABLE_PCT.max(2.0);
                    let better_loss = -dl >= LOSS_NOTABLE_PCT.max(2.0);
                    let slower = |p: f64, q: f64| q - p >= LATENCY_MS.max(p * LATENCY_SHARE);
                    let (worse_rtt, better_rtt) = match (va, vb) {
                        (Some(p), Some(q)) => (slower(p, q), slower(q, p)),
                        _ => (false, false),
                    };
                    if worse_loss || (worse_rtt && !better_loss) {
                        "worse"
                    } else if better_loss || better_rtt {
                        "better"
                    } else {
                        "same"
                    }
                }
            };
            CompareHop {
                ttl,
                addr_a: addr(x),
                addr_b: addr(y),
                loss_a: la,
                loss_b: lb,
                avg_a: va,
                avg_b: vb,
                dest: y.or(x).is_some_and(|h| h.is_destination),
                verdict: verdict.into(),
            }
        })
        .collect();
    CompareRes {
        before: a,
        after: b,
        hops,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use whilo_trace_engine::{HopSample, ProbeKind, ReplyKind};

    fn ip(a: u8, b: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, a, b))
    }

    /// `n` rounds to a destination at TTL 4; every `lose_every`-th is lost from hop 3 on; hop 2 is
    /// router `r2`; hop 3 answers in `rtt3` ms.
    fn rounds(n: i64, lose_every: i64, r2: u8, rtt3: u32) -> Vec<RoundRecord> {
        (0..n)
            .map(|k| {
                let lost = lose_every > 0 && k % lose_every == 0;
                let len = if lost { 3 } else { 4 };
                RoundRecord {
                    ts_ms: k * 1000,
                    probe: ProbeKind::Icmp,
                    dst: ip(9, 9),
                    path_len: len,
                    reached: !lost,
                    hops: (1..=len)
                        .map(|ttl| {
                            if lost && ttl >= 3 {
                                return HopSample::lost(ttl, ReplyKind::Timeout);
                            }
                            HopSample {
                                ttl,
                                kind: if ttl == 4 {
                                    ReplyKind::Echo
                                } else {
                                    ReplyKind::TimeExceeded
                                },
                                addr: Some(match ttl {
                                    2 => ip(2, r2),
                                    4 => ip(9, 9),
                                    t => ip(t, 1),
                                }),
                                rtt_us: Some(if ttl >= 3 { rtt3 * 1000 } else { 1000 }),
                            }
                        })
                        .collect(),
                }
            })
            .collect()
    }

    #[test]
    fn a_fix_shows_as_better_and_a_new_router_as_rerouted() {
        // Before: every fifth round lost from hop 3 on, hop 3 at 60 ms. After: no loss, 30 ms, and
        // hop 2 is another router.
        let r = compare(&rounds(100, 5, 1, 60), &rounds(100, 0, 7, 30));
        assert_eq!(r.before.loss_pct, Some(20.0));
        assert_eq!(r.after.loss_pct, Some(0.0));
        let v: Vec<&str> = r.hops.iter().map(|h| h.verdict.as_str()).collect();
        assert_eq!(
            v,
            vec!["same", "rerouted", "better", "better"],
            "{:?}",
            r.hops
        );
        assert!(r.hops[3].dest);
    }

    #[test]
    fn the_same_path_is_the_same() {
        let r = compare(&rounds(50, 10, 1, 40), &rounds(50, 10, 1, 41));
        assert!(r.hops.iter().all(|h| h.verdict == "same"), "{:?}", r.hops);
    }
}
