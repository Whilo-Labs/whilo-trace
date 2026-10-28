//! One planner thread per target: it resolves the host, starts a round every interval and
//! reports the rounds in the order they started.
//!
//! Each round runs on a worker thread with its own probe backend (its own ICMP handle or
//! socket), so rounds overlap: a short interval holds even when silent hops keep every round
//! waiting for the timeout. Workers are opened only when all others are busy, up to what the
//! longest possible round needs at the current interval.

use crate::probe::{new_prober, HopSample, Prober, ReplyKind, RoundSpec, MAX_TTL};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How often a hostname is resolved again, so targets behind dynamic DNS are followed.
const RESOLVE_EVERY: Duration = Duration::from_secs(300);
/// Wait before a failed re-resolve is tried again; the old address is probed meanwhile.
const RESOLVE_RETRY: Duration = Duration::from_secs(30);
/// Shortest pause between round starts, so a zero interval cannot flood the network.
pub const MIN_INTERVAL: Duration = Duration::from_millis(50);
/// Most rounds of one target in flight at once, whatever the interval.
const MAX_IN_FLIGHT: usize = 16;
/// A spare worker idle for this long is closed.
const IDLE_WORKER: Duration = Duration::from_secs(30);
/// What a round may take on top of its sends and its timeout before the backend gives up.
const ROUND_SLACK: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub struct MonitorConfig {
    pub host: String,
    /// Pause between round starts; `MonitorHandle::set_interval` changes it on the go.
    pub interval: Duration,
    /// How long a round waits for its slowest probe. Rounds overlap, so it may exceed `interval`.
    pub timeout: Duration,
    pub max_ttl: u8,
    pub payload: u16,
    pub send_gap: Duration,
}

impl MonitorConfig {
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            interval: Duration::from_millis(2500),
            timeout: Duration::from_millis(2000),
            max_ttl: 30,
            payload: 32,
            send_gap: Duration::from_millis(25),
        }
    }
}

/// Opens a probe backend. A monitor opens one per round in flight.
pub type ProberFactory = Box<dyn Fn() -> io::Result<Box<dyn Prober>> + Send>;

/// How a monitor starts; `spawn_monitor` takes the defaults. Write it as
/// `MonitorOptions { paused: true, ..Default::default() }`, so new fields do not break callers.
#[derive(Default)]
pub struct MonitorOptions {
    /// Start paused: no probe goes out until `set_paused(false)`.
    pub paused: bool,
    /// Probe backend; `None` is the platform one (`probe::new_prober`). Tests pass a fake.
    pub prober: Option<ProberFactory>,
}

/// What a round was sent with. Only ICMP echo exists for now; TCP and UDP come later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProbeKind {
    #[default]
    Icmp,
}

impl ProbeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProbeKind::Icmp => "icmp",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "icmp" => Some(ProbeKind::Icmp),
            _ => None,
        }
    }
}

/// Everything one round found on the way to the target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundRecord {
    /// When the round started, ms since the Unix epoch.
    pub ts_ms: i64,
    #[serde(default)]
    pub probe: ProbeKind,
    pub dst: IpAddr,
    /// TTL of the destination: where it answered, or where it is expected when it did not.
    pub path_len: u8,
    /// The destination answered in this round.
    pub reached: bool,
    /// One sample per TTL, `hops[i].ttl == i + 1`.
    pub hops: Vec<HopSample>,
}

impl RoundRecord {
    pub fn hop(&self, ttl: u8) -> Option<&HopSample> {
        let i = (ttl as usize).checked_sub(1)?;
        match self.hops.get(i) {
            Some(h) if h.ttl == ttl => Some(h),
            _ => self.hops.iter().find(|h| h.ttl == ttl),
        }
    }

