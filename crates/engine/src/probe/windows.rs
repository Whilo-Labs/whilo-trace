//! Windows backend: the ICMP API from iphlpapi.dll. It does not need administrator rights.
//!
//! All TTLs of a round are sent at once with `IcmpSendEcho2`, each with its own event,
//! and collected with `WaitForMultipleObjects`. The API reports RTT in whole milliseconds,
//! so the RTT is measured locally from the send call to the event instead.

use super::{micros, HopSample, Prober, ReplyKind, RoundSpec, MAX_TTL};
use std::ffi::c_void;
use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

type Handle = *mut c_void;

const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
const ERROR_IO_PENDING: u32 = 997;
const WAIT_TIMEOUT: u32 = 258;

const IP_SUCCESS: u32 = 0;
const IP_DEST_NET_UNREACHABLE: u32 = 11002;
const IP_DEST_HOST_UNREACHABLE: u32 = 11003;
const IP_DEST_PROT_UNREACHABLE: u32 = 11004;
const IP_DEST_PORT_UNREACHABLE: u32 = 11005;
const IP_REQ_TIMED_OUT: u32 = 11010;
const IP_TTL_EXPIRED_TRANSIT: u32 = 11013;
const IP_TTL_EXPIRED_REASSEM: u32 = 11014;

#[repr(C)]
struct IpOptionInformation {
    ttl: u8,
    tos: u8,
    flags: u8,
    options_size: u8,
    options_data: *mut u8,
}

#[repr(C)]
struct IcmpEchoReply {
    address: u32,
    status: u32,
    round_trip_time: u32,
    data_size: u16,
    reserved: u16,
    data: *mut c_void,
    options: IpOptionInformation,
}

#[link(name = "iphlpapi")]
extern "system" {
    fn IcmpCreateFile() -> Handle;
    fn IcmpCloseHandle(handle: Handle) -> i32;
    fn IcmpSendEcho2(
        handle: Handle,
        event: Handle,
        apc_routine: *const c_void,
        apc_context: *const c_void,
        destination: u32,
        request_data: *const c_void,
        request_size: u16,
        request_options: *const IpOptionInformation,
        reply_buffer: *mut c_void,
        reply_size: u32,
        timeout_ms: u32,
    ) -> u32;
    fn IcmpParseReplies(reply_buffer: *mut c_void, reply_size: u32) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateEventW(
        attributes: *const c_void,
        manual_reset: i32,
        initial_state: i32,
        name: *const u16,
    ) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn WaitForMultipleObjects(
        count: u32,
        handles: *const Handle,
        wait_all: i32,
        timeout_ms: u32,
    ) -> u32;
    fn GetLastError() -> u32;
}

pub struct IcmpApiProber {
    handle: Handle,
}

// The handle is used only by the thread that owns the prober.
unsafe impl Send for IcmpApiProber {}

impl IcmpApiProber {
    pub fn new() -> io::Result<Self> {
        let handle = unsafe { IcmpCreateFile() };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { handle })
    }
}

impl Drop for IcmpApiProber {
    fn drop(&mut self) {
        unsafe { IcmpCloseHandle(self.handle) };
    }
}

/// One request in flight. The driver writes into `buf` until `event` is signaled,
/// so the buffer must not be freed before that.
struct Pending {
    ttl: u8,
    event: Handle,
    buf: Vec<u8>,
    sent: Instant,
    in_flight: bool,
    result: Option<HopSample>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        if self.in_flight {
            unsafe { WaitForMultipleObjects(1, &self.event, 1, 10_000) };
        }
        unsafe { CloseHandle(self.event) };
    }
}

