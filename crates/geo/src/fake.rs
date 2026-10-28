//! Made-up places for the simulated network. Its routers have addresses from the ranges set
//! aside for documentation and benchmarks, which no database knows, so their places come from
//! the noise-free round trips of the simulated path instead: the provider is in Baku, as in the
//! approved mockup, the destination is in a city that light in fibre reaches within its round
//! trip, and every transit router is on the way there, no farther from home than its own round
//! trip allows. The same target and TTL always get the same place; nothing is random.

use crate::locate::{Locate, Located};
use crate::physics::{along, distance_km, fiber_reach_km, LatLon};
use std::fmt;
use std::net::IpAddr;

/// Share of the reach of light in fibre a made-up place may use. The window minimum of a
/// simulated hop stays above 0.88 of its noise-free round trip, so every city passes the check.
const MARGIN: f64 = 0.85;

/// The provider's network.
const CORE_AS: (u32, &str) = (64500, "Demo provider");

/// The network of a made-up destination.
const TARGET_AS: (u32, &str) = (64511, "Demo target network");

/// Made-up places for the simulated network.
pub struct FakeGeo {
    model: Box<dyn Fn(IpAddr) -> Vec<f64> + Send + Sync>,
}

impl FakeGeo {
    /// Places from `model`: the noise-free round trips in ms to TTL 1, 2, ... of the simulated
    /// path to a target, the target last (`whilo_trace_engine::sim::model_rtts`).
    pub fn new(model: impl Fn(IpAddr) -> Vec<f64> + Send + Sync + 'static) -> Self {
        FakeGeo {
            model: Box::new(model),
        }
    }
}

impl fmt::Debug for FakeGeo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeGeo").finish_non_exhaustive()
    }
}

