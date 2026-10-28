//! The DB-IP City Lite and ASN Lite databases: MMDB files read through a memory map, so the
//! 130 MB of the two stay on disk and only the pages a lookup touches are read.

use crate::locate::{Locate, Located};
use crate::GeoError;
use maxminddb::{MaxMindDbError, Mmap, Reader};
use serde::de::{self, Deserializer, Visitor};
use serde::Deserialize;
use std::fmt;
use std::io;
use std::net::IpAddr;
use std::path::Path;

/// What a database file says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbInfo {
    /// `database_type` from the metadata: "DBIP-City-Lite" for DB-IP City Lite.
    pub database_type: String,
    /// When the file was built, in seconds since 1970-01-01 UTC.
    pub build_epoch: u64,
    /// The release month, "2026-10", from the manifest next to the files.
    pub month: Option<String>,
}

/// The City database and, when there is one, the ASN database.
#[derive(Debug)]
pub struct GeoDb {
    city: Reader<Mmap>,
    asn: Option<Reader<Mmap>>,
    month: Option<String>,
}

impl GeoDb {
    /// Opens the City database at `city` and the ASN database at `asn`. The metadata has to
    /// say what each file is: `database_type` containing "City" and "ASN", in any case. A
    /// missing file is [`GeoError::Missing`]; a file of another type, cut short or damaged is
    /// [`GeoError::Invalid`].
    pub fn open(city: &Path, asn: Option<&Path>) -> Result<GeoDb, GeoError> {
        let city = open_mmdb(city, "City")?;
        let asn = asn.map(|p| open_mmdb(p, "ASN")).transpose()?;
        Ok(GeoDb {
            city,
            asn,
            month: None,
        })
    }

    /// The same databases, released in `month` ("YYYY-MM"), as the manifest says.
    pub fn with_month(self, month: Option<String>) -> Self {
        GeoDb { month, ..self }
    }

    /// The City database's type and build time, and the release month.
    pub fn info(&self) -> DbInfo {
        let meta = self.city.metadata();
        DbInfo {
            database_type: meta.database_type.clone(),
            build_epoch: meta.build_epoch,
            month: self.month.clone(),
        }
    }
}

/// Maps the database at `path` and checks that its metadata names a `kind` database. Updates
/// use it on their parts and drop the reader before renaming them.
pub(crate) fn open_mmdb(path: &Path, kind: &str) -> Result<Reader<Mmap>, GeoError> {
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    // SAFETY: the mapped file must not be truncated or changed in place while the reader
    // lives. This program never writes to a database file it has opened: updates are written
    // under new names (`*.part`, then renamed into place), and an old file is only deleted,
    // which leaves a mapping intact (Unix keeps the pages until they are unmapped, Windows
    // refuses to delete a mapped file). Other programs are not expected to modify the files in
    // the install or data folder.
    let reader = unsafe { Reader::open_mmap(path) }.map_err(|e| match e {
        MaxMindDbError::Io(e) if e.kind() == io::ErrorKind::NotFound => GeoError::Missing,
        MaxMindDbError::Io(e) | MaxMindDbError::Mmap(e) => GeoError::Io(format!("{name}: {e}")),
        e => GeoError::Invalid(format!("{name}: {e}")),
    })?;
    let ty = &reader.metadata().database_type;
    if !ty.to_lowercase().contains(&kind.to_lowercase()) {
        return Err(GeoError::Invalid(format!(
            "{name}: the database type is {ty:?}, expected {kind}"
        )));
    }
    Ok(reader)
}

impl Locate for GeoDb {
    fn locate(&self, _target: IpAddr, _ttl: u8, addr: IpAddr) -> Option<Located> {
        // An IPv4-mapped address is looked up as the IPv4 address it is: an IPv4-only file
        // refuses IPv6 lookups, and an IPv6 file finds it under ::ffff:0:0/96 only when its
        // writer put an alias of the IPv4 data there.
        let addr = addr.to_canonical();
        let mut found = Located::default();
        if let Some(rec) = record::<CityRecord>(&self.city, addr) {
            found.city = text(rec.city.names.en);
            found.region = rec.subdivisions.first().and_then(|s| text(s.names.en));
            found.cc = text(rec.country.iso_code);
            found.country_en = text(rec.country.names.en);
            found.country_ru = text(rec.country.names.ru);
            found.lat = rec.location.latitude;
            found.lon = rec.location.longitude;
        }
        if let Some(rec) = self.asn.as_ref().and_then(|r| record::<AsnRecord>(r, addr)) {
            found.asn = rec.autonomous_system_number;
            found.as_org = text(rec.autonomous_system_organization);
        }
        (found != Located::default()).then_some(found)
    }
}

