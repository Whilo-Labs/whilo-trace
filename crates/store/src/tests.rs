//! Tests of the store: migration of an old database, row order, settings, recent inputs,
//! and the bucket rules of the timelines.

use super::buckets::{Grid, History, LineStats, Pick};
use super::*;
use std::time::Instant;
use whilo_trace_engine::{hop_stats, lane, row_stats, slots, HopStatus};

fn ip(last: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(10, 0, 0, last))
}

/// Rounds from a pattern per hop, '+' answered and '.' lost; round k starts at `k * step` ms.
/// The last line is the destination.
fn rounds(pattern: &[&str], step: i64) -> Vec<RoundRecord> {
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
                            rtt_us: Some(1000 * ttl as u32 + 100 * (k as u32 % 7)),
                        }
                    } else {
                        HopSample::lost(ttl, ReplyKind::Timeout)
                    }
                })
                .collect();
            let reached = hops.last().unwrap().kind == ReplyKind::Echo;
            RoundRecord {
                ts_ms: k as i64 * step,
                probe: ProbeKind::Icmp,
                dst: ip(path_len),
                path_len,
                reached,
                hops,
            }
        })
        .collect()
}

/// A database file of its own for one test; removed when the test ends.
struct TempDb(PathBuf);

impl TempDb {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "whilo-trace-test-{}-{name}-{}.db",
            std::process::id(),
            now_ms()
        ));
        TempDb(p)
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        for ext in ["", "-wal", "-shm"] {
            let mut p = self.0.clone().into_os_string();
            p.push(ext);
            let _ = std::fs::remove_file(p);
        }
    }
}

fn memory() -> Store {
    Store::open(Path::new(":memory:")).unwrap()
}

