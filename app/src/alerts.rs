//! Alerts about a target, worked out by the core from the rounds in memory, so they come while the
//! window is closed: the destination stopped answering for a minute, or the loss to it over the
//! last two minutes reached the threshold. Only the destination counts: a router on the way that
//! limits its own answers is not a fault. Rounds in which no probe could be sent (no network on
//! this computer) count nowhere, and a target that has never answered raises nothing.
//!
//! An outage gives one alert, "not answering", and one more when it is over: while the latest
//! rounds all miss the destination, loss is not reported (it may be the start of an outage), and
//! after an outage the loss is judged over the rounds since it ended.

use std::net::IpAddr;
use whilo_trace_engine::{hop_stats_toward, not_sent, reached_dest_ttl, verdict, RoundRecord};

/// The destination has not answered for this long, in at least `DOWN_ROUNDS` rounds in a row.
pub const DOWN_AFTER_MS: i64 = 60_000;
pub const DOWN_ROUNDS: usize = 3;
/// Loss is judged over this span, and over at least `LOSS_ROUNDS` rounds when the span holds fewer.
pub const LOSS_SPAN_MS: i64 = 120_000;
pub const LOSS_ROUNDS: usize = 20;
/// How far back the alerts look for rounds: `LOSS_ROUNDS` at the longest interval, 60 s.
pub const LOOK_BACK_MS: i64 = 25 * 60_000;
/// Loss thresholds the settings offer, %.
pub const THRESHOLDS: [u32; 3] = [5, 10, 20];

/// A hop: its TTL and who answered there.
pub type Hop = (u8, Option<IpAddr>);

#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    /// Too few rounds to say anything, the destination never answered, or the latest rounds all
    /// missed it for less than a minute (an outage may be starting).
    Unknown,
    Fine {
        loss_pct: f64,
    },
    /// The destination has not answered for `for_ms`; `last`: the deepest hop that answered in
    /// the latest round.
    Down {
        for_ms: i64,
        last: Option<Hop>,
    },
    /// `pct` of the rounds of the last `span_ms` missed the destination; `origin`: where the loss
    /// starts.
    Loss {
        pct: f64,
        span_ms: i64,
        origin: Option<Hop>,
    },
}

/// What the rounds of a target say at `now_ms`. `rounds`: oldest first, up to `LOOK_BACK_MS`
/// back. `answered_before`: the destination answered at some point, so its silence is news.
/// `loss_from_ms`: loss counts only rounds from here on (the end of the last outage).
pub fn assess(
    rounds: &[RoundRecord],
    now_ms: i64,
    threshold_pct: f64,
    answered_before: bool,
    loss_from_ms: i64,
) -> Condition {
    if !answered_before {
        return Condition::Unknown;
    }
    let measured: Vec<&RoundRecord> = rounds.iter().filter(|r| !not_sent(r)).collect();
    // The latest rounds all missed the destination: down after a minute and 3 rounds, and until
    // then nothing, so an outage does not first read as loss.
    let streak = measured.iter().rev().take_while(|r| !r.reached).count();
    if streak >= DOWN_ROUNDS {
        let first = measured[measured.len() - streak].ts_ms;
        if now_ms - first < DOWN_AFTER_MS {
            return Condition::Unknown;
        }
        let last = measured.last().and_then(|r| {
            r.hops
                .iter()
                .rev()
                .find(|h| h.kind.answered())
                .map(|h| (h.ttl, h.addr))
        });
        return Condition::Down {
            for_ms: now_ms - first,
            last,
        };
    }
    // Loss over the last two minutes, or over the last 20 rounds when two minutes hold fewer.
    let since: Vec<&RoundRecord> = measured
        .iter()
        .copied()
        .filter(|r| r.ts_ms >= loss_from_ms)
        .collect();
    let recent: Vec<&RoundRecord> = since
        .iter()
        .copied()
        .filter(|r| r.ts_ms >= now_ms - LOSS_SPAN_MS)
        .collect();
    let window: Vec<&RoundRecord> = if recent.len() >= LOSS_ROUNDS {
        recent
    } else if since.len() >= LOSS_ROUNDS {
        since[since.len() - LOSS_ROUNDS..].to_vec()
    } else {
        return Condition::Unknown;
    };
    let lost = window.iter().filter(|r| !r.reached).count();
    let pct = 100.0 * lost as f64 / window.len() as f64;
    if lost == 0 || pct + 1e-9 < threshold_pct {
        return Condition::Fine { loss_pct: pct };
    }
    // Where it starts: the engine's own reading of these rounds.
    let owned: Vec<RoundRecord> = window.iter().map(|r| (*r).clone()).collect();
    let toward = reached_dest_ttl(&owned).or_else(|| reached_dest_ttl(rounds));
    let hops = hop_stats_toward(&owned, toward);
    let v = verdict(&owned, &hops);
    let origin = v
        .origin_ttl
        .filter(|t| v.dest_ttl != Some(*t))
        .map(|t| (t, v.origin_addr));
    Condition::Loss {
        pct,
        span_ms: now_ms - window[0].ts_ms,
        origin,
    }
}