    /// The destination's answer, or `None` when it was lost this round.
    pub fn destination(&self) -> Option<&HopSample> {
        if self.reached {
            self.hop(self.path_len)
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub enum MonitorEvent {
    Round(RoundRecord),
    Error(String),
}

pub struct MonitorHandle {
    ctl: Sender<Msg>,
    thread: Option<JoinHandle<()>>,
}

impl MonitorHandle {
    /// Asks the monitor to finish: no new round starts, rounds in flight are still reported,
    /// then the thread ends. Does not wait for it.
    pub fn stop(&self) {
        let _ = self.ctl.send(Msg::Stop);
    }

    /// Stops sending probes until resumed; the thread stays. Rounds already in flight are
    /// still reported. After a resume the next round starts one interval after the last one,
    /// or at once if that time has passed.
    pub fn set_paused(&self, paused: bool) {
        let _ = self.ctl.send(Msg::Pause(paused));
    }

    /// Changes the pause between round starts from the next round on: the next round starts
    /// one new interval after the last one, or at once if that time has passed.
    /// Values below `MIN_INTERVAL` are raised to it.
    pub fn set_interval(&self, interval: Duration) {
        let _ = self.ctl.send(Msg::Interval(interval));
    }

    /// Stops the monitor and waits until the rounds in flight are reported.
    pub fn stop_and_join(mut self) {
        self.stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for MonitorHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn spawn_monitor<F>(cfg: MonitorConfig, on_event: F) -> io::Result<MonitorHandle>
where
    F: FnMut(MonitorEvent) + Send + 'static,
{
    spawn_monitor_with(cfg, MonitorOptions::default(), on_event)
}

/// `spawn_monitor` with start options: start paused, or use another probe backend.
pub fn spawn_monitor_with<F>(
    cfg: MonitorConfig,
    opts: MonitorOptions,
    on_event: F,
) -> io::Result<MonitorHandle>
where
    F: FnMut(MonitorEvent) + Send + 'static,
{
    let (ctl, rx) = channel();
    let tx = ctl.clone();
    let thread = std::thread::Builder::new()
        .name(format!("probe {}", cfg.host))
        .spawn(move || Planner::new(cfg, opts, tx, rx, on_event).run())?;
    Ok(MonitorHandle {
        ctl,
        thread: Some(thread),
    })
}

enum Msg {
    Stop,
    Pause(bool),
    Interval(Duration),
    Resolved(Result<Ipv4Addr, String>),
    Done(Done),
}

/// A round handed to a worker.
struct Job {
    seq: u64,
    ts_ms: i64,
    spec: RoundSpec,
}

/// A round back from a worker.
struct Done {
    seq: u64,
    worker: u64,
    ts_ms: i64,
    dst: Ipv4Addr,
    result: io::Result<Vec<HopSample>>,
}

struct Worker {
    id: u64,
    jobs: Sender<Job>,
    thread: JoinHandle<()>,
    busy: bool,
    idle_since: Instant,
}

struct Planner<F> {
    cfg: MonitorConfig,
    make_prober: ProberFactory,
    on_event: F,
    /// Handed to workers and lookups, which answer through `rx`.
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    interval: Duration,
    paused: bool,
    stopping: bool,
    /// Planned start of the next round.
    next_at: Instant,
    /// Planned start of the last round.
    last_at: Option<Instant>,
    dst: Option<Ipv4Addr>,
    /// The host is an IPv4 address: nothing to look up.
    literal: bool,
    /// When to look the host up again; `None` while a lookup runs.
    resolve_at: Option<Instant>,
    /// Where rounds can stop, learned from the rounds reported so far.
    path_len: Option<u8>,
    workers: Vec<Worker>,
    worker_ids: u64,
    next_seq: u64,
    /// Next round to report; finished rounds wait in `finished` until their turn.
    next_out: u64,
    finished: BTreeMap<u64, Done>,
}

impl<F: FnMut(MonitorEvent)> Planner<F> {
    fn new(
        cfg: MonitorConfig,
        opts: MonitorOptions,
        tx: Sender<Msg>,
        rx: Receiver<Msg>,
        on_event: F,
    ) -> Self {
        let now = Instant::now();
        let literal = cfg.host.parse::<Ipv4Addr>().ok();
        Self {
            interval: cfg.interval.max(MIN_INTERVAL),
            cfg,
            make_prober: opts.prober.unwrap_or_else(|| Box::new(new_prober)),
            on_event,
            tx,
            rx,
            paused: opts.paused,
            stopping: false,
            next_at: now,
            last_at: None,
            dst: literal,
            literal: literal.is_some(),
            resolve_at: if literal.is_some() { None } else { Some(now) },
            path_len: None,
            workers: Vec::new(),
            worker_ids: 0,
            next_seq: 0,
            next_out: 0,
            finished: BTreeMap::new(),
        }
    }

    fn run(mut self) {
        // Open the first backend at once, so a missing permission shows up right away.
        if let Err(e) = self.add_worker() {
            (self.on_event)(MonitorEvent::Error(format!("cannot open ICMP: {e}")));
            return;
        }
        loop {
            let now = Instant::now();
            if !self.paused && !self.stopping {
                self.start_lookup(now);
                if self.dst.is_some() && now >= self.next_at {
                    self.tick(now);
                }
            }
            if self.stopping && self.in_flight() == 0 {
                break;
            }
            let msg = match self.wake_at() {
                Some(at) => {
                    match self
                        .rx
                        .recv_timeout(at.saturating_duration_since(Instant::now()))
                    {
                        Ok(msg) => msg,
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                None => match self.rx.recv() {
                    Ok(msg) => msg,
                    Err(_) => break,
                },
            };
            self.handle(msg);
        }
        for w in self.workers.drain(..) {
            drop(w.jobs);
            let _ = w.thread.join();
        }
    }

    /// Next time the planner has something to do on its own; `None` waits for a message.
    fn wake_at(&self) -> Option<Instant> {
        if self.paused || self.stopping {
            return None;
        }
        let round = self.dst.map(|_| self.next_at);
        let lookup = self.resolve_at.filter(|_| !self.literal);
        round.into_iter().chain(lookup).min()
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Stop => self.stopping = true,
            Msg::Pause(paused) => {
                if paused == self.paused {
                    return;
                }
                self.paused = paused;
                let now = Instant::now();
                if paused {
                    self.retire_idle(now, Duration::ZERO);
                } else {
                    self.next_at = self.next_at.max(now);
                }
            }
            Msg::Interval(interval) => {
                self.interval = interval.max(MIN_INTERVAL);
                if let Some(last) = self.last_at {
                    self.next_at = (last + self.interval).max(Instant::now());
                }
            }
            Msg::Resolved(result) => self.resolved(result),
            Msg::Done(done) => self.finish(done),
        }
    }

    /// Plans the next round and starts this one on a free worker. A round that finds every
    /// worker busy at the limit is skipped, not delayed.
    fn tick(&mut self, now: Instant) {
        let planned = self.next_at;
        if planned + self.interval > now {
            self.last_at = Some(planned);
            self.next_at = planned + self.interval;
        } else {
            // Far behind (the computer slept or the thread stalled): go on from now, no burst.
            self.last_at = Some(now);
            self.next_at = now + self.interval;
        }
        self.retire_idle(now, IDLE_WORKER);
        let Some(dst) = self.dst else { return };
        let Some(w) = self.free_worker() else { return };
        let job = Job {
            seq: self.next_seq,
            ts_ms: now_ms(),
            spec: RoundSpec {
                dst,
                max_ttl: self.path_len.unwrap_or(self.cfg.max_ttl),
                timeout: self.cfg.timeout,
                payload: self.cfg.payload,
                send_gap: self.cfg.send_gap,
            },
        };
        if self.workers[w].jobs.send(job).is_ok() {
            self.workers[w].busy = true;
            self.next_seq += 1;
        } else {
            // The worker thread is gone; forget it.
            let gone = self.workers.swap_remove(w);
            let _ = gone.thread.join();
        }
    }

    fn finish(&mut self, done: Done) {
        let now = Instant::now();
        if let Some(w) = self.workers.iter_mut().find(|w| w.id == done.worker) {
            w.busy = false;
            w.idle_since = now;
        }
        self.finished.insert(done.seq, done);
        while let Some(done) = self.finished.remove(&self.next_out) {
            self.next_out += 1;
            self.report(done);
        }
        if self.paused {
            self.retire_idle(now, Duration::ZERO);
        }
    }

    fn report(&mut self, done: Done) {
        match done.result {
            Ok(samples) => {
                let record = assemble(done.dst, done.ts_ms, samples);
                // Rounds to an address the host no longer resolves to tell nothing about the path.
                if self.dst == Some(done.dst) {
                    self.path_len = next_path_len(self.path_len, &record);
                }
                (self.on_event)(MonitorEvent::Round(record));
            }
            Err(e) => (self.on_event)(MonitorEvent::Error(format!(
                "probe to {} failed: {e}",
                done.dst
            ))),
        }
    }

    /// Looks the host up in the background when it is time, so a slow DNS server does not
    /// hold rounds back.
    fn start_lookup(&mut self, now: Instant) {
        if self.literal || self.resolve_at.is_none_or(|at| now < at) {
            return;
        }
        self.resolve_at = None;
        let (host, tx) = (self.cfg.host.clone(), self.tx.clone());
        let spawned = std::thread::Builder::new()
            .name(format!("resolve {host}"))
            .spawn(move || {
                let _ = tx.send(Msg::Resolved(resolve_v4(&host).map_err(|e| e.to_string())));
            });
        if let Err(e) = spawned {
            self.resolved(Err(e.to_string()));
        }
    }

    fn resolved(&mut self, result: Result<Ipv4Addr, String>) {
        let now = Instant::now();
        match result {
            Ok(ip) => {
                if self.dst != Some(ip) {
                    self.path_len = None;
                }
                self.dst = Some(ip);
                self.resolve_at = Some(now + RESOLVE_EVERY);
            }
            Err(e) if self.dst.is_none() => {
                (self.on_event)(MonitorEvent::Error(format!(
                    "cannot resolve {}: {e}",
                    self.cfg.host
                )));
                let retry = self
                    .interval
                    .clamp(Duration::from_secs(1), Duration::from_secs(10));
                self.resolve_at = Some(now + retry);
            }
            // Keep probing the old address.
            Err(_) => self.resolve_at = Some(now + RESOLVE_RETRY),
        }
    }

    fn in_flight(&self) -> usize {
        self.workers.iter().filter(|w| w.busy).count()
    }

    /// Rounds that may be in flight at once: as many as the longest possible round needs at
    /// this interval, plus one.
    fn max_in_flight(&self) -> usize {
        let ttl = u32::from(self.cfg.max_ttl.clamp(1, MAX_TTL));
        let longest = self.cfg.send_gap * ttl + self.cfg.timeout + ROUND_SLACK;
        let rounds = longest.as_nanos().div_ceil(self.interval.as_nanos().max(1));
        (rounds as usize).saturating_add(1).min(MAX_IN_FLIGHT)
    }

    /// The idle worker used last, or a new one while under the limit.
    fn free_worker(&mut self) -> Option<usize> {
        let idle = (0..self.workers.len())
            .filter(|&i| !self.workers[i].busy)
            .max_by_key(|&i| self.workers[i].idle_since);
        if idle.is_some() {
            return idle;
        }
        if self.workers.len() >= self.max_in_flight() {
            return None;
        }
        match self.add_worker() {
            Ok(i) => Some(i),
            Err(e) => {
                (self.on_event)(MonitorEvent::Error(format!("cannot open ICMP: {e}")));
                None
            }
        }
    }

    fn add_worker(&mut self) -> io::Result<usize> {
        let prober = (self.make_prober)()?;
        let id = self.worker_ids;
        self.worker_ids += 1;
        let (jobs, rx) = channel();
        let done = self.tx.clone();
        let thread = std::thread::Builder::new()
            .name(format!("probe {} #{id}", self.cfg.host))
            .spawn(move || work(id, prober, rx, done))?;
        self.workers.push(Worker {
            id,
            jobs,
            thread,
            busy: false,
            idle_since: Instant::now(),
        });
        Ok(self.workers.len() - 1)
    }

    /// Closes workers idle for at least `min_idle`, longest idle first, keeping one.
    fn retire_idle(&mut self, now: Instant, min_idle: Duration) {
        while self.workers.len() > 1 {
            let Some(i) = (0..self.workers.len())
                .filter(|&i| {
                    let w = &self.workers[i];
                    !w.busy && now.saturating_duration_since(w.idle_since) >= min_idle
                })
                .min_by_key(|&i| self.workers[i].idle_since)
            else {
                break;
            };
            let w = self.workers.swap_remove(i);
            drop(w.jobs);
            let _ = w.thread.join();
        }
    }
}

/// Runs rounds for the planner until it closes the job queue.
fn work(id: u64, mut prober: Box<dyn Prober>, jobs: Receiver<Job>, done: Sender<Msg>) {
    for job in jobs {
        // A panicking backend must not leave the planner waiting for this round forever.
        let result = catch_unwind(AssertUnwindSafe(|| prober.round(&job.spec)))
            .unwrap_or_else(|_| Err(io::Error::other("probe backend panicked")));
        let msg = Msg::Done(Done {
            seq: job.seq,
            worker: id,
            ts_ms: job.ts_ms,
            dst: job.spec.dst,
            result,
        });
        if done.send(msg).is_err() {
            break;
        }
    }
}

/// Where the next round can stop: at the destination, once it answered.
fn next_path_len(known: Option<u8>, record: &RoundRecord) -> Option<u8> {
    if record.reached {
        Some(record.path_len)
    } else if record
        .hops
        .last()
        .is_some_and(|h| h.kind == ReplyKind::TimeExceeded)
    {
        // The deepest probe still expired on the way: the route got longer.
        None
    } else {
        known
    }
}

/// Cuts a round at the destination. Probes with a larger TTL reach the destination too
/// and add nothing.
fn assemble(dst: Ipv4Addr, ts_ms: i64, mut samples: Vec<HopSample>) -> RoundRecord {
    samples.sort_by_key(|s| s.ttl);
    let end = samples
        .iter()
        .position(|s| matches!(s.kind, ReplyKind::Echo | ReplyKind::Unreachable));
    let (path_len, reached) = match end {
        Some(i) => (samples[i].ttl, samples[i].kind == ReplyKind::Echo),
        None => {
            // Nothing reached the end: keep one silent slot after the last hop that answered.
            let probed = samples.last().map_or(1, |s| s.ttl);
            let last_answered = samples
                .iter()
                .rev()
                .find(|s| s.kind.answered())
                .map_or(0, |s| s.ttl);
            ((last_answered + 1).min(probed), false)
        }
    };
    samples.truncate(path_len as usize);
    RoundRecord {
        ts_ms,
        probe: ProbeKind::Icmp,
        dst: IpAddr::V4(dst),
        path_len,
        reached,
        hops: samples,
    }
}

pub fn resolve_v4(host: &str) -> io::Result<Ipv4Addr> {
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return Ok(ip);
    }
    (host, 0)
        .to_socket_addrs()?
        .find_map(|a| match a.ip() {
            IpAddr::V4(v4) => Some(v4),
            IpAddr::V6(_) => None,
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no IPv4 address"))
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::thread::sleep;

    const HOST: &str = "10.0.0.9";
    const PATH: u8 = 4;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// What the fake backend saw, shared with the test.
    #[derive(Default)]
    struct Seen {
        starts: Vec<Instant>,
        max_ttls: Vec<u8>,
        opened: usize,
        live: usize,
        peak: usize,
    }

    type Delay = Arc<dyn Fn(usize) -> Duration + Send + Sync>;

    /// A path of `PATH` hops that all answer; round `k` (from 0) takes `delay(k)` and carries
    /// `k + 1` in every RTT, in ms, so the test can tell rounds apart.
    struct Fake {
        seen: Arc<Mutex<Seen>>,
        delay: Delay,
    }

    impl Prober for Fake {
        fn round(&mut self, spec: &RoundSpec) -> io::Result<Vec<HopSample>> {
            let k = {
                let mut s = self.seen.lock().unwrap();
                s.starts.push(Instant::now());
                s.max_ttls.push(spec.max_ttl);
                s.live += 1;
                s.peak = s.peak.max(s.live);
                s.starts.len() - 1
            };
            sleep((self.delay)(k));
            self.seen.lock().unwrap().live -= 1;
            Ok((1..=spec.max_ttl)
                .map(|ttl| {
                    let (kind, addr) = if ttl < PATH {
                        (ReplyKind::TimeExceeded, Ipv4Addr::new(10, 0, 0, ttl))
                    } else {
                        (ReplyKind::Echo, spec.dst)
                    };
                    HopSample {
                        ttl,
                        kind,
                        addr: Some(IpAddr::V4(addr)),
                        rtt_us: Some((k as u32 + 1) * 1000),
                    }
                })
                .collect())
        }
    }

    struct Run {
        handle: MonitorHandle,
        seen: Arc<Mutex<Seen>>,
        rounds: Arc<Mutex<Vec<RoundRecord>>>,
        errors: Arc<Mutex<Vec<String>>>,
    }

    impl Run {
        fn starts(&self) -> Vec<Instant> {
            self.seen.lock().unwrap().starts.clone()
        }
    }

    /// At most `(8 * 0 + 400 + 500) / interval + 1` rounds in flight: 10 at 100 ms.
    fn start(
        interval_ms: u64,
        paused: bool,
        delay: impl Fn(usize) -> Duration + Send + Sync + 'static,
    ) -> Run {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let delay: Delay = Arc::new(delay);
        let s = seen.clone();
        let prober: ProberFactory = Box::new(move || {
            s.lock().unwrap().opened += 1;
            let fake = Fake {
                seen: s.clone(),
                delay: delay.clone(),
            };
            Ok(Box::new(fake) as Box<dyn Prober>)
        });
        let mut cfg = MonitorConfig::new(HOST);
        cfg.interval = ms(interval_ms);
        cfg.timeout = ms(400);
        cfg.send_gap = Duration::ZERO;
        cfg.max_ttl = 8;
        let rounds = Arc::new(Mutex::new(Vec::new()));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let (r, e) = (rounds.clone(), errors.clone());
        let opts = MonitorOptions {
            paused,
            prober: Some(prober),
        };
        let handle = spawn_monitor_with(cfg, opts, move |ev| match ev {
            MonitorEvent::Round(x) => r.lock().unwrap().push(x),
            MonitorEvent::Error(m) => e.lock().unwrap().push(m),
        })
        .unwrap();
        Run {
            handle,
            seen,
            rounds,
            errors,
        }
    }

    /// Round starts keep the cadence of a 100 ms interval: in order, no long stall, about 100 ms apart on
    /// average. A late start is followed by an early one (the schedule catches up), so on a busy machine a
    /// single gap can be well under the interval.
    fn assert_cadence(gaps_ms: &[f64]) {
        assert!(!gaps_ms.is_empty());
        for g in gaps_ms {
            assert!(*g > 0.0 && *g <= 250.0, "gap {g} ms");
        }
        let mean = gaps_ms.iter().sum::<f64>() / gaps_ms.len() as f64;
        assert!((80.0..=125.0).contains(&mean), "mean gap {mean} ms");
    }

    #[test]
    fn rounds_overlap_to_keep_a_short_interval() {
        // Every round lasts 3.5 intervals, as when silent hops wait for the timeout.
        let run = start(100, false, |_| ms(350));
        sleep(ms(1250));
        let Run {
            handle,
            seen,
            rounds,
            errors,
        } = run;
        handle.stop_and_join();
        let seen = seen.lock().unwrap();
        let n = seen.starts.len();
        assert!((11..=14).contains(&n), "{n} rounds in 1.25 s at 100 ms");
        let gaps: Vec<f64> = seen
            .starts
            .windows(2)
            .map(|w| (w[1] - w[0]).as_secs_f64() * 1000.0)
            .collect();
        assert_cadence(&gaps);
        assert!(
            (3..=5).contains(&seen.peak),
            "{} rounds in flight",
            seen.peak
        );
        assert!(seen.opened <= 5, "{} backends", seen.opened);
        // Stopping still reports the rounds that were in flight.
        assert_eq!(rounds.lock().unwrap().len(), n);
        assert!(errors.lock().unwrap().is_empty());
    }

    #[test]
    fn rounds_come_out_in_start_order() {
        // Even rounds outlast the next two, so they finish out of order.
        let run = start(100, false, |k| ms(if k % 2 == 0 { 250 } else { 20 }));
        sleep(ms(750));
        run.handle.stop_and_join();
        let rounds = run.rounds.lock().unwrap();
        assert!(rounds.len() >= 6, "{} rounds", rounds.len());
        let order: Vec<u32> = rounds
            .iter()
            .map(|r| r.hops[0].rtt_us.unwrap() / 1000)
            .collect();
        let expected: Vec<u32> = (1..=rounds.len() as u32).collect();
        assert_eq!(order, expected);
        // Stamped with their start, not with the end: the stamps keep the cadence and never go back.
        let gaps: Vec<f64> = rounds
            .windows(2)
            .map(|w| (w[1].ts_ms - w[0].ts_ms) as f64)
            .collect();
        assert_cadence(&gaps);
    }

    #[test]
    fn pause_stops_probes_and_keeps_the_thread() {
        let run = start(100, false, |_| ms(10));
        sleep(ms(350));
        run.handle.set_paused(true);
        sleep(ms(50));
        let before = run.starts().len();
        assert!(
            (3..=5).contains(&before),
            "{before} rounds before the pause"
        );
        sleep(ms(500));
        assert_eq!(run.starts().len(), before, "probes went out while paused");

        let resumed = Instant::now();
        run.handle.set_paused(false);
        sleep(ms(250));
        let starts = run.starts();
        assert!(
            starts.len() >= before + 2,
            "{} rounds after resume",
            starts.len() - before
        );
        // The last round was long ago, so the next one starts at once.
        assert!(
            starts[before] - resumed < ms(60),
            "{:?}",
            starts[before] - resumed
        );

        run.handle.stop_and_join();
        assert_eq!(run.rounds.lock().unwrap().len(), starts.len());
    }

    #[test]
    fn a_monitor_can_start_paused() {
        let run = start(100, true, |_| ms(5));
        sleep(ms(300));
        assert!(run.starts().is_empty(), "probes went out while paused");
        // The backend is open, so a missing permission would have been reported already.
        assert_eq!(run.seen.lock().unwrap().opened, 1);
        run.handle.set_paused(false);
        sleep(ms(250));
        assert!(run.starts().len() >= 2);
        run.handle.stop_and_join();
    }

    #[test]
    fn interval_changes_from_the_next_round() {
        let run = start(400, false, |_| ms(5));
        sleep(ms(50));
        assert_eq!(run.starts().len(), 1);
        run.handle.set_interval(ms(100));
        sleep(ms(500));
        let starts = run.starts();
        // One new interval after the first round, not the old 400 ms.
        assert!(
            starts[1] - starts[0] < ms(160),
            "{:?}",
            starts[1] - starts[0]
        );
        assert!(starts.len() >= 5, "{} rounds", starts.len());

        let changed = Instant::now();
        let n = starts.len();
        run.handle.set_interval(ms(600));
        sleep(ms(400));
        assert_eq!(
            run.starts().len(),
            n,
            "started before the longer interval passed"
        );
        sleep(ms(350));
        let starts = run.starts();
        assert_eq!(starts.len(), n + 1);
        let gap = starts[n] - starts[n - 1];
        assert!(gap >= ms(580) && gap <= ms(700), "gap {gap:?}");
        assert!(starts[n] > changed);
        run.handle.stop_and_join();
    }

    #[test]
    fn short_rounds_need_one_backend_and_stop_at_the_destination() {
        let run = start(100, false, |_| ms(10));
        sleep(ms(450));
        run.handle.stop_and_join();
        let seen = run.seen.lock().unwrap();
        assert_eq!((seen.opened, seen.peak), (1, 1));
        // The first round probes up to max_ttl, later ones stop where the destination answered.
        assert_eq!(seen.max_ttls[0], 8);
        assert!(
            seen.max_ttls[1..].iter().all(|&t| t == PATH),
            "{:?}",
            seen.max_ttls
        );
        let rounds = run.rounds.lock().unwrap();
        assert!(rounds
            .iter()
            .all(|r| r.reached && r.path_len == PATH && r.hops.len() == PATH as usize));
    }

    #[test]
    fn rounds_in_flight_are_capped() {
        // Rounds hang far past their timeout: ticks are skipped instead of opening more backends.
        let run = start(100, false, |_| ms(1500));
        sleep(ms(1300));
        {
            let seen = run.seen.lock().unwrap();
            assert_eq!(seen.opened, 10);
            assert_eq!(seen.peak, 10);
            assert_eq!(seen.starts.len(), 10);
        }
        run.handle.stop_and_join();
        assert_eq!(run.rounds.lock().unwrap().len(), 10);
    }

    #[test]
    fn interval_is_never_below_the_floor() {
        let run = start(100, false, |_| ms(5));
        run.handle.set_interval(Duration::ZERO);
        sleep(ms(300));
        run.handle.stop_and_join();
        let n = run.seen.lock().unwrap().starts.len();
        assert!((4..=8).contains(&n), "{n} rounds in 300 ms");
    }

    #[test]
    fn a_backend_that_cannot_open_is_reported() {
        let prober: ProberFactory = Box::new(|| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "no ICMP here",
            ))
        });
        let opts = MonitorOptions {
            prober: Some(prober),
            ..Default::default()
        };
        let (tx, rx) = channel();
        let handle = spawn_monitor_with(MonitorConfig::new(HOST), opts, move |ev| {
            let _ = tx.send(ev);
        })
        .unwrap();
        match rx.recv_timeout(ms(2000)) {
            Ok(MonitorEvent::Error(m)) => assert!(m.contains("no ICMP here"), "{m}"),
            other => panic!("{other:?}"),
        }
        handle.stop_and_join();
    }

    #[test]
    fn handle_can_be_shared_between_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<MonitorHandle>();
    }

    #[test]
    fn route_that_got_longer_is_probed_in_full_again() {
        let mut hops: Vec<HopSample> = (1..=4)
            .map(|ttl| HopSample {
                ttl,
                kind: ReplyKind::TimeExceeded,
                addr: Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, ttl))),
                rtt_us: Some(1000),
            })
            .collect();
        let longer = assemble(Ipv4Addr::new(10, 0, 0, 9), 0, hops.clone());
        assert_eq!(next_path_len(Some(4), &longer), None);
        hops[3] = HopSample::lost(4, ReplyKind::Timeout);
        let lost = assemble(Ipv4Addr::new(10, 0, 0, 9), 0, hops);
        assert!(!lost.reached);
        assert_eq!(next_path_len(Some(4), &lost), Some(4));
    }

