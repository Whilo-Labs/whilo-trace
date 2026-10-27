#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Whilo Trace desktop app: several targets at once, a timeline per target and per pinned hop,
//! the full trace of one target. Commands and their JSON: `docs/ui-contract.md`.

mod contract;

use contract::{
    AddTargetReq, ExportRes, History, HopRef, IdRes, PartialHop, PathSummary, ReportReq, Row,
    Settings, SettingsPatch, Snapshot, SnapshotReq, Stats, Trace, TraceHop,
};
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use whilo_trace_engine::dns::DnsCache;
use whilo_trace_engine::stats::LOSS_NOTABLE_PCT;
use whilo_trace_engine::{
    hop_stats, now_ms, spawn_monitor_with, timeline, verdict, Bucket, HopStats, HopStatus,
    MonitorConfig, MonitorEvent, MonitorHandle, MonitorOptions, ProbeKind, Prober, ProberFactory,
    RoundRecord, Verdict,
};
use whilo_trace_store::buckets::{self, Grid, HopInfo, LineStats, Pick, Window};
use whilo_trace_store::report::{self, Document, PinnedLine, Report, Section};
use whilo_trace_store::{default_db_path, PinRow, RowKey, Store, TargetRow, DEFAULT_INTERVAL_MS};

/// Rounds kept in memory per target: a day and a bucket of the 24-hour window, so that window
/// never needs the disk. Older rounds stay in SQLite only.
const KEEP_MS: i64 = 25 * 3600 * 1000;
const DAY_MS: i64 = 86_400_000;
const HISTORY_SLOTS: u32 = 200;
/// Seconds between rounds the interface offers.
const INTERVALS: [f64; 7] = [0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0];
const WINDOWS: [(&str, i64); 6] = [
    ("5m", 300),
    ("10m", 600),
    ("30m", 1800),
    ("1h", 3600),
    ("6h", 21600),
    ("24h", 86400),
];
/// How far back to look for a hop's address when it is pinned.
const PIN_LOOK_BACK: usize = 2000;

/// What a target found, shared with its monitor thread.
struct TargetData {
    hist: buckets::History,
    error: Option<String>,
}

struct Target {
    host: String,
    probe: ProbeKind,
    interval_s: f64,
    paused: bool,
    position: i64,
    data: Arc<Mutex<TargetData>>,
    /// Dropping it stops the monitor.
    monitor: MonitorHandle,
}

#[derive(Clone)]
struct Pin {
    target_id: i64,
    ttl: u8,
    addr: Option<String>,
    position: i64,
}

/// Tells the interface that a target has new data (the `round` event).
type Notify = Arc<dyn Fn(i64) + Send + Sync>;

/// Opens a probe backend for a monitor. `None` in `Core` is the platform one; tests pass a fake,
/// so no probe leaves the machine.
type ProberSource = Arc<dyn Fn() -> std::io::Result<Box<dyn Prober>> + Send + Sync>;

struct Core {
    db_path: PathBuf,
    notify: Notify,
    prober: Option<ProberSource>,
    /// Writes: rounds from the monitors, and every change the interface makes.
    store: Arc<Mutex<Store>>,
    /// Long reads (the past, reports) on their own connection, so rounds are never held up.
    reader: Mutex<Store>,
    targets: Mutex<BTreeMap<i64, Target>>,
    pins: Mutex<BTreeMap<i64, Pin>>,
    settings: Mutex<Settings>,
    recent: Mutex<Vec<String>>,
    dns: DnsCache,
}

/// A poisoned lock still holds usable data: a panic in one command must not stop the others.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn db(e: impl std::fmt::Display) -> String {
    format!("db: {e}")
}

fn io(e: impl std::fmt::Display) -> String {
    format!("io: {e}")
}

/// What `resolve_v4` says when a name has only IPv6 addresses.
fn is_no_ipv4(text: &str) -> bool {
    text.ends_with("no IPv4 address")
}

/// The error code of a failed lookup.
fn resolve_code(e: &std::io::Error) -> &'static str {
    if is_no_ipv4(&e.to_string()) {
        "no_ipv4"
    } else {
        "resolve_failed"
    }
}

/// Turns a monitor message into an error code of the contract (docs/ui-contract.md):
/// "cannot resolve <host>: <why>", "cannot open <probe>: <os error>", "probe to <ip> failed: <os error>".
fn error_code(msg: &str) -> String {
    if msg.starts_with("cannot resolve") {
        return if is_no_ipv4(msg) {
            "no_ipv4"
        } else {
            "resolve_failed"
        }
        .into();
    }
    if msg.starts_with("cannot open") || msg.starts_with("probe to ") {
        let detail = msg.split_once(": ").map_or("", |(_, d)| d).trim();
        return if detail.is_empty() {
            "probe_unavailable".into()
        } else {
            format!("probe_unavailable: {detail}")
        };
    }
    io(msg)
}

fn row_key(id: &str) -> Option<RowKey> {
    if let Some(n) = id.strip_prefix('t') {
        n.parse().ok().map(RowKey::Target)
    } else if let Some(n) = id.strip_prefix('p') {
        n.parse().ok().map(RowKey::Pin)
    } else {
        None
    }
}

fn target_row_id(id: i64) -> String {
    format!("t{id}")
}

fn pin_row_id(id: i64) -> String {
    format!("p{id}")
}

/// Seconds between rounds of a target added without an interval.
const DEFAULT_INTERVAL_S: f64 = DEFAULT_INTERVAL_MS as f64 / 1000.0;

/// The nearest interval the interface offers, for intervals saved in the database.
fn snap_interval(s: f64) -> f64 {
    if !s.is_finite() {
        return DEFAULT_INTERVAL_S;
    }
    INTERVALS
        .into_iter()
        .min_by(|a, b| (a - s).abs().total_cmp(&(b - s).abs()))
        .unwrap_or(DEFAULT_INTERVAL_S)
}

/// An interval from the interface: one of the offered ones, else `invalid_interval`.
fn offered_interval(s: f64) -> Result<f64, String> {
    INTERVALS
        .into_iter()
        .find(|v| (v - s).abs() < 1e-6)
        .ok_or_else(|| "invalid_interval".into())
}

fn window_ms(w: &str) -> i64 {
    WINDOWS
        .iter()
        .find(|(name, _)| *name == w)
        .map_or(600, |(_, s)| *s)
        * 1000
}

impl Core {
    /// Opens the database and takes over what the window keeps between runs.
    fn open(db_path: PathBuf, notify: Notify) -> Result<Arc<Core>, String> {
        Self::open_with(db_path, notify, None)
    }

    /// `open` with another probe backend for the monitors.
    fn open_with(
        db_path: PathBuf,
        notify: Notify,
        prober: Option<ProberSource>,
    ) -> Result<Arc<Core>, String> {
        let store = Store::open(&db_path).map_err(db)?;
        let reader = Store::open(&db_path).map_err(db)?;
        let pins = store.pins().map_err(db)?;
        let settings = load_settings(&store.settings().map_err(db)?);
        let recent = store.recent().map_err(db)?;
        Ok(Arc::new(Core {
            db_path,
            notify,
            prober,
            store: Arc::new(Mutex::new(store)),
            reader: Mutex::new(reader),
            targets: Mutex::new(BTreeMap::new()),
            pins: Mutex::new(
                pins.into_iter()
                    .map(|p| {
                        let pin = Pin {
                            target_id: p.target_id,
                            ttl: p.ttl,
                            addr: p.addr,
                            position: p.position,
                        };
                        (p.id, pin)
                    })
                    .collect(),
            ),
            settings: Mutex::new(settings),
            recent: Mutex::new(recent),
            dns: DnsCache::new(),
        }))
    }

    /// Starts every target that was on the main screen.
    fn start_saved(&self) -> Result<(), String> {
        let rows = lock(&self.store).targets().map_err(db)?;
        for t in rows.iter().filter(|t| t.active) {
            if let Err(e) = self.start(t) {
                eprintln!("cannot start {}: {e}", t.host);
            }
        }
        Ok(())
    }

    /// Starts watching a saved target: its monitor at once, its last day from disk in the background.
    fn start(&self, row: &TargetRow) -> Result<(), String> {
        let mut targets = lock(&self.targets);
        if targets.contains_key(&row.id) {
            return Ok(());
        }
        let started = now_ms();
        let data = Arc::new(Mutex::new(TargetData {
            hist: buckets::History::new(KEEP_MS, started),
            error: None,
        }));
        let interval_s = snap_interval(row.interval_ms as f64 / 1000.0);
        let monitor = self.spawn(row.id, &row.host, interval_s, row.paused, &data)?;
        targets.insert(
            row.id,
            Target {
                host: row.host.clone(),
                probe: ProbeKind::parse(&row.probe).unwrap_or_default(),
                interval_s,
                paused: row.paused,
                position: row.position,
                data: data.clone(),
                monitor,
            },
        );
        drop(targets);

        let (path, notify, id) = (self.db_path.clone(), self.notify.clone(), row.id);
        let _ = std::thread::Builder::new()
            .name(format!("history {}", row.host))
            .spawn(move || {
                let older =
                    Store::open(&path).and_then(|s| s.load_rounds(id, started - KEEP_MS, started));
                {
                    let mut d = lock(&data);
                    match older {
                        Ok(rounds) => d.hist.prepend(rounds, started - KEEP_MS),
                        Err(e) => d.error = Some(db(e)),
                    }
                }
                notify(id);
            });
        Ok(())
    }

