//! `whilo-trace-cli`: watch several targets from a terminal, trace once, or export a report.

use clap::{Args, Parser, Subcommand};
use std::collections::VecDeque;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use whilo_trace_engine::dns::DnsCache;
use whilo_trace_engine::{
    hop_stats, now_ms, spawn_monitor, timeline, verdict, HopStats, MonitorConfig, MonitorEvent,
    ProbeKind, RoundRecord,
};
use whilo_trace_store::report::{self, fmt_ms, status_label, Report};
use whilo_trace_store::{default_db_path, Store};

#[derive(Parser)]
#[command(
    name = "whilo-trace-cli",
    version,
    about = "Watch the network path to several hosts at once"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Probe hosts continuously, show live per-hop statistics and save the history.
    Watch {
        #[arg(required = true)]
        hosts: Vec<String>,
        #[command(flatten)]
        probe: ProbeArgs,
        /// Statistics window shown in the table, e.g. 5m, 1h.
        #[arg(long, default_value = "5m")]
        window: String,
        /// Stop after this many rounds per host.
        #[arg(long)]
        rounds: Option<u32>,
        /// Do not write to the history database.
        #[arg(long)]
        no_save: bool,
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Trace once, like tracert or traceroute.
    Trace {
        host: String,
        #[command(flatten)]
        probe: ProbeArgs,
    },
    /// Write an HTML, CSV and JSON report for a host from the saved history.
    Report {
        host: String,
        /// How far back, e.g. 30m, 6h, 24h.
        #[arg(long, default_value = "1h")]
        last: String,
        #[arg(long, default_value = "reports")]
        out: PathBuf,
        /// Local time offset from UTC in minutes for the report, e.g. 240 for UTC+4.
        #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
        tz_offset: i32,
        #[arg(long)]
        db: Option<PathBuf>,
    },
}

#[derive(Args, Clone)]
struct ProbeArgs {
    /// Seconds between rounds.
    #[arg(long, default_value_t = 2.5)]
    interval: f64,
    /// Reply timeout in milliseconds; keep it below the interval.
    #[arg(long, default_value_t = 2000)]
    timeout_ms: u64,
    #[arg(long, default_value_t = 30)]
    max_ttl: u8,
    /// ICMP payload bytes.
    #[arg(long, default_value_t = 32)]
    payload: u16,
    /// Milliseconds between probes of one round.
    #[arg(long, default_value_t = 25)]
    send_gap_ms: u64,
}

