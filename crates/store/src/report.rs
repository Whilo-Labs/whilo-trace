//! Report export: a self-contained HTML page with inline SVG timelines, CSV and JSON.
//! Meant to be attached to a ticket for the ISP, so it carries the time range and method.
//! One report may cover several targets and the hops pinned from them.

use serde_json::{json, Value};
use std::fmt::Write as _;
use std::net::IpAddr;
use whilo_trace_engine::stats::LOSS_NOTABLE_PCT;
use whilo_trace_engine::{Bucket, HopStats, HopStatus, ProbeKind, RoundRecord, Verdict};

pub const GENERATOR: &str = concat!("Whilo Trace ", env!("CARGO_PKG_VERSION"));

/// One target over the report interval.
#[derive(Clone, Copy)]
pub struct Report<'a> {
    pub host: &'a str,
    pub dst: Option<IpAddr>,
    /// What the rounds were sent with; the texts name it, nothing assumes ICMP.
    pub probe: ProbeKind,
    pub from_ms: i64,
    pub to_ms: i64,
    /// Local time is UTC plus this many minutes.
    pub tz_offset_min: i32,
    pub interval_s: f64,
    pub hops: &'a [HopStats],
    /// Reverse DNS names, same order as `hops`.
    pub names: &'a [Option<String>],
    /// Destination latency and loss over `[from_ms, to_ms)`.
    pub timeline: &'a [Bucket],
    pub verdict: &'a Verdict,
    pub generated_ms: i64,
}

/// A hop pinned from a target: its own timeline in the report.
pub struct PinnedLine {
    pub ttl: u8,
    pub addr: Option<IpAddr>,
    pub name: Option<String>,
    pub timeline: Vec<Bucket>,
}

/// A target with the hops pinned from it.
pub struct Section<'a> {
    pub report: Report<'a>,
    pub pinned: Vec<PinnedLine>,
}

/// A whole report: one or more targets over the same interval.
pub struct Document<'a> {
    pub sections: Vec<Section<'a>>,
    pub from_ms: i64,
    pub to_ms: i64,
    pub tz_offset_min: i32,
    pub generated_ms: i64,
    /// Findings as the interface shows them, in its language; an empty list prints none.
    /// `None` prints the engine's own, in English: the command line.
    pub findings: Option<Vec<String>>,
    /// "ru" or "en": the language of the HTML page. CSV and JSON stay in English.
    pub lang: &'a str,
}

impl<'a> Document<'a> {
    /// A report of one target in English, as the command line writes it.
    pub fn single(r: Report<'a>) -> Self {
        Document {
            from_ms: r.from_ms,
            to_ms: r.to_ms,
            tz_offset_min: r.tz_offset_min,
            generated_ms: r.generated_ms,
            sections: vec![Section {
                report: r,
                pinned: Vec::new(),
            }],
            findings: None,
            lang: "en",
        }
    }

    /// The findings to print: the interface's, or without them the engine's per target.
    pub fn findings(&self) -> Vec<String> {
        if let Some(f) = &self.findings {
            return f.clone();
        }
        if let [one] = self.sections.as_slice() {
            return verdict_lines(one.report.verdict, one.report.probe);
        }
        self.sections
            .iter()
            .flat_map(|s| {
                let host = s.report.host;
                verdict_lines(s.report.verdict, s.report.probe)
                    .into_iter()
                    .map(move |l| format!("{host}: {l}"))
            })
            .collect()
    }

    /// Probe types of the report, in the order of the targets.
    fn probes(&self) -> Vec<ProbeKind> {
        let mut out: Vec<ProbeKind> = Vec::new();
        for s in &self.sections {
            if !out.contains(&s.report.probe) {
                out.push(s.report.probe);
            }
        }
        out
    }
}

/// The name of a probe type as the interface shows it.
pub fn probe_label(p: ProbeKind) -> &'static str {
    match p {
        ProbeKind::Icmp => "ICMP",
    }
}

/// What one probe of this type is, for the method line.
fn probe_method(p: ProbeKind) -> &'static str {
    match p {
        ProbeKind::Icmp => "ICMP echo",
    }
}