    fn spawn(
        &self,
        id: i64,
        host: &str,
        interval_s: f64,
        paused: bool,
        data: &Arc<Mutex<TargetData>>,
    ) -> Result<MonitorHandle, String> {
        let mut cfg = MonitorConfig::new(host.to_string());
        cfg.interval = Duration::from_secs_f64(interval_s);
        let (data, store, notify) = (data.clone(), self.store.clone(), self.notify.clone());
        let prober = self
            .prober
            .clone()
            .map(|open| -> ProberFactory { Box::new(move || open()) });
        // A target saved on pause starts paused: no probe and no lookup until it is resumed.
        spawn_monitor_with(
            cfg,
            MonitorOptions {
                paused,
                prober,
                ..Default::default()
            },
            move |event| {
                match event {
                    MonitorEvent::Round(round) => {
                        let saved = lock(&store).insert_round(id, &round);
                        let mut d = lock(&data);
                        d.error = saved.err().map(db);
                        d.hist.push(round);
                    }
                    MonitorEvent::Error(msg) => lock(&data).error = Some(error_code(&msg)),
                    #[allow(unreachable_patterns)]
                    _ => {}
                }
                notify(id);
            },
        )
        .map_err(io)
    }

    /// Positions changed in the database: take them over.
    fn reload_positions(&self) -> Result<(), String> {
        let (rows, pins) = {
            let s = lock(&self.store);
            (s.targets().map_err(db)?, s.pins().map_err(db)?)
        };
        let mut targets = lock(&self.targets);
        for r in rows {
            if let Some(t) = targets.get_mut(&r.id) {
                t.position = r.position;
            }
        }
        drop(targets);
        let mut mine = lock(&self.pins);
        for p in pins {
            if let Some(m) = mine.get_mut(&p.id) {
                m.position = p.position;
            }
        }
        Ok(())
    }

    fn pin_target(&self, pin: i64) -> Option<i64> {
        lock(&self.pins).get(&pin).map(|p| p.target_id)
    }

    /// The target behind a row id: itself, or the target a pinned hop belongs to.
    fn target_of(&self, row: &str) -> Result<i64, String> {
        match row_key(row) {
            Some(RowKey::Target(id)) => Ok(id),
            Some(RowKey::Pin(p)) => self.pin_target(p).ok_or_else(|| "no_such_target".into()),
            None => Err("no_such_target".into()),
        }
    }
}

/// The window of a snapshot on the bucket grid: bucket `k` covers `[k * b, (k + 1) * b)`.
struct Frame {
    n: usize,
    b: i64,
    from_k: i64,
    from_ms: i64,
    to_ms: i64,
    /// Buckets of the picked period `[fromMs, toMs)`, inside the window: from the bucket that holds
    /// fromMs up to the one that holds toMs - 1. Empty when the period misses the window.
    range: Option<Range<usize>>,
}

impl Frame {
    fn new(req: &SnapshotReq, now: i64) -> Self {
        let n = req.buckets.clamp(10, 2000) as usize;
        let b = ((window_ms(&req.window) as f64 / n as f64).round() as i64).max(1);
        // Live: the last bucket is the one now falls in. Past: the one that holds endMs.
        let last_k = match req.end_ms {
            None => now.div_euclid(b),
            Some(end) => (end.min(now) - 1).div_euclid(b),
        };
        let from_k = last_k - n as i64 + 1;
        let (from_ms, to_ms) = (from_k * b, (last_k + 1) * b);
        // Half-open like the window: toMs itself is not in the period.
        let range = req.range.map(|[a, z]| {
            let (a, z) = (a.min(z).max(from_ms), a.max(z).min(to_ms));
            let at = |t: i64| (t - from_ms).div_euclid(b) as usize;
            if z > a {
                at(a)..at(z - 1) + 1
            } else {
                0..0
            }
        });
        Frame {
            n,
            b,
            from_k,
            from_ms,
            to_ms,
            range,
        }
    }
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn r3s(v: Vec<Option<f64>>) -> Vec<Option<f64>> {
    v.into_iter().map(|x| x.map(r3)).collect()
}

/// Numbers of a line; `sent` counts the rounds that probed it (for a target, every round).
fn stats_of(s: &LineStats) -> Stats {
    Stats {
        loss_pct: r3(s.loss_pct()),
        no_reply_pct: r3(s.no_reply_pct()),
        avg: s.avg.map(r3),
        min: s.min.map(r3),
        max: s.max.map(r3),
        last: s.last.map(r3),
        last_lost: s.last_lost,
        jitter: s.jitter.map(r3),
        sent: s.sent,
        status: None,
    }
}

/// A silent hop does not answer this probe type at all: its misses say nothing about the path.
fn not_loss(s: &mut Stats) {
    s.loss_pct = 0.0;
    s.no_reply_pct = 0.0;
}

/// Top of the latency scale: a round number a little above the highest point of the line.
fn scale_top(series: &[Option<f64>]) -> f64 {
    let top = series.iter().flatten().copied().fold(0.0, f64::max);
    if top <= 0.0 {
        return 10.0;
    }
    let v = (top * 1.05).max(1.0);
    let p = 10f64.powf(v.log10().floor());
    for m in [1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0] {
        if v <= m * p * (1.0 + 1e-9) {
            return (m * p * 1e6).round() / 1e6;
        }
    }
    10.0 * p
}

fn status_name(s: HopStatus) -> &'static str {
    match s {
        HopStatus::Ok => "ok",
        HopStatus::Lossy => "loss",
        HopStatus::RateLimited => "partial",
        HopStatus::Silent => "silent",
        HopStatus::NoReply => "noreply",
    }
}

/// Status of a target row, from its destination: the same rule for the window and a period.
fn target_status(s: &LineStats) -> &'static str {
    if s.sent == 0 {
        "ok"
    } else if s.recv == 0 {
        "noreply"
    } else if s.loss_pct() >= LOSS_NOTABLE_PCT {
        "loss"
    } else {
        "ok"
    }
}

/// Status of the hop at `ttl` among the hops of a window or a period. A TTL that no round of it
/// probed is `noreply`, or `ok` when it has no rounds at all.
fn hop_status(hops: &[HopInfo], ttl: u8, rounds: u32) -> HopStatus {
    match hops.iter().find(|h| h.ttl == ttl) {
        Some(h) => h.status,
        None if rounds == 0 => HopStatus::Ok,
        None => HopStatus::NoReply,
    }
}

/// A period picked on a timeline: its buckets and what their rounds found, so its statuses
/// follow the same rules as the window's.
struct Period {
    buckets: Range<usize>,
    hops: Vec<HopInfo>,
    dest: LineStats,
}

impl Period {
    fn new(w: &Window, buckets: Range<usize>) -> Self {
        Period {
            hops: w.hops_in(buckets.clone()),
            dest: w.stats(Pick::Dest, buckets.clone()),
            buckets,
        }
    }

    fn hop_status(&self, ttl: u8) -> HopStatus {
        hop_status(&self.hops, ttl, self.dest.sent)
    }
}

/// One line of a window, ready for a row or a trace hop.
struct Drawn {
    series: Vec<Option<f64>>,
    lo: Vec<Option<f64>>,
    hi: Vec<Option<f64>>,
    loss: Vec<u32>,
    partial: Vec<u32>,
    sent_n: Vec<u32>,
    lost_n: Vec<u32>,
    max: f64,
    stats: Stats,
    /// The picked period, without its status: the caller knows the rule.
    range_stats: Option<Stats>,
}

fn draw(w: &Window, pick: Pick, period: Option<&Period>) -> Drawn {
    let line = w.line(pick);
    Drawn {
        max: scale_top(&line.avg),
        series: r3s(line.avg),
        lo: r3s(line.lo),
        hi: r3s(line.hi),
        loss: line.loss,
        partial: line.partial,
        sent_n: line.sent,
        lost_n: line.lost,
        stats: stats_of(&w.stats(pick, w.all())),
        range_stats: period.map(|p| stats_of(&w.stats(pick, p.buckets.clone()))),
    }
}

/// The destination line of a target.
fn draw_target(w: &Window, period: Option<&Period>) -> Drawn {
    let mut d = draw(w, Pick::Dest, period);
    if let (Some(rs), Some(p)) = (d.range_stats.as_mut(), period) {
        rs.status = Some(target_status(&p.dest).into());
    }
    d
}

/// A hop of the given status over the window. A silent hop gets no marks and no loss, the
/// interface hatches it; over the period the same goes by its status there.
fn draw_hop(w: &Window, ttl: u8, status: HopStatus, period: Option<&Period>) -> Drawn {
    let mut d = draw(w, Pick::Hop(ttl), period);
    if status == HopStatus::Silent {
        d.loss.clear();
        d.partial.clear();
        not_loss(&mut d.stats);
    }
    if let (Some(rs), Some(p)) = (d.range_stats.as_mut(), period) {
        let in_period = p.hop_status(ttl);
        rs.status = Some(status_name(in_period).into());
        if in_period == HopStatus::Silent {
            not_loss(rs);
        }
    }
    d
}

/// Everything one target contributes to a snapshot.
struct Computed {
    row: Row,
    /// Pinned hop rows of this target, by pin id.
    pins: Vec<(i64, Row)>,
    trace: Option<Trace>,
    /// Probes per round, for the rate.
    probes: usize,
}