/// An alert that is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Active {
    Down,
    Loss,
}

/// What changed for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    WentDown,
    Lossy,
    /// The destination answers again after an outage.
    Answers,
    /// The loss went below half the threshold.
    LossGone,
}

/// The next state of a target's alert and what to tell, from its condition now. Loss clears only
/// below half the threshold, so a loss near it does not flap. After an outage the alert clears:
/// the loss since then is judged anew (`assess` with the end of the outage as `loss_from_ms`).
pub fn step(
    active: Option<Active>,
    cond: &Condition,
    threshold_pct: f64,
) -> (Option<Active>, Option<Change>) {
    match (active, cond) {
        (a, Condition::Unknown) => (a, None),
        (Some(Active::Down), Condition::Down { .. }) => (Some(Active::Down), None),
        (_, Condition::Down { .. }) => (Some(Active::Down), Some(Change::WentDown)),
        (Some(Active::Down), _) => (None, Some(Change::Answers)),
        (Some(Active::Loss), Condition::Loss { .. }) => (Some(Active::Loss), None),
        (None, Condition::Loss { .. }) => (Some(Active::Loss), Some(Change::Lossy)),
        (Some(Active::Loss), Condition::Fine { loss_pct }) if *loss_pct < threshold_pct / 2.0 => {
            (None, Some(Change::LossGone))
        }
        (Some(Active::Loss), Condition::Fine { .. }) => (Some(Active::Loss), None),
        (None, Condition::Fine { .. }) => (None, None),
    }
}

/// "6 (10.0.0.6)": the TTL of a hop and who answered there.
fn hop_at(h: &Hop) -> String {
    match h.1 {
        Some(a) => format!("{} ({a})", h.0),
        None => h.0.to_string(),
    }
}