/// The schema of the prototype (commit 0484d8f), as it was written to disk.
const OLD_SCHEMA: &str = "
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS targets (
    id         INTEGER PRIMARY KEY,
    host       TEXT    NOT NULL UNIQUE,
    active     INTEGER NOT NULL DEFAULT 1,
    created_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS rounds (
    id        INTEGER PRIMARY KEY,
    target_id INTEGER NOT NULL REFERENCES targets(id),
    ts_ms     INTEGER NOT NULL,
    dst       TEXT    NOT NULL,
    path_len  INTEGER NOT NULL,
    reached   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS rounds_by_target_time ON rounds(target_id, ts_ms);
CREATE TABLE IF NOT EXISTS hops (
    round_id INTEGER NOT NULL REFERENCES rounds(id),
    ttl      INTEGER NOT NULL,
    kind     INTEGER NOT NULL,
    addr     TEXT,
    rtt_us   INTEGER,
    PRIMARY KEY (round_id, ttl)
) WITHOUT ROWID;
";

#[test]
fn migrates_an_old_database_without_losing_history() {
    let db = TempDb::new("migrate");
    {
        let c = Connection::open(&db.0).unwrap();
        c.pragma_update(None, "journal_mode", "WAL").unwrap();
        c.execute_batch(OLD_SCHEMA).unwrap();
        c.execute_batch(
            "INSERT INTO targets (id, host, active, created_ms) VALUES (1, '1.1.1.1', 1, 100), (2, 'example.com', 0, 200);
             INSERT INTO rounds (id, target_id, ts_ms, dst, path_len, reached) VALUES
                 (1, 1, 1000, '1.1.1.1', 3, 1), (2, 1, 3500, '1.1.1.1', 3, 0), (3, 2, 1200, '93.184.216.34', 2, 1);
             INSERT INTO hops (round_id, ttl, kind, addr, rtt_us) VALUES
                 (1, 1, 1, '192.168.0.1', 600), (1, 2, 3, NULL, NULL), (1, 3, 0, '1.1.1.1', 2100),
                 (2, 1, 1, '192.168.0.1', 700), (2, 2, 3, NULL, NULL), (2, 3, 3, NULL, NULL),
                 (3, 1, 1, '192.168.0.1', 500), (3, 2, 0, '93.184.216.34', 30000);",
        )
        .unwrap();
        let v: i64 = c
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 0);
    }

    let mut s = Store::open(&db.0).unwrap();
    assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
    let t = s.targets().unwrap();
    assert_eq!(t.len(), 2);
    assert_eq!(
        (t[0].id, t[0].host.as_str(), t[0].active),
        (1, "1.1.1.1", true)
    );
    assert_eq!((t[1].id, t[1].active), (2, false));
    assert!(t
        .iter()
        .all(|t| t.interval_ms == DEFAULT_INTERVAL_MS && !t.paused && t.probe == "icmp"));
    assert!(t[0].position < t[1].position, "old order is kept");

    let r = s.load_rounds(1, 0, 10_000).unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].probe, ProbeKind::Icmp);
    assert!(r[0].reached && !r[1].reached);
    assert_eq!(r[0].hops.len(), 3);
    assert_eq!(r[0].hops[2].rtt_us, Some(2100));
    assert_eq!(r[0].hops[0].addr, Some("192.168.0.1".parse().unwrap()));
    assert_eq!(r[0].hops[1].kind, ReplyKind::Timeout);
    assert_eq!(s.load_rounds(2, 0, 10_000).unwrap().len(), 1);
    assert_eq!(s.lost_times(1, 0, 10_000).unwrap(), vec![3500]);

    // The new tables work and new rounds keep their probe type.
    let new = rounds(&["++", "++"], 1000);
    s.insert_round(1, &new[0]).unwrap();
    assert_eq!(s.load_rounds(1, 0, 10_000).unwrap().len(), 3);
    let pin = s.pin_hop(1, 1, "icmp", Some("192.168.0.1")).unwrap();
    assert_eq!(s.pins().unwrap()[0].id, pin);
    s.set_setting("lang", "\"en\"").unwrap();
    s.add_recent("1.1.1.1").unwrap();
    drop(s);

    // Opening again changes nothing.
    let s = Store::open(&db.0).unwrap();
    assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(s.load_rounds(1, 0, 10_000).unwrap().len(), 3);
    assert_eq!(s.pins().unwrap().len(), 1);
    assert_eq!(
        s.settings().unwrap(),
        vec![("lang".into(), "\"en\"".into())]
    );
    assert_eq!(s.recent().unwrap(), vec!["1.1.1.1".to_string()]);
}

#[test]
fn a_new_database_has_the_current_schema() {
    let s = memory();
    assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
    assert!(s.targets().unwrap().is_empty());
}

#[test]
fn targets_and_pinned_hops_share_one_order() {
    let s = memory();
    let a = s.add_target("1.1.1.1").unwrap();
    let b = s.add_target_with("8.8.8.8", "icmp", Some(1000)).unwrap();
    let c = s.add_target("example.com").unwrap();
    let p = s.pin_hop(a, 1, "icmp", Some("192.168.0.1")).unwrap();
    assert_eq!(
        s.pin_hop(a, 1, "icmp", None).unwrap(),
        p,
        "a hop is pinned once"
    );
    assert_eq!(
        s.add_target("EXAMPLE.com").unwrap(),
        c,
        "hosts are compared without case"
    );
    assert_eq!(s.targets().unwrap()[1].interval_ms, 1000);

    s.set_order(&[RowKey::Pin(p), RowKey::Target(c), RowKey::Target(a)])
        .unwrap();
    let order: Vec<i64> = s.targets().unwrap().iter().map(|t| t.id).collect();
    assert_eq!(
        order,
        vec![c, a, b],
        "rows left out keep their place after the others"
    );
    let pin_pos = s.pins().unwrap()[0].position;
    assert!(s.targets().unwrap().iter().all(|t| t.position > pin_pos));

    // A new row goes to the end.
    let d = s.add_target("9.9.9.9").unwrap();
    assert_eq!(s.targets().unwrap().last().unwrap().id, d);

    // Removing a target takes its pinned hops along and keeps its rounds.
    let mut s = s;
    s.insert_round(a, &rounds(&["+"], 1)[0]).unwrap();
    s.remove_target(a).unwrap();
    assert!(s.pins().unwrap().is_empty());
    assert!(
        !s.targets()
            .unwrap()
            .iter()
            .find(|t| t.id == a)
            .unwrap()
            .active
    );
    assert_eq!(s.load_rounds(a, 0, 10).unwrap().len(), 1);
    // Added again it keeps its id and comes back at the end.
    assert_eq!(s.add_target("1.1.1.1").unwrap(), a);
    let t = s.targets().unwrap();
    assert_eq!(t.iter().filter(|t| t.active).last().unwrap().id, a);
    assert!(s.unpin_hop(p).is_ok_and(|gone| !gone));
}

