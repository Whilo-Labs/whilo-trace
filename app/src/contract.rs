//! What the interface asks for and gets back: the types behind the Tauri commands.
//! The same shapes are described with examples in `docs/ui-contract.md`; JSON keys are camelCase.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

fn default_buckets() -> u32 {
    120
}

fn yes() -> bool {
    true
}

/// A key that may be missing (`None`: leave the value as it is) or null (`Some(None)`: clear it).
fn some<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

/// What the screen is looking at.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotReq {
    /// "5m", "10m", "30m", "1h", "6h" or "24h".
    pub window: String,
    /// End of the window; `None` means live, the window ends now.
    pub end_ms: Option<i64>,
    /// A period picked on a timeline, `[fromMs, toMs)` like the window: toMs is not in it.
    /// Stats for it come back in `range_stats`.
    pub range: Option<[i64; 2]>,
    /// Row id of the target whose full trace is wanted (the trace screen). A pinned hop or an
    /// unknown id gets no trace.
    pub trace: Option<String>,
    #[serde(default = "default_buckets")]
    pub buckets: u32,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(rename = "N")]
    pub n: u32,
    pub now_ms: i64,
    pub from_ms: i64,
    pub to_ms: i64,
    pub live: bool,
    pub bucket_ms: f64,
    /// Rows of the main screen in the user's order: one per target and probe type, then pinned hops where the user put them.
    pub targets: Vec<Row>,
    pub trace: Option<Trace>,
    pub history: History,
    /// Always empty from the core; the browser mock uses it for extra loss marks.
    pub loss: Vec<u32>,
    pub settings: Settings,
    pub probes_available: Vec<String>,
    /// Probes per second over all running rows.
    pub rate_pps: f64,
    pub recent: Vec<String>,
    /// The database of places for the map.
    pub geo: GeoStatus,
}

/// One line of data: a target with one probe type, or a hop pinned from a trace.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    /// Stable id: "t12" is target 12 (ICMP), "p7" is pinned hop 7.
    pub id: String,
    pub target_id: i64,
    pub host: String,
    /// The second label: for a target typed as an IP its reverse DNS name, for a target typed as
    /// a name the IP it resolves to; for a pinned hop its reverse DNS name.
    pub name: Option<String>,
    pub ip: Option<String>,
    pub probe: String,
    /// Seconds between rounds.
    pub interval: f64,
    pub paused: bool,
    /// What keeps the row from getting data, as an error code: "resolve_failed", "no_ipv4",
    /// "probe_unavailable: <detail>", "db: <detail>", "io: <detail>".
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hop_of: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hop: Option<u8>,
    /// Average RTT in ms per bucket, `None` where nothing answered.
    pub series: Vec<Option<f64>>,
    pub lo: Vec<Option<f64>>,
    pub hi: Vec<Option<f64>>,
    /// Buckets with loss on the path (drawn red).
    pub loss: Vec<u32>,
    /// Buckets where a hop did not answer while the destination did (drawn amber).
    pub partial: Vec<u32>,
    /// Probes per bucket (rounds that probed the row in it), and how many of them got no answer.
    pub sent_n: Vec<u32>,
    pub lost_n: Vec<u32>,
    /// Top of the latency scale in ms.
    pub max: f64,
    /// "ok", "loss", "partial", "silent", "noreply" or, for a pinned hop, "unreachable".
    pub status: String,
    pub stats: Stats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range_stats: Option<Stats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathSummary>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub loss_pct: f64,
    pub no_reply_pct: f64,
    pub avg: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub last: Option<f64>,
    pub last_lost: bool,
    pub jitter: Option<f64>,
    /// Rounds of the period that probed the line: for a target every round of the period.
    pub sent: u32,
    /// Status over the picked period, by the rules of the row's `status`: only in `rangeStats`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// Where the path of a target loses packets, short enough to send with every row.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PathSummary {
    pub hops: u8,
    pub origin: Option<HopRef>,
    pub silent: Vec<u8>,
    pub partial: Vec<PartialHop>,
    /// Routers that say the destination is unreachable.
    pub unreachable: Vec<HopRef>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HopRef {
    pub ttl: u8,
    pub addr: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PartialHop {
    pub ttl: u8,
    pub addr: Option<String>,
    pub name: Option<String>,
    pub no_reply_pct: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Trace {
    /// Row id of the traced target.
    pub target: String,
    /// Rounds in the window.
    pub rounds: u32,
    pub hops: Vec<TraceHop>,
    /// Where the path starts on the map; null without one.
    pub home: Option<TraceHome>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TraceHop {
    pub ttl: u8,
    pub addr: Option<String>,
    pub name: Option<String>,
    /// As a row's status; "unreachable": the router says the destination is unreachable, and
    /// the path ends there (that hop is never `dest`).
    pub status: String,
    pub origin: bool,
    pub dest: bool,
    pub max: f64,
    pub series: Vec<Option<f64>>,
    pub lo: Vec<Option<f64>>,
    pub hi: Vec<Option<f64>>,
    pub loss: Vec<u32>,
    pub partial: Vec<u32>,
    /// Probes per bucket and how many of them got no answer, as in `Row`.
    pub sent_n: Vec<u32>,
    pub lost_n: Vec<u32>,
    /// Status per probe type, e.g. {"icmp": "silent"}.
    pub by_probe: BTreeMap<String, String>,
    pub stats: Stats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range_stats: Option<Stats>,
    /// The place of the hop; null for a hop without an address, and on every hop of a trace
    /// without a target address.
    pub geo: Option<HopGeo>,
}

/// Loss marks for the day strip: 200 slots over the 24 hours that end at `to_ms`.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub from_ms: i64,
    pub to_ms: i64,
    pub slots: u32,
    pub loss: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub warn_ms: f64,
    pub bad_ms: f64,
    /// "system", "light" or "dark".
    pub theme: String,
    /// "ru" or "en".
    pub lang: String,
    /// Window the main screen opens with.
    pub window: String,
    /// Look for a newer database of places when the map opens: at most one request a day,
    /// answered or not (`geo_check`).
    #[serde(default = "yes")]
    pub geo_check: bool,
    /// Home put on the map by hand; null: the first router placed in a city.
    #[serde(default)]
    pub home: Option<HomeSetting>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            warn_ms: 100.0,
            bad_ms: 200.0,
            theme: "system".into(),
            lang: "ru".into(),
            window: "10m".into(),
            geo_check: true,
            home: None,
        }
    }
}