struct TargetInfo {
    id: i64,
    host: String,
    probe: ProbeKind,
    interval_s: f64,
    paused: bool,
    position: i64,
    data: Arc<Mutex<TargetData>>,
}

fn compute(
    core: &Core,
    t: &TargetInfo,
    w: &Window,
    f: &Frame,
    pins: &[(i64, &Pin)],
    want_trace: bool,
    error: Option<String>,
    last: Option<&RoundRecord>,
) -> Computed {
    let probe = t.probe.as_str().to_string();
    let hops = w.hops();
    let rounds = w.rounds();
    let period = f.range.clone().map(|r| Period::new(w, r));
    let dest = draw_target(w, period.as_ref());
    let dst = last.map(|r| r.dst);
    let name = match t.host.parse::<IpAddr>() {
        Ok(ip) => core.dns.name(ip),
        Err(_) => dst.map(|ip| ip.to_string()),
    };
    let path = PathSummary {
        hops: w.dest_ttl().unwrap_or(0),
        origin: hops.iter().find(|h| h.loss_origin).map(|h| HopRef {
            ttl: h.ttl,
            addr: h.addr.map(|a| a.to_string()),
        }),
        silent: hops
            .iter()
            .filter(|h| h.status == HopStatus::Silent)
            .map(|h| h.ttl)
            .collect(),
        partial: hops
            .iter()
            .filter(|h| h.status == HopStatus::RateLimited)
            .map(|h| PartialHop {
                ttl: h.ttl,
                addr: h.addr.map(|a| a.to_string()),
                name: h.addr.and_then(|a| core.dns.name(a)),
                no_reply_pct: r3(w.stats(Pick::Hop(h.ttl), w.all()).no_reply_pct()),
            })
            .collect(),
    };
    let row = Row {
        id: target_row_id(t.id),
        target_id: t.id,
        host: t.host.clone(),
        name,
        ip: dst.map(|ip| ip.to_string()),
        probe: probe.clone(),
        interval: t.interval_s,
        paused: t.paused,
        error,
        hop_of: None,
        hop: None,
        series: dest.series,
        lo: dest.lo,
        hi: dest.hi,
        loss: dest.loss,
        partial: Vec::new(),
        sent_n: dest.sent_n,
        lost_n: dest.lost_n,
        max: dest.max,
        status: target_status(&w.stats(Pick::Dest, w.all())).into(),
        stats: dest.stats,
        range_stats: dest.range_stats,
        path: Some(path),
    };
    let pin_rows = pins
        .iter()
        .map(|(pid, p)| {
            let info = hops.iter().find(|h| h.ttl == p.ttl);
            let addr = info
                .and_then(|h| h.addr)
                .map(|a| a.to_string())
                .or_else(|| p.addr.clone());
            let status = hop_status(&hops, p.ttl, rounds);
            let d = draw_hop(w, p.ttl, status, period.as_ref());
            let row = Row {
                id: pin_row_id(*pid),
                target_id: t.id,
                host: addr.clone().unwrap_or_else(|| format!("hop {}", p.ttl)),
                name: info.and_then(|h| h.addr).and_then(|a| core.dns.name(a)),
                ip: addr,
                probe: probe.clone(),
                interval: t.interval_s,
                paused: t.paused,
                error: None,
                hop_of: Some(target_row_id(t.id)),
                hop: Some(p.ttl),
                series: d.series,
                lo: d.lo,
                hi: d.hi,
                loss: d.loss,
                partial: d.partial,
                sent_n: d.sent_n,
                lost_n: d.lost_n,
                max: d.max,
                status: status_name(status).into(),
                stats: d.stats,
                range_stats: d.range_stats,
                path: None,
            };
            (*pid, row)
        })
        .collect();
    let trace = want_trace.then(|| Trace {
        target: target_row_id(t.id),
        rounds,
        hops: hops
            .iter()
            .map(|h| {
                let d = draw_hop(w, h.ttl, h.status, period.as_ref());
                let status = status_name(h.status);
                TraceHop {
                    ttl: h.ttl,
                    addr: h.addr.map(|a| a.to_string()),
                    name: h.addr.and_then(|a| core.dns.name(a)),
                    status: status.into(),
                    origin: h.loss_origin,
                    dest: h.is_destination,
                    max: d.max,
                    series: d.series,
                    lo: d.lo,
                    hi: d.hi,
                    loss: d.loss,
                    partial: d.partial,
                    sent_n: d.sent_n,
                    lost_n: d.lost_n,
                    by_probe: BTreeMap::from([(probe.clone(), status.to_string())]),
                    stats: d.stats,
                    range_stats: d.range_stats,
                }
            })
            .collect(),
    });
    Computed {
        row,
        pins: pin_rows,
        trace,
        probes: last.map_or(0, |r| r.hops.len()),
    }
}

impl Core {
    fn snapshot(&self, req: &SnapshotReq) -> Result<Snapshot, String> {
        let now = now_ms();
        let f = Frame::new(req, now);
        let live = req.end_ms.is_none();
        let infos: Vec<TargetInfo> = lock(&self.targets)
            .iter()
            .map(|(&id, t)| TargetInfo {
                id,
                host: t.host.clone(),
                probe: t.probe,
                interval_s: t.interval_s,
                paused: t.paused,
                position: t.position,
                data: t.data.clone(),
            })
            .collect();
        let pins: BTreeMap<i64, Pin> = lock(&self.pins).clone();
        // Only a target row has a trace; a pinned hop or an unknown id gets `trace: null`.
        let trace_of = match req.trace.as_deref().and_then(row_key) {
            Some(RowKey::Target(id)) => Some(id),
            _ => None,
        };

        // Positions of every row: targets and pinned hops share one order.
        let mut order: Vec<(i64, u8, i64, Row)> = Vec::new();
        let mut trace = None;
        let mut rate = 0.0;
        let (h_from, h_to) = (f.to_ms - DAY_MS, f.to_ms);
        let slot_ms = DAY_MS / HISTORY_SLOTS as i64;
        let mut history: BTreeSet<u32> = BTreeSet::new();
        for t in &infos {
            let my_pins: Vec<(i64, &Pin)> = pins
                .iter()
                .filter(|(_, p)| p.target_id == t.id)
                .map(|(&id, p)| (id, p))
                .collect();
            let want_trace = trace_of == Some(t.id);
            let mut d = lock(&t.data);
            let error = d.error.clone();
            let last = d.hist.last().cloned();
            let in_memory = live || f.from_ms >= d.hist.complete_from();
            let c = if in_memory {
                let w = if live {
                    d.hist.live_window(f.b, f.from_k, f.n, now)
                } else {
                    // Never missing: in_memory is the condition past_window checks.
                    d.hist
                        .past_window(f.b, f.from_k, f.n)
                        .ok_or_else(|| db("past window out of memory"))?
                };
                compute(self, t, &w, &f, &my_pins, want_trace, error, last.as_ref())
            } else if let Some(w) = d.hist.kept_past(f.b, f.from_k, f.n) {
                compute(self, t, &w, &f, &my_pins, want_trace, error, last.as_ref())
            } else {
                drop(d);
                let rounds = lock(&self.reader)
                    .load_rounds(t.id, f.from_ms, f.to_ms)
                    .map_err(db)?;
                let grid = Grid::build(&rounds, f.b, f.from_k, f.n);
                let c = compute(
                    self,
                    t,
                    &grid.window(f.from_k, f.n),
                    &f,
                    &my_pins,
                    want_trace,
                    error,
                    last.as_ref(),
                );
                d = lock(&t.data);
                d.hist.keep_past(f.b, f.from_k, f.n, grid);
                c
            };
            // Loss marks of the day strip.
            if live || h_from >= d.hist.complete_from() {
                history.extend(
                    d.hist
                        .lost_between(h_from, h_to)
                        .map(|ts| ((ts - h_from) / slot_ms) as u32),
                );
                drop(d);
            } else {
                drop(d);
                let lost = lock(&self.reader)
                    .lost_times(t.id, h_from, h_to)
                    .map_err(db)?;
                history.extend(lost.into_iter().map(|ts| ((ts - h_from) / slot_ms) as u32));
            }
            if !t.paused && c.probes > 0 {
                rate += c.probes as f64 / t.interval_s;
            }
            if c.trace.is_some() {
                trace = c.trace;
            }
            order.push((t.position, 0, t.id, c.row));
            for (pid, row) in c.pins {
                let pos = pins.get(&pid).map_or(i64::MAX, |p| p.position);
                order.push((pos, 1, pid, row));
            }
        }
        order.sort_by_key(|(pos, kind, id, _)| (*pos, *kind, *id));
        Ok(Snapshot {
            n: f.n as u32,
            now_ms: now,
            from_ms: f.from_ms,
            to_ms: f.to_ms,
            live,
            bucket_ms: f.b as f64,
            targets: order.into_iter().map(|(_, _, _, row)| row).collect(),
            trace,
            history: History {
                from_ms: h_from,
                to_ms: h_to,
                slots: HISTORY_SLOTS,
                loss: history.into_iter().filter(|&s| s < HISTORY_SLOTS).collect(),
            },
            loss: Vec::new(),
            settings: lock(&self.settings).clone(),
            probes_available: vec![ProbeKind::Icmp.as_str().into()],
            rate_pps: (rate * 10.0).round() / 10.0,
            recent: lock(&self.recent).clone(),
        })
    }
}

#[tauri::command(async)]
fn snapshot(core: State<'_, Arc<Core>>, req: SnapshotReq) -> Result<Snapshot, String> {
    core.snapshot(&req)
}