#[test]
fn pause_and_interval_are_kept() {
    let s = memory();
    let a = s.add_target("1.1.1.1").unwrap();
    s.set_paused(a, true).unwrap();
    s.set_interval(a, 500).unwrap();
    let t = &s.targets().unwrap()[0];
    assert!(t.paused);
    assert_eq!(t.interval_ms, 500);
}

#[test]
fn settings_are_saved_by_key() {
    let s = memory();
    assert!(s.settings().unwrap().is_empty());
    s.set_setting("warnMs", "100").unwrap();
    s.set_setting("theme", "\"dark\"").unwrap();
    s.set_setting("warnMs", "80").unwrap();
    assert_eq!(
        s.settings().unwrap(),
        vec![
            ("theme".to_string(), "\"dark\"".to_string()),
            ("warnMs".to_string(), "80".to_string())
        ]
    );
}

#[test]
fn recent_inputs_are_newest_first_and_capped() {
    let s = memory();
    for i in 0..12 {
        s.add_recent(&format!("host{i}.example")).unwrap();
    }
    let r = s.recent().unwrap();
    assert_eq!(r.len(), RECENT_MAX);
    assert_eq!(r[0], "host11.example");
    assert_eq!(r[9], "host2.example");
    s.add_recent("HOST5.example").unwrap();
    let r = s.recent().unwrap();
    assert_eq!(r[0], "HOST5.example", "typed again: first, as typed now");
    assert_eq!(
        r.iter()
            .filter(|h| h.eq_ignore_ascii_case("host5.example"))
            .count(),
        1
    );
}

fn window_of(rs: &[RoundRecord], b: i64, n: usize) -> Grid {
    Grid::build(rs, b, 0, n)
}

