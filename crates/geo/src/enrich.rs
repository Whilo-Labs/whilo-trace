//! Places, the home and the speed-of-light check for the hops of one trace: a pure function
//! over a source of places, which the app runs on each snapshot of a trace.

use crate::locate::Locate;
use crate::physics::{distance_km, fiber_min_ms, vacuum_min_ms, verdict, LatLon, Verdict};
use crate::special::{classify, Special};
use std::net::IpAddr;

/// One hop of a trace, as the map needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HopIn {
    pub ttl: u8,
    /// Who answered at this TTL; none for a hop that never answered.
    pub addr: Option<IpAddr>,
    /// The shortest round trip over the window, ms; none without a timed answer.
    pub rtt_min_ms: Option<f64>,
    /// This hop is the destination.
    pub dest: bool,
}

/// How precise the place of a hop is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Level {
    /// The user's own network or the provider's NAT: drawn at home.
    Own,
    /// A city with coordinates.
    City,
    /// Only the country.
    Country,
    /// No place.
    #[default]
    None,
}

impl Level {
    /// The name in the interface contract.
    pub fn name(self) -> &'static str {
        match self {
            Level::Own => "own",
            Level::City => "city",
            Level::Country => "country",
            Level::None => "none",
        }
    }
}

/// The place of one hop and its speed-of-light check. Coordinates are rounded to 0.01°,
/// distances to 0.1 km, times to 0.01 ms.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HopGeo {
    pub level: Level,
    /// The special-purpose class of an address that is not looked up: the user's own network
    /// (level own) and, outside simulation, the other special ranges (level none). Simulation
    /// looks up the documentation and benchmark ranges as public routers, and like public
    /// routers they have no class here; a hop with a place never has one.
    pub special: Option<Special>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub cc: Option<String>,
    pub country_en: Option<String>,
    pub country_ru: Option<String>,
    /// The point the source gave, kept even when the check finds it impossible.
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub asn: Option<u32>,
    pub as_org: Option<String>,
    /// Great-circle distance from home.
    pub dist_km: Option<f64>,
    /// The shortest round trip over that distance in fibre.
    pub fiber_ms: Option<f64>,
    /// The shortest round trip over that distance in vacuum.
    pub vacuum_ms: Option<f64>,
    /// The round trip the check used: the hop's window minimum.
    pub rtt_ms: Option<f64>,
    /// How much longer than light in fibre the answer took; only for an ok verdict.
    pub extra_ms: Option<f64>,
    pub verdict: Option<Verdict>,
}

/// Where the trace starts; distances count from here.
#[derive(Debug, Clone, PartialEq)]
pub struct Home {
    /// Rounded to 0.01°.
    pub at: LatLon,
    pub city: Option<String>,
    pub cc: Option<String>,
    pub country_en: Option<String>,
    pub country_ru: Option<String>,
    /// The user put it there; a manual home has no names.
    pub manual: bool,
    /// The hop it was taken from.
    pub ttl: Option<u8>,
}