/// Status names in English, as CSV files and the command line write them.
pub fn status_label(s: HopStatus, probe: ProbeKind) -> String {
    status_text(s, "en", probe)
}

/// Status names as the interface shows them, neutral; the hint explains them.
pub fn status_text(s: HopStatus, lang: &str, probe: ProbeKind) -> String {
    let ru = lang == "ru";
    match s {
        HopStatus::Ok => pick(ru, "в норме", "OK").into(),
        HopStatus::Silent if ru => format!("не отвечает на {}", probe_label(probe)),
        HopStatus::Silent => format!("No reply to {}", probe_label(probe)),
        HopStatus::RateLimited => {
            pick(ru, "отвечает не на все пробы", "Answers some probes").into()
        }
        HopStatus::Lossy => pick(ru, "потери", "Loss").into(),
        HopStatus::NoReply => pick(ru, "нет ответа", "No reply").into(),
    }
}

fn pick(ru: bool, a: &'static str, b: &'static str) -> &'static str {
    if ru {
        a
    } else {
        b
    }
}

/// Plain-language findings in English, most important first; `probe` is what the rounds were
/// sent with.
pub fn verdict_lines(v: &Verdict, probe: ProbeKind) -> Vec<String> {
    let p = probe_label(probe);
    let mut lines = Vec::new();
    match (v.dest_loss_pct, v.dest_ttl) {
        (None, _) | (_, None) => lines.push("No data yet.".to_string()),
        (Some(loss), Some(_)) if loss >= 100.0 => lines.push(format!(
            "The destination did not answer at all in {} rounds.",
            v.rounds
        )),
        (Some(loss), Some(_)) if loss < LOSS_NOTABLE_PCT => lines.push(format!(
            "No notable loss to the destination: {:.1}% over {} rounds, average {}.",
            loss,
            v.rounds,
            fmt_ms(v.dest_avg_ms)
        )),
        (Some(loss), Some(_)) => {
            lines.push(format!(
                "Packet loss to the destination: {:.1}% over {} rounds.",
                loss, v.rounds
            ));
            if let Some(ttl) = v.origin_ttl {
                let at = v.origin_addr.map(|a| format!(" ({a})")).unwrap_or_default();
                lines.push(format!(
                    "The loss starts at hop {ttl}{at} and carries on to the destination."
                ));
            }
        }
    }
    match v.silent_hops.as_slice() {
        [] => {}
        [one] => lines.push(format!(
            "Hop {one} does not answer {p} probes but passes traffic on; that is not packet loss."
        )),
        many => lines.push(format!(
            "Hops {} do not answer {p} probes but pass traffic on; that is not packet loss.",
            join(many)
        )),
    }
    match v.rate_limited_hops.as_slice() {
        [] => {}
        [one] => lines.push(format!(
            "Hop {one} answers only some {p} probes while later hops answer them; that is not packet loss."
        )),
        many => lines.push(format!(
            "Hops {} answer only some {p} probes while later hops answer them; that is not packet loss.",
            join(many)
        )),
    }
    lines
}