#[test]
fn bucket_rules_loss_partial_and_gaps() {
    // Two rounds per bucket (rounds every 1 s, buckets of 2 s).
    // Hop 1 misses answers while the destination answers (partial), hop 2 never answers
    // (silent), hop 3 and the destination lose rounds 4 and 5 together (loss on the path).
    let r = rounds(
        &["+.+++.++++", "..........", "++++..++++", "++++..++++"],
        1000,
    );
    let g = window_of(&r, 2000, 6);
    let w = g.window(0, 6);

    let dest = w.line(Pick::Dest);
    assert_eq!(
        dest.loss,
        vec![2],
        "a bucket is red when a round in it missed the destination"
    );
    assert!(dest.partial.is_empty(), "a target is never amber");
    assert_eq!(dest.avg[2], None, "no answer in the bucket: no line");
    assert!(dest.avg[0].is_some() && dest.avg[1].is_some());
    assert_eq!(dest.avg[5], None, "a bucket without rounds is empty");
    assert!(!dest.loss.contains(&5), "and not loss");

    let hop1 = w.line(Pick::Hop(1));
    assert_eq!(
        hop1.partial,
        vec![0],
        "no answer while the destination answered"
    );
    assert_eq!(
        hop1.loss,
        vec![2],
        "no answer while the destination was lost too"
    );
    // Both in one bucket: round 4 is a hop miss, round 5 loss on the path. Red wins.
    let r2 = rounds(&["++++..", "+++++."], 1000);
    let l2 = window_of(&r2, 2000, 3).window(0, 3).line(Pick::Hop(1));
    assert_eq!((l2.loss, l2.partial), (vec![2], vec![]));

    let hop2 = w.line(Pick::Hop(2));
    assert!(
        hop2.avg.iter().all(Option::is_none),
        "a silent hop has no line"
    );

    // Probes per bucket and how many got no answer: "k of n" in the hints.
    assert_eq!(dest.sent, vec![2, 2, 2, 2, 2, 0]);
    assert_eq!(dest.lost, vec![0, 0, 2, 0, 0, 0]);
    assert_eq!(hop1.sent, vec![2, 2, 2, 2, 2, 0]);
    assert_eq!(
        hop1.lost,
        vec![1, 0, 1, 0, 0, 0],
        "path loss and misses alike"
    );
    assert_eq!(hop2.lost, hop2.sent);

    let hops = w.hops();
    assert_eq!(hops[1].status, HopStatus::Silent);
    assert_eq!(hops.len(), 4);
    assert!(hops[3].is_destination);

    let s = w.stats(Pick::Dest, w.all());
    assert_eq!((s.sent, s.recv, s.lost_path), (10, 8, 2));
    assert!((s.loss_pct() - 20.0).abs() < 1e-9);
    assert_eq!(s.no_reply_pct(), 0.0);
    assert!(!s.last_lost);
    let s1 = w.stats(Pick::Hop(1), 0..6);
    assert!((s1.no_reply_pct() - 10.0).abs() < 1e-9, "{s1:?}");
    assert!((s1.loss_pct() - 10.0).abs() < 1e-9, "{s1:?}");
    // A period of buckets: the end is not in it.
    let s2 = w.stats(Pick::Dest, 2..3);
    assert_eq!(
        (s2.sent, s2.lost_path, s2.last_lost, s2.avg),
        (2, 2, true, None)
    );
    assert_eq!(w.stats(Pick::Dest, 1..3).sent, 4);
    assert_eq!(w.stats(Pick::Dest, 3..3), LineStats::default(), "empty");
    assert_eq!(w.stats(Pick::Dest, 4..40).sent, 2, "cut to the window");
    assert_eq!(w.rounds_in(1..3), 4);
    // Statuses of a period: hop 3 loses with the destination in buckets 1..3, not after.
    let in_loss = w.hops_in(1..3);
    assert_eq!(in_loss[1].status, HopStatus::Silent);
    assert_eq!(in_loss[2].status, HopStatus::Lossy);
    assert_eq!(in_loss[3].status, HopStatus::Lossy);
    assert_eq!(in_loss.iter().filter(|h| h.loss_origin).count(), 1);
    let clean = w.hops_in(3..5);
    assert_eq!(clean[2].status, HopStatus::Ok);
    assert_eq!(clean[1].status, HopStatus::Silent);
    assert!(w.hops_in(5..6).is_empty(), "no rounds, no hops");
}

