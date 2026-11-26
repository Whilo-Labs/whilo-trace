//! Changes of the route over a run of rounds, read from the rounds themselves: the destination
//! moved to another address, a router on the way was replaced, or the destination answers at
//! another TTL. A load balancer that spreads rounds over two routers is not a change: a router
//! counts as replaced only when the routers of the last `SPAN` answers at its TTL share none with
//! the `SPAN` answers before them.

use crate::monitor::RoundRecord;
use crate::stats::not_sent;
use std::net::IpAddr;

/// Answers on each side of a change.
pub const SPAN: usize = 8;
/// Router changes this close together are one change of the route (it moved from the lowest TTL on).
const SAME_CHANGE_MS: i64 = 60_000;

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// The rounds go to another address: the name of the target resolves to it now.
    Address { from: IpAddr, to: IpAddr },
    /// The router answering at `ttl` was replaced (and maybe the ones after it).
    Router {
        ttl: u8,
        from: Option<IpAddr>,
        to: Option<IpAddr>,
    },
    /// The destination answers at another TTL.
    Length { from: u8, to: u8 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteChange {
    /// When the new state was first seen, ms since the epoch.
    pub ts_ms: i64,
    pub change: Change,
}

/// The most common value of `xs`.
fn most_common<T: PartialEq + Copy>(xs: &[T]) -> Option<T> {
    let mut best: Option<(T, usize)> = None;
    for &x in xs {
        let n = xs.iter().filter(|&&y| y == x).count();
        if best.is_none_or(|(_, m)| n > m) {
            best = Some((x, n));
        }
    }
    best.map(|(x, _)| x)
}

/// Where a sequence settled on other values: the indexes `i` at which the `SPAN` values before
/// `i` and the `SPAN` from `i` on share none.
fn shifts<T: PartialEq + Copy>(xs: &[T]) -> Vec<usize> {
    let mut out = Vec::new();
    if xs.len() < 2 * SPAN {
        return out;
    }
    for i in SPAN..=xs.len() - SPAN {
        let (before, after) = (&xs[i - SPAN..i], &xs[i..i + SPAN]);
        if before.iter().all(|b| !after.contains(b))
            && out.last().is_none_or(|&j: &usize| i >= j + SPAN)
        {
            // The first index of the new values: the last one before it is still old.
            out.push(i);
        }
    }
    out
}

/// The changes of the route over `rounds` (oldest first), oldest first.
pub fn route_changes(rounds: &[RoundRecord]) -> Vec<RouteChange> {
    let rounds: Vec<&RoundRecord> = rounds.iter().filter(|r| !not_sent(r)).collect();
    let mut out = Vec::new();
    // The destination address: a round to another address is a new one at once.
    for w in rounds.windows(2) {
        if w[0].dst != w[1].dst {
            out.push(RouteChange {
                ts_ms: w[1].ts_ms,
                change: Change::Address {
                    from: w[0].dst,
                    to: w[1].dst,
                },
            });
        }
    }
    // The TTL of the destination, over the rounds that reached it.
    let reached: Vec<&&RoundRecord> = rounds.iter().filter(|r| r.reached).collect();
    let lens: Vec<u8> = reached.iter().map(|r| r.path_len).collect();
    for i in shifts(&lens) {
        let (from, to) = (
            most_common(&lens[i - SPAN..i]),
            most_common(&lens[i..i + SPAN]),
        );
        if let (Some(from), Some(to)) = (from, to) {
            out.push(RouteChange {
                ts_ms: reached[i].ts_ms,
                change: Change::Length { from, to },
            });
        }
    }
    // Routers on the way: every TTL before the destination, over the rounds where it answered.
    let dest = most_common(&lens).unwrap_or(u8::MAX);
    let deepest = rounds.iter().map(|r| r.path_len).max().unwrap_or(0);
    let mut routers: Vec<RouteChange> = Vec::new();
    let last_dst = rounds.last().map(|l| l.dst);
    for ttl in 1..deepest.min(dest) {
        // Only the rounds to the current address: another address has a path of its own.
        let answers: Vec<(i64, IpAddr)> = rounds
            .iter()
            .filter(|r| Some(r.dst) == last_dst)
            .filter_map(|r| {
                r.hop(ttl)
                    .filter(|h| h.kind.answered())
                    .and_then(|h| h.addr)
                    .map(|a| (r.ts_ms, a))
            })
            .collect();
        let addrs: Vec<IpAddr> = answers.iter().map(|a| a.1).collect();
        for i in shifts(&addrs) {
            routers.push(RouteChange {
                ts_ms: answers[i].0,
                change: Change::Router {
                    ttl,
                    from: most_common(&addrs[i - SPAN..i]),
                    to: most_common(&addrs[i..i + SPAN]),
                },
            });
        }
    }
    // Changes at several TTLs at about the same time are one change of the route, from the lowest.
    routers.sort_by_key(|c| c.ts_ms);
    let mut merged: Vec<RouteChange> = Vec::new();
    for c in routers {
        match merged.last_mut() {
            Some(last) if c.ts_ms - last.ts_ms <= SAME_CHANGE_MS => {
                if let (Change::Router { ttl: a, .. }, Change::Router { ttl: b, .. }) =
                    (&last.change, &c.change)
                {
                    if b < a {
                        last.change = c.change;
                    }
                }
            }
            _ => merged.push(c),
        }
    }
    out.extend(merged);
    out.sort_by_key(|c| c.ts_ms);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::ProbeKind;
    use crate::probe::{HopSample, ReplyKind};
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, a, b))
    }

    /// A round at `k` s to `dst`, reaching it at TTL `len`; `at(ttl)` is who answers there.
    fn round(k: i64, dst: IpAddr, len: u8, at: impl Fn(u8) -> IpAddr) -> RoundRecord {
        RoundRecord {
            ts_ms: k * 1000,
            probe: ProbeKind::Icmp,
            dst,
            path_len: len,
            reached: true,
            hops: (1..=len)
                .map(|ttl| HopSample {
                    ttl,
                    kind: if ttl == len {
                        ReplyKind::Echo
                    } else {
                        ReplyKind::TimeExceeded
                    },
                    addr: Some(if ttl == len { dst } else { at(ttl) }),
                    rtt_us: Some(1000),
                })
                .collect(),
        }
    }

    #[test]
    fn a_replaced_router_is_one_change_and_a_load_balancer_none() {
        let dst = ip(9, 9);
        // 40 rounds: hop 2 alternates between two routers all along (a load balancer); from round
        // 20 on, hops 3 and 4 are other routers.
        let rs: Vec<RoundRecord> = (0..40)
            .map(|k| {
                round(k, dst, 5, |ttl| match ttl {
                    2 => ip(2, (k % 2) as u8),
                    t if k >= 20 && t >= 3 => ip(t, 100),
                    t => ip(t, 1),
                })
            })
            .collect();
        let c = route_changes(&rs);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0].ts_ms, 20_000);
        assert_eq!(
            c[0].change,
            Change::Router {
                ttl: 3,
                from: Some(ip(3, 1)),
                to: Some(ip(3, 100))
            }
        );
    }

    #[test]
    fn a_new_address_and_a_longer_path_are_changes() {
        let (a, b) = (ip(9, 9), ip(8, 8));
        let rs: Vec<RoundRecord> = (0..40)
            .map(|k| {
                if k < 20 {
                    round(k, a, 5, |t| ip(t, 1))
                } else {
                    round(k, b, 7, |t| ip(t, 1))
                }
            })
            .collect();
        let c = route_changes(&rs);
        assert!(c.contains(&RouteChange {
            ts_ms: 20_000,
            change: Change::Address { from: a, to: b }
        }));
        assert!(c.contains(&RouteChange {
            ts_ms: 20_000,
            change: Change::Length { from: 5, to: 7 }
        }));
    }

    #[test]
    fn a_short_detour_is_not_a_change() {
        // Hop 3 is another router for 4 rounds only: fewer than a span.
        let dst = ip(9, 9);
        let rs: Vec<RoundRecord> = (0..40)
            .map(|k| {
                round(k, dst, 5, |t| {
                    if t == 3 && (18..22).contains(&k) {
                        ip(3, 50)
                    } else {
                        ip(t, 1)
                    }
                })
            })
            .collect();
        assert!(route_changes(&rs).is_empty());
    }
}
