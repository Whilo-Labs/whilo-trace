//! Path probing engine for Whilo Trace.
//!
//! Every round sends one ICMP echo per TTL to a target at the same time, collects who
//! answered at each hop and how fast, and keeps the rounds so statistics can be computed
//! over any time window. Rounds of one target overlap, so a short interval holds even
//! when every round waits for the timeout.

pub mod dns;
pub mod monitor;
pub mod probe;
pub mod stats;

pub use monitor::{
    now_ms, spawn_monitor, spawn_monitor_with, MonitorConfig, MonitorEvent, MonitorHandle,
    MonitorOptions, ProbeKind, ProberFactory, RoundRecord, MIN_INTERVAL,
};
pub use probe::{HopSample, Prober, ReplyKind, RoundSpec};
pub use stats::{
    hop_stats, lane, outcome, row_stats, slots, timeline, verdict, Bucket, HopStats, HopStatus,
    Lane, Outcome, RowStats, Slot, Verdict,
};
