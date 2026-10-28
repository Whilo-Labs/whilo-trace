//! A made-up network for trying Whilo Trace without sending anything. Hops, latency, routers
//! that answer only part of the probes, silent hops and loss are worked out here, so no probe
//! and no DNS query leaves the computer. The desktop app runs on it with
//! `WHILO_TRACE_SIMULATE=1`; the command line always uses the real network.
//!
//! A word in the host name picks what the path does:
//!
//! | Word | Path |
//! |---|---|
//! | `loss` | loss on the path from hop 7: a quarter of the rounds lost from there on |
//! | `flaky` | loss on the path from hop 3: a third of the rounds, 70 seconds in every 5 minutes |
//! | `down` | the destination never answers, nothing does after a few hops |
//! | `unreachable` | a router says the destination is unreachable |
//! | `silent` | a router on the way never answers |
//! | `limit` | a router answers only part of the probes sent to it |
//! | `far` | a long path, high latency |
//! | `jitter` | latency spikes from a hop on |
//!
//! A word counts when it is a whole part of the name between dots or hyphens: `loss.example`,
//! `my-loss-host.test`, but not `lossless.example`; the first word of the table wins. Any other
//! name or address gets a mix of its own, the same on every run. Addresses of the home network
//! and loopback are one hop away, the provider's carrier-grade NAT two; addresses in
//! 192.0.2.0/24 behave as `down`. Names ending in `.invalid` do not resolve, and IPv6 addresses
//! have no IPv4 address.
//!
//! Loss on the path drops a whole round from its hop on, as a congested link drops a burst of
//! probes sent within a fraction of a second, and a latency spike holds up a whole round from
//! its hop on. Every made-up address comes from the ranges set aside for documentation,
//! benchmarks, private networks and carrier-grade NAT.

use crate::monitor::{now_ms, Resolver};
use crate::probe::{micros, HopSample, Prober, ReplyKind, RoundSpec, MAX_TTL};
use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Made-up destinations come from 198.18.0.0/15, set aside for benchmarks (RFC 2544): the
/// scenario from bit 12 up, a hash of the name below it.
const SIM_BASE: u32 = u32::from_be_bytes([198, 18, 0, 0]);
const SIM_SIZE: u32 = 1 << 17;
const GOLDEN: u64 = 0x9e37_79b9_7f4a_7c15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Mixed,
    Loss,
    Flaky,
    Down,
    Unreachable,
    Silent,
    Limit,
    Far,
    Jitter,
}

/// In address order: the index is what a made-up address carries.
const SCENARIOS: [Scenario; 9] = [
    Scenario::Mixed,
    Scenario::Loss,
    Scenario::Flaky,
    Scenario::Down,
    Scenario::Unreachable,
    Scenario::Silent,
    Scenario::Limit,
    Scenario::Far,
    Scenario::Jitter,
];

/// Words of a host name and what they pick; the first one that matches wins.
const WORDS: [(&str, Scenario); 8] = [
    ("down", Scenario::Down),
    ("unreachable", Scenario::Unreachable),
    ("loss", Scenario::Loss),
    ("flaky", Scenario::Flaky),
    ("silent", Scenario::Silent),
    ("limit", Scenario::Limit),
    ("far", Scenario::Far),
    ("jitter", Scenario::Jitter),
];

fn scenario_of_name(name: &str) -> Scenario {
    let parts: Vec<&str> = name.split(['.', '-']).collect();
    WORDS
        .iter()
        .find(|(word, _)| parts.contains(word))
        .map_or(Scenario::Mixed, |&(_, s)| s)
}

fn scenario_of(ip: Ipv4Addr) -> Scenario {
    let n = u32::from(ip);
    if n.wrapping_sub(SIM_BASE) < SIM_SIZE {
        let i = ((n - SIM_BASE) >> 12) as usize;
        SCENARIOS.get(i).copied().unwrap_or(Scenario::Mixed)
    } else if ip.octets()[..3] == [192, 0, 2] {
        Scenario::Down
    } else {
        Scenario::Mixed
    }
}

/// FNV-1a and the SplitMix64 finalizer: the same value on every run and every build, which
/// std's hasher does not promise.
fn hash(bytes: &[u8]) -> u64 {
    mix(bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    }))
}

fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// SplitMix64: the noise of single probes.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(GOLDEN);
        mix(self.0)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    /// `ms` with the noise of a real path: a few percent either way.
    fn noisy(&mut self, ms: f64) -> f64 {
        // The sum of three uniforms is close enough to a bell curve.
        let n = (self.unit() + self.unit() + self.unit() - 1.5) * 2.0;
        (ms * (1.0 + 0.04 * n) + 0.3 * self.unit()).max(0.05)
    }
}

/// The path to one destination. Everything but loss episodes and the noise of single rounds
/// follows from the address, so it is the same on every run.
struct Path {
    /// TTL of the destination.
    len: u8,
    /// Round trip to the destination, ms.
    base_ms: f64,
    /// A router that answers only this share of the probes sent to it; the path through it is fine.
    limit: Option<(u8, f64)>,
    /// A router that never answers; the path through it is fine.
    silent: Option<u8>,
    /// This share of the rounds lost from this hop on, to the destination.
    loss: Option<(u8, f64)>,
    /// Loss that comes and goes.
    episodes: Option<Episodes>,
    /// Nothing answers beyond this hop.
    dead_after: Option<u8>,
    /// The router after `dead_after` says the destination is unreachable.
    unreachable: bool,
    /// Share of the rounds held up by a latency spike from hop `spike_from` on.
    spikes: f64,
    spike_from: u8,
    seed: u64,
}

/// `share` of the rounds lost from hop `from` on, for `last_s` seconds of every `every_s`.
struct Episodes {
    every_s: i64,
    last_s: i64,
    offset_s: i64,
    from: u8,
    share: f64,
}

impl Path {
    fn to(dst: Ipv4Addr) -> Self {
        let h = hash(&dst.octets());
        let mut p = Path {
            len: 1,
            base_ms: 0.5,
            limit: None,
            silent: None,
            loss: None,
            episodes: None,
            dead_after: None,
            unreachable: false,
            spikes: 0.0,
            spike_from: 1,
            seed: h,
        };
        // A host of the home network answers at once, the provider's NAT gateway behind it.
        match dst.octets() {
            [127, ..] | [10, ..] | [192, 168, ..] => return p,
            [172, b, ..] if (16..=31).contains(&b) => return p,
            [100, b, ..] if (64..=127).contains(&b) => {
                p.len = 2;
                p.base_ms = 3.0;
                return p;
            }
            _ => {}
        }
        // Choices that differ between destinations and stay the same between runs.
        let pick = |shift: u32, n: u8| ((h >> shift) % u64::from(n)) as u8;
        p.len = 7 + pick(0, 7);
        p.base_ms = [6.0, 14.0, 30.0, 48.0, 75.0][pick(4, 5) as usize] + f64::from(pick(8, 12));
        let scenario = scenario_of(dst);
        match scenario {
            Scenario::Mixed => {
                // A provider router that limits its answers on a third of the paths, a silent
                // transit router on a quarter, loss now and then on a third.
                if pick(12, 3) == 0 {
                    p.limit = Some((3, 0.55));
                }
                if pick(14, 4) == 0 {
                    p.silent = Some(5);
                }
                if pick(16, 3) == 0 {
                    let every_s = 600 + 2 * i64::from(pick(18, 240));
                    p.episodes = Some(Episodes {
                        every_s,
                        last_s: 45 + i64::from(pick(26, 60)),
                        offset_s: (h >> 40) as i64 % every_s,
                        from: 3 + pick(32, p.len - 3),
                        share: 0.15 + f64::from(pick(36, 25)) / 100.0,
                    });
                }
            }
            Scenario::Loss => {
                p.len = p.len.max(9);
                p.loss = Some((7, 0.25));
            }
            Scenario::Flaky => {
                p.episodes = Some(Episodes {
                    every_s: 300,
                    last_s: 70,
                    offset_s: (h >> 40) as i64 % 300,
                    from: 3,
                    share: 0.35,
                });
            }
            Scenario::Down | Scenario::Unreachable => {
                let dead = 4 + pick(20, 3);
                p.len = p.len.max(dead + 3);
                p.dead_after = Some(dead);
                p.unreachable = scenario == Scenario::Unreachable;
            }
            Scenario::Silent => p.silent = Some(4 + pick(20, 2)),
            Scenario::Limit => p.limit = Some((3, 0.4)),
            Scenario::Far => {
                p.len = 16 + pick(0, 7);
                p.base_ms = 170.0 + f64::from(pick(8, 90));
            }
            Scenario::Jitter => {
                p.spikes = 0.08;
                p.spike_from = 3 + pick(24, p.len - 3);
            }
        }
        p
    }