impl ProbeArgs {
    fn config(&self, host: &str) -> MonitorConfig {
        MonitorConfig {
            host: host.to_string(),
            interval: Duration::from_secs_f64(self.interval.max(0.2)),
            timeout: Duration::from_millis(self.timeout_ms),
            max_ttl: self.max_ttl,
            payload: self.payload,
            send_gap: Duration::from_millis(self.send_gap_ms),
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.cmd {
        Cmd::Watch {
            hosts,
            probe,
            window,
            rounds,
            no_save,
            db,
        } => watch(hosts, probe, &window, rounds, no_save, db),
        Cmd::Trace { host, probe } => trace(&host, probe),
        Cmd::Report {
            host,
            last,
            out,
            tz_offset,
            db,
        } => export(&host, &last, out, tz_offset, db),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

type AnyResult<T> = Result<T, Box<dyn std::error::Error>>;

struct Watched {
    host: String,
    id: Option<i64>,
    history: VecDeque<RoundRecord>,
    error: Option<String>,
    rounds: u32,
}

fn watch(
    hosts: Vec<String>,
    probe: ProbeArgs,
    window: &str,
    max_rounds: Option<u32>,
    no_save: bool,
    db: Option<PathBuf>,
) -> AnyResult<()> {
    let window_ms = parse_duration_ms(window)?;
    let mut store = if no_save {
        None
    } else {
        Some(Store::open(&db.unwrap_or_else(default_db_path))?)
    };
    let (tx, rx) = mpsc::channel::<(usize, MonitorEvent)>();
    let mut watched = Vec::new();
    let mut monitors = Vec::new();
    for (i, host) in hosts.iter().enumerate() {
        let id = match &store {
            Some(s) => Some(s.add_target(host)?),
            None => None,
        };
        watched.push(Watched {
            host: host.clone(),
            id,
            history: VecDeque::new(),
            error: None,
            rounds: 0,
        });
        let tx = tx.clone();
        monitors.push(spawn_monitor(probe.config(host), move |ev| {
            let _ = tx.send((i, ev));
        })?);
    }
    drop(tx);

    enable_ansi();
    let live = std::io::stdout().is_terminal();
    let dns = DnsCache::new();
    let mut last_draw = Instant::now() - Duration::from_secs(1);
    while let Ok((i, ev)) = rx.recv() {
        let w = &mut watched[i];
        match ev {
            MonitorEvent::Round(round) => {
                if let (Some(s), Some(id)) = (store.as_mut(), w.id) {
                    if let Err(e) = s.insert_round(id, &round) {
                        w.error = Some(format!("history: {e}"));
                    }
                }
                w.history.push_back(round);
                let cutoff = now_ms() - window_ms;
                while w.history.front().map_or(false, |r| r.ts_ms < cutoff) {
                    w.history.pop_front();
                }
                w.rounds += 1;
            }
            MonitorEvent::Error(msg) => w.error = Some(msg),
        }
        let done = max_rounds.map_or(false, |n| watched.iter().all(|w| w.rounds >= n));
        if live && !done {
            if last_draw.elapsed() >= Duration::from_millis(400) {
                print!("\x1b[H\x1b[2J{}", render(&watched, &dns, window));
                let _ = std::io::stdout().flush();
                last_draw = Instant::now();
            }
        } else if !live && watched[i].rounds > 0 && max_rounds.is_none() {
            println!("{}", render_one(&watched[i], &dns, window));
        }
        if done {
            break;
        }
    }
    for m in monitors {
        m.stop();
    }
    if max_rounds.is_some() {
        // Give reverse DNS a moment so the final table has names.
        for w in &watched {
            let rounds: Vec<RoundRecord> = w.history.iter().cloned().collect();
            for h in hop_stats(&rounds) {
                if let Some(a) = h.addr {
                    dns.name(a);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(1500));
        if live {
            print!("\x1b[H\x1b[2J");
        }
        println!("{}", render(&watched, &dns, window));
    }
    Ok(())
}

fn render(watched: &[Watched], dns: &DnsCache, window: &str) -> String {
    watched
        .iter()
        .map(|w| render_one(w, dns, window))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_one(w: &Watched, dns: &DnsCache, window: &str) -> String {
    let rounds: Vec<RoundRecord> = w.history.iter().cloned().collect();
    let hops = hop_stats(&rounds);
    let v = verdict(&rounds, &hops);
    let probe = rounds.last().map(|r| r.probe).unwrap_or_default();
    let dst = rounds
        .last()
        .map(|r| r.dst.to_string())
        .unwrap_or_else(|| "?".into());
    let mut out = format!(
        "\x1b[1m{}\x1b[0m ({})  window {}  rounds {}  loss {}  avg {}\n",
        w.host,
        dst,
        window,
        rounds.len(),
        v.dest_loss_pct
            .map(|l| format!("{l:.1}%"))
            .unwrap_or_else(|| "-".into()),
        fmt_ms(v.dest_avg_ms)
    );
    if let Some(e) = &w.error {
        out.push_str(&format!("  ! {e}\n"));
    }
    out.push_str(&table(&hops, dns, probe));
    for line in report::verdict_lines(&v, probe) {
        out.push_str(&format!("  > {line}\n"));
    }
    out
}

fn table(hops: &[HopStats], dns: &DnsCache, probe: ProbeKind) -> String {
    let mut out = format!(
        "  {:>3}  {:<15}  {:<28} {:>6} {:>5} {:>9} {:>9} {:>9} {:>9}  {}\n",
        "hop", "address", "name", "loss", "sent", "last", "avg", "min", "max", "status"
    );
    for h in hops {
        let addr = h.addr.map(|a| a.to_string()).unwrap_or_else(|| "*".into());
        let name = h.addr.and_then(|a| dns.name(a)).unwrap_or_default();
        let name: String = name.chars().take(28).collect();
        let mark = if h.loss_origin {
            " <- loss starts here"
        } else {
            ""
        };
        out.push_str(&format!(
            "  {:>3}  {:<15}  {:<28} {:>5.1}% {:>5} {:>9} {:>9} {:>9} {:>9}  {}{}\n",
            h.ttl,
            addr,
            name,
            h.loss_pct,
            h.sent,
            fmt_ms(h.last_ms),
            fmt_ms(h.avg_ms),
            fmt_ms(h.min_ms),
            fmt_ms(h.max_ms),
            status_label(h.status, probe),
            mark
        ));
    }
    out
}

fn trace(host: &str, probe: ProbeArgs) -> AnyResult<()> {
    let (tx, rx) = mpsc::channel();
    let monitor = spawn_monitor(probe.config(host), move |ev| {
        let _ = tx.send(ev);
    })?;
    let ev = rx.recv()?;
    monitor.stop();
    match ev {
        MonitorEvent::Error(e) => Err(e.into()),
        MonitorEvent::Round(round) => {
            println!("trace to {} ({}), {} hops", host, round.dst, round.path_len);
            for h in &round.hops {
                let name = h
                    .addr
                    .and_then(|a| dns_lookup_blocking(a))
                    .unwrap_or_default();
                let addr = h.addr.map(|a| a.to_string()).unwrap_or_else(|| "*".into());
                println!(
                    "  {:>3}  {:<15}  {:>9}  {}",
                    h.ttl,
                    addr,
                    fmt_ms(h.rtt_ms()),
                    name
                );
            }
            Ok(())
        }
    }
}

fn dns_lookup_blocking(ip: std::net::IpAddr) -> Option<String> {
    let cache = DnsCache::new();
    let deadline = Instant::now() + Duration::from_millis(1500);
    loop {
        if let Some(n) = cache.name(ip) {
            return Some(n);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn export(
    host: &str,
    last: &str,
    out: PathBuf,
    tz_offset: i32,
    db: Option<PathBuf>,
) -> AnyResult<()> {
    let store = Store::open(&db.unwrap_or_else(default_db_path))?;
    let target = store
        .find_target(host)?
        .ok_or_else(|| format!("no history for {host}"))?;
    let to_ms = now_ms();
    let from_ms = to_ms - parse_duration_ms(last)?;
    let rounds = store.load_rounds(target.id, from_ms, to_ms)?;
    if rounds.is_empty() {
        return Err(format!("no rounds for {host} in the last {last}").into());
    }
    let hops = hop_stats(&rounds);
    let v = verdict(&rounds, &hops);
    let names: Vec<Option<String>> = hops
        .iter()
        .map(|h| h.addr.and_then(dns_lookup_blocking))
        .collect();
    let interval_s = if rounds.len() > 1 {
        (rounds[rounds.len() - 1].ts_ms - rounds[0].ts_ms) as f64
            / 1000.0
            / (rounds.len() - 1) as f64
    } else {
        0.0
    };
    let line = timeline(
        &rounds,
        None,
        from_ms,
        to_ms,
        report::bucket_count(to_ms - from_ms, interval_s, 240),
    );
    let r = Report {
        host: &target.host,
        dst: rounds.last().map(|r| r.dst),
        probe: ProbeKind::parse(&target.probe).unwrap_or_default(),
        from_ms,
        to_ms,
        tz_offset_min: tz_offset,
        interval_s,
        hops: &hops,
        names: &names,
        timeline: &line,
        verdict: &v,
        generated_ms: to_ms,
    };
    std::fs::create_dir_all(&out)?;
    let base = format!(
        "{}-{}",
        safe_name(&target.host),
        report::file_stamp(to_ms, tz_offset)
    );
    let files = [
        (format!("{base}.html"), report::html(&r)),
        (format!("{base}-hops.csv"), report::hops_csv(&r)),
        (format!("{base}-samples.csv"), report::samples_csv(&rounds)),
        (format!("{base}.json"), report::json(&r)),
    ];
    for (name, body) in files {
        let path = out.join(name);
        std::fs::write(&path, body)?;
        println!("{}", path.display());
    }
    Ok(())
}

fn safe_name(host: &str) -> String {
    host.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn parse_duration_ms(s: &str) -> AnyResult<i64> {
    let s = s.trim();
    let (num, unit) = s.split_at(
        s.find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(s.len()),
    );
    let n: f64 = num.parse().map_err(|_| format!("bad duration: {s}"))?;
    let mult = match unit {
        "" | "s" => 1_000.0,
        "m" => 60_000.0,
        "h" => 3_600_000.0,
        "d" => 86_400_000.0,
        _ => return Err(format!("bad duration unit in {s}, use s, m, h or d").into()),
    };
    Ok((n * mult) as i64)
}

/// Lets the Windows console understand the escape codes used to redraw the screen.
#[cfg(windows)]
fn enable_ansi() {
    use std::ffi::c_void;
    extern "system" {
        fn GetStdHandle(id: u32) -> *mut c_void;
        fn GetConsoleMode(h: *mut c_void, mode: *mut u32) -> i32;
        fn SetConsoleMode(h: *mut c_void, mode: u32) -> i32;
    }
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(h, &mut mode) != 0 {
            SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        }
    }
}

#[cfg(not(windows))]
fn enable_ansi() {}
