//! The speed-of-light check: how far from home a router can be when its answer came back in a
//! given time. A round trip covers the way twice; light covers about 300 km per millisecond in
//! vacuum and two thirds of that in fibre, so an answer in 10 ms comes from at most about
//! 1500 km away, and from at most about 1000 km over fibre.

/// A point on the globe, in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatLon {
    pub lat: f64,
    pub lon: f64,
}

impl LatLon {
    pub const fn new(lat: f64, lon: f64) -> Self {
        LatLon { lat, lon }
    }

    /// Finite and on the globe: latitude within ±90°, longitude within ±180°.
    pub fn is_valid(self) -> bool {
        self.lat.is_finite()
            && self.lon.is_finite()
            && self.lat.abs() <= 90.0
            && self.lon.abs() <= 180.0
    }
}

/// Mean radius of the Earth.
pub const EARTH_RADIUS_KM: f64 = 6371.0;

/// The speed of light in vacuum.
pub const LIGHT_KM_PER_S: f64 = 299_792.458;

/// Refractive index of fibre: light in it is 1.5 times slower than in vacuum.
pub const FIBER_INDEX: f64 = 1.5;

/// Slack for a home known only to the city and for the granularity of timers.
pub const TOLERANCE_MS: f64 = 1.0;

/// Great-circle distance (haversine).
pub fn distance_km(a: LatLon, b: LatLon) -> f64 {
    let (pa, pb) = (a.lat.to_radians(), b.lat.to_radians());
    let dp = pb - pa;
    let dl = (b.lon - a.lon).to_radians();
    let h = (dp / 2.0).sin().powi(2) + pa.cos() * pb.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * h.sqrt().min(1.0).asin()
}

/// The shortest round trip over `dist_km` in vacuum: about `dist_km / 149.9` ms.
pub fn vacuum_min_ms(dist_km: f64) -> f64 {
    2000.0 * dist_km / LIGHT_KM_PER_S
}

/// The shortest round trip over `dist_km` in fibre: about `dist_km / 99.93` ms.
pub fn fiber_min_ms(dist_km: f64) -> f64 {
    vacuum_min_ms(dist_km) * FIBER_INDEX
}

/// How far an answer in `rtt_ms` can come from over fibre: about `99.93 * rtt_ms` km.
pub fn fiber_reach_km(rtt_ms: f64) -> f64 {
    rtt_ms * LIGHT_KM_PER_S / (2000.0 * FIBER_INDEX)
}

/// Whether a router can be at a place, given its distance from home and its round trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verdict {
    /// Light in fibre makes it there and back in time.
    Ok,
    /// Only light in vacuum would make it: the place is probably wrong, or the path is
    /// unusually straight and fast.
    Doubtful,
    /// Not even light in vacuum makes it: the place is wrong, as for an anycast address that
    /// answers from a nearby copy.
    Impossible,
}

impl Verdict {
    /// The name in the interface contract.
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Ok => "ok",
            Verdict::Doubtful => "doubtful",
            Verdict::Impossible => "impossible",
        }
    }
}

/// The verdict on a router `dist_km` from home that answered in `rtt_ms`, with
/// [`TOLERANCE_MS`] of slack.
pub fn verdict(dist_km: f64, rtt_ms: f64) -> Verdict {
    if rtt_ms + TOLERANCE_MS < vacuum_min_ms(dist_km) {
        Verdict::Impossible
    } else if rtt_ms + TOLERANCE_MS < fiber_min_ms(dist_km) {
        Verdict::Doubtful
    } else {
        Verdict::Ok
    }
}