/// A name or an IPv4 address as typed: no spaces, no characters that cannot be in a host name.
fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with(['.', '-'])
        && host
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
}

impl Core {
    fn add(&self, req: &AddTargetReq) -> Result<IdRes, String> {
        let host = req.host.trim().to_string();
        if !valid_host(&host) {
            return Err("invalid_host".into());
        }
        let probe = match req.probe.as_deref() {
            None => ProbeKind::Icmp,
            Some(p) => ProbeKind::parse(p).ok_or("probe_unavailable")?,
        };
        let interval_s = match req.interval_s {
            None => DEFAULT_INTERVAL_S,
            Some(s) => offered_interval(s)?,
        };
        let existing = lock(&self.targets)
            .iter()
            .find(|(_, t)| t.host.eq_ignore_ascii_case(&host) && t.probe == probe)
            .map(|(&id, _)| id);
        if let Some(id) = existing {
            self.remember(&host)?;
            return Ok(IdRes {
                id: target_row_id(id),
            });
        }
        // Resolve first, so a typo does not become a target.
        whilo_trace_engine::monitor::resolve_v4(&host).map_err(|e| resolve_code(&e))?;
        let interval_ms = (interval_s * 1000.0).round() as i64;
        self.remember(&host)?;
        let row = {
            let s = lock(&self.store);
            let id = s
                .add_target_with(&host, probe.as_str(), Some(interval_ms))
                .map_err(db)?;
            s.targets()
                .map_err(db)?
                .into_iter()
                .find(|t| t.id == id)
                .ok_or("no_such_target")?
        };
        self.start(&row)?;
        Ok(IdRes {
            id: target_row_id(row.id),
        })
    }

    /// A host typed into the add field goes to the top of the recent inputs.
    fn remember(&self, host: &str) -> Result<(), String> {
        let s = lock(&self.store);
        s.add_recent(host).map_err(db)?;
        *lock(&self.recent) = s.recent().map_err(db)?;
        Ok(())
    }

    fn remove(&self, id: &str) -> Result<(), String> {
        let tid = match row_key(id) {
            Some(RowKey::Target(t)) => t,
            Some(RowKey::Pin(p)) => return self.unpin(p),
            None => return Err("no_such_target".into()),
        };
        // Dropping the target stops its monitor after the rounds in flight; history stays on disk.
        let removed = lock(&self.targets).remove(&tid);
        if removed.is_none() {
            return Err("no_such_target".into());
        }
        lock(&self.pins).retain(|_, p| p.target_id != tid);
        lock(&self.store).remove_target(tid).map_err(db)
    }

    fn pause(&self, id: &str, paused: bool) -> Result<(), String> {
        let tid = self.target_of(id)?;
        {
            let mut targets = lock(&self.targets);
            let t = targets.get_mut(&tid).ok_or("no_such_target")?;
            if t.paused == paused {
                return Ok(());
            }
            t.paused = paused;
            t.monitor.set_paused(paused);
        }
        lock(&self.store).set_paused(tid, paused).map_err(db)
    }

    fn set_interval(&self, id: &str, interval_s: f64) -> Result<(), String> {
        let interval_s = offered_interval(interval_s)?;
        let tid = self.target_of(id)?;
        {
            let mut targets = lock(&self.targets);
            let t = targets.get_mut(&tid).ok_or("no_such_target")?;
            t.interval_s = interval_s;
            t.monitor.set_interval(Duration::from_secs_f64(interval_s));
        }
        lock(&self.store)
            .set_interval(tid, (interval_s * 1000.0).round() as i64)
            .map_err(db)
    }

    fn order(&self, ids: &[String]) -> Result<(), String> {
        let keys: Vec<RowKey> = ids.iter().filter_map(|id| row_key(id)).collect();
        lock(&self.store).set_order(&keys).map_err(db)?;
        self.reload_positions()
    }

    fn pin(&self, target_id: &str, ttl: u8) -> Result<IdRes, String> {
        let tid = self.target_of(target_id)?;
        let (data, probe) = {
            let targets = lock(&self.targets);
            let t = targets.get(&tid).ok_or("no_such_target")?;
            (t.data.clone(), t.probe)
        };
        if ttl == 0 || ttl > whilo_trace_engine::probe::MAX_TTL {
            return Err("no_such_hop".into());
        }
        // A hop that never answered has nothing to show on its own timeline.
        let addr = lock(&data)
            .hist
            .addr_at(ttl, PIN_LOOK_BACK)
            .ok_or("no_such_hop")?
            .to_string();
        let pin: PinRow = {
            let s = lock(&self.store);
            let id = s
                .pin_hop(tid, ttl, probe.as_str(), Some(&addr))
                .map_err(db)?;
            s.pins()
                .map_err(db)?
                .into_iter()
                .find(|p| p.id == id)
                .ok_or("no_such_hop")?
        };
        lock(&self.pins).insert(
            pin.id,
            Pin {
                target_id: pin.target_id,
                ttl: pin.ttl,
                addr: pin.addr,
                position: pin.position,
            },
        );
        Ok(IdRes {
            id: pin_row_id(pin.id),
        })
    }

    fn unpin(&self, id: i64) -> Result<(), String> {
        let known = lock(&self.pins).remove(&id).is_some();
        let deleted = lock(&self.store).unpin_hop(id).map_err(db)?;
        if known || deleted {
            Ok(())
        } else {
            Err("no_such_hop".into())
        }
    }

    fn change_settings(&self, patch: &SettingsPatch) -> Result<Settings, String> {
        let mut s = lock(&self.settings);
        let changed = apply_settings(&mut s, patch);
        let store = lock(&self.store);
        for (key, value) in changed {
            store.set_setting(key, &value).map_err(db)?;
        }
        Ok(s.clone())
    }
}

#[tauri::command(async)]
fn add_target(core: State<'_, Arc<Core>>, req: AddTargetReq) -> Result<IdRes, String> {
    core.add(&req)
}

#[tauri::command]
fn remove_target(core: State<'_, Arc<Core>>, id: String) -> Result<(), String> {
    core.remove(&id)
}

#[tauri::command]
fn set_paused(core: State<'_, Arc<Core>>, id: String, paused: bool) -> Result<(), String> {
    core.pause(&id, paused)
}

#[tauri::command]
fn set_interval(core: State<'_, Arc<Core>>, id: String, interval_s: f64) -> Result<(), String> {
    core.set_interval(&id, interval_s)
}

#[tauri::command]
fn set_order(core: State<'_, Arc<Core>>, ids: Vec<String>) -> Result<(), String> {
    core.order(&ids)
}

#[tauri::command]
fn pin_hop(core: State<'_, Arc<Core>>, target_id: String, ttl: u8) -> Result<IdRes, String> {
    core.pin(&target_id, ttl)
}

#[tauri::command]
fn unpin_hop(core: State<'_, Arc<Core>>, id: String) -> Result<(), String> {
    match row_key(&id) {
        Some(RowKey::Pin(p)) => core.unpin(p),
        _ => Err("no_such_hop".into()),
    }
}

#[tauri::command]
fn get_settings(core: State<'_, Arc<Core>>) -> Settings {
    lock(&core.settings).clone()
}

/// Applies the valid fields of a patch; returns what changed as key and JSON text.
fn apply_settings(s: &mut Settings, p: &SettingsPatch) -> Vec<(&'static str, String)> {
    let ms = |v: f64| v.is_finite() && v > 0.0 && v < 1e6;
    let mut changed = Vec::new();
    if let Some(v) = p.warn_ms.filter(|v| ms(*v)) {
        s.warn_ms = v;
        changed.push(("warnMs", v.to_string()));
    }
    if let Some(v) = p.bad_ms.filter(|v| ms(*v)) {
        s.bad_ms = v;
        changed.push(("badMs", v.to_string()));
    }
    if let Some(v) = p
        .theme
        .as_deref()
        .filter(|v| ["system", "light", "dark"].contains(v))
    {
        s.theme = v.into();
        changed.push(("theme", format!("\"{v}\"")));
    }
    if let Some(v) = p.lang.as_deref().filter(|v| ["ru", "en"].contains(v)) {
        s.lang = v.into();
        changed.push(("lang", format!("\"{v}\"")));
    }
    if let Some(v) = p
        .window
        .as_deref()
        .filter(|v| WINDOWS.iter().any(|(w, _)| w == v))
    {
        s.window = v.into();
        changed.push(("window", format!("\"{v}\"")));
    }
    changed
}

/// Saved settings over the defaults; a value that does not fit is left at its default.
fn load_settings(saved: &[(String, String)]) -> Settings {
    let mut p = SettingsPatch::default();
    for (key, value) in saved {
        let v: serde_json::Value = serde_json::from_str(value).unwrap_or_default();
        match key.as_str() {
            "warnMs" => p.warn_ms = v.as_f64(),
            "badMs" => p.bad_ms = v.as_f64(),
            "theme" => p.theme = v.as_str().map(String::from),
            "lang" => p.lang = v.as_str().map(String::from),
            "window" => p.window = v.as_str().map(String::from),
            _ => {}
        }
    }
    let mut s = Settings::default();
    apply_settings(&mut s, &p);
    s
}

#[tauri::command]
fn set_settings(core: State<'_, Arc<Core>>, settings: SettingsPatch) -> Result<Settings, String> {
    core.change_settings(&settings)
}