    /// The platform backend at 0.5 s. Sends real ICMP, so it runs only by hand:
    /// `cargo test -p whilo-trace-engine -- --ignored --nocapture`.
    #[test]
    #[ignore = "sends real ICMP to 1.1.1.1"]
    fn real_backend_keeps_half_a_second() {
        let mut cfg = MonitorConfig::new("1.1.1.1");
        cfg.interval = ms(500);
        let (tx, rx) = channel();
        let handle = spawn_monitor(cfg, move |ev| {
            let _ = tx.send(ev);
        })
        .unwrap();
        sleep(ms(6000));
        handle.stop_and_join();
        let mut rounds = Vec::new();
        for ev in rx.try_iter() {
            match ev {
                MonitorEvent::Round(r) => rounds.push(r),
                MonitorEvent::Error(e) => panic!("{e}"),
            }
        }
        let waited = rounds
            .iter()
            .filter(|r| r.hops.iter().any(|h| h.kind == ReplyKind::Timeout))
            .count();
        eprintln!(
            "{} rounds, {} reached, {} waited for the timeout, path {:?}",
            rounds.len(),
            rounds.iter().filter(|r| r.reached).count(),
            waited,
            rounds.last().map(|r| r.path_len)
        );
        assert!(rounds.len() >= 11, "{} rounds in 6 s", rounds.len());
        for w in rounds.windows(2) {
            let gap = w[1].ts_ms - w[0].ts_ms;
            assert!((300..=800).contains(&gap), "rounds {gap} ms apart");
        }
    }
}