    /// Loss on the way at `now_s`: the steady one or an episode, whichever is worse.
    fn loss_at(&self, now_s: i64) -> Option<(u8, f64)> {
        let episode = self
            .episodes
            .as_ref()
            .filter(|e| (now_s + e.offset_s).rem_euclid(e.every_s) < e.last_s)
            .map(|e| (e.from, e.share));
        match (self.loss, episode) {
            (Some(a), Some(b)) => Some(if a.1 >= b.1 { a } else { b }),
            (a, b) => a.or(b),
        }
    }

    /// Who answers for `ttl`: the home router, the provider behind carrier-grade NAT and its
    /// core (shared by every path), then transit routers of this path, then the destination.
    fn addr(&self, ttl: u8, dst: Ipv4Addr) -> Ipv4Addr {
        if ttl >= self.len {
            return dst;
        }
        match ttl {
            1 => Ipv4Addr::new(192, 168, 1, 1),
            2 => Ipv4Addr::new(100, 64, 0, 1),
            3 | 4 => Ipv4Addr::new(203, 0, 113, 10 * (1 + (self.seed >> 48) % 3) as u8 + ttl),
            _ => {
                let x = (self.seed >> 8).wrapping_add(u64::from(ttl) * 37) % 250;
                Ipv4Addr::new(198, 51, 100, 1 + x as u8)
            }
        }
    }

    /// Round trip to hop `ttl` without noise, ms. It grows along the path, and every router
    /// takes a little time of its own to answer.
    fn rtt_ms(&self, ttl: u8) -> f64 {
        if ttl >= self.len {
            return self.base_ms;
        }
        let own = ((self.seed >> (ttl % 40)) % 30) as f64 / 10.0;
        if ttl == 1 {
            return 0.4 + own / 10.0;
        }
        let f = f64::from(ttl) / f64::from(self.len);
        0.4 + (self.base_ms - 0.4) * f.powf(1.4) + own
    }
}

/// Round trips without noise to every hop of the path to `target`, ms: TTL 1 first, the
/// destination last. The route map places its made-up routers by them. Empty for an IPv6
/// address, which the simulated network does not reach.
pub fn model_rtts(target: IpAddr) -> Vec<f64> {
    let IpAddr::V4(dst) = target else {
        return Vec::new();
    };
    let path = Path::to(dst);
    (1..=path.len).map(|ttl| path.rtt_ms(ttl)).collect()
}

/// The simulated network: one for the app, shared by every monitor.
pub struct SimNet {
    /// Rounds take as long as real ones; tests turn it off.
    realtime: bool,
    seeds: AtomicU64,
    /// Destinations by the name they were looked up with, for their reverse name.
    names: Mutex<HashMap<Ipv4Addr, String>>,
}

impl SimNet {
    pub fn new() -> Arc<Self> {
        Self::with(true, now_ms() as u64)
    }

    /// `realtime: false` answers a round at once. `seed` makes the noise of a prober used alone
    /// repeat; probers take their seeds in the order they open.
    pub fn with(realtime: bool, seed: u64) -> Arc<Self> {
        Arc::new(Self {
            realtime,
            seeds: AtomicU64::new(seed),
            names: Mutex::default(),
        })
    }