/// The point at fraction `f` of the great circle from `a` to `b`. Antipodes have no single
/// great circle between them; the nearer end stands in.
pub(crate) fn along(a: LatLon, b: LatLon, f: f64) -> LatLon {
    let unit = |p: LatLon| {
        let (lat, lon) = (p.lat.to_radians(), p.lon.to_radians());
        [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()]
    };
    let angle = distance_km(a, b) / EARTH_RADIUS_KM;
    let s = angle.sin();
    if s.abs() < 1e-9 {
        return if f < 0.5 { a } else { b };
    }
    let (ka, kb) = (((1.0 - f) * angle).sin() / s, (f * angle).sin() / s);
    let (va, vb) = (unit(a), unit(b));
    let [x, y, z] = [0, 1, 2].map(|i| ka * va[i] + kb * vb[i]);
    LatLon::new(z.atan2(x.hypot(y)).to_degrees(), y.atan2(x).to_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAKU: LatLon = LatLon::new(40.4093, 49.8671);
    const FRANKFURT: LatLon = LatLon::new(50.1109, 8.6821);
    const ASHBURN: LatLon = LatLon::new(39.0438, -77.4874);
    const MOUNTAIN_VIEW: LatLon = LatLon::new(37.3861, -122.0839);

    fn close(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() < eps
    }

    #[test]
    fn distances_along_great_circles() {
        // Worked out separately with the same formula and radius.
        for (to, km) in [
            (FRANKFURT, 3350.3538),
            (ASHBURN, 9691.9082),
            (MOUNTAIN_VIEW, 11325.8085),
        ] {
            let d = distance_km(BAKU, to);
            assert!(close(d, km, 1e-3), "{to:?}: {d}");
            assert!(close(distance_km(to, BAKU), d, 1e-9));
        }
        assert_eq!(distance_km(BAKU, BAKU), 0.0);
        let quarter = distance_km(LatLon::new(0.0, 0.0), LatLon::new(0.0, 90.0));
        assert!(close(
            quarter,
            EARTH_RADIUS_KM * std::f64::consts::FRAC_PI_2,
            1e-6
        ));
        // Antipodes: half the circumference, never NaN; the arcsine is coarse next to 1.
        let half = distance_km(LatLon::new(40.0, 50.0), LatLon::new(-40.0, -130.0));
        assert!(
            close(half, EARTH_RADIUS_KM * std::f64::consts::PI, 0.01),
            "{half}"
        );
        // Across the antimeridian the short way.
        let d = distance_km(LatLon::new(0.0, 179.5), LatLon::new(0.0, -179.5));
        assert!(close(d, 111.19, 0.01), "{d}");
    }

    #[test]
    fn the_shortest_round_trips_in_fibre_and_vacuum() {
        // Baku to Frankfurt: about 33.5 ms in fibre and 22.4 ms in vacuum.
        let d = distance_km(BAKU, FRANKFURT);
        assert!(close(fiber_min_ms(d), 33.5267, 1e-3), "{}", fiber_min_ms(d));
        assert!(
            close(vacuum_min_ms(d), 22.3512, 1e-3),
            "{}",
            vacuum_min_ms(d)
        );
        // About d / 99.93 and d / 149.9.
        assert!(close(fiber_reach_km(1.0), 99.930_819, 1e-6));
        assert!(close(vacuum_min_ms(149.896_229), 1.0, 1e-9));
        assert!(close(fiber_min_ms(fiber_reach_km(42.0)), 42.0, 1e-9));
        assert_eq!(fiber_min_ms(0.0), 0.0);
        // Baku to Mountain View.
        let d = distance_km(BAKU, MOUNTAIN_VIEW);
        assert!(close(fiber_min_ms(d), 113.3365, 1e-3));
        assert!(close(vacuum_min_ms(d), 75.5577, 1e-3));
    }

    #[test]
    fn verdicts_of_the_demo_route() {
        // Hop 8 placed in Ashburn answered in 36.2 ms: light in vacuum needs 64.7.
        assert_eq!(verdict(9692.0, 36.2), Verdict::Impossible);
        // Hop 10 in Frankfurt in 52.4 ms: fibre needs 33.5, 18.9 ms to spare.
        let d = distance_km(BAKU, FRANKFURT);
        assert_eq!(verdict(d, 52.4), Verdict::Ok);
        assert!(close(52.4 - fiber_min_ms(d), 18.873, 1e-3));
        // Faster than fibre, slower than vacuum.
        assert_eq!(verdict(d, 25.0), Verdict::Doubtful);
        // 8.8.8.8 placed in Mountain View answered in 53.1 ms.
        assert_eq!(
            verdict(distance_km(BAKU, MOUNTAIN_VIEW), 53.1),
            Verdict::Impossible
        );
        assert_eq!(verdict(0.0, 0.0), Verdict::Ok);
    }

    #[test]
    fn the_tolerance_is_one_millisecond_on_both_limits() {
        // 10 ms in fibre, 6.67 ms in vacuum.
        let d = fiber_reach_km(10.0);
        let vacuum = vacuum_min_ms(d);
        assert!(close(vacuum, 20.0 / 3.0, 1e-9));
        assert_eq!(verdict(d, 9.0 + 1e-9), Verdict::Ok);
        assert_eq!(verdict(d, 9.0 - 1e-9), Verdict::Doubtful);
        assert_eq!(verdict(d, vacuum - 1.0 + 1e-9), Verdict::Doubtful);
        assert_eq!(verdict(d, vacuum - 1.0 - 1e-9), Verdict::Impossible);
    }

    #[test]
    fn points_off_the_globe_are_not_valid() {
        for (lat, lon) in [(90.0, 180.0), (-90.0, -180.0), (0.0, 0.0), (40.41, 49.87)] {
            assert!(LatLon::new(lat, lon).is_valid(), "{lat} {lon}");
        }
        for (lat, lon) in [
            (90.01, 0.0),
            (-90.01, 0.0),
            (0.0, 180.01),
            (0.0, -180.01),
            (f64::NAN, 0.0),
            (0.0, f64::INFINITY),
        ] {
            assert!(!LatLon::new(lat, lon).is_valid(), "{lat} {lon}");
        }
    }

    #[test]
    fn verdicts_have_the_names_of_the_contract() {
        assert_eq!(Verdict::Ok.name(), "ok");
        assert_eq!(Verdict::Doubtful.name(), "doubtful");
        assert_eq!(Verdict::Impossible.name(), "impossible");
    }

    #[test]
    fn points_along_a_great_circle() {
        let whole = distance_km(BAKU, FRANKFURT);
        assert!(distance_km(along(BAKU, FRANKFURT, 0.0), BAKU) < 1e-6);
        assert!(distance_km(along(BAKU, FRANKFURT, 1.0), FRANKFURT) < 1e-6);
        for f in [0.1, 0.25, 0.5, 0.9] {
            let p = along(BAKU, FRANKFURT, f);
            let (a, b) = (distance_km(BAKU, p), distance_km(p, FRANKFURT));
            // On the circle: the two parts add up to the whole, in the right shares.
            assert!(close(a + b, whole, 1e-6), "{f}: {p:?}");
            assert!(close(a, f * whole, 1e-6), "{f}: {a}");
        }
        // The way to Mountain View goes north of both ends.
        let mid = along(BAKU, MOUNTAIN_VIEW, 0.5);
        assert!(mid.lat > 60.0, "{mid:?}");
        assert_eq!(along(BAKU, BAKU, 0.5), BAKU);
    }
}