/// Places of the hops of one trace to `target`, the home, and the speed-of-light check of
/// every city against it. One entry per hop, in the order given; none for a hop without an
/// address.
///
/// - The user's own network and the provider's NAT are drawn at home and never looked up.
/// - Other special-purpose addresses get no place, except in simulation (`simulated`), where
///   the documentation and benchmark ranges stand in for public routers and come out as
///   public routers do, without a special class.
/// - Home is `manual_home` when the user set one (one off the globe is ignored), else the first
///   hop in TTL order placed in a city. The destination never becomes home, neither on its own
///   row nor on another row with `target`'s address: on a path with nothing public before it,
///   as through a VPN, home would land on the far end.
/// - With a home, a hop placed in a city that has a round trip gets the distance, the shortest
///   round trips in fibre and vacuum, the verdict, and the time over fibre when the verdict is
///   ok. The home hop itself is 0 km away and ok. A country or no place gets no check.
pub fn enrich(
    loc: &dyn Locate,
    target: IpAddr,
    hops: &[HopIn],
    manual_home: Option<LatLon>,
    simulated: bool,
) -> (Option<Home>, Vec<Option<HopGeo>>) {
    let mut found: Vec<Option<(HopGeo, Option<LatLon>)>> = hops
        .iter()
        .map(|h| {
            h.addr
                .map(|addr| place(loc, target, h.ttl, addr, simulated))
        })
        .collect();

    let manual_home = manual_home.filter(|p| p.is_valid());
    let from = match manual_home {
        Some(_) => None,
        None => auto_home(hops, &found, target),
    };
    let at = manual_home.or_else(|| from.and_then(|i| found[i].as_ref()?.1));
    let Some(at) = at else {
        return (None, strip(found));
    };
    let named = from.and_then(|i| found[i].as_ref()).map(|(g, _)| g);
    let home = Home {
        at: LatLon::new(round(at.lat, 2), round(at.lon, 2)),
        city: named.and_then(|g| g.city.clone()),
        cc: named.and_then(|g| g.cc.clone()),
        country_en: named.and_then(|g| g.country_en.clone()),
        country_ru: named.and_then(|g| g.country_ru.clone()),
        manual: manual_home.is_some(),
        ttl: from.map(|i| hops[i].ttl),
    };

    for (i, (hop, f)) in hops.iter().zip(found.iter_mut()).enumerate() {
        let Some((g, Some(point))) = f else { continue };
        if g.level != Level::City {
            continue;
        }
        let rtt = hop.rtt_min_ms.filter(|r| r.is_finite() && *r >= 0.0);
        if from == Some(i) {
            check(g, 0.0, rtt);
            g.verdict = Some(Verdict::Ok);
        } else if rtt.is_some() {
            check(g, distance_km(at, *point), rtt);
        }
    }
    (Some(home), strip(found))
}

/// The place of one address, before the check, and its unrounded point.
fn place(
    loc: &dyn Locate,
    target: IpAddr,
    ttl: u8,
    addr: IpAddr,
    simulated: bool,
) -> (HopGeo, Option<LatLon>) {
    let special = classify(addr);
    match special {
        Some(s) if s.is_own() => {
            let own = HopGeo {
                level: Level::Own,
                special,
                ..HopGeo::default()
            };
            return (own, None);
        }
        Some(_) if !simulated => {
            let nowhere = HopGeo {
                special,
                ..HopGeo::default()
            };
            return (nowhere, None);
        }
        _ => {}
    }
    // A public router, or in simulation a special address standing in for one: like a public
    // router it has no class.
    let Some(rec) = loc.locate(target, ttl, addr) else {
        return (HopGeo::default(), None);
    };
    let (level, point) = (rec.level(), rec.point());
    let g = HopGeo {
        level,
        lat: point.map(|p| round(p.lat, 2)),
        lon: point.map(|p| round(p.lon, 2)),
        city: rec.city,
        region: rec.region,
        cc: rec.cc,
        country_en: rec.country_en,
        country_ru: rec.country_ru,
        asn: rec.asn,
        as_org: rec.as_org,
        ..HopGeo::default()
    };
    (g, point)
}

/// The first hop in TTL order placed in a city, the destination aside: its own row and any
/// other row that answered with its address, as rows do past the destination's after the
/// route changed length. Own addresses are never placed in a city, and other special ones
/// only in simulation.
fn auto_home(
    hops: &[HopIn],
    found: &[Option<(HopGeo, Option<LatLon>)>],
    target: IpAddr,
) -> Option<usize> {
    let target = target.to_canonical();
    hops.iter()
        .zip(found)
        .enumerate()
        .filter(|(_, (h, f))| {
            !h.dest
                && h.addr.map(|a| a.to_canonical()) != Some(target)
                && f.as_ref()
                    .is_some_and(|(g, at)| g.level == Level::City && at.is_some())
        })
        .min_by_key(|(i, (h, _))| (h.ttl, *i))
        .map(|(i, _)| i)
}