    /// Looks a host up as `monitor::resolve_v4` does, without a DNS server: a name gets a
    /// made-up address that carries its scenario, an IPv4 address stays itself.
    pub fn resolve(&self, host: &str) -> io::Result<Ipv4Addr> {
        let host = host.trim();
        if let Ok(ip) = host.parse::<Ipv4Addr>() {
            return Ok(ip);
        }
        if host.parse::<Ipv6Addr>().is_ok() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no IPv4 address"));
        }
        let name = host.trim_end_matches('.').to_lowercase();
        let blank = name.is_empty() || name.chars().any(|c| c.is_whitespace() || c.is_control());
        if blank || name == "invalid" || name.ends_with(".invalid") {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no such host on the simulated network",
            ));
        }
        let scenario = scenario_of_name(&name);
        let index = SCENARIOS.iter().position(|&s| s == scenario).unwrap_or(0) as u32;
        let ip = Ipv4Addr::from(SIM_BASE + (index << 12) + (hash(name.as_bytes()) & 0xfff) as u32);
        lock(&self.names).insert(ip, host.to_string());
        Ok(ip)
    }

    /// `resolve` as a monitor option.
    pub fn resolver(self: &Arc<Self>) -> Resolver {
        let net = self.clone();
        Arc::new(move |host: &str| net.resolve(host))
    }

    /// The reverse name of an address on the simulated network: the name a destination was
    /// looked up with, made-up names for the provider's and transit routers, none for the rest.
    pub fn name(&self, ip: IpAddr) -> Option<String> {
        let IpAddr::V4(v4) = ip else { return None };
        if let Some(name) = lock(&self.names).get(&v4) {
            return Some(name.clone());
        }
        let [a, b, c, d] = v4.octets();
        match (a, b, c) {
            (100, 64, 0) => Some("cgnat.provider.example".into()),
            (203, 0, 113) => Some(format!("core{}-{}.provider.example", d / 10, d % 10)),
            (198, 51, 100) => Some(format!("ae-{d}.transit.example")),
            _ => None,
        }
    }

    /// A probe backend for one round in flight.
    pub fn prober(&self) -> Box<dyn Prober> {
        // Mixed, so that probers do not read the same stream a few numbers apart.
        let seed = mix(self.seeds.fetch_add(GOLDEN, Ordering::Relaxed));
        Box::new(SimProber {
            realtime: self.realtime,
            rng: Rng(seed),
        })
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct SimProber {
    realtime: bool,
    rng: Rng,
}

impl Prober for SimProber {
    fn round(&mut self, spec: &RoundSpec) -> io::Result<Vec<HopSample>> {
        let path = Path::to(spec.dst);
        // Loss on the path takes the whole round from its hop on, and so does a latency spike.
        let dropped_from = path
            .loss_at(now_ms() / 1000)
            .filter(|&(_, share)| self.rng.chance(share))
            .map(|(from, _)| from);
        let spike_ms = (path.spikes > 0.0 && self.rng.chance(path.spikes))
            .then(|| 40.0 + 120.0 * self.rng.unit());
        // The TTLs the platform backends probe.
        let samples: Vec<HopSample> = (1..=spec.max_ttl.clamp(1, MAX_TTL))
            .map(|ttl| self.probe(&path, spec, ttl, dropped_from, spike_ms))
            .collect();
        if self.realtime {
            std::thread::sleep(round_time(spec, &samples));
        }
        Ok(samples)
    }
}

impl SimProber {
    fn probe(
        &mut self,
        path: &Path,
        spec: &RoundSpec,
        ttl: u8,
        dropped_from: Option<u8>,
        spike_ms: Option<f64>,
    ) -> HopSample {
        let lost = HopSample::lost(ttl, ReplyKind::Timeout);
        if dropped_from.is_some_and(|from| ttl >= from) {
            return lost;
        }
        if let Some(dead) = path.dead_after.filter(|&d| ttl > d) {
            if !path.unreachable {
                return lost;
            }
            let rtt = self.rng.noisy(path.rtt_ms(dead + 1));
            let from = path.addr(dead + 1, spec.dst);
            return answer(spec, ttl, ReplyKind::Unreachable, from, rtt);
        }
        let at_dst = ttl >= path.len;
        if !at_dst && path.silent == Some(ttl) {
            return lost;
        }
        if !at_dst
            && path
                .limit
                .is_some_and(|(hop, share)| hop == ttl && !self.rng.chance(share))
        {
            return lost;
        }
        let kind = if at_dst {
            ReplyKind::Echo
        } else {
            ReplyKind::TimeExceeded
        };
        let spike = spike_ms.filter(|_| ttl >= path.spike_from).unwrap_or(0.0);
        let rtt = self.rng.noisy(path.rtt_ms(ttl)) + spike;
        answer(spec, ttl, kind, path.addr(ttl, spec.dst), rtt)
    }
}

/// An answer that came back after `rtt_ms`, or a timeout when that is too late.
fn answer(spec: &RoundSpec, ttl: u8, kind: ReplyKind, addr: Ipv4Addr, rtt_ms: f64) -> HopSample {
    let rtt = Duration::from_secs_f64(rtt_ms / 1000.0);
    if rtt >= spec.timeout {
        return HopSample::lost(ttl, ReplyKind::Timeout);
    }
    HopSample {
        ttl,
        kind,
        addr: Some(IpAddr::V4(addr)),
        rtt_us: Some(micros(rtt)),
    }
}

/// How long a real round with these answers takes: the sends, then the slowest answer, or the
/// timeout when one never came.
fn round_time(spec: &RoundSpec, samples: &[HopSample]) -> Duration {
    let sends = spec.send_gap * samples.len().saturating_sub(1) as u32;
    if samples.iter().any(|s| s.rtt_us.is_none()) {
        return sends + spec.timeout;
    }
    let slowest = samples.iter().filter_map(|s| s.rtt_us).max().unwrap_or(0);
    sends + Duration::from_micros(slowest.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::{
        assemble, next_path_len, spawn_monitor_with, MonitorConfig, MonitorEvent, MonitorOptions,
        ProberFactory, RoundRecord,
    };
    use crate::stats::{hop_stats, HopStats, HopStatus};

    fn spec(dst: Ipv4Addr, max_ttl: u8, timeout_ms: u64) -> RoundSpec {
        RoundSpec {
            dst,
            max_ttl,
            timeout: Duration::from_millis(timeout_ms),
            payload: 32,
            send_gap: Duration::ZERO,
        }
    }

    /// Share of the probes answered at each TTL over `n` rounds to `host`, up to one past the
    /// destination.
    fn answered(host: &str, n: usize) -> (Path, Vec<f64>) {
        let net = SimNet::with(false, 42);
        let dst = net.resolve(host).unwrap();
        let path = Path::to(dst);
        let mut prober = net.prober();
        let mut hits = vec![0usize; path.len as usize + 1];
        for _ in 0..n {
            for s in prober.round(&spec(dst, path.len + 1, 2000)).unwrap() {
                if s.kind.answered() {
                    hits[s.ttl as usize - 1] += 1;
                }
            }
        }
        (path, hits.iter().map(|&h| h as f64 / n as f64).collect())
    }

    /// The statuses the app derives (`stats::hop_stats`) from 400 rounds to `host`, sent and
    /// assembled as the monitor does: all 30 TTLs until the destination answers, then up to it.
    fn statuses(host: &str) -> (Path, Vec<HopStats>) {
        let net = SimNet::with(false, 42);
        let dst = net.resolve(host).unwrap();
        let path = Path::to(dst);
        let mut prober = net.prober();
        let mut known = None;
        let rounds: Vec<RoundRecord> = (0..400)
            .map(|i| {
                let samples = prober.round(&spec(dst, known.unwrap_or(30), 2000)).unwrap();
                let round = assemble(dst, i * 2500, samples);
                known = next_path_len(known, &round);
                round
            })
            .collect();
        (path, hop_stats(&rounds))
    }

    #[test]
    fn names_pick_scenarios_and_keep_their_address() {
        let net = SimNet::with(false, 1);
        let loss = net.resolve("loss.example").unwrap();
        // The same on every run and every build: a changed hash moves every made-up target.
        assert_eq!(loss, Ipv4Addr::new(198, 18, 20, 6));
        assert_eq!(net.resolve(" LOSS.example. ").unwrap(), loss);
        let of = |host: &str| scenario_of(net.resolve(host).unwrap());
        assert_eq!(of("my-loss-host.test"), Scenario::Loss);
        assert_eq!(of("down-and-loss.example"), Scenario::Down);
        assert_eq!(of("unreachable.example"), Scenario::Unreachable);
        assert_eq!(of("far.example"), Scenario::Far);
        for host in [
            "example.com",
            "download.example",
            "farm.example",
            "lossless.example",
        ] {
            assert_eq!(of(host), Scenario::Mixed, "{host}");
        }
        assert_eq!(net.resolve("8.8.8.8").unwrap(), Ipv4Addr::new(8, 8, 8, 8));
        assert_eq!(scenario_of(Ipv4Addr::new(192, 0, 2, 1)), Scenario::Down);
        assert!(net.resolve("under_score.example").is_ok());
        assert!(net.resolve("typo.invalid").is_err());
        assert!(net.resolve("bad host").is_err());
        assert!(net.resolve(" ").is_err());
        let v6 = net.resolve("::1").unwrap_err().to_string();
        assert!(v6.ends_with("no IPv4 address"), "{v6}");
    }

    #[test]
    fn reverse_names_come_from_the_simulated_network() {
        let net = SimNet::with(false, 1);
        let dst = net.resolve("Loss.Example").unwrap();
        assert_eq!(net.name(IpAddr::V4(dst)).as_deref(), Some("Loss.Example"));
        assert_eq!(
            net.name(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)))
                .as_deref(),
            Some("cgnat.provider.example")
        );
        assert_eq!(net.name(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))), None);
    }

    #[test]
    fn paths_are_the_same_on_every_run() {
        // Fixed numbers: a hash that changed between runs or builds would move them.
        assert_eq!(Path::to(Ipv4Addr::new(1, 1, 1, 1)).len, 9);
        assert_eq!(Path::to(Ipv4Addr::new(8, 8, 8, 8)).len, 11);
        let dst = Ipv4Addr::new(198, 51, 100, 7);
        let p = Path::to(dst);
        assert!((2..p.len).all(|ttl| p.rtt_ms(ttl) > p.rtt_ms(1)));
        assert_eq!(p.addr(1, dst), Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(p.addr(p.len, dst), dst);
        assert_eq!(p.addr(p.len + 3, dst), dst);
    }

    #[test]
    fn model_round_trips_are_the_path_without_noise() {
        // Fixed numbers: the route map puts its made-up routers where these allow.
        let rtts = model_rtts(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)));
        let want = [
            0.42,
            3.801_973_110_606,
            5.878_894_161_806,
            7.755_435_548_890,
            8.199_266_092_797,
            9.389_151_988_026,
            12.609_828_344_360,
            15.349_669_651_862,
            17.899_458_504_833,
            20.351_666_047_328,
            20.0,
        ];
        assert_eq!(rtts.len(), want.len());
        for (i, (got, want)) in rtts.iter().zip(want).enumerate() {
            assert!((got - want).abs() < 1e-9, "hop {}: {got}", i + 1);
        }
        // Hosts at home are one hop away; IPv6 is not reached.
        assert_eq!(model_rtts(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))), [0.5]);
        assert!(model_rtts(IpAddr::V6(Ipv6Addr::LOCALHOST)).is_empty());
    }

    #[test]
    fn hosts_at_home_are_one_hop_away() {
        let net = SimNet::with(false, 1);
        for (dst, len) in [
            (Ipv4Addr::new(192, 168, 1, 1), 1),
            (Ipv4Addr::new(10, 0, 0, 5), 1),
            (Ipv4Addr::new(127, 0, 0, 1), 1),
            (Ipv4Addr::new(100, 64, 0, 1), 2),
        ] {
            let round = net.prober().round(&spec(dst, 30, 2000)).unwrap();
            let first = round.iter().find(|s| s.kind == ReplyKind::Echo).unwrap();
            assert_eq!(
                (first.ttl, first.addr),
                (len, Some(IpAddr::V4(dst))),
                "{dst}"
            );
        }
    }

    #[test]
    fn rounds_probe_the_ttls_of_the_platform_backends() {
        let net = SimNet::with(false, 1);
        let dst = Ipv4Addr::new(198, 51, 100, 7);
        assert_eq!(net.prober().round(&spec(dst, 0, 2000)).unwrap().len(), 1);
        let many = net.prober().round(&spec(dst, 200, 2000)).unwrap();
        assert_eq!(many.len(), usize::from(MAX_TTL));
    }

    #[test]
    fn a_router_that_limits_its_answers_and_a_clean_path_behind_it() {
        let (path, share) = answered("limit.example", 400);
        let (hop, _) = path.limit.unwrap();
        for (i, s) in share.iter().enumerate() {
            let ttl = i as u8 + 1;
            if ttl == hop {
                assert!((0.3..=0.5).contains(s), "hop {ttl}: {s}");
            } else {
                assert_eq!(*s, 1.0, "hop {ttl}");
            }
        }
    }

    #[test]
    fn a_silent_router_and_answers_behind_it() {
        let (path, share) = answered("silent.example", 50);
        let hop = path.silent.unwrap();
        for (i, s) in share.iter().enumerate() {
            let ttl = i as u8 + 1;
            assert_eq!(*s, if ttl == hop { 0.0 } else { 1.0 }, "hop {ttl}");
        }
    }

    #[test]
    fn loss_drops_whole_rounds_from_hop_7() {
        let net = SimNet::with(false, 42);
        let dst = net.resolve("loss.example").unwrap();
        let path = Path::to(dst);
        assert!(path.len >= 9);
        let mut prober = net.prober();
        let mut dropped = 0;
        for _ in 0..600 {
            let round = prober.round(&spec(dst, path.len, 2000)).unwrap();
            let (near, far) = round.split_at(6);
            assert!(near.iter().all(|s| s.kind.answered()));
            let lost = far.iter().filter(|s| !s.kind.answered()).count();
            assert!(
                lost == 0 || lost == far.len(),
                "{lost} of {} lost",
                far.len()
            );
            dropped += usize::from(lost > 0);
        }
        assert!((90..=210).contains(&dropped), "{dropped} of 600 rounds");
    }

    #[test]
    fn jitter_holds_up_whole_rounds_from_its_hop() {
        let net = SimNet::with(false, 42);
        let dst = net.resolve("jitter.example").unwrap();
        let path = Path::to(dst);
        let mut prober = net.prober();
        // Noise stays within 12 % and 0.3 ms; a spike adds at least 40 ms.
        let late = |s: &HopSample| s.rtt_ms().unwrap() > path.rtt_ms(s.ttl) * 1.13 + 0.5;
        let mut spiked = 0;
        for _ in 0..400 {
            let round = prober.round(&spec(dst, path.len, 2000)).unwrap();
            let (before, after) = round.split_at(usize::from(path.spike_from) - 1);
            assert!(!before.iter().any(late));
            let n = after.iter().filter(|&s| late(s)).count();
            assert!(n == 0 || n == after.len(), "{n} of {} late", after.len());
            spiked += usize::from(n > 0);
        }
        assert!((10..=70).contains(&spiked), "{spiked} of 400 rounds");
    }

    #[test]
    fn scenarios_show_the_statuses_they_are_for() {
        // Loss on the path from hop 7: every hop from there on loses the same rounds.
        let (path, h) = statuses("loss.example");
        let origin: Vec<u8> = h.iter().filter(|x| x.loss_origin).map(|x| x.ttl).collect();
        assert_eq!(origin, [7], "{h:?}");
        for x in &h {
            let want = if x.ttl < 7 {
                HopStatus::Ok
            } else {
                HopStatus::Lossy
            };
            assert_eq!(x.status, want, "hop {}: {h:?}", x.ttl);
        }
        assert!(h[usize::from(path.len) - 1].is_destination);

        // The router that says the destination is unreachable ends the path, and it is not
        // the destination.
        let (path, h) = statuses("unreachable.example");
        let dead = usize::from(path.dead_after.unwrap());
        assert_eq!(h.len(), dead + 1);
        assert_eq!(h[dead].status, HopStatus::Unreachable);
        assert!(h.iter().all(|x| !x.is_destination && !x.loss_origin));
        assert!(h[..dead].iter().all(|x| x.status == HopStatus::Ok));

        let (path, h) = statuses("limit.example");
        let (hop, _) = path.limit.unwrap();
        for x in &h {
            let want = if x.ttl == hop {
                HopStatus::RateLimited
            } else {
                HopStatus::Ok
            };
            assert_eq!(x.status, want, "hop {}: {h:?}", x.ttl);
        }

        let (path, h) = statuses("silent.example");
        let hop = path.silent.unwrap();
        for x in &h {
            let want = if x.ttl == hop {
                HopStatus::Silent
            } else {
                HopStatus::Ok
            };
            assert_eq!(x.status, want, "hop {}: {h:?}", x.ttl);
        }

        let (path, h) = statuses("down.example");
        let dead = usize::from(path.dead_after.unwrap());
        assert_eq!(h.len(), dead + 1);
        assert_eq!(h[dead].status, HopStatus::NoReply);
        assert!(h[..dead].iter().all(|x| x.status == HopStatus::Ok));
    }

    #[test]
    fn a_destination_that_is_down_never_answers() {
        let (path, share) = answered("down.example", 50);
        let dead = path.dead_after.unwrap();
        for (i, s) in share.iter().enumerate() {
            let ttl = i as u8 + 1;
            assert_eq!(*s, if ttl <= dead { 1.0 } else { 0.0 }, "hop {ttl}");
        }
    }

    #[test]
    fn an_unreachable_destination_is_reported_by_the_router_before_it() {
        let net = SimNet::with(false, 3);
        let dst = net.resolve("unreachable.example").unwrap();
        let path = Path::to(dst);
        let dead = path.dead_after.unwrap();
        let round = net.prober().round(&spec(dst, 30, 2000)).unwrap();
        let first = round
            .iter()
            .find(|s| s.kind == ReplyKind::Unreachable)
            .unwrap();
        assert_eq!(first.ttl, dead + 1);
        assert!(first.ttl < path.len);
        assert_ne!(first.addr, Some(IpAddr::V4(dst)));
        assert!(round.iter().all(|s| s.kind != ReplyKind::Echo));
    }

    #[test]
    fn loss_episodes_come_and_go() {
        let net = SimNet::with(false, 1);
        let path = Path::to(net.resolve("flaky.example").unwrap());
        let on: Vec<(u8, f64)> = (0..600).filter_map(|t| path.loss_at(t)).collect();
        assert_eq!(on.len(), 140, "70 s in every 5 minutes");
        assert!(on.iter().all(|&(from, share)| from == 3 && share == 0.35));
    }

    #[test]
    fn a_round_takes_the_slowest_answer_or_the_timeout() {
        let hop = |ttl, rtt_us: Option<u32>| HopSample {
            ttl,
            kind: if rtt_us.is_some() {
                ReplyKind::TimeExceeded
            } else {
                ReplyKind::Timeout
            },
            addr: None,
            rtt_us,
        };
        let mut s = spec(Ipv4Addr::new(198, 51, 100, 7), 3, 2000);
        s.send_gap = Duration::from_millis(25);
        let ms = Duration::from_millis;
        let answered = [hop(1, Some(500)), hop(2, Some(40_000)), hop(3, Some(9_000))];
        assert_eq!(round_time(&s, &answered), ms(50) + ms(40));
        let one_lost = [hop(1, Some(500)), hop(2, None), hop(3, Some(9_000))];
        assert_eq!(round_time(&s, &one_lost), ms(50) + ms(2000));
    }

    #[test]
    fn a_monitor_runs_on_it_without_dns() {
        let net = SimNet::with(false, 9);
        let p = net.clone();
        let prober: ProberFactory = Box::new(move || Ok(p.prober()));
        let mut cfg = MonitorConfig::new("somewhere.example");
        cfg.interval = Duration::from_millis(50);
        let opts = MonitorOptions {
            prober: Some(prober),
            resolve: Some(net.resolver()),
            ..Default::default()
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = spawn_monitor_with(cfg, opts, move |ev| {
            let _ = tx.send(ev);
        })
        .unwrap();
        let event = rx.recv_timeout(Duration::from_secs(5));
        handle.stop_and_join();
        match event {
            Ok(MonitorEvent::Round(r)) => {
                let dst = net.resolve("somewhere.example").unwrap();
                assert_eq!(r.dst, IpAddr::V4(dst));
                assert!(!r.hops.is_empty());
            }
            other => panic!("{other:?}"),
        }
    }
}