#[tauri::command]
fn recent_inputs(core: State<'_, Arc<Core>>) -> Vec<String> {
    lock(&core.recent).clone()
}

/// What a report needs of one target, owned, so the report can borrow it.
struct ReportData {
    host: String,
    probe: ProbeKind,
    interval_s: f64,
    rounds: Vec<RoundRecord>,
    hops: Vec<HopStats>,
    names: Vec<Option<String>>,
    timeline: Vec<Bucket>,
    verdict: Verdict,
    pinned: Vec<PinnedLine>,
}

fn safe_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

impl Core {
    /// Rounds of `[from_ms, to_ms)`: from memory when it has them, else from disk.
    fn rounds_of(
        &self,
        data: &Mutex<TargetData>,
        id: i64,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<RoundRecord>, String> {
        {
            let mut d = lock(data);
            if from_ms >= d.hist.complete_from() {
                return Ok(d.hist.rounds_between(from_ms, to_ms).to_vec());
            }
        }
        lock(&self.reader)
            .load_rounds(id, from_ms, to_ms)
            .map_err(db)
    }

    /// Writes the report into a new folder under `req.dir`, or under `<docs>/Whilo Trace`.
    fn export(&self, req: ReportReq, docs: Option<PathBuf>) -> Result<ExportRes, String> {
        let (from_ms, to_ms) = (req.from_ms.min(req.to_ms), req.from_ms.max(req.to_ms));
        if to_ms <= from_ms {
            return Err("io: empty period".into());
        }
        // Targets in the order of the rows, with the hops pinned from them.
        let mut wanted: Vec<(i64, Vec<u8>)> = Vec::new();
        for row in &req.rows {
            let (tid, ttl) = match row_key(row) {
                Some(RowKey::Target(t)) => (t, None),
                Some(RowKey::Pin(p)) => match lock(&self.pins).get(&p) {
                    Some(pin) => (pin.target_id, Some(pin.ttl)),
                    None => continue,
                },
                None => continue,
            };
            let i = match wanted.iter().position(|(t, _)| *t == tid) {
                Some(i) => i,
                None => {
                    wanted.push((tid, Vec::new()));
                    wanted.len() - 1
                }
            };
            if let Some(ttl) = ttl {
                if !wanted[i].1.contains(&ttl) {
                    wanted[i].1.push(ttl);
                }
            }
        }
        let mut datas: Vec<ReportData> = Vec::new();
        for (tid, ttls) in wanted {
            let (host, probe, interval_s, data) = {
                let targets = lock(&self.targets);
                let Some(t) = targets.get(&tid) else { continue };
                (t.host.clone(), t.probe, t.interval_s, t.data.clone())
            };
            let rounds = self.rounds_of(&data, tid, from_ms, to_ms)?;
            let hops = hop_stats(&rounds);
            let v = verdict(&rounds, &hops);
            let n = report::bucket_count(to_ms - from_ms, interval_s, 300);
            let line = timeline(&rounds, None, from_ms, to_ms, n);
            let pinned = ttls
                .into_iter()
                .map(|ttl| {
                    let addr = hops.iter().find(|h| h.ttl == ttl).and_then(|h| h.addr);
                    PinnedLine {
                        ttl,
                        addr,
                        name: addr.and_then(|a| self.dns.name(a)),
                        timeline: timeline(&rounds, Some(ttl), from_ms, to_ms, n),
                    }
                })
                .collect();
            datas.push(ReportData {
                host,
                probe,
                interval_s,
                rounds,
                hops,
                names: Vec::new(),
                timeline: line,
                verdict: v,
                pinned,
            });
        }
        if datas.is_empty() {
            return Err("no_such_target".into());
        }
        // Reverse DNS runs in the background; give pending names a moment.
        let ask = |d: &ReportData| -> Vec<Option<String>> {
            d.hops
                .iter()
                .map(|h| h.addr.and_then(|a| self.dns.name(a)))
                .collect()
        };
        let mut names: Vec<Vec<Option<String>>> = datas.iter().map(ask).collect();
        if names.iter().flatten().any(Option::is_none) {
            std::thread::sleep(Duration::from_millis(600));
            names = datas.iter().map(ask).collect();
        }
        for (d, n) in datas.iter_mut().zip(names) {
            d.names = n;
        }

        let generated_ms = now_ms();
        let lang = lock(&self.settings).lang.clone();
        let doc = Document {
            sections: datas
                .iter()
                .map(|d| Section {
                    report: Report {
                        host: &d.host,
                        dst: d.rounds.last().map(|r| r.dst),
                        probe: d.probe,
                        from_ms,
                        to_ms,
                        tz_offset_min: req.tz_offset_min,
                        interval_s: d.interval_s,
                        hops: &d.hops,
                        names: &d.names,
                        timeline: &d.timeline,
                        verdict: &d.verdict,
                        generated_ms,
                    },
                    pinned: d
                        .pinned
                        .iter()
                        .map(|p| PinnedLine {
                            ttl: p.ttl,
                            addr: p.addr,
                            name: p.name.clone(),
                            timeline: p.timeline.clone(),
                        })
                        .collect(),
                })
                .collect(),
            from_ms,
            to_ms,
            tz_offset_min: req.tz_offset_min,
            generated_ms,
            // Exactly what the dialog shows, in its language; none when it shows none.
            findings: Some(req.findings.clone()),
            lang: &lang,
        };

        let formats: Vec<&str> = {
            let known: Vec<&str> = ["html", "png", "csv", "json"]
                .into_iter()
                .filter(|f| req.formats.iter().any(|x| x == f))
                .collect();
            if known.is_empty() {
                vec!["html"]
            } else {
                known
            }
        };
        let mut files: Vec<(&str, Vec<u8>)> = Vec::new();
        for f in formats {
            match f {
                "html" => files.push(("report.html", report::document_html(&doc).into_bytes())),
                "csv" => {
                    files.push(("hops.csv", report::document_hops_csv(&doc).into_bytes()));
                    let samples: Vec<(&str, &[RoundRecord])> = datas
                        .iter()
                        .map(|d| (d.host.as_str(), d.rounds.as_slice()))
                        .collect();
                    files.push((
                        "samples.csv",
                        report::document_samples_csv(&samples).into_bytes(),
                    ));
                }
                "json" => files.push(("report.json", report::document_json(&doc).into_bytes())),
                "png" => {
                    if let Some(png) = req.png_base64.as_deref().and_then(decode_base64) {
                        files.push(("timeline.png", png));
                    }
                }
                _ => {}
            }
        }

        let base: PathBuf = match req.dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            Some(dir) => PathBuf::from(dir),
            None => docs.ok_or("io: no Documents folder")?.join("Whilo Trace"),
        };
        let name = match datas.as_slice() {
            [one] => safe_name(&one.host),
            _ => "whilo-trace".to_string(),
        };
        let stem = format!("{name}-{}", report::file_stamp(to_ms, req.tz_offset_min));
        let mut dir = base.join(&stem);
        let mut k = 2;
        while dir.exists() {
            dir = base.join(format!("{stem}-{k}"));
            k += 1;
        }
        std::fs::create_dir_all(&dir).map_err(io)?;
        let mut written = Vec::new();
        for (name, body) in files {
            std::fs::write(dir.join(name), body).map_err(io)?;
            written.push(name.to_string());
        }
        Ok(ExportRes {
            dir: dir.display().to_string(),
            files: written,
        })
    }
}

#[tauri::command(async)]
fn export_report(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    req: ReportReq,
) -> Result<ExportRes, String> {
    core.export(req, app.path().document_dir().ok())
}

/// Folder dialog for the report. Runs off the main thread: the dialog blocks until it closes.
#[tauri::command]
async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let start = app
        .path()
        .document_dir()
        .ok()
        .map(|d| d.join("Whilo Trace"))
        .filter(|d| d.is_dir())
        .or_else(|| app.path().document_dir().ok());
    let title = match app.try_state::<Arc<Core>>() {
        Some(core) if lock(&core.settings).lang == "en" => "Folder for the report",
        _ => "Папка для отчета",
    };
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = app.dialog().file().set_title(title);
        if let Some(window) = app.get_webview_window("main") {
            dialog = dialog.set_parent(&window);
        }
        if let Some(dir) = start {
            dialog = dialog.set_directory(dir);
        }
        dialog.blocking_pick_folder()
    })
    .await
    .map_err(io)?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string()))
}

/// Files a report writes; `open_folder` opens no other kind of file.
const REPORT_FILES: [&str; 4] = ["html", "png", "csv", "json"];

/// What `open_folder` may open: a folder, or a file of a report (opened in its default program).
fn openable(path: &str) -> Result<PathBuf, String> {
    let path = path.trim();
    #[cfg(windows)]
    let path = path.replace('/', "\\");
    let p = PathBuf::from(path);
    let report_file = || {
        p.is_file()
            && p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| REPORT_FILES.iter().any(|x| e.eq_ignore_ascii_case(x)))
    };
    if p.is_dir() || report_file() {
        Ok(p)
    } else {
        Err(io(format!("not a report folder or file: {}", p.display())))
    }
}

/// Shows a report folder, or opens a file of it: the report dialog opens report.html this way.
#[tauri::command]
fn open_folder(path: String) -> Result<(), String> {
    let p = openable(&path)?;
    #[cfg(windows)]
    let cmd = "explorer";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = "xdg-open";
    std::process::Command::new(cmd)
        .arg(&p)
        .spawn()
        .map(|_| ())
        .map_err(io)
}

