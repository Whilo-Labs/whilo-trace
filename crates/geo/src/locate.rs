//! What a lookup of one address returns, and the trait every source of places implements: the
//! databases, the made-up places of the simulated network, tables in tests.

use crate::enrich::Level;
use crate::physics::LatLon;
use std::net::IpAddr;

/// The place and the autonomous system of an address, as far as the source knows them. City
/// and region names are what the source has (English in DB-IP Lite); country names come in
/// English and Russian.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Located {
    pub city: Option<String>,
    pub region: Option<String>,
    /// ISO 3166 alpha-2 country code.
    pub cc: Option<String>,
    pub country_en: Option<String>,
    pub country_ru: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub asn: Option<u32>,
    pub as_org: Option<String>,
}

impl Located {
    /// The coordinates, when both are there and on the globe.
    pub fn point(&self) -> Option<LatLon> {
        Some(LatLon::new(self.lat?, self.lon?)).filter(|p| p.is_valid())
    }

    /// How precise the place is: a city needs its name and coordinates, a country only its
    /// code (coordinates or not); anything less is no place at all.
    pub fn level(&self) -> Level {
        if self.city.is_some() && self.point().is_some() {
            Level::City
        } else if self.cc.is_some() {
            Level::Country
        } else {
            Level::None
        }
    }
}

/// A source of places.
pub trait Locate: Send + Sync {
    /// Place and AS of `addr`, seen at `ttl` on the path to `target`. Real databases ignore
    /// target and ttl.
    fn locate(&self, target: IpAddr, ttl: u8, addr: IpAddr) -> Option<Located>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(city: Option<&str>, cc: Option<&str>, at: Option<(f64, f64)>) -> Located {
        Located {
            city: city.map(Into::into),
            cc: cc.map(Into::into),
            lat: at.map(|p| p.0),
            lon: at.map(|p| p.1),
            ..Located::default()
        }
    }

    #[test]
    fn the_level_follows_what_the_record_has() {
        let here = Some((50.11, 8.68));
        for (rec, want) in [
            (
                place(Some("Frankfurt am Main"), Some("DE"), here),
                Level::City,
            ),
            // A city with coordinates is a city even without a country.
            (place(Some("Frankfurt am Main"), None, here), Level::City),
            // Without coordinates a city name is not enough.
            (
                place(Some("Frankfurt am Main"), Some("DE"), None),
                Level::Country,
            ),
            (
                place(None, Some("BG"), Some((42.73, 25.49))),
                Level::Country,
            ),
            (place(None, Some("BG"), None), Level::Country),
            (place(None, None, here), Level::None),
            (place(Some("Frankfurt am Main"), None, None), Level::None),
            (Located::default(), Level::None),
            (
                Located {
                    asn: Some(15169),
                    as_org: Some("Google LLC".into()),
                    ..Located::default()
                },
                Level::None,
            ),
        ] {
            assert_eq!(rec.level(), want, "{rec:?}");
        }
    }

    #[test]
    fn coordinates_off_the_globe_are_no_point() {
        assert_eq!(
            place(None, None, Some((40.41, 49.87))).point(),
            Some(LatLon::new(40.41, 49.87))
        );
        for at in [
            (f64::NAN, 8.0),
            (50.0, f64::INFINITY),
            (90.5, 8.0),
            (50.0, -180.5),
        ] {
            let rec = place(Some("Nowhere"), Some("ZZ"), Some(at));
            assert_eq!(rec.point(), None, "{at:?}");
            assert_eq!(rec.level(), Level::Country, "{at:?}");
        }
        let half = Located {
            lat: Some(50.0),
            ..Located::default()
        };
        assert_eq!(half.point(), None);
    }

    struct Everywhere;

    impl Locate for Everywhere {
        fn locate(&self, _: IpAddr, ttl: u8, _: IpAddr) -> Option<Located> {
            Some(Located {
                city: Some(format!("City {ttl}")),
                ..Located::default()
            })
        }
    }

    #[test]
    fn sources_work_behind_a_shared_pointer() {
        // The app keeps its source as a trait object shared between threads.
        let source: std::sync::Arc<dyn Locate> = std::sync::Arc::new(Everywhere);
        let ip: IpAddr = "8.8.8.8".parse().unwrap();
        let other = source.clone();
        let found = std::thread::spawn(move || other.locate(ip, 7, ip))
            .join()
            .unwrap();
        assert_eq!(found.unwrap().city.as_deref(), Some("City 7"));
        let boxed: Box<dyn Locate> = Box::new(Everywhere);
        assert!(boxed.locate(ip, 1, ip).is_some());
    }
}