/// The record of `addr`: none when the database has none or it cannot be decoded.
fn record<'a, T: Deserialize<'a>>(reader: &'a Reader<Mmap>, addr: IpAddr) -> Option<T> {
    reader.lookup(addr).ok()?.decode().ok().flatten()
}

/// An owned copy of a name; none for an empty one.
fn text(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

// The parts of the records the map uses. The rest (continent, geoname ids, more languages,
// subdivision codes) is skipped whatever its type: the decoder of maxminddb 0.32 takes a number
// only in the exact width the field asks for, so the full `geoip2::City` would lose a whole
// record over a field the map never reads. For the same reason the numbers here take any width.

/// A DB-IP City Lite record.
#[derive(Deserialize)]
struct CityRecord<'a> {
    #[serde(borrow, default)]
    city: Named<'a>,
    #[serde(borrow, default)]
    country: Country<'a>,
    #[serde(default)]
    location: Location,
    #[serde(borrow, default)]
    subdivisions: Vec<Named<'a>>,
}

#[derive(Deserialize, Default)]
struct Named<'a> {
    #[serde(borrow, default)]
    names: Names<'a>,
}

#[derive(Deserialize, Default)]
struct Names<'a> {
    #[serde(borrow, default)]
    en: Option<&'a str>,
    #[serde(borrow, default)]
    ru: Option<&'a str>,
}

#[derive(Deserialize, Default)]
struct Country<'a> {
    #[serde(borrow, default)]
    iso_code: Option<&'a str>,
    #[serde(borrow, default)]
    names: Names<'a>,
}

#[derive(Deserialize, Default)]
struct Location {
    #[serde(default, deserialize_with = "number")]
    latitude: Option<f64>,
    #[serde(default, deserialize_with = "number")]
    longitude: Option<f64>,
}

/// A DB-IP ASN Lite record, named as in GeoLite2 ASN.
#[derive(Deserialize)]
struct AsnRecord<'a> {
    #[serde(default, deserialize_with = "as_number")]
    autonomous_system_number: Option<u32>,
    #[serde(borrow, default)]
    autonomous_system_organization: Option<&'a str>,
}

/// A number stored in any MMDB numeric type.
fn number<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    struct AnyNumber;

    impl Visitor<'_> for AnyNumber {
        type Value = Option<f64>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a number")
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
            Ok(Some(v))
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(Some(v as f64))
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            Ok(Some(v as f64))
        }

        fn visit_u128<E: de::Error>(self, v: u128) -> Result<Self::Value, E> {
            Ok(Some(v as f64))
        }
    }

    d.deserialize_any(AnyNumber)
}