fn join(ttls: &[u8]) -> String {
    ttls.iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn hops_csv(r: &Report) -> String {
    let mut out = String::from(
        "ttl,address,name,status,loss_pct,sent,recv,last_ms,avg_ms,min_ms,max_ms,jitter_ms\n",
    );
    for (h, name) in r
        .hops
        .iter()
        .zip(r.names.iter().chain(std::iter::repeat(&None)))
    {
        let _ = writeln!(out, "{}", hop_csv_fields(h, name.as_deref(), r.probe));
    }
    out
}

/// The hop tables of every target, with the target in the first column.
pub fn document_hops_csv(d: &Document) -> String {
    let mut out = String::from(
        "target,ttl,address,name,status,loss_pct,sent,recv,last_ms,avg_ms,min_ms,max_ms,jitter_ms\n",
    );
    for s in &d.sections {
        let r = &s.report;
        for (h, name) in r
            .hops
            .iter()
            .zip(r.names.iter().chain(std::iter::repeat(&None)))
        {
            let _ = writeln!(
                out,
                "{},{}",
                csv_field(r.host),
                hop_csv_fields(h, name.as_deref(), r.probe)
            );
        }
    }
    out
}

fn hop_csv_fields(h: &HopStats, name: Option<&str>, probe: ProbeKind) -> String {
    format!(
        "{},{},{},{},{:.2},{},{},{},{},{},{},{}",
        h.ttl,
        h.addr.map(|a| a.to_string()).unwrap_or_default(),
        csv_field(name.unwrap_or("")),
        csv_field(&status_label(h.status, probe)),
        h.loss_pct,
        h.sent,
        h.recv,
        opt(h.last_ms),
        opt(h.avg_ms),
        opt(h.min_ms),
        opt(h.max_ms),
        opt(h.jitter_ms),
    )
}

/// Every probe of every round: the raw data behind the report.
pub fn samples_csv(rounds: &[RoundRecord]) -> String {
    let mut out = String::from("time_utc,ts_ms,ttl,kind,address,rtt_ms\n");
    for round in rounds {
        sample_lines(&mut out, None, round);
    }
    out
}

/// Every probe of every round of several targets, with the target in the first column.
pub fn document_samples_csv(targets: &[(&str, &[RoundRecord])]) -> String {
    let mut out = String::from("target,time_utc,ts_ms,probe,ttl,kind,address,rtt_ms\n");
    for (host, rounds) in targets {
        for round in rounds.iter() {
            sample_lines(&mut out, Some(host), round);
        }
    }
    out
}

fn sample_lines(out: &mut String, host: Option<&str>, round: &RoundRecord) {
    let t = format_time(round.ts_ms, 0);
    for h in &round.hops {
        let kind = serde_json::to_value(h.kind)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        if let Some(host) = host {
            let _ = write!(out, "{},", csv_field(host));
            let _ = write!(out, "{},{},{},", t, round.ts_ms, round.probe.as_str());
        } else {
            let _ = write!(out, "{},{},", t, round.ts_ms);
        }
        let _ = writeln!(
            out,
            "{},{},{},{}",
            h.ttl,
            kind,
            h.addr.map(|a| a.to_string()).unwrap_or_default(),
            opt(h.rtt_ms())
        );
    }
}

pub fn json(r: &Report) -> String {
    let mut doc = target_json(r, true);
    doc["generator"] = json!(GENERATOR);
    doc["generated_utc"] = json!(format_time(r.generated_ms, 0));
    doc["from_utc"] = json!(format_time(r.from_ms, 0));
    doc["to_utc"] = json!(format_time(r.to_ms, 0));
    doc["from_ms"] = json!(r.from_ms);
    doc["to_ms"] = json!(r.to_ms);
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

/// One target; `engine_findings` adds the engine's own findings (the command line only).
fn target_json(r: &Report, engine_findings: bool) -> Value {
    let hops: Vec<Value> = r
        .hops
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let mut v = serde_json::to_value(h).unwrap_or(Value::Null);
            v["name"] = json!(r.names.get(i).cloned().flatten());
            v
        })
        .collect();
    let mut v = json!({
        "host": r.host,
        "destination": r.dst,
        "probe": r.probe.as_str(),
        "method": method(r, "en"),
        "verdict": r.verdict,
        "hops": hops,
        "timeline": r.timeline,
    });
    if engine_findings {
        v["findings"] = json!(verdict_lines(r.verdict, r.probe));
    }
    v
}