/// A point on the globe, in degrees, rounded to 0.1°.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HomeSetting {
    pub lat: f64,
    pub lon: f64,
}

/// `set_settings` takes only the fields that change.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub warn_ms: Option<f64>,
    pub bad_ms: Option<f64>,
    pub theme: Option<String>,
    pub lang: Option<String>,
    pub window: Option<String>,
    pub geo_check: Option<bool>,
    /// No key leaves the home as it is; null clears it.
    #[serde(default, deserialize_with = "some")]
    pub home: Option<Option<HomeSetting>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTargetReq {
    pub host: String,
    pub probe: Option<String>,
    pub interval_s: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IdRes {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportReq {
    /// Row ids to include (targets and pinned hops).
    pub rows: Vec<String>,
    pub from_ms: i64,
    pub to_ms: i64,
    /// Any of "html", "png", "csv", "json".
    pub formats: Vec<String>,
    pub tz_offset_min: i32,
    /// Findings as the dialog shows them, already in the interface language; empty: none.
    #[serde(default)]
    pub findings: Vec<String>,
    /// Timeline picture drawn by the interface, for "png".
    pub png_base64: Option<String>,
    /// Folder picked by the user; `None` means Documents/Whilo Trace. The report goes into a new
    /// folder `<name>-<time>` inside it.
    pub dir: Option<String>,
    /// The route map drawn by the interface (PNG): written as map.png and shown in report.html.
    #[serde(default)]
    pub map_png_base64: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportRes {
    pub dir: String,
    pub files: Vec<String>,
}

/// The database of places, in every snapshot.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeoStatus {
    /// "ok", "missing", "unreadable" or "simulated".
    pub state: String,
    /// Release month of the database, "2026-10".
    pub month: Option<String>,
    /// "bundled", "downloaded" or "simulated"; null without a database.
    pub source: Option<String>,
    /// The last download of this run; null when there was none.
    pub download: Option<GeoDownload>,
}

impl Default for GeoStatus {
    fn default() -> Self {
        GeoStatus {
            state: "missing".into(),
            month: None,
            source: None,
            download: None,
        }
    }
}

/// A download of the database of places: in `Snapshot.geo` and the payload of the `geo` event.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeoDownload {
    /// "running", "done", "failed" or "cancelled".
    pub state: String,
    pub month: String,
    /// Bytes received so far.
    pub received: u64,
    /// Bytes of both files, when the server tells their sizes.
    pub total: Option<u64>,
    /// An error code: "net: <detail>", "http: 404", "geo_invalid: <detail>", "io: <detail>",
    /// "geo_cancelled".
    pub error: Option<String>,
}

/// Where the path starts on the map.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceHome {
    pub lat: f64,
    pub lon: f64,
    pub city: Option<String>,
    /// ISO 3166 alpha-2 country code.
    pub cc: Option<String>,
    /// In the interface language.
    pub country: Option<String>,
    /// "auto": the first router placed in a city; "manual": put by the user (no names then).
    pub source: String,
    /// The hop it was taken from.
    pub ttl: Option<u8>,
}

/// The place of a hop and its speed-of-light check.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HopGeo {
    /// "own" (drawn at home), "city", "country" or "none".
    pub level: String,
    /// The special-purpose range of an address that was not looked up: "private", "cgnat",
    /// "loopback", "link-local", "documentation", "benchmarking", "multicast", "reserved",
    /// "unspecified", "broadcast".
    pub special: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub cc: Option<String>,
    /// In the interface language.
    pub country: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub asn: Option<u32>,
    pub as_org: Option<String>,
    /// Great-circle distance from home, km.
    pub dist_km: Option<f64>,
    /// The shortest round trip over that distance in fibre and in vacuum, ms.
    pub fiber_ms: Option<f64>,
    pub vacuum_ms: Option<f64>,
    /// The round trip checked: the hop's minimum over the window, ms.
    pub rtt_ms: Option<f64>,
    /// How much longer than light in fibre the answer took; only with verdict "ok".
    pub extra_ms: Option<f64>,
    /// "ok", "doubtful" or "impossible".
    pub verdict: Option<String>,
}

/// What `geo_check` found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GeoCheckRes {
    pub offer: Option<GeoOffer>,
    /// "offer", "current", "none", "missing", "cached", "asked", "disabled", "simulated" or
    /// "error".
    pub reason: String,
}

/// Newer databases that are out.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GeoOffer {
    pub month: String,
    /// Size of the compressed City file when the server tells it; the ASN file adds about 5 MB.
    pub bytes: Option<u64>,
}

/// A picture to save: the route map as the interface drew it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveImageReq {
    /// Folder picked by the user; `None` means Documents/Whilo Trace.
    pub dir: Option<String>,
    /// File name without `.png`.
    pub name: String,
    pub png_base64: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SaveImageRes {
    pub path: String,
}