/// An AS number stored in any MMDB numeric type; none when it is not one.
fn as_number<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    let n = number(d)?;
    Ok(
        n.filter(|n| n.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(n))
            .map(|n| n as u32),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mmdb_writer::ipnet::IpNet;
    use mmdb_writer::{IpVersion, Ipv4Aliasing, Value, Writer};
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    /// A folder of its own for one test; removed when the test ends. Declare it before the
    /// database: a mapped file cannot be deleted on Windows.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir().join(format!(
                "whilo-trace-geo-test-{}-{name}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }

        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let p = self.0.join(name);
            fs::write(&p, bytes).unwrap();
            p
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// 2026-10-01 00:00 UTC.
    const BUILT: u64 = 1_790_812_800;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn net(s: &str) -> IpNet {
        s.parse().unwrap()
    }

    /// Without the aliases of the IPv4 data under ::ffff:0:0/96 that the writer adds by
    /// default: nobody has checked that DB-IP's files have them, so the reader must not rely
    /// on them.
    fn writer(database_type: &str, version: IpVersion) -> Writer {
        Writer::builder(database_type)
            .ip_version(version)
            .ipv4_aliasing(Ipv4Aliasing::Disabled)
            .languages(["de", "en", "fr", "ru"])
            .build_epoch(UNIX_EPOCH + Duration::from_secs(BUILT))
            .build()
    }

    /// A City database shaped like DB-IP City Lite: English city and region names, the
    /// country in several languages, and geoname ids in whatever width the writer picked
    /// (64-bit here, from JSON).
    fn city_db(database_type: &str, version: IpVersion) -> Vec<u8> {
        let mut w = writer(database_type, version);
        let frankfurt = json!({
            "city": {"names": {"en": "Frankfurt am Main"}},
            "continent": {
                "code": "EU",
                "geoname_id": 6_255_148,
                "names": {"de": "Europa", "en": "Europe", "ru": "Европа"}
            },
            "country": {
                "geoname_id": 2_921_044,
                "is_in_european_union": true,
                "iso_code": "DE",
                "names": {"de": "Deutschland", "en": "Germany", "fr": "Allemagne", "ru": "Германия"}
            },
            "location": {"latitude": 50.1109, "longitude": 8.6821},
            "subdivisions": [{"geoname_id": 2_905_330, "iso_code": "HE", "names": {"en": "Hesse"}}]
        });
        w.insert(net("5.0.10.0/24"), &frankfurt).unwrap();
        let bulgaria = json!({
            "country": {"iso_code": "BG", "names": {"en": "Bulgaria", "ru": "Болгария"}},
            "location": {"latitude": 42.7339, "longitude": 25.4858}
        });
        w.insert(net("5.0.9.0/24"), &bulgaria).unwrap();
        // A record that does not decode: a latitude that is not a number.
        let broken = json!({
            "city": {"names": {"en": "Broken"}},
            "location": {"latitude": "north", "longitude": 1.0}
        });
        w.insert(net("5.0.11.0/24"), &broken).unwrap();
        // Coordinates as 32-bit floats and a blank region name.
        let istanbul = Value::map([
            (
                "city",
                Value::map([("names", Value::map([("en", "Istanbul")]))]),
            ),
            ("country", Value::map([("iso_code", "TR")])),
            (
                "location",
                Value::map([
                    ("latitude", Value::Float(41.0082)),
                    ("longitude", Value::Float(28.9784)),
                ]),
            ),
            (
                "subdivisions",
                Value::Array(vec![Value::map([("names", Value::map([("en", " ")]))])]),
            ),
        ]);
        w.insert_value(net("5.0.12.0/24"), istanbul).unwrap();
        if version == IpVersion::V6 {
            let google = json!({
                "city": {"names": {"en": "Mountain View"}},
                "country": {"iso_code": "US", "names": {"en": "United States", "ru": "США"}},
                "location": {"latitude": 37.3861, "longitude": -122.0839},
                "subdivisions": [{"names": {"en": "California"}}]
            });
            w.insert(net("2a00:1450::/32"), &google).unwrap();
        }
        w.to_bytes().unwrap()
    }

    /// An ASN database shaped like DB-IP ASN Lite, with AS numbers in three widths.
    fn asn_db(database_type: &str) -> Vec<u8> {
        let mut w = writer(database_type, IpVersion::V6);
        let as_record = |n: Value, org: &str| {
            Value::map([
                ("autonomous_system_number", n),
                ("autonomous_system_organization", org.into()),
            ])
        };
        w.insert_value(
            net("5.0.10.0/24"),
            as_record(Value::from(64504u32), "Transit DE"),
        )
        .unwrap();
        w.insert_value(
            net("8.8.8.0/24"),
            as_record(Value::from(15169u64), "Google LLC"),
        )
        .unwrap();
        w.insert_value(
            net("5.0.12.0/24"),
            as_record(Value::from(64500u16), "Demo provider"),
        )
        .unwrap();
        w.to_bytes().unwrap()
    }

    fn open_pair(dir: &TempDir) -> GeoDb {
        let city = dir.file("city.mmdb", &city_db("DBIP-City-Lite", IpVersion::V6));
        let asn = dir.file("asn.mmdb", &asn_db("DBIP-ASN-Lite (compat=GeoLite2-ASN)"));
        GeoDb::open(&city, Some(&asn)).unwrap()
    }

    fn here(db: &GeoDb, addr: &str) -> Option<Located> {
        db.locate(ip("8.8.8.8"), 9, ip(addr))
    }

    #[test]
    fn places_and_networks_come_from_dbip_shaped_files() {
        let dir = TempDir::new("read");
        let db = open_pair(&dir);
        assert_eq!(
            here(&db, "5.0.10.77"),
            Some(Located {
                city: Some("Frankfurt am Main".into()),
                region: Some("Hesse".into()),
                cc: Some("DE".into()),
                country_en: Some("Germany".into()),
                country_ru: Some("Германия".into()),
                lat: Some(50.1109),
                lon: Some(8.6821),
                asn: Some(64504),
                as_org: Some("Transit DE".into()),
            })
        );
        // An IPv4-mapped address finds the same record.
        assert_eq!(here(&db, "::ffff:5.0.10.77"), here(&db, "5.0.10.77"));

        let bg = here(&db, "5.0.9.1").unwrap();
        assert_eq!(bg.level(), crate::Level::Country);
        assert_eq!(
            (bg.cc.as_deref(), bg.city, bg.asn),
            (Some("BG"), None, None)
        );

        // A network only the ASN database knows, its number stored in 64 bits.
        let google = here(&db, "8.8.8.8").unwrap();
        assert_eq!(google.asn, Some(15169));
        assert_eq!(google.as_org.as_deref(), Some("Google LLC"));
        assert_eq!(
            (google.cc.as_deref(), google.level()),
            (None, crate::Level::None)
        );

        // 32-bit coordinates, a 16-bit AS number, a blank region.
        let istanbul = here(&db, "5.0.12.1").unwrap();
        assert!(
            (istanbul.lat.unwrap() - 41.0082).abs() < 1e-5,
            "{istanbul:?}"
        );
        assert!(
            (istanbul.lon.unwrap() - 28.9784).abs() < 1e-5,
            "{istanbul:?}"
        );
        assert_eq!((istanbul.region, istanbul.asn), (None, Some(64500)));

        let mv = here(&db, "2a00:1450:4001::1").unwrap();
        assert_eq!(mv.city.as_deref(), Some("Mountain View"));
        assert_eq!(mv.region.as_deref(), Some("California"));
    }

    #[test]
    fn unknown_addresses_and_broken_records_find_nothing() {
        let dir = TempDir::new("none");
        let db = open_pair(&dir);
        assert_eq!(here(&db, "9.9.9.9"), None);
        assert_eq!(here(&db, "2a01::1"), None);
        // The record does not decode; the place is unknown rather than wrong.
        assert_eq!(here(&db, "5.0.11.1"), None);
    }

    #[test]
    fn the_full_geoip2_schema_would_lose_these_records() {
        // Why the records above are read with a schema of their own: the 64-bit geoname ids
        // that a writer may pick fail `geoip2::City` as a whole.
        let reader = Reader::from_source(city_db("DBIP-City-Lite", IpVersion::V6)).unwrap();
        let found = reader.lookup(ip("5.0.10.1")).unwrap();
        assert!(found.has_data());
        assert!(found.decode::<maxminddb::geoip2::City>().is_err());
        assert!(found.decode::<CityRecord>().unwrap().is_some());
    }

    #[test]
    fn places_come_without_an_asn_database_too() {
        let dir = TempDir::new("city-only");
        let city = dir.file("city.mmdb", &city_db("DBIP-City-Lite", IpVersion::V6));
        let db = GeoDb::open(&city, None).unwrap();
        let frankfurt = here(&db, "5.0.10.1").unwrap();
        assert_eq!(frankfurt.city.as_deref(), Some("Frankfurt am Main"));
        assert_eq!((frankfurt.asn, frankfurt.as_org), (None, None));
        assert_eq!(here(&db, "8.8.8.8"), None);
    }

    #[test]
    fn ipv6_addresses_find_nothing_in_an_ipv4_database() {
        let dir = TempDir::new("v4");
        let city = dir.file("city.mmdb", &city_db("DBIP-City-Lite", IpVersion::V4));
        let db = GeoDb::open(&city, None).unwrap();
        assert!(here(&db, "5.0.10.1").is_some());
        assert_eq!(here(&db, "2a00:1450:4001::1"), None);
        // An IPv4-mapped address is an IPv4 one.
        assert_eq!(here(&db, "::ffff:5.0.10.1"), here(&db, "5.0.10.1"));
    }

    #[test]
    fn info_gives_the_type_the_build_time_and_the_month() {
        let dir = TempDir::new("info");
        let db = open_pair(&dir);
        assert_eq!(
            db.info(),
            DbInfo {
                database_type: "DBIP-City-Lite".into(),
                build_epoch: BUILT,
                month: None,
            }
        );
        let db = db.with_month(Some("2026-10".into()));
        assert_eq!(db.info().month.as_deref(), Some("2026-10"));
        assert!(here(&db, "5.0.10.1").is_some());
    }

    #[test]
    fn missing_files_are_missing() {
        let dir = TempDir::new("missing");
        let city = dir.file("city.mmdb", &city_db("DBIP-City-Lite", IpVersion::V6));
        let nothing = dir.0.join("dbip-city-lite-2026-10.mmdb");
        assert_eq!(GeoDb::open(&nothing, None).unwrap_err(), GeoError::Missing);
        assert_eq!(
            GeoDb::open(&city, Some(&dir.0.join("asn.mmdb"))).unwrap_err(),
            GeoError::Missing
        );
    }

    #[test]
    fn the_metadata_has_to_name_the_right_database() {
        let dir = TempDir::new("type");
        let city = dir.file("city.mmdb", &city_db("DBIP-City-Lite", IpVersion::V6));
        let asn = dir.file("asn.mmdb", &asn_db("DBIP-ASN-Lite"));
        // Swapped files.
        let e = GeoDb::open(&asn, None).unwrap_err();
        assert_eq!(
            e.to_string(),
            "geo_invalid: asn.mmdb: the database type is \"DBIP-ASN-Lite\", expected City"
        );
        let e = GeoDb::open(&city, Some(&city)).unwrap_err();
        assert!(
            matches!(&e, GeoError::Invalid(why) if why.ends_with("expected ASN")),
            "{e}"
        );
        // Any case, any decoration.
        for (city_type, asn_type) in [
            ("dbip-city-lite", "dbip-asn-lite"),
            ("GeoLite2-City", "GeoLite2-ASN"),
            ("DBIP-City-Lite", "DBIP-ASN-Lite (compat=GeoLite2-ASN)"),
        ] {
            let city = dir.file("c.mmdb", &city_db(city_type, IpVersion::V6));
            let asn = dir.file("a.mmdb", &asn_db(asn_type));
            let db = GeoDb::open(&city, Some(&asn));
            assert!(db.is_ok(), "{city_type} / {asn_type}: {:?}", db.err());
        }
    }

    #[test]
    fn files_cut_short_or_damaged_are_invalid() {
        let dir = TempDir::new("broken");
        let whole = city_db("DBIP-City-Lite", IpVersion::V6);
        for (name, bytes) in [
            ("half.mmdb", &whole[..whole.len() / 2]),
            ("tail.mmdb", &whole[..whole.len() - 10]),
            ("empty.mmdb", &[][..]),
            ("text.mmdb", &b"not a database"[..]),
        ] {
            let p = dir.file(name, bytes);
            match GeoDb::open(&p, None) {
                Err(GeoError::Invalid(why)) => assert!(why.starts_with(name), "{why}"),
                other => panic!("{name}: {other:?}"),
            }
        }
    }

    #[test]
    fn one_database_serves_every_thread() {
        fn shared<T: Send + Sync>() {}
        shared::<GeoDb>();
        let dir = TempDir::new("threads");
        let db: Arc<dyn Locate> = Arc::new(open_pair(&dir));
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let db = db.clone();
                std::thread::spawn(move || db.locate(ip("8.8.8.8"), 3, ip("5.0.10.1")))
            })
            .collect();
        for w in workers {
            assert_eq!(w.join().unwrap().unwrap().cc.as_deref(), Some("DE"));
        }
    }
}