pub fn document_json(d: &Document) -> String {
    let targets: Vec<Value> = d
        .sections
        .iter()
        .map(|s| {
            let mut v = target_json(&s.report, d.findings.is_none());
            v["pinned"] = Value::Array(
                s.pinned
                    .iter()
                    .map(|p| {
                        json!({ "ttl": p.ttl, "address": p.addr, "name": p.name, "timeline": p.timeline })
                    })
                    .collect(),
            );
            v
        })
        .collect();
    let doc = json!({
        "generator": GENERATOR,
        "generated_utc": format_time(d.generated_ms, 0),
        "from_utc": format_time(d.from_ms, 0),
        "to_utc": format_time(d.to_ms, 0),
        "from_ms": d.from_ms,
        "to_ms": d.to_ms,
        "tz_offset_min": d.tz_offset_min,
        "findings": d.findings(),
        "targets": targets,
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

fn method(r: &Report, lang: &str) -> String {
    let m = probe_method(r.probe);
    if lang == "ru" {
        format!(
            "{m}, одна проба на каждый TTL раз в {} с, статистика хопов за весь период",
            num(r.interval_s, 1, true)
        )
    } else {
        format!(
            "{m}, one probe per TTL every {:.1} s, per-hop statistics over the whole interval",
            r.interval_s
        )
    }
}

fn title_of(r: &Report) -> String {
    match r.dst {
        Some(ip) if ip.to_string() != r.host => format!("{} ({})", r.host, ip),
        _ => r.host.to_string(),
    }
}

pub fn html(r: &Report) -> String {
    document_html(&Document::single(*r))
}

/// Labels of the HTML page.
struct Words {
    title: &'static str,
    interval: &'static str,
    to: &'static str,
    rounds: &'static str,
    generated: &'static str,
    dest_chart: &'static str,
    hop_chart: &'static str,
    hops: &'static str,
    avg_line: &'static str,
    band: &'static str,
    loss: &'static str,
    cols: [&'static str; 11],
    unknown: &'static str,
    method: &'static str,
    times_in: &'static str,
}

/// The note under the report: the statuses that are not loss, named as in the hop table.
fn note(ru: bool, probes: &[ProbeKind]) -> String {
    let lang = if ru { "ru" } else { "en" };
    let quote = |s: String| {
        if ru {
            format!("«{s}»")
        } else {
            format!("\"{s}\"")
        }
    };
    let silent: Vec<String> = probes
        .iter()
        .map(|&p| quote(status_text(HopStatus::Silent, lang, p)))
        .collect();
    let partial = quote(status_text(
        HopStatus::RateLimited,
        lang,
        ProbeKind::default(),
    ));
    let named = match silent.as_slice() {
        [] => partial,
        many => format!("{} {} {partial}", many.join(", "), pick(ru, "или", "or")),
    };
    if ru {
        format!("Хоп со статусом {named} пропускает трафик дальше; на соединение влияют только потери, которые продолжаются до цели.")
    } else {
        format!("A hop marked {named} forwards traffic normally; only loss that continues to the destination affects the connection.")
    }
}

fn words(ru: bool) -> Words {
    if ru {
        Words {
            title: "Отчет о сетевом пути",
            interval: "Период",
            to: "по",
            rounds: "раундов",
            generated: "Создан",
            dest_chart: "Задержка и потери до цели",
            hop_chart: "Хоп",
            hops: "Хопы",
            avg_line: "средняя задержка",
            band: "от минимума до максимума",
            loss: "потери",
            cols: [
                "Хоп",
                "Адрес",
                "Имя",
                "Потери",
                "Отправлено",
                "Средняя",
                "Мин.",
                "Макс.",
                "Джиттер",
                "Последний",
                "Статус",
            ],
            unknown: "адрес неизвестен",
            method: "Метод",
            times_in: "Время",
        }
    } else {
        Words {
            title: "Network path report",
            interval: "Interval",
            to: "to",
            rounds: "rounds",
            generated: "Generated",
            dest_chart: "Destination latency and loss",
            hop_chart: "Hop",
            hops: "Hops",
            avg_line: "average latency",
            band: "min to max",
            loss: "packet loss",
            cols: [
                "Hop", "Address", "Name", "Loss", "Sent", "Avg", "Min", "Max", "Jitter", "Last",
                "Status",
            ],
            unknown: "address unknown",
            method: "Method",
            times_in: "Times in",
        }
    }
}

pub fn document_html(d: &Document) -> String {
    let ru = d.lang == "ru";
    let w = words(ru);
    let tz = tz_label(d.tz_offset_min);
    let single = d.sections.len() == 1;
    let heading = match d.sections.as_slice() {
        [one] => format!("{}: {}", w.title, title_of(&one.report)),
        many => {
            let hosts: Vec<&str> = many.iter().map(|s| s.report.host).collect();
            if hosts.is_empty() {
                w.title.to_string()
            } else {
                format!("{}: {}", w.title, hosts.join(", "))
            }
        }
    };
    let rounds_all: usize = d.sections.iter().map(|s| s.report.verdict.rounds).sum();
    let lines = d.findings();
    // No findings: no box for them.
    let findings = if lines.is_empty() {
        String::new()
    } else {
        let items: String = lines
            .iter()
            .map(|l| format!("<li>{}</li>", esc(l)))
            .collect();
        format!("\n<ul class=\"findings\">{items}</ul>")
    };
    let mut body = String::new();
    for s in &d.sections {
        let r = &s.report;
        if !single {
            let _ = write!(
                body,
                "<h2>{}</h2><div class=\"meta\">{} {}, {}</div>",
                esc(&title_of(r)),
                r.verdict.rounds,
                w.rounds,
                esc(&method(r, d.lang))
            );
        }
        let _ = write!(
            body,
            "<h3>{}</h3>{}{}",
            w.dest_chart,
            svg_timeline_in(r.timeline, d.from_ms, d.to_ms, d.tz_offset_min, ru),
            legend(&w)
        );
        for p in &s.pinned {
            let who = match (&p.addr, &p.name) {
                (Some(a), Some(n)) => format!("{a} ({n})"),
                (Some(a), None) => a.to_string(),
                (None, _) => w.unknown.to_string(),
            };
            let _ = write!(
                body,
                "<h3>{} {}: <span class=\"mono\">{}</span></h3>{}",
                w.hop_chart,
                p.ttl,
                esc(&who),
                svg_timeline_in(&p.timeline, d.from_ms, d.to_ms, d.tz_offset_min, ru)
            );
        }
        let _ = write!(
            body,
            "<h3>{}</h3><table><thead><tr>{}</tr></thead><tbody>{}</tbody></table>",
            w.hops,
            header_cells(&w),
            hop_rows(r, d.lang)
        );
    }
    let methods = method_any(d);
    format!(
        r#"<!doctype html>
<html lang="{lang}"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{heading}</title>
<style>
body {{ font: 14px/1.45 -apple-system, "Segoe UI", Roboto, sans-serif; color: #1c2230; background: #fff; margin: 0; }}
main {{ max-width: 1040px; margin: 0 auto; padding: 28px 20px 40px; }}
h1 {{ font-size: 22px; margin: 0 0 4px; }}
h2 {{ font-size: 18px; margin: 36px 0 2px; }}
h3 {{ font-size: 13px; margin: 24px 0 8px; text-transform: uppercase; letter-spacing: .04em; color: #5b6475; font-weight: 600; }}
h3 .mono {{ text-transform: none; letter-spacing: 0; }}
.meta {{ color: #5b6475; }}
.meta b {{ color: #1c2230; font-weight: 600; }}
ul.findings {{ background: #f4f6fa; border-radius: 8px; padding: 12px 16px 12px 32px; margin: 16px 0 0; }}
ul.findings li + li {{ margin-top: 4px; }}
svg {{ width: 100%; height: auto; display: block; }}
table {{ border-collapse: collapse; width: 100%; font-size: 13px; }}
th, td {{ padding: 6px 8px; border-bottom: 1px solid #e6e9f0; text-align: left; white-space: nowrap; }}
th {{ color: #5b6475; font-weight: 600; }}
td.num, th.num {{ text-align: right; font-variant-numeric: tabular-nums; }}
.mono {{ font-family: ui-monospace, Consolas, monospace; }}
.badge {{ border-radius: 10px; padding: 1px 8px; font-size: 12px; background: #e8f5ee; color: #17663f; }}
tr.silent td, tr.noreply td {{ color: #8a92a3; }}
tr.silent .badge, tr.noreply .badge {{ background: #eef0f4; color: #5b6475; }}
tr.limited .badge {{ background: #fff4e0; color: #8a5a00; }}
tr.loss .badge {{ background: #fdeaea; color: #b42318; }}
tr.origin td {{ background: #fff5f5; font-weight: 600; }}
.legend {{ color: #5b6475; font-size: 12px; margin-top: 6px; }}
.legend i {{ display: inline-block; width: 10px; height: 10px; border-radius: 2px; margin: 0 4px 0 14px; vertical-align: -1px; }}
footer {{ margin-top: 28px; color: #8a92a3; font-size: 12px; }}
</style></head>
<body><main>
<h1>{heading}</h1>
<div class="meta">{interval} <b>{from}</b> {to_word} <b>{to}</b> ({tz}), {dur}, {rounds} {rounds_word}. {generated} {gen}.</div>{findings}
{body}
<footer>{generator}. {method_word}: {method}. {times_in} {tz}. {note}</footer>
</main></body></html>
"#,
        lang = if ru { "ru" } else { "en" },
        heading = esc(&heading),
        interval = w.interval,
        from = format_time(d.from_ms, d.tz_offset_min),
        to_word = w.to,
        to = format_time(d.to_ms, d.tz_offset_min),
        tz = tz,
        dur = fmt_duration(d.to_ms - d.from_ms, ru),
        rounds = rounds_all,
        rounds_word = w.rounds,
        generated = w.generated,
        gen = format_time(d.generated_ms, d.tz_offset_min),
        findings = findings,
        body = body,
        generator = GENERATOR,
        method_word = w.method,
        method = esc(&methods),
        times_in = w.times_in,
        note = esc(&note(ru, &d.probes())),
    )
}

/// The method line; targets with different intervals each get theirs.
fn method_any(d: &Document) -> String {
    let mut seen: Vec<String> = Vec::new();
    for s in &d.sections {
        let m = method(&s.report, d.lang);
        if !seen.contains(&m) {
            seen.push(m);
        }
    }
    seen.join("; ")
}

fn legend(w: &Words) -> String {
    format!(
        "<div class=\"legend\"><i style=\"background:#3b6fd8\"></i>{}<i style=\"background:#3b6fd833\"></i>{}<i style=\"background:#e5484d\"></i>{}</div>",
        w.avg_line, w.band, w.loss
    )
}

fn header_cells(w: &Words) -> String {
    w.cols
        .iter()
        .enumerate()
        .map(|(i, c)| {
            if (3..=9).contains(&i) {
                format!("<th class=\"num\">{c}</th>")
            } else {
                format!("<th>{c}</th>")
            }
        })
        .collect()
}

fn hop_rows(r: &Report, lang: &str) -> String {
    let ru = lang == "ru";
    let mut rows = String::new();
    for (i, h) in r.hops.iter().enumerate() {
        let name = r.names.get(i).cloned().flatten().unwrap_or_default();
        let class = match h.status {
            HopStatus::Ok => "ok",
            HopStatus::Silent => "silent",
            HopStatus::RateLimited => "limited",
            HopStatus::Lossy => "loss",
            HopStatus::NoReply => "noreply",
        };
        let mark = if h.loss_origin { " origin" } else { "" };
        let _ = write!(
            rows,
            "<tr class=\"{class}{mark}\"><td>{}</td><td class=\"mono\">{}</td><td>{}</td><td class=\"num\">{}%</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td><td><span class=\"badge\">{}</span></td></tr>",
            h.ttl,
            h.addr.map(|a| a.to_string()).unwrap_or_else(|| "*".into()),
            esc(&name),
            num(h.loss_pct, 1, ru),
            h.sent,
            ms_in(h.avg_ms, ru),
            ms_in(h.min_ms, ru),
            ms_in(h.max_ms, ru),
            ms_in(h.jitter_ms, ru),
            ms_in(h.last_ms, ru),
            esc(&status_text(h.status, lang, r.probe)),
        );
    }
    rows
}

/// Latency band, average line and loss bars, as inline SVG.
pub fn svg_timeline(buckets: &[Bucket], from_ms: i64, to_ms: i64, tz_offset_min: i32) -> String {
    svg_timeline_in(buckets, from_ms, to_ms, tz_offset_min, false)
}

fn svg_timeline_in(
    buckets: &[Bucket],
    from_ms: i64,
    to_ms: i64,
    tz_offset_min: i32,
    ru: bool,
) -> String {
    let (w, h) = (1000.0_f64, 260.0_f64);
    let (left, right, top, bottom) = (52.0, 12.0, 10.0, 26.0);
    let pw = w - left - right;
    let ph = h - top - bottom;
    let n = buckets.len().max(1) as f64;
    let mut peaks: Vec<f64> = buckets.iter().filter_map(|b| b.max_ms).collect();
    peaks.sort_by(|a, b| a.total_cmp(b));
    let p95 = peaks
        .get(((peaks.len() as f64) * 0.95) as usize)
        .or(peaks.last())
        .copied()
        .unwrap_or(1.0);
    let ymax = nice_ceil((p95 * 1.15).max(1.0));
    let x = |i: f64| left + (i + 0.5) * pw / n;
    let y = |ms: f64| top + ph - (ms.min(ymax) / ymax) * ph;
    let unit = if ru { "мс" } else { "ms" };

    let mut s = format!(
        r#"<svg viewBox="0 0 {w} {h}" xmlns="http://www.w3.org/2000/svg" font-family="sans-serif" font-size="11">"#
    );
    for k in 0..=4 {
        let v = ymax * k as f64 / 4.0;
        let yy = y(v);
        let label = if ru {
            trim_num(v).replace('.', ",")
        } else {
            trim_num(v)
        };
        let _ = write!(
            s,
            r##"<line x1="{left}" x2="{x2}" y1="{yy:.1}" y2="{yy:.1}" stroke="#e6e9f0"/><text x="{tx}" y="{ty:.1}" text-anchor="end" fill="#5b6475">{label} {unit}</text>"##,
            x2 = w - right,
            tx = left - 6.0,
            ty = yy + 4.0,
        );
    }
    for k in 0..=4 {
        let t = from_ms + (to_ms - from_ms) * k / 4;
        let xx = left + pw * k as f64 / 4.0;
        let anchor = match k {
            0 => "start",
            4 => "end",
            _ => "middle",
        };
        let _ = write!(
            s,
            r##"<text x="{xx:.1}" y="{ty}" text-anchor="{anchor}" fill="#5b6475">{label}</text>"##,
            ty = h - 8.0,
            label = format_clock(t, tz_offset_min)
        );
    }
    let bw = (pw / n).max(1.0);
    for (i, b) in buckets.iter().enumerate() {
        if b.lost > 0 && b.sent > 0 {
            let share = b.lost as f64 / b.sent as f64;
            let _ = write!(
                s,
                r##"<rect x="{:.1}" y="{top}" width="{bw:.1}" height="{ph}" fill="#e5484d" opacity="{:.2}"/>"##,
                left + i as f64 * pw / n,
                0.25 + 0.75 * share
            );
        }
    }
    for run in runs(buckets) {
        let mut pts = String::new();
        for &i in &run {
            let _ = write!(
                pts,
                "{:.1},{:.1} ",
                x(i as f64),
                y(buckets[i].max_ms.unwrap_or(0.0))
            );
        }
        for &i in run.iter().rev() {
            let _ = write!(
                pts,
                "{:.1},{:.1} ",
                x(i as f64),
                y(buckets[i].min_ms.unwrap_or(0.0))
            );
        }
        if let [only] = run.as_slice() {
            let _ = write!(
                s,
                r##"<circle cx="{:.1}" cy="{:.1}" r="2.2" fill="#3b6fd8"/>"##,
                x(*only as f64),
                y(buckets[*only].avg_ms.unwrap_or(0.0))
            );
            continue;
        }
        let _ = write!(
            s,
            r##"<polygon points="{pts}" fill="#3b6fd8" fill-opacity="0.18"/>"##
        );
        let line: String = run
            .iter()
            .map(|&i| {
                format!(
                    "{:.1},{:.1}",
                    x(i as f64),
                    y(buckets[i].avg_ms.unwrap_or(0.0))
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        let _ = write!(
            s,
            r##"<polyline points="{line}" fill="none" stroke="#3b6fd8" stroke-width="1.6"/>"##
        );
    }
    s.push_str("</svg>");
    s
}

/// Index runs of consecutive buckets that have latency data.
fn runs(buckets: &[Bucket]) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut cur = Vec::new();
    for (i, b) in buckets.iter().enumerate() {
        if b.avg_ms.is_some() {
            cur.push(i);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Timeline slices for a report: about one round per slice, at most `max`, so the line
/// has no holes on short intervals.
pub fn bucket_count(span_ms: i64, interval_s: f64, max: usize) -> usize {
    if interval_s <= 0.0 {
        return max;
    }
    ((span_ms as f64 / 1000.0 / interval_s).floor() as usize).clamp(10, max)
}

fn nice_ceil(v: f64) -> f64 {
    let p = 10f64.powf(v.log10().floor());
    for m in [1.0, 2.0, 2.5, 5.0, 10.0] {
        if v <= m * p {
            return m * p;
        }
    }
    10.0 * p
}

fn trim_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

fn opt(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.2}")).unwrap_or_default()
}

/// A number with `digits` decimals; a decimal comma in Russian.
fn num(v: f64, digits: usize, ru: bool) -> String {
    let s = format!("{v:.digits$}");
    if ru {
        s.replace('.', ",")
    } else {
        s
    }
}

pub fn fmt_ms(v: Option<f64>) -> String {
    ms_in(v, false)
}

fn ms_in(v: Option<f64>, ru: bool) -> String {
    let unit = if ru { "мс" } else { "ms" };
    match v {
        None => "-".into(),
        Some(x) if x < 10.0 => format!("{} {unit}", num(x, 1, ru)),
        Some(x) => format!("{x:.0} {unit}"),
    }
}

fn fmt_duration(ms: i64, ru: bool) -> String {
    let s = ms / 1000;
    match s {
        s if s < 120 => format!("{s} {}", pick(ru, "с", "s")),
        s if s < 7200 => format!("{} {}", s / 60, pick(ru, "мин", "min")),
        s => format!("{} {}", num(s as f64 / 3600.0, 1, ru), pick(ru, "ч", "h")),
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn tz_label(offset_min: i32) -> String {
    let sign = if offset_min < 0 { '-' } else { '+' };
    let m = offset_min.abs();
    format!("UTC{sign}{:02}:{:02}", m / 60, m % 60)
}

/// `YYYY-MM-DD HH:MM:SS` in UTC shifted by `offset_min`.
pub fn format_time(ms: i64, offset_min: i32) -> String {
    let (y, mo, d, h, mi, s) = civil(ms + offset_min as i64 * 60_000);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

/// `YYYYMMDD-HHMMSS`, for file names.
pub fn file_stamp(ms: i64, offset_min: i32) -> String {
    let (y, mo, d, h, mi, s) = civil(ms + offset_min as i64 * 60_000);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

fn format_clock(ms: i64, offset_min: i32) -> String {
    let (_, _, _, h, mi, _) = civil(ms + offset_min as i64 * 60_000);
    format!("{h:02}:{mi:02}")
}

/// Unix milliseconds to calendar fields (proleptic Gregorian, UTC).
fn civil(ms: i64) -> (i64, u32, u32, u32, u32, u32) {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (
        y,
        m,
        d,
        (sod / 3600) as u32,
        (sod % 3600 / 60) as u32,
        (sod % 60) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_times() {
        assert_eq!(format_time(0, 0), "1970-01-01 00:00:00");
        // 2026-10-27 11:20:31 UTC
        assert_eq!(format_time(1_793_100_031_000, 240), "2026-10-27 15:20:31");
        assert_eq!(tz_label(240), "UTC+04:00");
        assert_eq!(tz_label(-330), "UTC-05:30");
    }

    #[test]
    fn russian_numbers_use_a_decimal_comma() {
        assert_eq!(ms_in(Some(2.26), true), "2,3 мс");
        assert_eq!(ms_in(Some(2.26), false), "2.3 ms");
        assert_eq!(fmt_duration(5_400_000, true), "90 мин");
        assert_eq!(fmt_duration(9_000_000, true), "2,5 ч");
    }
}