/// The cached buckets give the same numbers as the engine's from-scratch functions.
#[test]
fn buckets_match_the_engine() {
    let r = rounds(
        &[
            "+..+..+..+..+..+..+..+",
            "......................",
            "+++.+++.++.+++.+++.+++",
            "++.+++.+++.++++.+++.++",
            "+++.+++.++.+++.+++.+++",
        ],
        700,
    );
    let (b, n) = (1000, 16);
    let g = Grid::build(&r, b, 0, n);
    let w = g.window(0, n);
    let (from, to) = (0, b * n as i64);
    // A picked period: buckets 4..12, that is [4000, 12000) ms.
    let (p0, p1) = (4usize, 12usize);
    let (pfrom, pto) = (p0 as i64 * b, p1 as i64 * b);
    let close = |a: Option<f64>, z: Option<f64>| match (a, z) {
        (Some(a), Some(z)) => (a - z).abs() < 1e-9,
        (a, z) => a.is_none() && z.is_none(),
    };
    for (pick, ttl) in [
        (Pick::Dest, None),
        (Pick::Hop(1), Some(1)),
        (Pick::Hop(2), Some(2)),
        (Pick::Hop(3), Some(3)),
        (Pick::Hop(5), Some(5)),
    ] {
        let mine = w.line(pick);
        let their_slots = slots(&r, ttl, from, to, n);
        let theirs = lane(&their_slots);
        assert_eq!(mine.loss, theirs.loss, "{pick:?}");
        assert_eq!(mine.partial, theirs.partial, "{pick:?}");
        assert_eq!(mine.lo, theirs.lo, "{pick:?}");
        assert_eq!(mine.hi, theirs.hi, "{pick:?}");
        for (a, z) in mine.avg.iter().zip(&theirs.series) {
            assert!(close(*a, *z));
        }
        let sent: Vec<u32> = their_slots.iter().map(|s| s.sent).collect();
        let lost: Vec<u32> = their_slots.iter().map(|s| s.lost + s.no_reply).collect();
        assert_eq!((&mine.sent, &mine.lost), (&sent, &lost), "{pick:?}");
        for (s, e) in [
            (w.stats(pick, w.all()), row_stats(&r, ttl, from, to)),
            (w.stats(pick, p0..p1), row_stats(&r, ttl, pfrom, pto)),
        ] {
            let s: LineStats = s;
            assert_eq!(s.sent, e.sent, "{pick:?}");
            assert!((s.loss_pct() - e.loss_pct).abs() < 1e-9, "{pick:?}");
            assert!((s.no_reply_pct() - e.no_reply_pct).abs() < 1e-9, "{pick:?}");
            assert_eq!(s.last_lost, e.last_lost, "{pick:?}");
            assert_eq!(s.last, e.last, "{pick:?}");
            assert_eq!(s.min, e.min, "{pick:?}");
            assert_eq!(s.max, e.max, "{pick:?}");
            assert!(close(s.avg, e.avg), "{pick:?}");
            assert!(
                close(s.jitter, e.jitter),
                "{pick:?} {:?} {:?}",
                s.jitter,
                e.jitter
            );
        }
    }
    let in_period: Vec<RoundRecord> = r
        .iter()
        .filter(|x| x.ts_ms >= pfrom && x.ts_ms < pto)
        .cloned()
        .collect();
    assert_eq!(w.rounds_in(p0..p1) as usize, in_period.len());
    for (mine, theirs) in [
        (w.hops(), hop_stats(&r)),
        (w.hops_in(p0..p1), hop_stats(&in_period)),
    ] {
        assert_eq!(mine.len(), theirs.len());
        for (m, t) in mine.iter().zip(&theirs) {
            assert_eq!(
                (m.ttl, m.status, m.loss_origin, m.is_destination),
                (t.ttl, t.status, t.loss_origin, t.is_destination)
            );
            assert_eq!((m.sent, m.recv, m.addr), (t.sent, t.recv, t.addr));
        }
        assert!(mine.iter().any(|h| h.loss_origin));
    }
}

/// Rounds added one by one give the same buckets as building from all of them.
#[test]
fn live_buckets_follow_new_rounds() {
    let r = rounds(&["+.++.++++.++++", "++++++++++++++", "+++.++++.+++.+"], 900);
    let mut h = History::new(1_000_000, 0);
    // The grid exists before the rounds come.
    let (b, n) = (2000, 8);
    assert!(h.live_window(b, 0, n, 0).line(Pick::Dest).loss.is_empty());
    for x in &r {
        h.push(x.clone());
    }
    let live = h.live_window(b, 0, n, 0).line(Pick::Dest);
    let built = Grid::build(&r, b, 0, n).window(0, n).line(Pick::Dest);
    assert_eq!(live, built);
    assert_eq!(h.lost_between(0, 100_000).count(), 3);
    assert_eq!(h.rounds_between(900, 2700).len(), 2);
}

