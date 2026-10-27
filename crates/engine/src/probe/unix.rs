//! Unix backend: ICMP datagram ("ping") sockets, no root needed.
//!
//! Linux: the user's group must be inside `net.ipv4.ping_group_range`. Echo replies come
//! back as normal datagrams, Time Exceeded and Unreachable arrive on the socket error
//! queue (`IP_RECVERR`). macOS: everything arrives as normal datagrams with the IP header.

use super::{micros, HopSample, Prober, ReplyKind, RoundSpec, MAX_TTL};
use std::io;
use std::mem::{size_of, zeroed};
use std::net::{IpAddr, Ipv4Addr};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Instant;

/// Sockets opened by this process. Outside Linux replies are matched by identifier only, so
/// every socket (another target, another round in flight) needs its own.
static OPENED: AtomicU16 = AtomicU16::new(0);

const ICMP_ECHO_REPLY: u8 = 0;
const ICMP_DEST_UNREACH: u8 = 3;
const ICMP_ECHO_REQUEST: u8 = 8;
const ICMP_TIME_EXCEEDED: u8 = 11;

pub struct DgramProber {
    fd: OwnedFd,
    ident: u16,
    seq: u16,
}

enum Received {
    Nothing,
    Ignored,
    Reply {
        seq: u16,
        kind: ReplyKind,
        addr: Ipv4Addr,
    },
}

impl DgramProber {
    pub fn new() -> io::Result<Self> {
        let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, libc::IPPROTO_ICMP) };
        if raw < 0 {
            let err = io::Error::last_os_error();
            #[cfg(target_os = "linux")]
            if err.raw_os_error() == Some(libc::EACCES) {
                return Err(io::Error::new(
                    err.kind(),
                    "ICMP datagram sockets are not allowed for this user; widen sysctl net.ipv4.ping_group_range",
                ));
            }
            return Err(err);
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        set_nonblocking(fd.as_raw_fd())?;
        #[cfg(target_os = "linux")]
        set_int(fd.as_raw_fd(), libc::IPPROTO_IP, libc::IP_RECVERR, 1)?;
        Ok(Self {
            fd,
            ident: (std::process::id() as u16).wrapping_add(OPENED.fetch_add(1, Ordering::Relaxed)),
            seq: 0,
        })
    }

    fn recv_one(&self, buf: &mut [u8]) -> Received {
        let fd = self.fd.as_raw_fd();
        #[cfg(target_os = "linux")]
        if let Some(r) = recv_error_queue(fd, buf) {
            return r;
        }
        recv_datagram(fd, buf, self.ident)
    }
}