impl Locate for FakeGeo {
    /// The place of the router at `ttl` on the way to `target`, whatever its address: the
    /// simulated network reuses transit addresses on paths of very different lengths.
    fn locate(&self, target: IpAddr, ttl: u8, _addr: IpAddr) -> Option<Located> {
        let rtts = (self.model)(target);
        let len = u8::try_from(rtts.len()).ok().filter(|&n| n > 0)?;
        let rtt = |t: u8| rtts[usize::from(t) - 1];
        let ttl = ttl.clamp(1, len);
        let (to, asn, org) = destination(target, rtt(len));
        if ttl == len {
            return Some(to.located(asn, org));
        }
        // The home router and the provider's NAT never get here; its core is in Baku.
        if ttl <= 4 {
            return Some(BAKU.located(CORE_AS.0, CORE_AS.1));
        }
        // 7 in 63 transit routers have no record at all, 9 in 63 only their country.
        let pick = hash(&key(target, ttl)) % 63;
        if pick < 7 {
            return None;
        }
        // On the great circle to the destination, as far as the round trip has come since the
        // provider, then at the nearest city light in fibre reaches in that time.
        let (start, end) = (rtt(len.min(3)), rtt(len));
        let share = if end > start {
            ((rtt(ttl) - start) / (end - start)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let point = along(BAKU.at(), to.at(), share);
        let reach = MARGIN * fiber_reach_km(rtt(ttl));
        let near = CITIES
            .iter()
            .filter(|c| distance_km(BAKU.at(), c.at()) <= reach)
            .min_by(|a, b| distance_km(point, a.at()).total_cmp(&distance_km(point, b.at())))
            .unwrap_or(&BAKU);
        let asn = 64501 + (hash(near.name.as_bytes()) % 9) as u32;
        let mut found = near.located(asn, &format!("Transit {}", near.cc));
        if pick < 16 {
            // Only the country; the point stays the city's, within the reach of the round trip.
            found.city = None;
            found.region = None;
        }
        Some(found)
    }
}

/// Where the destination is and its network. A public address typed as it is goes where real
/// databases put it, near or not; any other destination goes to a city light in fibre reaches
/// within its round trip, one of the farther half of them, so that a long round trip goes far.
fn destination(target: IpAddr, rtt_ms: f64) -> (&'static City, u32, &'static str) {
    if let Some(&(_, city, asn, org)) = LITERALS.iter().find(|l| IpAddr::from(l.0) == target) {
        return (city, asn, org);
    }
    let reach = MARGIN * fiber_reach_km(rtt_ms);
    let mut fits: Vec<(f64, &'static City)> = CITIES[1..]
        .iter()
        .map(|c| (distance_km(BAKU.at(), c.at()), c))
        .filter(|&(d, _)| d <= reach)
        .collect();
    fits.sort_by(|a, b| a.0.total_cmp(&b.0));
    let far = &fits[fits.len() / 2..];
    let city = match far.len() {
        0 => &BAKU,
        n => far[(hash(&key(target, 0)) % n as u64) as usize].1,
    };
    (city, TARGET_AS.0, TARGET_AS.1)
}

/// FNV-1a and the SplitMix64 finalizer, as the simulated network hashes: the same value on
/// every run and every build.
fn hash(bytes: &[u8]) -> u64 {
    let mut z = bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The target's address and a TTL, as bytes to hash.
fn key(target: IpAddr, ttl: u8) -> Vec<u8> {
    let mut k = match target {
        IpAddr::V4(a) => a.octets().to_vec(),
        IpAddr::V6(a) => a.octets().to_vec(),
    };
    k.push(ttl);
    k
}

/// A city a made-up router can be in; names as DB-IP gives them, in English.
struct City {
    name: &'static str,
    region: &'static str,
    cc: &'static str,
    lat: f64,
    lon: f64,
}

impl City {
    const fn new(
        name: &'static str,
        region: &'static str,
        cc: &'static str,
        lat: f64,
        lon: f64,
    ) -> Self {
        City {
            name,
            region,
            cc,
            lat,
            lon,
        }
    }

    fn at(&self) -> LatLon {
        LatLon::new(self.lat, self.lon)
    }

    fn located(&self, asn: u32, as_org: &str) -> Located {
        let land = country(self.cc);
        Located {
            city: Some(self.name.into()),
            region: Some(self.region.into()),
            cc: Some(self.cc.into()),
            country_en: land.map(|c| c.en.into()),
            country_ru: land.map(|c| c.ru.into()),
            lat: Some(self.lat),
            lon: Some(self.lon),
            asn: Some(asn),
            as_org: Some(as_org.into()),
        }
    }
}

/// The names of a country in English and Russian.
struct Country {
    cc: &'static str,
    en: &'static str,
    ru: &'static str,
}

impl Country {
    const fn new(cc: &'static str, en: &'static str, ru: &'static str) -> Self {
        Country { cc, en, ru }
    }
}

fn country(cc: &str) -> Option<&'static Country> {
    COUNTRIES.iter().find(|c| c.cc == cc)
}

const BAKU: City = City::new("Baku", "Baku", "AZ", 40.4093, 49.8671);
const SAN_FRANCISCO: City = City::new("San Francisco", "California", "US", 37.7749, -122.4194);
const MOUNTAIN_VIEW: City = City::new("Mountain View", "California", "US", 37.3861, -122.0839);

/// Home first, then by distance from it.
static CITIES: [City; 44] = [
    BAKU,
    City::new("Sumqayit", "Sumqayit", "AZ", 40.5897, 49.6686),
    City::new("Lankaran", "Lankaran", "AZ", 38.7529, 48.8475),
    City::new("Ganja", "Ganja", "AZ", 40.6828, 46.3606),
    City::new("Makhachkala", "Dagestan", "RU", 42.9849, 47.5047),
    City::new("Aktau", "Mangystau", "KZ", 43.6532, 51.1975),
    City::new("Tabriz", "East Azerbaijan", "IR", 38.0800, 46.2919),
    City::new("Tbilisi", "Tbilisi", "GE", 41.7151, 44.8271),
    City::new("Yerevan", "Yerevan", "AM", 40.1792, 44.4991),
    City::new("Tehran", "Tehran", "IR", 35.6892, 51.3890),
    City::new("Ashgabat", "Ashgabat", "TM", 37.9601, 58.3261),
    City::new("Rostov-on-Don", "Rostov Oblast", "RU", 47.2357, 39.7015),
    City::new("Ankara", "Ankara", "TR", 39.9334, 32.8597),
    City::new("Tashkent", "Tashkent", "UZ", 41.2995, 69.2401),
    City::new("Istanbul", "Istanbul", "TR", 41.0082, 28.9784),
    City::new("Dubai", "Dubai", "AE", 25.2048, 55.2708),
    City::new("Moscow", "Moscow", "RU", 55.7558, 37.6173),
    City::new("Bucharest", "Bucharest", "RO", 44.4268, 26.1025),
    City::new("Cairo", "Cairo Governorate", "EG", 30.0444, 31.2357),
    City::new("Sofia", "Sofia-Capital", "BG", 42.6977, 23.3219),
    City::new("Warsaw", "Mazovia", "PL", 52.2297, 21.0122),
    City::new("Budapest", "Budapest", "HU", 47.4979, 19.0402),
    City::new("Vienna", "Vienna", "AT", 48.2082, 16.3738),
    City::new("Helsinki", "Uusimaa", "FI", 60.1699, 24.9384),
    City::new("Delhi", "Delhi", "IN", 28.6139, 77.2090),
    City::new("Stockholm", "Stockholm", "SE", 59.3293, 18.0686),
    City::new("Berlin", "Berlin", "DE", 52.5200, 13.4050),
    City::new("Milan", "Lombardy", "IT", 45.4642, 9.1900),
    City::new("Frankfurt am Main", "Hesse", "DE", 50.1109, 8.6821),
    City::new("Amsterdam", "North Holland", "NL", 52.3676, 4.9041),
    City::new("Paris", "Île-de-France", "FR", 48.8566, 2.3522),
    City::new("London", "England", "GB", 51.5072, -0.1276),
    City::new("Madrid", "Madrid", "ES", 40.4168, -3.7038),
    City::new("Nairobi", "Nairobi County", "KE", -1.2921, 36.8219),
    City::new("Beijing", "Beijing", "CN", 39.9042, 116.4074),
    City::new("Hong Kong", "Hong Kong", "HK", 22.3193, 114.1694),
    City::new("Singapore", "Singapore", "SG", 1.3521, 103.8198),
    City::new("Tokyo", "Tokyo", "JP", 35.6762, 139.6503),
    City::new("New York", "New York", "US", 40.7128, -74.0060),
    City::new("Ashburn", "Virginia", "US", 39.0438, -77.4874),
    SAN_FRANCISCO,
    MOUNTAIN_VIEW,
    City::new("São Paulo", "São Paulo", "BR", -23.5505, -46.6333),
    City::new("Sydney", "New South Wales", "AU", -33.8688, 151.2093),
];

/// Public addresses typed as they are, where real databases put them.
static LITERALS: [([u8; 4], &City, u32, &str); 4] = [
    ([8, 8, 8, 8], &MOUNTAIN_VIEW, 15169, "Google LLC"),
    ([8, 8, 4, 4], &MOUNTAIN_VIEW, 15169, "Google LLC"),
    ([1, 1, 1, 1], &SAN_FRANCISCO, 13335, "Cloudflare, Inc."),
    ([1, 0, 0, 1], &SAN_FRANCISCO, 13335, "Cloudflare, Inc."),
];

static COUNTRIES: [Country; 33] = [
    Country::new("AE", "United Arab Emirates", "ОАЭ"),
    Country::new("AM", "Armenia", "Армения"),
    Country::new("AT", "Austria", "Австрия"),
    Country::new("AU", "Australia", "Австралия"),
    Country::new("AZ", "Azerbaijan", "Азербайджан"),
    Country::new("BG", "Bulgaria", "Болгария"),
    Country::new("BR", "Brazil", "Бразилия"),
    Country::new("CN", "China", "Китай"),
    Country::new("DE", "Germany", "Германия"),
    Country::new("EG", "Egypt", "Египет"),
    Country::new("ES", "Spain", "Испания"),
    Country::new("FI", "Finland", "Финляндия"),
    Country::new("FR", "France", "Франция"),
    Country::new("GB", "United Kingdom", "Великобритания"),
    Country::new("GE", "Georgia", "Грузия"),
    Country::new("HK", "Hong Kong", "Гонконг"),
    Country::new("HU", "Hungary", "Венгрия"),
    Country::new("IN", "India", "Индия"),
    Country::new("IR", "Iran", "Иран"),
    Country::new("IT", "Italy", "Италия"),
    Country::new("JP", "Japan", "Япония"),
    Country::new("KE", "Kenya", "Кения"),
    Country::new("KZ", "Kazakhstan", "Казахстан"),
    Country::new("NL", "Netherlands", "Нидерланды"),
    Country::new("PL", "Poland", "Польша"),
    Country::new("RO", "Romania", "Румыния"),
    Country::new("RU", "Russia", "Россия"),
    Country::new("SE", "Sweden", "Швеция"),
    Country::new("SG", "Singapore", "Сингапур"),
    Country::new("TM", "Turkmenistan", "Туркменистан"),
    Country::new("TR", "Türkiye", "Турция"),
    Country::new("UZ", "Uzbekistan", "Узбекистан"),
    Country::new("US", "United States", "США"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enrich::{enrich, HopIn, Level};
    use crate::physics::{fiber_min_ms, verdict, Verdict};
    use std::collections::HashSet;
    use std::net::Ipv4Addr;
    use whilo_trace_engine::sim::{model_rtts, SimNet};

    fn geo() -> FakeGeo {
        FakeGeo::new(model_rtts)
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn len(target: IpAddr) -> u8 {
        model_rtts(target).len() as u8
    }

    /// Destinations of every scenario of the simulated network, named the way users name
    /// hosts, and public addresses typed as they are.
    fn targets() -> Vec<IpAddr> {
        let net = SimNet::with(false, 1);
        let words = [
            "",
            "loss",
            "flaky",
            "down",
            "unreachable",
            "silent",
            "limit",
            "far",
            "jitter",
        ];
        let mut out: Vec<IpAddr> = words
            .iter()
            .flat_map(|word| (0..30).map(move |i| format!("{word}-{i}.example")))
            .map(|name| IpAddr::V4(net.resolve(name.trim_start_matches('-')).unwrap()))
            .collect();
        out.extend(["9.9.9.9", "77.88.8.8", "5.255.255.5", "198.51.100.7"].map(ip));
        out
    }

    #[test]
    fn every_made_up_place_passes_the_fibre_check_with_a_margin() {
        let geo = geo();
        let (mut cities, mut countries) = (0, 0);
        for target in targets() {
            let rtts = model_rtts(target);
            for ttl in 3..=len(target) {
                let Some(p) = geo.locate(target, ttl, target) else {
                    continue;
                };
                // A record with only the country has a point too, and the map draws it there.
                let Some(at) = p.point() else {
                    panic!("{target} hop {ttl}: no point in {p:?}");
                };
                let d = distance_km(BAKU.at(), at);
                let rtt = rtts[usize::from(ttl) - 1];
                // Light in fibre needs at most 85 % of the noise-free round trip...
                assert!(
                    fiber_min_ms(d) <= 0.85 * rtt + 1e-9,
                    "{target} hop {ttl}: {:?} at {d:.0} km in {rtt:.2} ms",
                    p.city.or(p.cc)
                );
                // ...so the check passes even for a window minimum at 0.88 of it.
                assert_eq!(verdict(d, 0.88 * rtt), Verdict::Ok, "{target} hop {ttl}");
                match p.level() {
                    Level::City => cities += 1,
                    Level::Country => countries += 1,
                    other => panic!("{target} hop {ttl}: {other:?}"),
                }
            }
        }
        assert!(
            cities > 1500 && countries > 150,
            "{cities} cities, {countries} countries"
        );
    }

    #[test]
    fn destinations_go_as_far_as_their_round_trip_allows() {
        let geo = geo();
        let net = SimNet::with(false, 1);
        let mut far = HashSet::new();
        for i in 0..30 {
            let target = IpAddr::V4(net.resolve(&format!("far-{i}.example")).unwrap());
            let p = geo.locate(target, len(target), target).unwrap();
            assert!(
                distance_km(BAKU.at(), p.point().unwrap()) > 2500.0,
                "{target}: {p:?}"
            );
            assert_eq!(p.asn, Some(64511));
            assert_eq!(p.as_org.as_deref(), Some("Demo target network"));
            far.insert(p.city.unwrap());
        }
        assert!(far.len() >= 8, "{far:?}");
        // A short round trip stays in the region.
        let mut near = HashSet::new();
        for target in targets() {
            let rtts = model_rtts(target);
            if rtts.last().is_some_and(|&ms| ms < 12.0) {
                let p = geo.locate(target, len(target), target).unwrap();
                assert!(distance_km(BAKU.at(), p.point().unwrap()) < 1020.0, "{p:?}");
                near.insert(p.city.unwrap());
            }
        }
        assert!(near.len() >= 3, "{near:?}");
    }

    #[test]
    fn typed_public_addresses_are_where_real_databases_put_them() {
        let geo = geo();
        for (addr, city, asn, org) in [
            ("8.8.8.8", "Mountain View", 15169, "Google LLC"),
            ("8.8.4.4", "Mountain View", 15169, "Google LLC"),
            ("1.1.1.1", "San Francisco", 13335, "Cloudflare, Inc."),
            ("1.0.0.1", "San Francisco", 13335, "Cloudflare, Inc."),
        ] {
            let target = ip(addr);
            let p = geo.locate(target, len(target), target).unwrap();
            assert_eq!(p.city.as_deref(), Some(city), "{addr}");
            assert_eq!(
                (p.region.as_deref(), p.cc.as_deref()),
                (Some("California"), Some("US"))
            );
            assert_eq!(
                (p.asn, p.as_org.as_deref()),
                (Some(asn), Some(org)),
                "{addr}"
            );
        }
        // Too far for the round trip: the check says so, as it would with a real database.
        let target = ip("8.8.8.8");
        let rtts = model_rtts(target);
        let p = geo.locate(target, len(target), target).unwrap();
        let d = distance_km(BAKU.at(), p.point().unwrap());
        assert_ne!(verdict(d, *rtts.last().unwrap()), Verdict::Ok);
    }

    #[test]
    fn the_provider_is_at_home_in_baku() {
        let geo = geo();
        for target in targets() {
            for ttl in [3, 4] {
                let p = geo.locate(target, ttl, target).unwrap();
                assert_eq!(p.city.as_deref(), Some("Baku"));
                assert_eq!(p.cc.as_deref(), Some("AZ"));
                assert_eq!(p.country_ru.as_deref(), Some("Азербайджан"));
                assert_eq!(
                    (p.asn, p.as_org.as_deref()),
                    (Some(64500), Some("Demo provider"))
                );
            }
        }
    }

    #[test]
    fn transit_routers_vary_as_real_ones_do() {
        let geo = geo();
        let (mut none, mut country, mut city) = (0u32, 0u32, 0u32);
        let mut networks = HashSet::new();
        for target in targets() {
            for ttl in 5..len(target) {
                let Some(p) = geo.locate(target, ttl, target) else {
                    none += 1;
                    continue;
                };
                let asn = p.asn.unwrap();
                assert!((64501..=64509).contains(&asn), "{p:?}");
                let cc = p.cc.as_deref().unwrap();
                assert_eq!(p.as_org, Some(format!("Transit {cc}")));
                networks.insert(asn);
                match p.level() {
                    Level::City => city += 1,
                    Level::Country => {
                        country += 1;
                        assert_eq!((p.city.as_deref(), p.region.as_deref()), (None, None));
                        assert!(p.point().is_some(), "{p:?}");
                    }
                    other => panic!("{other:?}: {p:?}"),
                }
            }
        }
        let all = f64::from(none + country + city);
        // About 1 in 9 without a record, 1 in 7 with only the country.
        let (none_share, country_share) = (f64::from(none) / all, f64::from(country) / all);
        assert!((0.07..0.16).contains(&none_share), "{none} of {all}");
        assert!((0.10..0.19).contains(&country_share), "{country} of {all}");
        assert!(networks.len() >= 6, "{networks:?}");
    }

    #[test]
    fn the_same_target_and_hop_always_get_the_same_place() {
        let (a, b) = (geo(), geo());
        for target in targets() {
            for ttl in 0..=len(target) + 2 {
                assert_eq!(
                    a.locate(target, ttl, target),
                    b.locate(target, ttl, ip("192.0.2.1")),
                    "{target} hop {ttl}"
                );
            }
        }
    }

    /// The places of hops 3 to the destination: a city, a country code, or "-" for none.
    fn route_of(geo: &FakeGeo, target: IpAddr) -> String {
        let places: Vec<String> = (3..=len(target))
            .map(|ttl| match geo.locate(target, ttl, target) {
                Some(p) => p.city.or(p.cc).unwrap_or_default(),
                None => "-".into(),
            })
            .collect();
        places.join(", ")
    }

    #[test]
    fn places_are_the_same_on_every_run_and_build() {
        // Fixed places: a changed hash, table or model would move them.
        let geo = geo();
        let net = SimNet::with(false, 1);
        let loss = IpAddr::V4(net.resolve("loss.example").unwrap());
        assert_eq!(
            route_of(&geo, loss),
            "Baku, Baku, Tbilisi, Rostov-on-Don, -, RO, RO, HU, Milan, Milan"
        );
        // North over the Caspian toward California, as far as 20 ms allow; the destination
        // where databases put 8.8.8.8.
        assert_eq!(
            route_of(&geo, ip("8.8.8.8")),
            "Baku, Baku, KZ, Aktau, Aktau, -, RU, Rostov-on-Don, Mountain View"
        );
    }

    #[test]
    fn no_path_no_place() {
        let v6 = ip("2a00:1450:4001::1");
        assert_eq!(geo().locate(v6, 3, v6), None);
        let nothing = FakeGeo::new(|_| Vec::new());
        assert_eq!(nothing.locate(ip("8.8.8.8"), 3, ip("8.8.8.8")), None);
        assert_eq!(format!("{nothing:?}"), "FakeGeo { .. }");
    }

    #[test]
    fn every_city_has_its_country_and_a_place_on_the_globe() {
        let mut names = HashSet::new();
        for c in &CITIES {
            assert!(names.insert(c.name), "{} twice", c.name);
            let land = country(c.cc).unwrap_or_else(|| panic!("{}: no country {}", c.name, c.cc));
            assert!(!land.en.is_empty() && !land.ru.is_empty());
            let p = c.located(1, "x").point();
            assert!(p.is_some(), "{}", c.name);
        }
        assert_eq!(CITIES[0].name, "Baku");
        // In the order of the comment: by distance from home.
        let d: Vec<f64> = CITIES
            .iter()
            .map(|c| distance_km(BAKU.at(), c.at()))
            .collect();
        assert!(d.windows(2).all(|w| w[0] <= w[1]), "{d:?}");
    }

    #[test]
    fn a_simulated_trace_starts_in_baku_and_passes_the_check() {
        let geo = geo();
        let net = SimNet::with(false, 1);
        for name in [
            "loss.example",
            "far.example",
            "example.com",
            "jitter.example",
            "8.8.8.8",
        ] {
            let target = IpAddr::V4(net.resolve(name).unwrap());
            let rtts = model_rtts(target);
            let last = rtts.len();
            // The addresses of the simulated network, and the worst window minimum it gives:
            // 0.88 of the noise-free round trip.
            let hops: Vec<HopIn> = (1..=last)
                .map(|t| {
                    let ttl = t as u8;
                    let addr = match t {
                        1 => ip("192.168.1.1"),
                        2 => ip("100.64.0.1"),
                        3 | 4 => IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10 + ttl)),
                        t if t == last => target,
                        _ => IpAddr::V4(Ipv4Addr::new(198, 51, 100, 30 + ttl)),
                    };
                    HopIn {
                        ttl,
                        addr: Some(addr),
                        rtt_min_ms: Some(0.88 * rtts[t - 1]),
                        dest: t == last,
                    }
                })
                .collect();
            let (home, out) = enrich(&geo, target, &hops, None, true);
            let home = home.unwrap();
            assert_eq!(
                (home.ttl, home.city.as_deref()),
                (Some(3), Some("Baku")),
                "{name}"
            );
            for (h, g) in hops.iter().zip(&out) {
                let g = g.as_ref().unwrap();
                if h.ttl <= 2 {
                    assert_eq!(g.level, Level::Own, "{name} hop {}", h.ttl);
                } else if g.level == Level::City && !(h.dest && name == "8.8.8.8") {
                    assert_eq!(g.verdict, Some(Verdict::Ok), "{name} hop {}: {g:?}", h.ttl);
                }
            }
            // Outside simulation the same addresses have no places.
            let (home, _) = enrich(&geo, target, &hops, None, false);
            assert_eq!(home, None, "{name}");
        }
    }
}