#[test]
fn a_long_pause_does_not_fill_the_grid() {
    let mut h = History::new(i64::MAX / 4, 0);
    let mut r = rounds(&["++", "++"], 1000);
    let b = 1000;
    let _ = h.live_window(b, 0, 10, 0);
    h.push(r.remove(0));
    let mut late = r.remove(0);
    late.ts_ms = 10 * 86_400_000;
    h.push(late);
    let k = 10 * 86_400 - 9;
    let w = h.live_window(b, k, 10, 0);
    assert_eq!(w.rounds(), 1);
    assert!(w.line(Pick::Dest).avg[9].is_some());
}

#[test]
fn history_from_disk_goes_in_front() {
    let r = rounds(&["++.+++", "++.+++"], 1000);
    let mut h = History::new(1_000_000, 3000);
    for x in &r[3..] {
        h.push(x.clone());
    }
    h.prepend(r.clone(), 0);
    assert_eq!(h.len(), 6, "rounds already in memory are not doubled");
    assert_eq!(h.complete_from(), 0);
    assert_eq!(h.lost_between(0, 10_000).collect::<Vec<_>>(), vec![2000]);
    assert_eq!(
        h.past_window(1000, 0, 6).unwrap().line(Pick::Dest).loss,
        vec![2]
    );
    // Out of order arrivals stay sorted by start time.
    let mut early = r[0].clone();
    early.ts_ms = 4500;
    h.push(early);
    assert_eq!(h.rounds_between(4000, 5000).len(), 2);
    assert!(h
        .rounds_between(0, 10_000)
        .windows(2)
        .all(|w| w[0].ts_ms <= w[1].ts_ms));
    let mut h2 = History::new(1_000_000, 5000);
    assert!(
        h2.past_window(1000, 0, 6).is_none(),
        "older than memory: read from disk"
    );
}

/// A day of rounds every 2.5 s for 7 targets with 10 hops: what one snapshot of the 24-hour
/// window costs once the buckets are built. Run with `cargo test --release -- --ignored --nocapture`.
#[test]
#[ignore]
fn day_of_seven_targets_is_quick() {
    let day = 86_400_000i64;
    let step = 2500i64;
    let pattern: Vec<String> = (0..10)
        .map(|i| {
            (0..64)
                .map(|k| if (k * 7 + i * 3) % 41 == 0 { '.' } else { '+' })
                .collect()
        })
        .collect();
    let refs: Vec<&str> = pattern.iter().map(|s| s.as_str()).collect();
    let base = rounds(&refs, step);
    let now = 1_793_100_000_000i64;
    let mut targets: Vec<History> = (0..7)
        .map(|_| History::new(25 * 3_600_000, now - day))
        .collect();
    let t0 = Instant::now();
    for h in &mut targets {
        let mut ts = now - day;
        let mut i = 0;
        while ts < now {
            let mut r = base[i % base.len()].clone();
            r.ts_ms = ts;
            h.push(r);
            ts += step;
            i += 1;
        }
    }
    let fill = t0.elapsed();
    let n = 120usize;
    let b = day / n as i64;
    let from_k = now.div_euclid(b) - n as i64 + 1;
    let snap = |targets: &mut Vec<History>| {
        let mut sum = 0.0;
        for (i, h) in targets.iter_mut().enumerate() {
            let w = h.live_window(b, from_k, n, now);
            let line = w.line(Pick::Dest);
            let st = w.stats(Pick::Dest, 0..n);
            let hops = w.hops();
            sum += st.avg.unwrap_or(0.0) + line.loss.len() as f64 + hops.len() as f64;
            if i == 0 {
                for hop in &hops {
                    sum += w.line(Pick::Hop(hop.ttl)).loss.len() as f64;
                    sum += w.stats(Pick::Hop(hop.ttl), 0..n).sent as f64;
                }
            }
        }
        sum
    };
    let t1 = Instant::now();
    let first = snap(&mut targets);
    let build = t1.elapsed();
    let t2 = Instant::now();
    let mut again = 0.0;
    for _ in 0..10 {
        again = snap(&mut targets);
    }
    let each = t2.elapsed() / 10;
    // A new round lands in the cached buckets.
    let t3 = Instant::now();
    let mut r = base[0].clone();
    r.ts_ms = now + 10;
    targets[0].push(r);
    let push = t3.elapsed();
    println!(
        "rounds per target {}, fill {fill:?}, first snapshot (builds buckets) {build:?}, next snapshots {each:?}, push {push:?}",
        targets[0].len()
    );
    assert_eq!(first, again);
    assert!(each.as_millis() < 100, "{each:?}");
}

