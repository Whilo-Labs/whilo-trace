//! Places of the routers on a path, for the route map of Whilo Trace.
//!
//! An address gets its place from the DB-IP City Lite and ASN Lite databases (MMDB files read
//! through a memory map). Addresses set aside by IANA are never looked up: the home network and
//! the provider's carrier-grade NAT are drawn at home, the rest has no place. The first router
//! with a city is home, unless the user put home somewhere by hand, and every city on the way is
//! checked against the speed of light: an answer that came back faster than light in fibre
//! could carry it there and back marks the place as doubtful, faster than light in vacuum as
//! impossible. The simulated network gets made-up places that pass that check.
//!
//! Looking up a place reads local files only; it never touches the network. The databases come
//! with the program, and [`update`] fetches DB-IP's newer monthly files when it is asked to.

pub mod db;
pub mod enrich;
pub mod fake;
pub mod files;
pub mod locate;
pub mod physics;
pub mod special;
pub mod update;

#[cfg(test)]
mod testing;

pub use db::{DbInfo, GeoDb};
pub use enrich::{enrich, Home, HopGeo, HopIn, Level};
pub use fake::FakeGeo;
pub use files::{Candidate, Layout, Source};
pub use locate::{Locate, Located};
pub use physics::{
    distance_km, fiber_min_ms, fiber_reach_km, vacuum_min_ms, verdict, LatLon, Verdict,
    TOLERANCE_MS,
};
pub use special::{classify, Special};
pub use update::{Offer, Updater};

use std::fmt;
use std::io;

/// What went wrong with the database of places. `Display` gives the short code the interface
/// reads: `geo_missing`, `geo_invalid: <detail>`, `net: <detail>`, `http: 503`, `io: <detail>`,
/// `geo_cancelled`, `geo_busy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeoError {
    /// There is no database file.
    Missing,
    /// A file that is not the database it should be: another type, cut short or damaged.
    Invalid(String),
    /// The connection failed: no network, a timeout, TLS.
    Net(String),
    /// The server answered with this HTTP status.
    Http(u16),
    /// A local file could not be read or written.
    Io(String),
    /// The user stopped the download.
    Cancelled,
    /// A download is already running.
    Busy,
}

impl fmt::Display for GeoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GeoError::Missing => f.write_str("geo_missing"),
            GeoError::Invalid(why) => write!(f, "geo_invalid: {why}"),
            GeoError::Net(why) => write!(f, "net: {why}"),
            GeoError::Http(status) => write!(f, "http: {status}"),
            GeoError::Io(why) => write!(f, "io: {why}"),
            GeoError::Cancelled => f.write_str("geo_cancelled"),
            GeoError::Busy => f.write_str("geo_busy"),
        }
    }
}

impl std::error::Error for GeoError {}

impl From<io::Error> for GeoError {
    fn from(e: io::Error) -> Self {
        GeoError::Io(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_read_as_the_codes_of_the_interface() {
        for (e, code) in [
            (GeoError::Missing, "geo_missing"),
            (
                GeoError::Invalid("city.mmdb: cut short".into()),
                "geo_invalid: city.mmdb: cut short",
            ),
            (GeoError::Net("timed out".into()), "net: timed out"),
            (GeoError::Http(503), "http: 503"),
            (
                GeoError::Io("Access is denied.".into()),
                "io: Access is denied.",
            ),
            (GeoError::Cancelled, "geo_cancelled"),
            (GeoError::Busy, "geo_busy"),
        ] {
            assert_eq!(e.to_string(), code, "{e:?}");
        }
        let io = GeoError::from(io::Error::other("disk full"));
        assert_eq!(io.to_string(), "io: disk full");
    }
}