/// Accepts a plain base64 string or a `data:` URL.
fn decode_base64(s: &str) -> Option<Vec<u8>> {
    let s = s.rsplit(',').next()?;
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let bytes: Vec<u8> = s
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
        .collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            acc |= val(c)? << (18 - 6 * i);
        }
        let n = chunk.len().saturating_sub(1);
        out.extend_from_slice(&acc.to_be_bytes()[1..1 + n]);
    }
    Some(out)
}

/// The window behind the page in the page's own background (`--bg` in ui/base/tokens.css): its edges stay in
/// the theme while the window grows and before the first paint, instead of flashing white.
fn window_background(dark: bool) -> tauri::window::Color {
    if dark {
        tauri::window::Color(0x1C, 0x1C, 0x1C, 255)
    } else {
        tauri::window::Color(0xED, 0xED, 0xED, 255)
    }
}

/// The page tells which theme it shows (the "system" setting follows the system theme).
#[tauri::command]
fn set_background(window: tauri::WebviewWindow, dark: bool) -> Result<(), String> {
    window
        .set_background_color(Some(window_background(dark)))
        .map_err(|e| format!("io: {e}"))
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let notify: Notify = Arc::new(move |id| {
                let _ = handle.emit("round", id);
            });
            let core = Core::open(default_db_path(), notify)?;
            core.start_saved()?;
            if let Some(window) = app.get_webview_window("main") {
                let theme = lock(&core.settings).theme.clone();
                let dark = match theme.as_str() {
                    "dark" => true,
                    "light" => false,
                    _ => matches!(window.theme(), Ok(tauri::Theme::Dark)),
                };
                let _ = window.set_background_color(Some(window_background(dark)));
            }
            app.manage(core);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            add_target,
            remove_target,
            set_paused,
            set_interval,
            set_order,
            pin_hop,
            unpin_hop,
            get_settings,
            set_settings,
            recent_inputs,
            export_report,
            pick_folder,
            open_folder,
            set_background
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Whilo Trace");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_base64() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64("data:image/png;base64,aGk=").unwrap(), b"hi");
        assert_eq!(decode_base64("YWJj").unwrap(), b"abc");
    }

    #[test]
    fn scale_tops_are_round_numbers() {
        assert_eq!(scale_top(&[Some(7.8), None]), 10.0);
        assert_eq!(scale_top(&[Some(16.5)]), 20.0);
        assert_eq!(scale_top(&[Some(128.0)]), 150.0);
        assert_eq!(scale_top(&[Some(231.0)]), 250.0);
        assert_eq!(scale_top(&[Some(0.6)]), 1.0);
        assert_eq!(scale_top(&[None]), 10.0);
    }

    #[test]
    fn intervals_snap_to_the_offered_ones() {
        assert_eq!(snap_interval(2.5), 2.5);
        assert_eq!(snap_interval(0.4), 0.5);
        assert_eq!(snap_interval(7.0), 5.0);
        assert_eq!(snap_interval(1000.0), 60.0);
        assert_eq!(snap_interval(f64::NAN), 2.5);
    }

    #[test]
    fn the_interface_may_set_only_offered_intervals() {
        for s in INTERVALS {
            assert_eq!(offered_interval(s), Ok(s));
        }
        for s in [0.0, 0.4, 2.0, 7.0, 120.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(offered_interval(s).unwrap_err(), "invalid_interval", "{s}");
        }
    }

    #[test]
    fn monitor_messages_become_error_codes() {
        // The texts of the engine's MonitorEvent::Error.
        let no_v4 = whilo_trace_engine::monitor::resolve_v4("::1").unwrap_err();
        assert_eq!(resolve_code(&no_v4), "no_ipv4");
        assert_eq!(
            error_code(&format!("cannot resolve ::1: {no_v4}")),
            "no_ipv4"
        );
        assert_eq!(
            error_code("cannot resolve example.invalid: No such host is known. (os error 11001)"),
            "resolve_failed"
        );
        assert_eq!(
            error_code("cannot open ICMP: Access is denied. (os error 5)"),
            "probe_unavailable: Access is denied. (os error 5)"
        );
        assert_eq!(
            error_code("probe to 1.1.1.1 failed: The handle is invalid. (os error 6)"),
            "probe_unavailable: The handle is invalid. (os error 6)"
        );
        assert_eq!(error_code("cannot open ICMP"), "probe_unavailable");
        assert_eq!(error_code("something else"), "io: something else");
    }

    #[test]
    fn row_ids_round_trip() {
        assert_eq!(row_key("t12"), Some(RowKey::Target(12)));
        assert_eq!(row_key("p7"), Some(RowKey::Pin(7)));
        assert_eq!(row_key("x1"), None);
        assert_eq!(row_key("t"), None);
    }

    #[test]
    fn settings_ignore_values_that_do_not_fit() {
        let s = load_settings(&[
            ("warnMs".into(), "80".into()),
            ("badMs".into(), "-1".into()),
            ("theme".into(), "\"dark\"".into()),
            ("lang".into(), "\"de\"".into()),
            ("window".into(), "\"1h\"".into()),
        ]);
        assert_eq!(s.warn_ms, 80.0);
        assert_eq!(s.bad_ms, 200.0);
        assert_eq!(s.theme, "dark");
        assert_eq!(s.lang, "ru");
        assert_eq!(s.window, "1h");
    }

    use std::net::Ipv4Addr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;
    use whilo_trace_engine::{HopSample, ReplyKind, RoundSpec};

    /// A probe backend that never touches the network: hop 1 and the destination at TTL 2
    /// answer. Counts the rounds it ran.
    struct FakeProber(Arc<AtomicUsize>);

    impl Prober for FakeProber {
        fn round(&mut self, spec: &RoundSpec) -> std::io::Result<Vec<HopSample>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            let hop = |ttl, kind, addr| HopSample {
                ttl,
                kind,
                addr: Some(IpAddr::V4(addr)),
                rtt_us: Some(500 * ttl as u32),
            };
            Ok(vec![
                hop(1, ReplyKind::TimeExceeded, Ipv4Addr::new(10, 0, 1, 1)),
                hop(2, ReplyKind::Echo, spec.dst),
            ])
        }
    }

    fn fake_backend(rounds: &Arc<AtomicUsize>) -> ProberSource {
        let rounds = rounds.clone();
        Arc::new(move || Ok(Box::new(FakeProber(rounds.clone())) as Box<dyn Prober>))
    }

    /// Stops every monitor and waits for it, so the database is closed before its folder goes.
    fn shut_down(core: Arc<Core>) {
        let targets = std::mem::take(&mut *lock(&core.targets));
        for t in targets.into_values() {
            t.monitor.stop_and_join();
        }
        drop(core);
    }

    /// Waits up to `secs` for `done`.
    fn wait_for(secs: u64, what: &str, mut done: impl FnMut() -> bool) {
        let t0 = Instant::now();
        while !done() {
            assert!(t0.elapsed() < Duration::from_secs(secs), "{what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A folder of its own for one test, removed at the end.
    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir(name: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!(
            "whilo-trace-app-test-{}-{name}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }

    /// One round: `path[i]` says whether TTL i + 1 answered; the last one is the destination.
    fn round(ts_ms: i64, path: &[bool], dst: Ipv4Addr) -> RoundRecord {
        let n = path.len() as u8;
        let hops = path
            .iter()
            .enumerate()
            .map(|(i, &ok)| {
                let ttl = i as u8 + 1;
                if !ok {
                    return HopSample::lost(ttl, ReplyKind::Timeout);
                }
                let (kind, addr) = if ttl == n {
                    (ReplyKind::Echo, dst)
                } else {
                    (ReplyKind::TimeExceeded, Ipv4Addr::new(10, 0, ttl, 1))
                };
                HopSample {
                    ttl,
                    kind,
                    addr: Some(IpAddr::V4(addr)),
                    rtt_us: Some(1000 * ttl as u32),
                }
            })
            .collect();
        RoundRecord {
            ts_ms,
            probe: ProbeKind::Icmp,
            dst: IpAddr::V4(dst),
            path_len: n,
            reached: *path.last().unwrap(),
            hops,
        }
    }

    fn req(window: &str, end_ms: Option<i64>) -> SnapshotReq {
        SnapshotReq {
            window: window.into(),
            end_ms,
            range: None,
            trace: None,
            buckets: 120,
        }
    }

    /// Saved history in, snapshots and a report out: the commands without the window.
    /// Monitors use the fake backend, so no probe leaves the machine.
    #[test]
    fn snapshot_and_report_from_a_saved_history() {
        let dir = temp_dir("core");
        let db_path = dir.0.join("history.db");
        let now = now_ms();
        let (ip_a, ip_b) = (Ipv4Addr::new(10, 0, 0, 5), Ipv4Addr::new(10, 0, 0, 9));
        {
            let mut s = Store::open(&db_path).unwrap();
            let a = s.add_target("10.0.0.5").unwrap();
            let b = s.add_target("10.0.0.9").unwrap();
            s.set_paused(a, true).unwrap();
            s.set_paused(b, true).unwrap();
            // The last 10 minutes, a round every 2.5 s. Target a: hop 2 never answers,
            // hop 3 and the destination lose rounds 20..30 together.
            for k in 0..240 {
                let ts = now - 600_000 + k * 2500;
                let lost = (20..30).contains(&k);
                s.insert_round(a, &round(ts, &[true, false, !lost, !lost], ip_a))
                    .unwrap();
                s.insert_round(b, &round(ts, &[true, true], ip_b)).unwrap();
            }
            // Two days ago: an hour of rounds, every tenth lost.
            for k in 0..60 {
                let ts = now - 2 * DAY_MS + k * 60_000;
                s.insert_round(a, &round(ts, &[true, true, true, k % 10 != 0], ip_a))
                    .unwrap();
            }
            s.pin_hop(a, 1, "icmp", Some("10.0.1.1")).unwrap();
        }

        let probes = Arc::new(AtomicUsize::new(0));
        let core = Core::open_with(
            db_path.clone(),
            Arc::new(|_| {}),
            Some(fake_backend(&probes)),
        )
        .unwrap();
        core.start_saved().unwrap();
        let datas: Vec<Arc<Mutex<TargetData>>> = lock(&core.targets)
            .values()
            .map(|t| t.data.clone())
            .collect();
        assert_eq!(datas.len(), 2);
        wait_for(20, "history did not load", || {
            datas.iter().all(|d| lock(d).hist.len() >= 240)
        });

        let mut r = req("10m", None);
        r.trace = Some("t1".into());
        let snap = core.snapshot(&r).unwrap();
        assert!(snap.live);
        assert_eq!((snap.n, snap.bucket_ms), (120, 5000.0));
        assert_eq!(snap.to_ms - snap.from_ms, 600_000);
        assert!(snap.from_ms <= now && now < snap.to_ms);
        let ids: Vec<&str> = snap.targets.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["t1", "t2", "p1"]);
        let a = &snap.targets[0];
        assert_eq!(
            (a.host.as_str(), a.ip.as_deref()),
            ("10.0.0.5", Some("10.0.0.5"))
        );
        assert!(a.paused);
        assert_eq!(a.series.len(), 120);
        assert_eq!(a.status, "loss");
        assert!(
            a.stats.sent >= 238 && a.stats.sent <= 240,
            "{}",
            a.stats.sent
        );
        assert!(!a.loss.is_empty() && a.partial.is_empty());
        // Probes per bucket: every round of the window, the 10 lost ones without an answer.
        assert_eq!((a.sent_n.len(), a.lost_n.len()), (120, 120));
        assert_eq!(a.sent_n.iter().sum::<u32>(), a.stats.sent);
        assert_eq!(a.lost_n.iter().sum::<u32>(), 10);
        assert!(
            a.stats.status.is_none(),
            "a status of its own only for a period"
        );
        let path = a.path.as_ref().unwrap();
        assert_eq!(path.hops, 4);
        assert_eq!(path.origin.as_ref().map(|o| o.ttl), Some(3));
        assert_eq!(path.silent, vec![2]);
        let b = &snap.targets[1];
        assert_eq!((b.status.as_str(), b.loss.len()), ("ok", 0));
        assert!(b.stats.avg.is_some_and(|v| (v - 2.0).abs() < 1e-9));
        let p = &snap.targets[2];
        assert_eq!((p.hop_of.as_deref(), p.hop), (Some("t1"), Some(1)));
        assert_eq!((p.host.as_str(), p.status.as_str()), ("10.0.1.1", "ok"));
        assert!(p.paused, "a pinned hop of a paused target is paused too");
        assert!(p.path.is_none());
        assert_eq!(p.sent_n.iter().sum::<u32>(), p.stats.sent);
        let trace = snap.trace.as_ref().unwrap();
        assert_eq!(trace.target, "t1");
        assert_eq!(trace.hops.len(), 4);
        let silent = &trace.hops[1];
        assert_eq!(silent.status, "silent");
        assert!(silent.series.iter().all(Option::is_none));
        assert!(silent.loss.is_empty() && silent.partial.is_empty());
        assert_eq!(silent.stats.loss_pct, 0.0, "a silent hop is not loss");
        let st = &silent.stats;
        assert_eq!(
            (st.avg, st.min, st.max, st.last, st.jitter),
            (None, None, None, None, None)
        );
        assert_eq!(silent.lost_n, silent.sent_n, "no probe answered");
        assert!(!trace.hops[2].loss.is_empty(), "loss on the path at hop 3");
        assert!(trace.hops[2].origin && trace.hops[3].dest);
        assert_eq!(
            trace.hops[2].by_probe.get("icmp").map(String::as_str),
            Some("loss")
        );
        assert_eq!(trace.rounds, a.stats.sent);
        assert!(!snap.history.loss.is_empty());
        assert_eq!(snap.history.to_ms - snap.history.from_ms, DAY_MS);
        assert_eq!(snap.rate_pps, 0.0, "paused targets send nothing");
        assert_eq!(snap.probes_available, ["icmp"]);

        // Only a target row has a trace.
        for id in ["p1", "t99", "x1", ""] {
            let mut r = req("10m", None);
            r.trace = Some(id.into());
            assert!(core.snapshot(&r).unwrap().trace.is_none(), "{id}");
        }

        // A picked period is [from, to): the bucket that starts at `to` is not in it.
        let picked = |range: [i64; 2]| {
            let mut r = req("10m", None);
            r.range = Some(range);
            r.trace = Some("t1".into());
            core.snapshot(&r).unwrap()
        };
        let rounds_in = |from: i64, to: i64| {
            (0..240)
                .map(|k| now - 600_000 + k * 2500)
                .filter(|ts| (from..to).contains(ts))
                .count() as u32
        };
        let f0 = snap.from_ms;
        for range in [[f0 + 60_000, f0 + 70_000], [f0 + 70_000, f0 + 60_000]] {
            let rs = picked(range).targets[0].range_stats.clone().unwrap();
            assert_eq!(rs.sent, 4, "two buckets of two rounds, {range:?}");
        }
        // The loss of rounds 20..30 is in this period: statuses for it follow the window's rules.
        let lossy = picked([f0 + 30_000, f0 + 90_000]);
        let rs = |row: &Row| row.range_stats.clone().unwrap();
        let ta = &lossy.targets[0];
        assert_eq!(rs(ta).sent, rounds_in(f0 + 30_000, f0 + 90_000));
        assert_eq!(rs(ta).status.as_deref(), Some("loss"));
        assert!((rs(ta).loss_pct - 100.0 * 10.0 / rs(ta).sent as f64).abs() < 1e-3);
        assert_eq!(rs(&lossy.targets[1]).status.as_deref(), Some("ok"));
        assert_eq!(
            rs(&lossy.targets[2]).status.as_deref(),
            Some("ok"),
            "pinned hop 1"
        );
        let hops = &lossy.trace.as_ref().unwrap().hops;
        let hop_st = |i: usize| hops[i].range_stats.clone().unwrap();
        assert_eq!(hop_st(1).status.as_deref(), Some("silent"));
        assert_eq!(hop_st(1).loss_pct, 0.0);
        assert_eq!(hop_st(2).status.as_deref(), Some("loss"));
        assert_eq!(hop_st(3).status.as_deref(), Some("loss"));
        // The last minute has no loss: the same target is fine there.
        let clean = picked([snap.to_ms - 60_000, snap.to_ms]);
        assert_eq!(rs(&clean.targets[0]).status.as_deref(), Some("ok"));
        let hops = &clean.trace.as_ref().unwrap().hops;
        assert_eq!(
            hops[2].range_stats.as_ref().unwrap().status.as_deref(),
            Some("ok")
        );
        // A period outside the window has no rounds.
        let outside = picked([f0 - 20_000, f0 - 10_000]);
        let rs0 = rs(&outside.targets[0]);
        assert_eq!((rs0.sent, rs0.status.as_deref()), (0, Some("ok")));
        assert!(rs0.avg.is_none());

        // Order, pins, interval and settings are kept in the database.
        core.order(&["p1".into(), "t2".into()]).unwrap();
        let ids: Vec<String> = core
            .snapshot(&req("10m", None))
            .unwrap()
            .targets
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, ["p1", "t2", "t1"]);
        assert_eq!(
            core.pin("t1", 2).unwrap_err(),
            "no_such_hop",
            "a silent hop"
        );
        assert_eq!(core.pin("t1", 3).unwrap().id, "p2");
        assert_eq!(core.pin("t1", 3).unwrap().id, "p2", "pinned once");
        assert_eq!(core.pin("t9", 1).unwrap_err(), "no_such_target");
        assert_eq!(
            core.set_interval("t2", 7.0).unwrap_err(),
            "invalid_interval"
        );
        assert_eq!(core.set_interval("t9", 5.0).unwrap_err(), "no_such_target");
        core.set_interval("t2", 5.0).unwrap();
        core.pause("t2", false).unwrap();
        core.pause("t2", true).unwrap();
        // What add_target refuses before any lookup.
        let add = |host: &str, interval_s: Option<f64>| {
            core.add(&AddTargetReq {
                host: host.into(),
                probe: None,
                interval_s,
            })
        };
        assert_eq!(add("bad host", None).unwrap_err(), "invalid_host");
        assert_eq!(add("10.0.0.11", Some(3.0)).unwrap_err(), "invalid_interval");
        assert_eq!(add("::1", Some(1.0)).unwrap_err(), "no_ipv4");
        assert_eq!(add("10.0.0.9", None).unwrap().id, "t2", "already there");
        let s = core
            .change_settings(&SettingsPatch {
                lang: Some("en".into()),
                warn_ms: Some(80.0),
                theme: Some("blue".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            (s.lang.as_str(), s.warn_ms, s.theme.as_str()),
            ("en", 80.0, "system")
        );
        {
            let st = Store::open(&db_path).unwrap();
            let t2 = st
                .targets()
                .unwrap()
                .into_iter()
                .find(|t| t.id == 2)
                .unwrap();
            assert_eq!((t2.interval_ms, t2.paused), (5000, true));
            assert_eq!(load_settings(&st.settings().unwrap()).warn_ms, 80.0);
            assert_eq!(st.pins().unwrap().len(), 2);
        }

        // Two days back: older than memory, read from disk.
        let end = now - 2 * DAY_MS + 3_600_000;
        let past = core.snapshot(&req("1h", Some(end))).unwrap();
        assert!(!past.live);
        assert!(past.to_ms >= end && past.to_ms - end < 30_000);
        let pa = past.targets.iter().find(|r| r.id == "t1").unwrap();
        assert!(pa.stats.sent >= 59, "{}", pa.stats.sent);
        assert!(
            (pa.stats.loss_pct - 10.0).abs() < 2.0,
            "{}",
            pa.stats.loss_pct
        );
        assert!(!past.history.loss.is_empty());
        let again = core.snapshot(&req("1h", Some(end))).unwrap();
        assert_eq!(again.targets[0].series, past.targets[0].series);

        // A report of two targets and a pinned hop.
        let report_req = || ReportReq {
            rows: vec!["t1".into(), "p1".into(), "t2".into()],
            from_ms: now - 600_000,
            to_ms: now,
            formats: vec!["html".into(), "csv".into(), "json".into(), "png".into()],
            tz_offset_min: 240,
            findings: vec!["Потери начинаются на хопе 3.".into()],
            png_base64: Some("data:image/png;base64,aGk=".into()),
            dir: Some(dir.0.display().to_string()),
        };
        let res = core.export(report_req(), None).unwrap();
        assert_eq!(
            res.files,
            [
                "report.html",
                "timeline.png",
                "hops.csv",
                "samples.csv",
                "report.json"
            ]
        );
        // A new folder inside the picked one, named like the core names it for the interface.
        let out = PathBuf::from(&res.dir);
        assert_eq!(out.parent(), Some(dir.0.as_path()));
        let stem = out.file_name().unwrap().to_string_lossy().to_string();
        assert!(stem.starts_with("whilo-trace-"), "{stem}");
        let html = std::fs::read_to_string(out.join("report.html")).unwrap();
        assert!(html.contains("10.0.0.5") && html.contains("10.0.0.9"));
        assert!(html.contains("<li>Потери начинаются на хопе 3.</li>"));
        assert!(!html.contains("Packet loss"), "no findings of the engine");
        assert!(html.contains("Network path report"));
        assert!(html.contains("Hop 1: <span class=\"mono\">10.0.1.1</span>"));
        assert!(html.contains("<span class=\"badge\">No reply to ICMP</span>"));
        let csv = std::fs::read_to_string(out.join("hops.csv")).unwrap();
        assert!(csv.starts_with("target,ttl,"));
        assert!(csv.contains("\n10.0.0.9,2,10.0.0.9,"));
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out.join("report.json")).unwrap())
                .unwrap();
        assert_eq!(json["targets"].as_array().unwrap().len(), 2);
        assert_eq!(json["targets"][0]["pinned"][0]["ttl"], 1);
        assert_eq!(json["findings"][0], "Потери начинаются на хопе 3.");
        assert!(json["targets"][0].get("findings").is_none());
        assert_eq!(std::fs::read(out.join("timeline.png")).unwrap(), b"hi");
        let res2 = core.export(report_req(), None).unwrap();
        assert_ne!(
            res2.dir, res.dir,
            "a second report does not overwrite the first"
        );
        // One target, no findings in the dialog: the folder has its name, the report no findings.
        let one = core
            .export(
                ReportReq {
                    rows: vec!["t2".into()],
                    findings: Vec::new(),
                    formats: vec!["html".into(), "json".into()],
                    ..report_req()
                },
                None,
            )
            .unwrap();
        let out = PathBuf::from(&one.dir);
        let stem = out.file_name().unwrap().to_string_lossy().to_string();
        assert!(stem.starts_with("10.0.0.9-"), "{stem}");
        let html = std::fs::read_to_string(out.join("report.html")).unwrap();
        assert!(!html.contains("class=\"findings\"") && !html.contains("No notable loss"));
        let json = std::fs::read_to_string(out.join("report.json")).unwrap();
        assert!(!json.contains("No notable loss"));

        // Removing a target takes its pinned hops along.
        core.remove("t1").unwrap();
        let ids: Vec<String> = core
            .snapshot(&req("10m", None))
            .unwrap()
            .targets
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, ["t2"]);
        assert_eq!(core.remove("t1").unwrap_err(), "no_such_target");
        shut_down(core);
    }

    #[test]
    fn live_frame_ends_with_the_bucket_of_now() {
        let frame = |range: [i64; 2]| {
            let req = SnapshotReq {
                window: "10m".into(),
                end_ms: None,
                range: Some(range),
                trace: None,
                buckets: 120,
            };
            Frame::new(&req, 1_600_002)
        };
        let f = frame([1_000_000 + 7_000, 1_000_000 + 2_000]);
        assert_eq!(f.b, 5000);
        assert_eq!(f.to_ms, 1_605_000);
        assert_eq!(f.from_ms, 1_005_000);
        assert_eq!(f.range, Some(0..1), "cut to the window");
        // Half-open: a period that ends where a bucket starts does not take that bucket.
        assert_eq!(frame([1_010_000, 1_020_000]).range, Some(1..3));
        assert_eq!(frame([1_010_000, 1_020_001]).range, Some(1..4));
        assert_eq!(frame([1_012_500, 1_013_000]).range, Some(1..2));
        assert_eq!(frame([1_600_000, 1_700_000]).range, Some(119..120));
        assert_eq!(
            frame([900_000, 1_000_000]).range,
            Some(0..0),
            "before the window"
        );
        assert_eq!(frame([1_010_000, 1_010_000]).range, Some(0..0), "empty");
    }

    /// Saved targets start as they were saved: a paused one sends nothing until it is resumed,
    /// at its saved interval. What a monitor reports comes back as an error code of the row.
    #[test]
    fn saved_targets_start_as_saved() {
        let dir = temp_dir("start");
        let db_path = dir.0.join("history.db");
        {
            let s = Store::open(&db_path).unwrap();
            let a = s.add_target_with("10.0.0.7", "icmp", Some(500)).unwrap();
            s.set_paused(a, true).unwrap();
            // A literal IPv6 address: the lookup needs no DNS and finds no IPv4 address.
            s.add_target("::1").unwrap();
        }
        let probes = Arc::new(AtomicUsize::new(0));
        let core = Core::open_with(db_path, Arc::new(|_| {}), Some(fake_backend(&probes))).unwrap();
        core.start_saved().unwrap();
        let row = |host: &str| {
            core.snapshot(&req("10m", None))
                .unwrap()
                .targets
                .into_iter()
                .find(|t| t.host == host)
                .unwrap()
        };
        wait_for(10, "no error from the monitor", || {
            row("::1").error.is_some()
        });
        assert_eq!(row("::1").error.as_deref(), Some("no_ipv4"));
        let a = row("10.0.0.7");
        assert!(a.paused && a.error.is_none());
        assert_eq!(a.interval, 0.5);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            probes.load(Ordering::SeqCst),
            0,
            "a paused target sent probes"
        );
        core.pause(&a.id, false).unwrap();
        wait_for(10, "no rounds after resume", || {
            probes.load(Ordering::SeqCst) >= 2
        });
        wait_for(10, "rounds did not reach the row", || {
            row("10.0.0.7").stats.sent >= 1
        });
        core.pause(&a.id, true).unwrap();

        // A backend that cannot open: the row says so with the system's words.
        let dir2 = temp_dir("noprobe");
        let db2 = dir2.0.join("history.db");
        Store::open(&db2).unwrap().add_target("10.0.0.8").unwrap();
        let refuse: ProberSource = Arc::new(|| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "no ICMP here",
            ))
        });
        let core2 = Core::open_with(db2, Arc::new(|_| {}), Some(refuse)).unwrap();
        core2.start_saved().unwrap();
        let error = || {
            core2.snapshot(&req("10m", None)).unwrap().targets[0]
                .error
                .clone()
        };
        wait_for(10, "no error from the backend", || error().is_some());
        assert_eq!(error().as_deref(), Some("probe_unavailable: no ICMP here"));
        shut_down(core);
        shut_down(core2);
    }

    #[test]
    fn only_report_folders_and_files_open() {
        let dir = temp_dir("open");
        let page = dir.0.join("report.html");
        let tool = dir.0.join("tool.exe");
        std::fs::write(&page, "<p>").unwrap();
        std::fs::write(&tool, "MZ").unwrap();
        fn path(p: &std::path::Path) -> String {
            p.display().to_string()
        }
        assert_eq!(openable(&path(&dir.0)).unwrap(), dir.0);
        assert_eq!(openable(&path(&page)).unwrap(), page);
        assert!(openable(&path(&tool)).unwrap_err().starts_with("io: "));
        assert!(openable(&path(&dir.0.join("gone.html"))).is_err());
        assert!(openable("").is_err());
        #[cfg(windows)]
        assert_eq!(openable(&path(&page).replace('\\', "/")).unwrap(), page);
    }
}