/// Reading a day of one target back from disk (the past older than memory, and the start).
/// Run with `cargo test --release -- --ignored --nocapture`.
#[test]
#[ignore]
fn a_day_reads_back_from_disk() {
    let db = TempDb::new("day");
    let mut s = Store::open(&db.0).unwrap();
    let id = s.add_target("1.1.1.1").unwrap();
    let pattern: Vec<String> = (0..10).map(|_| "+".repeat(8)).collect();
    let refs: Vec<&str> = pattern.iter().map(|x| x.as_str()).collect();
    let base = rounds(&refs, 2500);
    let day = 86_400_000i64;
    let t0 = Instant::now();
    let mut ts = 0;
    let mut i = 0;
    while ts < day {
        let mut r = base[i % base.len()].clone();
        r.ts_ms = ts;
        s.insert_round(id, &r).unwrap();
        ts += 2500;
        i += 1;
    }
    let write = t0.elapsed();
    let t1 = Instant::now();
    let back = s.load_rounds(id, 0, day).unwrap();
    let read = t1.elapsed();
    let t2 = Instant::now();
    let lost = s.lost_times(id, 0, day).unwrap();
    let lost_t = t2.elapsed();
    println!(
        "rounds {}, write {write:?}, read a day {read:?}, lost times {lost_t:?}",
        back.len()
    );
    assert_eq!(back.len(), i);
    assert!(lost.is_empty());
}