/// Title and text of the notification, in the interface language.
pub fn text(host: &str, change: Change, cond: &Condition, lang: &str) -> (String, String) {
    let en = lang == "en";
    let pair = |ru: String, en_: String| if en { en_ } else { ru };
    let mins = |ms: i64| ((ms + 30_000) / 60_000).max(1);
    match (change, cond) {
        (Change::WentDown, Condition::Down { for_ms, last }) => {
            let min = (for_ms / 60_000).max(1);
            let where_ = match last {
                Some(h) => pair(
                    format!(" Последним ответил хоп {}.", hop_at(h)),
                    format!(" The last to answer was hop {}.", hop_at(h)),
                ),
                None => pair(
                    " Не отвечает уже первый хоп: проблема рядом с вами.".into(),
                    " Not even the first hop answers: the trouble is close to you.".into(),
                ),
            };
            (
                pair(
                    format!("{host}: не отвечает"),
                    format!("{host}: not answering"),
                ),
                pair(
                    format!("Нет ответа {min} мин.{where_}"),
                    format!("No answer for {min} min.{where_}"),
                ),
            )
        }
        (
            Change::Lossy,
            Condition::Loss {
                pct,
                span_ms,
                origin,
            },
        ) => {
            let where_ = match origin {
                Some(h) => pair(
                    format!(" Начинаются на хопе {}.", hop_at(h)),
                    format!(" The loss starts at hop {}.", hop_at(h)),
                ),
                None => pair(
                    " Где начинаются, по хопам не видно.".into(),
                    " Where it starts cannot be told from the hops.".into(),
                ),
            };
            let m = mins(*span_ms);
            (
                pair(
                    format!("{host}: потери {pct:.0} %"),
                    format!("{host}: loss {pct:.0}%"),
                ),
                pair(
                    format!("Потери до цели {pct:.0} % за последние {m} мин.{where_}"),
                    format!(
                        "{pct:.0}% of the probes to it were lost over the last {m} min.{where_}"
                    ),
                ),
            )
        }
        (Change::Answers, _) => (
            pair(
                format!("{host}: снова отвечает"),
                format!("{host}: answers again"),
            ),
            pair(
                "Связь с целью восстановилась.".into(),
                "The connection to it is back.".into(),
            ),
        ),
        (Change::LossGone, _) => (
            pair(
                format!("{host}: потери прошли"),
                format!("{host}: loss is over"),
            ),
            pair(
                "Пробы до цели снова доходят.".into(),
                "The probes to it get through again.".into(),
            ),
        ),
        (Change::WentDown, _) | (Change::Lossy, _) => (
            pair(format!("{host}: сбой"), format!("{host}: trouble")),
            pair("У цели сбой.".into(), "The destination has trouble.".into()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use whilo_trace_engine::{HopSample, ProbeKind, ReplyKind};

    const LEN: u8 = 4;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, last))
    }

    /// A round at `t` s: it reaches the destination, or is lost from `lost_from` on.
    fn round(t: i64, lost_from: Option<u8>) -> RoundRecord {
        let path_len = lost_from.unwrap_or(LEN);
        let hops: Vec<HopSample> = (1..=path_len)
            .map(|ttl| {
                if lost_from.is_some_and(|f| ttl >= f) {
                    HopSample::lost(ttl, ReplyKind::Timeout)
                } else {
                    HopSample {
                        ttl,
                        kind: if ttl == LEN {
                            ReplyKind::Echo
                        } else {
                            ReplyKind::TimeExceeded
                        },
                        addr: Some(ip(ttl)),
                        rtt_us: Some(1000 * ttl as u32),
                    }
                }
            })
            .collect();
        RoundRecord {
            ts_ms: t * 1000,
            probe: ProbeKind::Icmp,
            dst: ip(LEN),
            path_len,
            reached: lost_from.is_none(),
            hops,
        }
    }

    fn offline(t: i64) -> RoundRecord {
        RoundRecord {
            ts_ms: t * 1000,
            probe: ProbeKind::Icmp,
            dst: ip(LEN),
            path_len: 1,
            reached: false,
            hops: vec![HopSample::lost(1, ReplyKind::Error)],
        }
    }

    /// The rounds that started by `s` seconds.
    fn upto(rs: &[RoundRecord], s: i64) -> Vec<RoundRecord> {
        rs.iter().filter(|r| r.ts_ms <= s * 1000).cloned().collect()
    }

    #[test]
    fn an_outage_is_down_after_a_minute_and_not_loss_before() {
        // Rounds every 5 s: fine until 100 s, then lost from hop 3 on.
        let rs: Vec<RoundRecord> = (0..40)
            .map(|k| round(k * 5, (k * 5 >= 100).then_some(3)))
            .collect();
        let at = |s: i64| assess(&upto(&rs, s), s * 1000, 10.0, true, 0);
        assert_eq!(
            at(140),
            Condition::Unknown,
            "40 s of silence: neither loss nor down yet"
        );
        match at(195) {
            Condition::Down { for_ms, last } => {
                assert_eq!(for_ms, 95_000);
                assert_eq!(last, Some((2, Some(ip(2)))), "hop 2 answered last");
            }
            c => panic!("{c:?}"),
        }
        assert_eq!(
            assess(&rs, 195_000, 10.0, false, 0),
            Condition::Unknown,
            "never answered: no alert"
        );
    }

    #[test]
    fn loss_over_two_minutes_names_where_it_starts() {
        // A round every 5 s for 2 minutes, every fifth one lost from hop 3 on: 20 %.
        let rs: Vec<RoundRecord> = (0..25)
            .map(|k| round(k * 5, (k % 5 == 4).then_some(3)))
            .collect();
        match assess(&rs, 120_000, 10.0, true, 0) {
            Condition::Loss {
                pct,
                span_ms,
                origin,
            } => {
                assert!((pct - 20.0).abs() < 1e-9, "{pct}");
                assert_eq!(span_ms, 120_000);
                assert_eq!(origin, Some((3, Some(ip(3)))));
            }
            c => panic!("{c:?}"),
        }
        assert!(matches!(
            assess(&rs, 120_000, 25.0, true, 0),
            Condition::Fine { .. }
        ));
    }

    #[test]
    fn rounds_without_a_network_raise_nothing() {
        let mut rs: Vec<RoundRecord> = (0..24).map(|k| round(k * 5, None)).collect();
        rs.extend((24..40).map(|k| offline(k * 5)));
        assert!(matches!(
            assess(&rs, 200_000, 10.0, true, 0),
            Condition::Fine { loss_pct } if loss_pct == 0.0
        ));
    }

    #[test]
    fn a_long_interval_takes_the_last_twenty_rounds() {
        // One round a minute: two minutes hold two of them, so the last twenty count.
        let rs: Vec<RoundRecord> = (0..20)
            .map(|k| round(k * 60, (k == 18 || k == 19).then_some(4)))
            .collect();
        match assess(&rs, 1_200_000, 10.0, true, 0) {
            Condition::Loss { pct, span_ms, .. } => assert_eq!((pct, span_ms), (10.0, 1_200_000)),
            c => panic!("{c:?}"),
        }
        assert_eq!(
            assess(&rs[..10], 600_000, 10.0, true, 0),
            Condition::Unknown,
            "ten rounds are too few"
        );
    }

    #[test]
    fn after_an_outage_the_loss_counts_from_its_end() {
        // Fine, then 90 s without the destination, then fine again from 150 s on.
        let rs: Vec<RoundRecord> = (0..60)
            .map(|k| round(k * 5, (60..150).contains(&(k * 5)).then_some(3)))
            .collect();
        let at = |s: i64, from: i64| assess(&upto(&rs, s), s * 1000, 10.0, true, from * 1000);
        assert!(
            matches!(at(200, 0), Condition::Loss { .. }),
            "without the boundary the outage reads as loss"
        );
        assert_eq!(
            at(200, 150),
            Condition::Unknown,
            "11 rounds since its end are too few"
        );
        assert!(matches!(at(250, 150), Condition::Fine { loss_pct } if loss_pct == 0.0));
    }

    #[test]
    fn alerts_come_once_and_clear_with_a_margin() {
        let loss = |p: f64| Condition::Loss {
            pct: p,
            span_ms: 120_000,
            origin: None,
        };
        let fine = |p: f64| Condition::Fine { loss_pct: p };
        let down = Condition::Down {
            for_ms: 60_000,
            last: None,
        };
        let s = |a, c: &Condition| step(a, c, 10.0);
        assert_eq!(
            s(None, &loss(15.0)),
            (Some(Active::Loss), Some(Change::Lossy))
        );
        assert_eq!(
            s(Some(Active::Loss), &loss(30.0)),
            (Some(Active::Loss), None)
        );
        assert_eq!(
            s(Some(Active::Loss), &fine(8.0)),
            (Some(Active::Loss), None),
            "under the threshold, not under half"
        );
        assert_eq!(
            s(Some(Active::Loss), &fine(4.0)),
            (None, Some(Change::LossGone))
        );
        assert_eq!(
            s(Some(Active::Loss), &down),
            (Some(Active::Down), Some(Change::WentDown))
        );
        assert_eq!(s(Some(Active::Down), &down), (Some(Active::Down), None));
        assert_eq!(
            s(Some(Active::Down), &loss(40.0)),
            (None, Some(Change::Answers))
        );
        assert_eq!(
            s(Some(Active::Down), &fine(0.0)),
            (None, Some(Change::Answers))
        );
        assert_eq!(
            s(Some(Active::Down), &Condition::Unknown),
            (Some(Active::Down), None)
        );
    }

    #[test]
    fn the_text_names_the_target_and_the_hop() {
        let down = Condition::Down {
            for_ms: 125_000,
            last: Some((6, Some(ip(6)))),
        };
        let (t, b) = text("example.com", Change::WentDown, &down, "ru");
        assert_eq!(t, "example.com: не отвечает");
        assert_eq!(b, "Нет ответа 2 мин. Последним ответил хоп 6 (10.0.0.6).");
        let loss = Condition::Loss {
            pct: 18.4,
            span_ms: 120_000,
            origin: Some((7, Some(ip(7)))),
        };
        let (t, b) = text("example.com", Change::Lossy, &loss, "ru");
        assert_eq!(t, "example.com: потери 18 %");
        assert_eq!(
            b,
            "Потери до цели 18 % за последние 2 мин. Начинаются на хопе 7 (10.0.0.7)."
        );
        let (t, b) = text("example.com", Change::Lossy, &loss, "en");
        assert_eq!(t, "example.com: loss 18%");
        assert_eq!(
            b,
            "18% of the probes to it were lost over the last 2 min. The loss starts at hop 7 (10.0.0.7)."
        );
    }
}