impl IcmpApiProber {
    fn send(
        &self,
        ttl: u8,
        destination: u32,
        payload: &[u8],
        timeout_ms: u32,
        buf_len: usize,
    ) -> io::Result<Pending> {
        let event = unsafe { CreateEventW(null(), 1, 0, null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut p = Pending {
            ttl,
            event,
            buf: vec![0u8; buf_len],
            sent: Instant::now(),
            in_flight: false,
            result: None,
        };
        let options = IpOptionInformation {
            ttl,
            tos: 0,
            flags: 0,
            options_size: 0,
            options_data: null_mut(),
        };
        p.sent = Instant::now();
        let rc = unsafe {
            IcmpSendEcho2(
                self.handle,
                p.event,
                null(),
                null(),
                destination,
                payload.as_ptr().cast(),
                payload.len() as u16,
                &options,
                p.buf.as_mut_ptr().cast(),
                p.buf.len() as u32,
                timeout_ms,
            )
        };
        if rc != 0 {
            let rtt = p.sent.elapsed();
            p.result = Some(parse(ttl, &mut p.buf, rtt));
        } else {
            match unsafe { GetLastError() } {
                ERROR_IO_PENDING => p.in_flight = true,
                err => p.result = Some(failed(ttl, err)),
            }
        }
        Ok(p)
    }
}

impl Prober for IcmpApiProber {
    fn round(&mut self, spec: &RoundSpec) -> io::Result<Vec<HopSample>> {
        let max_ttl = spec.max_ttl.clamp(1, MAX_TTL);
        let destination = u32::from_ne_bytes(spec.dst.octets());
        let payload = vec![b'W'; spec.payload as usize];
        let timeout_ms = spec.timeout.as_millis().min(u32::MAX as u128) as u32;
        // Reply struct, echoed data, room for an ICMP error and IO_STATUS_BLOCK, plus headroom.
        let buf_len = std::mem::size_of::<IcmpEchoReply>() + payload.len() + 8 + 16 + 64;

        let mut pending: Vec<Pending> = Vec::with_capacity(max_ttl as usize);
        let mut next_ttl: u8 = 1;
        let mut next_send = Instant::now();
        let mut deadline: Option<Instant> = None;
        loop {
            // Sending and collecting share one loop, so a reply that arrives while the burst
            // is still going out gets its timestamp right away.
            while next_ttl <= max_ttl && Instant::now() >= next_send {
                pending.push(self.send(next_ttl, destination, &payload, timeout_ms, buf_len)?);
                next_ttl += 1;
                next_send += spec.send_gap;
                if next_ttl > max_ttl {
                    deadline = Some(Instant::now() + spec.timeout + Duration::from_millis(500));
                }
            }
            let waiting: Vec<usize> = (0..pending.len())
                .filter(|&i| pending[i].in_flight)
                .collect();
            if deadline.is_some() && waiting.is_empty() {
                break;
            }
            let wake = deadline.unwrap_or(next_send);
            let left = wake.saturating_duration_since(Instant::now());
            if waiting.is_empty() {
                std::thread::sleep(left);
                continue;
            }
            let handles: Vec<Handle> = waiting.iter().map(|&i| pending[i].event).collect();
            let rc = unsafe {
                WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, ceil_ms(left))
            };
            let now = Instant::now();
            if (rc as usize) < handles.len() {
                let p = &mut pending[waiting[rc as usize]];
                p.in_flight = false;
                let rtt = now.duration_since(p.sent);
                p.result = Some(parse(p.ttl, &mut p.buf, rtt));
            } else if rc == WAIT_TIMEOUT && deadline.is_none() {
                // Time to send the next probe.
            } else {
                // The driver missed its own timeout, or the wait failed. Count the rest as
                // lost; Pending::drop keeps their buffers alive until the driver lets go.
                for &i in &waiting {
                    pending[i].result = Some(HopSample::lost(pending[i].ttl, ReplyKind::Timeout));
                }
                break;
            }
        }

        Ok((1..=max_ttl)
            .map(|ttl| {
                pending
                    .iter_mut()
                    .find(|p| p.ttl == ttl)
                    .and_then(|p| p.result.take())
                    .unwrap_or_else(|| HopSample::lost(ttl, ReplyKind::Error))
            })
            .collect())
    }
}

fn ceil_ms(d: Duration) -> u32 {
    d.as_micros().div_ceil(1000).min(u32::MAX as u128) as u32
}

fn failed(ttl: u8, err: u32) -> HopSample {
    let kind = if err == IP_REQ_TIMED_OUT {
        ReplyKind::Timeout
    } else {
        ReplyKind::Error
    };
    HopSample::lost(ttl, kind)
}

fn parse(ttl: u8, buf: &mut [u8], rtt: Duration) -> HopSample {
    let count = unsafe { IcmpParseReplies(buf.as_mut_ptr().cast(), buf.len() as u32) };
    let last_error = unsafe { GetLastError() };
    let reply: IcmpEchoReply = unsafe { std::ptr::read_unaligned(buf.as_ptr().cast()) };
    if count == 0 && reply.address == 0 {
        return failed(ttl, last_error);
    }
    let kind = match reply.status {
        IP_SUCCESS => ReplyKind::Echo,
        IP_TTL_EXPIRED_TRANSIT | IP_TTL_EXPIRED_REASSEM => ReplyKind::TimeExceeded,
        IP_DEST_NET_UNREACHABLE
        | IP_DEST_HOST_UNREACHABLE
        | IP_DEST_PROT_UNREACHABLE
        | IP_DEST_PORT_UNREACHABLE => ReplyKind::Unreachable,
        IP_REQ_TIMED_OUT => return HopSample::lost(ttl, ReplyKind::Timeout),
        _ => return HopSample::lost(ttl, ReplyKind::Error),
    };
    HopSample {
        ttl,
        kind,
        addr: Some(IpAddr::V4(Ipv4Addr::from(reply.address.to_ne_bytes()))),
        rtt_us: Some(micros(rtt)),
    }
}
