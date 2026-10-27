//! Platform probe backends.

use serde::{Deserialize, Serialize};
use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

/// Upper bound for TTLs probed in one round (also the `WaitForMultipleObjects` limit on Windows).
pub const MAX_TTL: u8 = 64;

/// What came back for one probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyKind {
    /// Echo reply from the destination.
    Echo,
    /// ICMP Time Exceeded from a router on the way.
    TimeExceeded,
    /// ICMP Destination Unreachable.
    Unreachable,
    /// Nothing arrived before the timeout.
    Timeout,
    /// The probe could not be sent or the reply made no sense.
    Error,
}

impl ReplyKind {
    pub fn answered(self) -> bool {
        matches!(
            self,
            ReplyKind::Echo | ReplyKind::TimeExceeded | ReplyKind::Unreachable
        )
    }

    /// Stable code for storage.
    pub fn code(self) -> u8 {
        match self {
            ReplyKind::Echo => 0,
            ReplyKind::TimeExceeded => 1,
            ReplyKind::Unreachable => 2,
            ReplyKind::Timeout => 3,
            ReplyKind::Error => 4,
        }
    }

    pub fn from_code(code: u8) -> Self {
        match code {
            0 => ReplyKind::Echo,
            1 => ReplyKind::TimeExceeded,
            2 => ReplyKind::Unreachable,
            3 => ReplyKind::Timeout,
            _ => ReplyKind::Error,
        }
    }
}

/// Result of the probe sent with one TTL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HopSample {
    pub ttl: u8,
    pub kind: ReplyKind,
    pub addr: Option<IpAddr>,
    /// Round-trip time in microseconds, measured locally from send to reply.
    pub rtt_us: Option<u32>,
}

impl HopSample {
    pub fn lost(ttl: u8, kind: ReplyKind) -> Self {
        Self {
            ttl,
            kind,
            addr: None,
            rtt_us: None,
        }
    }

    pub fn rtt_ms(&self) -> Option<f64> {
        self.rtt_us.map(|us| us as f64 / 1000.0)
    }
}

#[derive(Debug, Clone)]
pub struct RoundSpec {
    pub dst: Ipv4Addr,
    pub max_ttl: u8,
    pub timeout: Duration,
    pub payload: u16,
    /// Pause between probes of one round; zero sends the whole round as one burst.
    pub send_gap: Duration,
}

/// Sends one probe per TTL in `1..=max_ttl` at the same time and waits for all of them.
/// Returns one sample per TTL, in TTL order. A monitor opens one prober per round in flight
/// and runs each on its own thread, so a prober never sees two rounds at once.
pub trait Prober: Send {
    fn round(&mut self, spec: &RoundSpec) -> io::Result<Vec<HopSample>>;
}

/// Opens the probe backend for this platform. None of them needs administrator or root.
pub fn new_prober() -> io::Result<Box<dyn Prober>> {
    #[cfg(windows)]
    {
        Ok(Box::new(windows::IcmpApiProber::new()?))
    }
    #[cfg(unix)]
    {
        Ok(Box::new(unix::DgramProber::new()?))
    }
}

pub(crate) fn micros(d: Duration) -> u32 {
    d.as_micros().min(u32::MAX as u128) as u32
}