impl Prober for DgramProber {
    fn round(&mut self, spec: &RoundSpec) -> io::Result<Vec<HopSample>> {
        let fd = self.fd.as_raw_fd();
        let max_ttl = spec.max_ttl.clamp(1, MAX_TTL) as usize;
        let mut buf = [0u8; 1500];
        // Late answers from the previous round would only confuse this one.
        for _ in 0..4096 {
            if matches!(self.recv_one(&mut buf), Received::Nothing) {
                break;
            }
        }

        let base = self.seq;
        self.seq = self.seq.wrapping_add(MAX_TTL as u16);
        let dst = sockaddr_v4(spec.dst);
        let mut sent: Vec<Option<Instant>> = vec![None; max_ttl];
        let mut results: Vec<Option<HopSample>> = vec![None; max_ttl];
        let mut waiting = 0usize;
        let mut next_ttl = 1usize;
        let mut next_send = Instant::now();
        let mut deadline: Option<Instant> = None;

        loop {
            // Sending and receiving share one loop, so a reply that arrives while the burst
            // is still going out gets its timestamp right away.
            while next_ttl <= max_ttl && Instant::now() >= next_send {
                let ttl = next_ttl;
                set_int(fd, libc::IPPROTO_IP, libc::IP_TTL, ttl as libc::c_int)?;
                let packet = echo_request(self.ident, base.wrapping_add(ttl as u16), spec.payload);
                let started = Instant::now();
                let mut rc = send_to(fd, &packet, &dst);
                if rc < 0 {
                    // A pending ICMP error from the previous probe can fail one send; retry once.
                    rc = send_to(fd, &packet, &dst);
                }
                if rc < 0 {
                    results[ttl - 1] = Some(HopSample::lost(ttl as u8, ReplyKind::Error));
                } else {
                    sent[ttl - 1] = Some(started);
                    waiting += 1;
                }
                next_ttl += 1;
                next_send += spec.send_gap;
                if next_ttl > max_ttl {
                    deadline = Some(Instant::now() + spec.timeout);
                }
            }
            let now = Instant::now();
            if let Some(d) = deadline {
                if waiting == 0 || now >= d {
                    break;
                }
            }
            let wake = deadline.unwrap_or(next_send);
            let wait_ms = wake
                .saturating_duration_since(now)
                .as_micros()
                .div_ceil(1000)
                .min(i32::MAX as u128) as libc::c_int;
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let rc = unsafe { libc::poll(&mut pfd, 1, wait_ms) };
            if rc < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }
            if rc == 0 {
                continue;
            }
            // Bounded: a stream of socket errors must not spin this loop forever.
            for _ in 0..4096 {
                match self.recv_one(&mut buf) {
                    Received::Nothing => break,
                    Received::Ignored => {}
                    Received::Reply { seq, kind, addr } => {
                        let ttl = seq.wrapping_sub(base) as usize;
                        if !(1..=max_ttl).contains(&ttl) || results[ttl - 1].is_some() {
                            continue;
                        }
                        if let Some(started) = sent[ttl - 1] {
                            results[ttl - 1] = Some(HopSample {
                                ttl: ttl as u8,
                                kind,
                                addr: Some(IpAddr::V4(addr)),
                                rtt_us: Some(micros(started.elapsed())),
                            });
                            waiting -= 1;
                        }
                    }
                }
            }
        }

        Ok(results
            .into_iter()
            .enumerate()
            .map(|(i, r)| r.unwrap_or_else(|| HopSample::lost(i as u8 + 1, ReplyKind::Timeout)))
            .collect())
    }
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn set_int(fd: RawFd, level: libc::c_int, name: libc::c_int, value: libc::c_int) -> io::Result<()> {
    let rc = unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            (&value as *const libc::c_int).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn sockaddr_v4(ip: Ipv4Addr) -> libc::sockaddr_in {
    let mut sa: libc::sockaddr_in = unsafe { zeroed() };
    sa.sin_family = libc::AF_INET as libc::sa_family_t;
    sa.sin_addr = libc::in_addr {
        s_addr: u32::from_ne_bytes(ip.octets()),
    };
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    {
        sa.sin_len = size_of::<libc::sockaddr_in>() as u8;
    }
    sa
}

fn send_to(fd: RawFd, packet: &[u8], dst: &libc::sockaddr_in) -> isize {
    unsafe {
        libc::sendto(
            fd,
            packet.as_ptr().cast(),
            packet.len(),
            0,
            (dst as *const libc::sockaddr_in).cast(),
            size_of::<libc::sockaddr_in>() as libc::socklen_t,
        ) as isize
    }
}

fn echo_request(ident: u16, seq: u16, payload: u16) -> Vec<u8> {
    let mut packet = vec![b'W'; 8 + payload as usize];
    packet[0] = ICMP_ECHO_REQUEST;
    packet[1] = 0;
    packet[2..4].copy_from_slice(&[0, 0]);
    packet[4..6].copy_from_slice(&ident.to_be_bytes());
    packet[6..8].copy_from_slice(&seq.to_be_bytes());
    let sum = checksum(&packet);
    packet[2..4].copy_from_slice(&sum.to_be_bytes());
    packet
}

fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = data
        .chunks(2)
        .map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) as u32)
        .sum();
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Strips an IPv4 header if the kernel left one in front of the ICMP message.
fn skip_ip_header(data: &[u8]) -> &[u8] {
    if data.first().is_some_and(|b| b >> 4 == 4) {
        let ihl = ((data[0] & 0x0f) as usize) * 4;
        if data.len() >= ihl {
            return &data[ihl..];
        }
    }
    data
}