/// Distance and the shortest round trips for a hop `dist_km` from home, and the verdict when
/// its round trip is known.
fn check(g: &mut HopGeo, dist_km: f64, rtt_ms: Option<f64>) {
    let fiber = fiber_min_ms(dist_km);
    g.dist_km = Some(round(dist_km, 1));
    g.fiber_ms = Some(round(fiber, 2));
    g.vacuum_ms = Some(round(vacuum_min_ms(dist_km), 2));
    if let Some(rtt) = rtt_ms {
        let v = verdict(dist_km, rtt);
        g.rtt_ms = Some(round(rtt, 2));
        g.extra_ms = (v == Verdict::Ok).then(|| round((rtt - fiber).max(0.0), 2));
        g.verdict = Some(v);
    }
}

fn strip(found: Vec<Option<(HopGeo, Option<LatLon>)>>) -> Vec<Option<HopGeo>> {
    found.into_iter().map(|f| f.map(|(g, _)| g)).collect()
}

/// `x` to `digits` decimals; adding zero turns -0.0 into 0.0.
fn round(x: f64, digits: i32) -> f64 {
    let k = 10f64.powi(digits);
    (x * k).round() / k + 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locate::Located;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Places by address, and every address it was asked about.
    #[derive(Default)]
    struct Table {
        places: HashMap<IpAddr, Located>,
        asked: Mutex<Vec<IpAddr>>,
    }

    impl Table {
        fn with(mut self, addr: &str, place: Located) -> Self {
            self.places.insert(ip(addr), place);
            self
        }

        fn asked(&self) -> Vec<IpAddr> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl Locate for Table {
        fn locate(&self, _: IpAddr, _: u8, addr: IpAddr) -> Option<Located> {
            self.asked.lock().unwrap().push(addr);
            self.places.get(&addr).cloned()
        }
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn hop(ttl: u8, addr: Option<&str>, rtt_min_ms: Option<f64>, dest: bool) -> HopIn {
        HopIn {
            ttl,
            addr: addr.map(ip),
            rtt_min_ms,
            dest,
        }
    }

    /// Country code, English and Russian name.
    const AZ: [&str; 3] = ["AZ", "Azerbaijan", "Азербайджан"];
    const GE: [&str; 3] = ["GE", "Georgia", "Грузия"];
    const TR: [&str; 3] = ["TR", "Türkiye", "Турция"];
    const US: [&str; 3] = ["US", "United States", "США"];
    const DE: [&str; 3] = ["DE", "Germany", "Германия"];

    fn city(name: &str, country: [&str; 3], lat: f64, lon: f64) -> Located {
        let [cc, en, ru] = country;
        Located {
            city: Some(name.into()),
            cc: Some(cc.into()),
            country_en: Some(en.into()),
            country_ru: Some(ru.into()),
            lat: Some(lat),
            lon: Some(lon),
            asn: Some(64500),
            as_org: Some("Demo provider".into()),
            ..Located::default()
        }
    }

    fn baku() -> Located {
        city("Baku", AZ, 40.4093, 49.8671)
    }

    fn frankfurt() -> Located {
        city("Frankfurt am Main", DE, 50.1109, 8.6821)
    }

    /// The places of the mockup's demo route (`common/demo-route.js`).
    fn places(addrs: [&str; 8]) -> Table {
        let [core1, core2, tbilisi, istanbul, ashburn, bg, fra, google] = addrs;
        let bulgaria = Located {
            cc: Some("BG".into()),
            country_en: Some("Bulgaria".into()),
            country_ru: Some("Болгария".into()),
            lat: Some(42.7339),
            lon: Some(25.4858),
            asn: Some(64503),
            ..Located::default()
        };
        let mut mountain_view = city("Mountain View", US, 37.3861, -122.0839);
        mountain_view.asn = Some(15169);
        mountain_view.as_org = Some("Google LLC".into());
        Table::default()
            .with(core1, baku())
            .with(core2, baku())
            .with(tbilisi, city("Tbilisi", GE, 41.7151, 44.8271))
            .with(istanbul, city("Istanbul", TR, 41.0082, 28.9784))
            .with(ashburn, city("Ashburn", US, 39.0438, -77.4874))
            .with(bg, bulgaria)
            .with(fra, frankfurt())
            .with(google, mountain_view)
    }

    /// Public addresses for the demo route, a made-up allocation.
    const PUBLIC: [&str; 8] = [
        "5.0.3.1", "5.0.4.1", "5.0.6.1", "5.0.7.1", "5.0.8.1", "5.0.9.1", "5.0.10.1", "8.8.8.8",
    ];

    /// The addresses of the mockup: documentation ranges for every public router.
    const MOCKUP: [&str; 8] = [
        "198.51.100.17",
        "198.51.100.94",
        "203.0.113.5",
        "203.0.113.66",
        "203.0.113.130",
        "192.0.2.10",
        "192.0.2.77",
        "8.8.8.8",
    ];

    /// The demo route: own network, provider, a silent hop, transit and 8.8.8.8, with the
    /// window minima of the mockup.
    fn route(addrs: [&str; 8]) -> Vec<HopIn> {
        let [core1, core2, tbilisi, istanbul, ashburn, bg, fra, google] = addrs;
        vec![
            hop(1, Some("192.168.1.1"), Some(0.6), false),
            hop(2, Some("100.64.0.1"), Some(1.8), false),
            hop(3, Some(core1), Some(2.4), false),
            hop(4, Some(core2), Some(2.8), false),
            hop(5, None, None, false),
            hop(6, Some(tbilisi), Some(9.4), false),
            hop(7, Some(istanbul), Some(31.0), false),
            hop(8, Some(ashburn), Some(36.2), false),
            hop(9, Some(bg), Some(39.0), false),
            hop(10, Some(fra), Some(52.4), false),
            hop(11, Some(google), Some(53.1), true),
        ]
    }

    fn close(a: Option<f64>, b: Option<f64>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => (a - b).abs() < 1e-9,
            (a, b) => a == b,
        }
    }

    fn levels(geo: &[Option<HopGeo>]) -> String {
        let names: Vec<&str> = geo
            .iter()
            .map(|g| g.as_ref().map_or("-", |g| g.level.name()))
            .collect();
        names.join(" ")
    }

    /// TTL; distance, fibre and vacuum minimum, round trip; time over fibre; verdict.
    type Checked = (u8, [f64; 4], Option<f64>, Verdict);

    fn assert_checks(geo: &[Option<HopGeo>], want: &[Checked]) {
        for &(ttl, [dist, fiber, vacuum, rtt], extra, v) in want {
            let g = geo[usize::from(ttl) - 1].as_ref().unwrap();
            let ok = close(g.dist_km, Some(dist))
                && close(g.fiber_ms, Some(fiber))
                && close(g.vacuum_ms, Some(vacuum))
                && close(g.rtt_ms, Some(rtt))
                && close(g.extra_ms, extra)
                && g.verdict == Some(v);
            assert!(ok, "hop {ttl}: {g:?}");
        }
    }

    #[test]
    fn the_demo_route_gets_its_places_and_checks() {
        let table = places(PUBLIC);
        let hops = route(PUBLIC);
        let (home, geo) = enrich(&table, ip("8.8.8.8"), &hops, None, false);
        assert_eq!(geo.len(), hops.len());
        assert_eq!(
            home,
            Some(Home {
                at: LatLon::new(40.41, 49.87),
                city: Some("Baku".into()),
                cc: Some("AZ".into()),
                country_en: Some("Azerbaijan".into()),
                country_ru: Some("Азербайджан".into()),
                manual: false,
                ttl: Some(3),
            })
        );
        assert_eq!(
            levels(&geo),
            "own own city city - city city city country city city"
        );

        // Worked out separately with the same formulas.
        use Verdict::{Impossible, Ok as Fine};
        assert_checks(
            &geo,
            &[
                (3, [0.0, 0.0, 0.0, 2.4], Some(2.4), Fine),
                (4, [0.0, 0.0, 0.0, 2.8], Some(2.8), Fine),
                (6, [446.7, 4.47, 2.98, 9.4], Some(4.93), Fine),
                (7, [1757.8, 17.59, 11.73, 31.0], Some(13.41), Fine),
                (8, [9691.9, 96.99, 64.66, 36.2], None, Impossible),
                (10, [3350.4, 33.53, 22.35, 52.4], Some(18.87), Fine),
                (11, [11325.8, 113.34, 75.56, 53.1], None, Impossible),
            ],
        );

        // The source's names, AS and rounded point come through.
        let fra = geo[9].as_ref().unwrap();
        assert_eq!(fra.city.as_deref(), Some("Frankfurt am Main"));
        assert_eq!(fra.country_ru.as_deref(), Some("Германия"));
        assert_eq!((fra.lat, fra.lon), (Some(50.11), Some(8.68)));
        let google = geo[10].as_ref().unwrap();
        assert_eq!(google.asn, Some(15169));
        assert_eq!((google.lat, google.lon), (Some(37.39), Some(-122.08)));
        // Only the country: a point, no city, no check.
        let bg = geo[8].as_ref().unwrap();
        assert_eq!((bg.cc.as_deref(), bg.city.as_deref()), (Some("BG"), None));
        assert_eq!((bg.lat, bg.lon), (Some(42.73), Some(25.49)));
        assert_eq!((bg.dist_km, bg.rtt_ms, bg.verdict), (None, None, None));
        let own = HopGeo {
            level: Level::Own,
            special: Some(Special::Private),
            ..HopGeo::default()
        };
        assert_eq!(geo[0].as_ref(), Some(&own));
        assert_eq!(geo[1].as_ref().unwrap().special, Some(Special::Cgnat));
        assert_eq!(geo[2].as_ref().unwrap().special, None);

        // The home network and the provider's NAT never reach the source.
        let asked = table.asked();
        assert_eq!(asked.len(), 8, "{asked:?}");
        assert!(!asked.contains(&ip("192.168.1.1")) && !asked.contains(&ip("100.64.0.1")));
    }

    #[test]
    fn documentation_addresses_get_no_place_outside_simulation() {
        let table = places(MOCKUP);
        let (home, geo) = enrich(&table, ip("8.8.8.8"), &route(MOCKUP), None, false);
        // Only 8.8.8.8 is public, and the destination never becomes home.
        assert_eq!(home, None);
        assert_eq!(table.asked(), [ip("8.8.8.8")]);
        assert_eq!(
            levels(&geo),
            "own own none none - none none none none none city"
        );
        for ttl in [3, 4, 6, 7, 8, 9, 10] {
            let g = geo[ttl - 1].as_ref().unwrap();
            assert_eq!(g.special, Some(Special::Documentation), "hop {ttl}");
            assert_eq!((g.city.as_deref(), g.lat), (None, None), "hop {ttl}");
        }
        // Without a home nothing is checked, but the place stays.
        let google = geo[10].as_ref().unwrap();
        assert_eq!((google.dist_km, google.verdict), (None, None));
    }

    #[test]
    fn simulation_looks_up_documentation_addresses_as_public_routers() {
        let table = places(MOCKUP);
        let (home, geo) = enrich(&table, ip("8.8.8.8"), &route(MOCKUP), None, true);
        let home = home.unwrap();
        assert_eq!((home.ttl, home.city.as_deref()), (Some(3), Some("Baku")));
        assert_eq!(
            levels(&geo),
            "own own city city - city city city country city city"
        );
        // Own addresses are never looked up, simulation or not, and keep their class.
        let asked = table.asked();
        assert_eq!(asked.len(), 8, "{asked:?}");
        assert!(!asked.contains(&ip("192.168.1.1")));
        assert_eq!(geo[0].as_ref().unwrap().special, Some(Special::Private));
        assert_eq!(geo[1].as_ref().unwrap().special, Some(Special::Cgnat));
        // The stand-ins come out as public routers do: no class, as with a real database.
        for g in geo.iter().flatten().filter(|g| g.level != Level::Own) {
            assert_eq!(g.special, None, "{g:?}");
        }
        let tbilisi = geo[5].as_ref().unwrap();
        assert!(close(tbilisi.dist_km, Some(446.7)));
        assert_eq!(geo[7].as_ref().unwrap().verdict, Some(Verdict::Impossible));
        // One the source does not know is no different.
        let unknown = [hop(3, Some("198.51.100.200"), Some(9.0), false)];
        let (_, geo) = enrich(&table, ip("8.8.8.8"), &unknown, None, true);
        assert_eq!(geo, [Some(HopGeo::default())]);
    }

    #[test]
    fn the_target_never_becomes_home_on_any_row() {
        // Through a VPN nothing public answers before the target. After the route changed
        // length within the window the target's address shows up on a row past the
        // destination's too, or before it; home must not land on the far end either way.
        let table = places(PUBLIC);
        let google = Some("8.8.8.8");
        for target_rows in [
            [(4, false), (5, true), (6, false)],
            [(4, true), (5, false), (6, false)],
            [(4, false), (5, false), (6, true)],
        ] {
            let mut hops = vec![
                hop(1, Some("10.8.0.1"), Some(30.0), false),
                hop(2, None, None, false),
                hop(3, Some("192.168.100.1"), Some(31.0), false),
            ];
            hops.extend(
                target_rows
                    .iter()
                    .map(|&(ttl, dest)| hop(ttl, google, Some(53.1), dest)),
            );
            let (home, geo) = enrich(&table, ip("8.8.8.8"), &hops, None, false);
            assert_eq!(home, None, "{target_rows:?}");
            // The target's place stays, unchecked.
            for g in &geo[3..] {
                let g = g.as_ref().unwrap();
                assert_eq!(g.city.as_deref(), Some("Mountain View"));
                assert_eq!((g.dist_km, g.verdict), (None, None));
            }
        }
        // An IPv4-mapped target is the same target.
        let hops = [
            hop(1, Some("10.8.0.1"), Some(30.0), false),
            hop(2, google, Some(53.1), true),
            hop(3, google, Some(53.1), false),
        ];
        let (home, _) = enrich(&table, ip("::ffff:8.8.8.8"), &hops, None, false);
        assert_eq!(home, None);
        // A public router before the target is still home.
        let mut hops = route(PUBLIC);
        hops.push(hop(12, google, Some(53.4), false));
        let (home, _) = enrich(&table, ip("8.8.8.8"), &hops, None, false);
        assert_eq!(home.unwrap().ttl, Some(3));
    }

    #[test]
    fn a_manual_home_replaces_the_first_city() {
        let table = places(PUBLIC);
        let at = LatLon::new(40.4, 49.9);
        let (home, geo) = enrich(&table, ip("8.8.8.8"), &route(PUBLIC), Some(at), false);
        assert_eq!(
            home,
            Some(Home {
                at,
                city: None,
                cc: None,
                country_en: None,
                country_ru: None,
                manual: true,
                ttl: None,
            })
        );
        // The provider's router in Baku is now an ordinary hop 3 km away.
        let core = geo[2].as_ref().unwrap();
        let d = distance_km(at, LatLon::new(40.4093, 49.8671));
        assert!(close(core.dist_km, Some(round(d, 1))), "{core:?}");
        assert!((2.0..4.0).contains(&d), "{d}");
        assert_eq!(core.verdict, Some(Verdict::Ok));
        let fra = geo[9].as_ref().unwrap();
        assert_eq!(fra.verdict, Some(Verdict::Ok));
        assert!((3340.0..3360.0).contains(&fra.dist_km.unwrap()), "{fra:?}");

        // A home off the globe is no home: the first city takes over.
        let nowhere = LatLon::new(f64::NAN, 49.9);
        let (home, _) = enrich(&table, ip("8.8.8.8"), &route(PUBLIC), Some(nowhere), false);
        let home = home.unwrap();
        assert_eq!((home.manual, home.ttl), (false, Some(3)));
    }

    #[test]
    fn faster_than_fibre_is_doubtful_and_extra_time_is_never_negative() {
        let table = Table::default()
            .with("5.0.3.1", baku())
            .with("5.0.4.1", frankfurt())
            .with("5.0.5.1", frankfurt());
        let fiber = fiber_min_ms(distance_km(
            LatLon::new(40.4093, 49.8671),
            LatLon::new(50.1109, 8.6821),
        ));
        let hops = [
            hop(3, Some("5.0.3.1"), Some(2.0), false),
            // Between vacuum and fibre.
            hop(4, Some("5.0.4.1"), Some(25.0), false),
            // Within the tolerance below fibre: ok, nothing extra.
            hop(5, Some("5.0.5.1"), Some(fiber - 0.5), true),
        ];
        let (_, geo) = enrich(&table, ip("5.0.5.1"), &hops, None, false);
        let doubtful = geo[1].as_ref().unwrap();
        assert_eq!(doubtful.verdict, Some(Verdict::Doubtful));
        assert_eq!(doubtful.extra_ms, None);
        assert!(close(doubtful.fiber_ms, Some(33.53)));
        let tight = geo[2].as_ref().unwrap();
        assert_eq!(tight.verdict, Some(Verdict::Ok));
        assert_eq!(tight.extra_ms, Some(0.0));
    }

    #[test]
    fn hops_without_a_round_trip_get_no_check_but_home_stays_at_zero() {
        let table = places(PUBLIC);
        let mut hops = route(PUBLIC);
        // Answers that carry no time, as "destination unreachable" does, and a broken one.
        hops[2].rtt_min_ms = None;
        hops[9].rtt_min_ms = None;
        hops[5].rtt_min_ms = Some(f64::NAN);
        let (home, geo) = enrich(&table, ip("8.8.8.8"), &hops, None, false);
        assert_eq!(home.unwrap().ttl, Some(3));
        let core = geo[2].as_ref().unwrap();
        assert_eq!((core.dist_km, core.fiber_ms), (Some(0.0), Some(0.0)));
        assert_eq!((core.rtt_ms, core.extra_ms), (None, None));
        assert_eq!(core.verdict, Some(Verdict::Ok));
        for i in [5, 9] {
            let g = geo[i].as_ref().unwrap();
            assert_eq!(g.level, Level::City, "{g:?}");
            assert_eq!((g.dist_km, g.verdict), (None, None), "{g:?}");
        }
    }

    #[test]
    fn home_is_the_first_city_in_ttl_order_whatever_the_order_given() {
        let table = places(PUBLIC);
        let mut hops = route(PUBLIC);
        hops.reverse();
        let (home, geo) = enrich(&table, ip("8.8.8.8"), &hops, None, false);
        assert_eq!(home.unwrap().ttl, Some(3));
        // Answers stay in the order given.
        assert_eq!(
            levels(&geo),
            "city city country city city city - city city own own"
        );
        // The provider's routers are both at home; hop 4 is checked like any other.
        let hop4 = geo[7].as_ref().unwrap();
        assert_eq!((hop4.dist_km, hop4.verdict), (Some(0.0), Some(Verdict::Ok)));
    }

    #[test]
    fn no_city_no_home() {
        let only_country = Located {
            cc: Some("AZ".into()),
            ..Located::default()
        };
        let table = Table::default().with("5.0.2.1", only_country);
        let hops = [
            hop(1, Some("10.0.0.1"), Some(0.5), false),
            hop(2, Some("5.0.2.1"), Some(1.5), false),
            hop(3, Some("5.0.3.1"), Some(3.5), false),
            hop(4, None, None, true),
        ];
        let (home, geo) = enrich(&table, ip("5.0.4.1"), &hops, None, false);
        assert_eq!(home, None);
        assert_eq!(levels(&geo), "own country none -");
        // A public address the source does not know has no place.
        assert_eq!(geo[2], Some(HopGeo::default()));
        assert_eq!(
            enrich(&table, ip("5.0.4.1"), &[], None, false),
            (None, vec![])
        );
    }

    #[test]
    fn levels_have_the_names_of_the_contract() {
        assert_eq!(Level::Own.name(), "own");
        assert_eq!(Level::City.name(), "city");
        assert_eq!(Level::Country.name(), "country");
        assert_eq!(Level::None.name(), "none");
    }

    #[test]
    fn rounding_keeps_the_decimals_and_drops_negative_zero() {
        assert_eq!(round(33.5267, 2), 33.53);
        assert_eq!(round(3350.3538, 1), 3350.4);
        assert_eq!(round(-122.0839, 2), -122.08);
        assert!(round(-0.001, 2).is_sign_positive());
    }
}