#[test]
fn a_report_covers_several_targets_in_the_window_language() {
    use super::report::{self, Document, PinnedLine, Report, Section};
    use whilo_trace_engine::{timeline, verdict};
    let a = rounds(&["++++++++", "........", "+++.++.+", "+++.++.+"], 1000);
    let b = rounds(&["++++++++", "++++++++"], 1000);
    let (ha, hb) = (hop_stats(&a), hop_stats(&b));
    let (va, vb) = (verdict(&a, &ha), verdict(&b, &hb));
    let (ta, tb) = (
        timeline(&a, None, 0, 8000, 8),
        timeline(&b, None, 0, 8000, 8),
    );
    let names_a = vec![None; ha.len()];
    let names_b = vec![None; hb.len()];
    let section = |host, rounds: &[RoundRecord], hops, names, line, v| Report {
        host,
        dst: rounds.last().map(|r| r.dst),
        probe: ProbeKind::Icmp,
        from_ms: 0,
        to_ms: 8000,
        tz_offset_min: 240,
        interval_s: 1.0,
        hops,
        names,
        timeline: line,
        verdict: v,
        generated_ms: 8000,
    };
    let doc_with = |findings: Option<Vec<String>>| Document {
        sections: vec![
            Section {
                report: section("10.0.0.4", &a, &ha, &names_a, &ta, &va),
                pinned: vec![PinnedLine {
                    ttl: 1,
                    addr: Some(ip(1)),
                    name: Some("router".into()),
                    timeline: timeline(&a, Some(1), 0, 8000, 8),
                }],
            },
            Section {
                report: section("10.0.0.2", &b, &hb, &names_b, &tb, &vb),
                pinned: Vec::new(),
            },
        ],
        from_ms: 0,
        to_ms: 8000,
        tz_offset_min: 240,
        generated_ms: 8000,
        findings,
        lang: "ru",
    };
    let doc = doc_with(None);
    let html = report::document_html(&doc);
    assert!(html.contains("<html lang=\"ru\">"));
    assert!(html.contains("Отчет о сетевом пути: 10.0.0.4, 10.0.0.2"));
    assert!(html.contains("<h2>10.0.0.4</h2>") && html.contains("<h2>10.0.0.2</h2>"));
    assert!(html.contains("Хоп 1: <span class=\"mono\">10.0.0.1 (router)</span>"));
    // Statuses name the probe type of the data, as the interface does.
    assert!(html.contains("<span class=\"badge\">не отвечает на ICMP</span>"));
    assert!(html.contains("Хоп со статусом «не отвечает на ICMP» или «отвечает не на все пробы»"));
    assert!(html.contains("ICMP echo, одна проба на каждый TTL раз в 1,0 с"));
    assert!(html.contains("25,0%"), "decimal comma in the hop table");
    assert!(!html.contains("Network path report"));
    // The command line has no findings of its own: every target gets the engine's, with its name.
    assert!(
        html.contains("<li>10.0.0.4: Packet loss to the destination: 25.0% over 8 rounds.</li>")
    );
    let csv = report::document_hops_csv(&doc);
    assert_eq!(csv.lines().count(), 1 + ha.len() + hb.len());
    assert!(csv
        .lines()
        .nth(1)
        .unwrap()
        .starts_with("10.0.0.4,1,10.0.0.1,"));
    assert!(csv.contains(",No reply to ICMP,"), "{csv}");
    let samples = report::document_samples_csv(&[("10.0.0.4", &a), ("10.0.0.2", &b)]);
    assert_eq!(samples.lines().count(), 1 + 8 * 4 + 8 * 2);
    let json: serde_json::Value = serde_json::from_str(&report::document_json(&doc)).unwrap();
    assert_eq!(json["targets"].as_array().unwrap().len(), 2);
    assert_eq!(json["targets"][0]["pinned"][0]["name"], "router");
    assert_eq!(json["targets"][0]["probe"], "icmp");
    assert_eq!(
        json["findings"].as_array().unwrap().len(),
        report::verdict_lines(&va, ProbeKind::Icmp).len()
            + report::verdict_lines(&vb, ProbeKind::Icmp).len()
    );
    assert!(report::verdict_lines(&va, ProbeKind::Icmp).iter().any(|l| l
        == "Hop 2 does not answer ICMP probes but passes traffic on; that is not packet loss."));

    // The window's findings go in as they are; none means none, not the engine's English ones.
    let shown = doc_with(Some(vec!["Потери начинаются на хопе 3.".into()]));
    let html = report::document_html(&shown);
    assert!(html.contains("<ul class=\"findings\"><li>Потери начинаются на хопе 3.</li></ul>"));
    assert!(!html.contains("Packet loss"));
    let none = doc_with(Some(Vec::new()));
    let html = report::document_html(&none);
    assert!(!html.contains("class=\"findings\"") && !html.contains("<li>"));
    let json: serde_json::Value = serde_json::from_str(&report::document_json(&none)).unwrap();
    assert_eq!(json["findings"], serde_json::json!([]));
    assert!(json["targets"][0].get("findings").is_none());
    assert!(!report::document_json(&none).contains("Packet loss"));
}