fn recv_datagram(fd: RawFd, buf: &mut [u8], ident: u16) -> Received {
    let mut from: libc::sockaddr_in = unsafe { zeroed() };
    let mut from_len = size_of::<libc::sockaddr_in>() as libc::socklen_t;
    let n = unsafe {
        libc::recvfrom(
            fd,
            buf.as_mut_ptr().cast(),
            buf.len(),
            libc::MSG_DONTWAIT,
            (&mut from as *mut libc::sockaddr_in).cast(),
            &mut from_len,
        )
    };
    if n < 0 {
        let err = io::Error::last_os_error();
        return if err.kind() == io::ErrorKind::WouldBlock {
            Received::Nothing
        } else {
            Received::Ignored
        };
    }
    let addr = Ipv4Addr::from(from.sin_addr.s_addr.to_ne_bytes());
    let icmp = skip_ip_header(&buf[..n as usize]);
    if icmp.len() < 8 {
        return Received::Ignored;
    }
    // Linux rewrites the identifier to the socket's own; elsewhere it must be ours.
    let ours = |id: &[u8]| cfg!(target_os = "linux") || u16::from_be_bytes([id[0], id[1]]) == ident;
    match icmp[0] {
        ICMP_ECHO_REPLY if ours(&icmp[4..6]) => Received::Reply {
            seq: u16::from_be_bytes([icmp[6], icmp[7]]),
            kind: ReplyKind::Echo,
            addr,
        },
        t @ (ICMP_TIME_EXCEEDED | ICMP_DEST_UNREACH) => {
            let inner = skip_ip_header(&icmp[8..]);
            if inner.len() < 8 || inner[0] != ICMP_ECHO_REQUEST || !ours(&inner[4..6]) {
                return Received::Ignored;
            }
            let kind = if t == ICMP_TIME_EXCEEDED {
                ReplyKind::TimeExceeded
            } else {
                ReplyKind::Unreachable
            };
            Received::Reply {
                seq: u16::from_be_bytes([inner[6], inner[7]]),
                kind,
                addr,
            }
        }
        _ => Received::Ignored,
    }
}

/// Reads one ICMP error from the socket error queue. `None` means the queue is empty.
#[cfg(target_os = "linux")]
fn recv_error_queue(fd: RawFd, buf: &mut [u8]) -> Option<Received> {
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast(),
        iov_len: buf.len(),
    };
    let mut control = [0u8; 512];
    let mut msg: libc::msghdr = unsafe { zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast();
    msg.msg_controllen = control.len() as _;
    let n = unsafe { libc::recvmsg(fd, &mut msg, libc::MSG_ERRQUEUE | libc::MSG_DONTWAIT) };
    if n < 0 {
        return None;
    }
    // The queued data is our own echo request, which carries the sequence number.
    let data = &buf[..n as usize];
    if data.len() < 8 {
        return Some(Received::Ignored);
    }
    let seq = u16::from_be_bytes([data[6], data[7]]);
    let mut cmsg = unsafe { libc::CMSG_FIRSTHDR(&msg) };
    while !cmsg.is_null() {
        let hdr = unsafe { std::ptr::read_unaligned(cmsg) };
        if hdr.cmsg_level == libc::SOL_IP && hdr.cmsg_type == libc::IP_RECVERR {
            let data_ptr = unsafe { libc::CMSG_DATA(cmsg) };
            let ee: libc::sock_extended_err = unsafe { std::ptr::read_unaligned(data_ptr.cast()) };
            if ee.ee_origin == libc::SO_EE_ORIGIN_ICMP {
                let offender: libc::sockaddr_in = unsafe {
                    std::ptr::read_unaligned(
                        data_ptr.add(size_of::<libc::sock_extended_err>()).cast(),
                    )
                };
                let addr = Ipv4Addr::from(offender.sin_addr.s_addr.to_ne_bytes());
                let kind = match ee.ee_type {
                    ICMP_TIME_EXCEEDED => ReplyKind::TimeExceeded,
                    ICMP_DEST_UNREACH => ReplyKind::Unreachable,
                    _ => return Some(Received::Ignored),
                };
                return Some(Received::Reply { seq, kind, addr });
            }
        }
        cmsg = unsafe { libc::CMSG_NXTHDR(&msg, cmsg) };
    }
    Some(Received::Ignored)
}
