//! Helpers for the tests of the file layout and of updates: folders of their own and tiny
//! databases.

use mmdb_writer::ipnet::IpNet;
use mmdb_writer::{IpVersion, Writer};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A folder of its own for one test; removed when the test ends. Declare it before any
/// database opened from it: a mapped file cannot be deleted on Windows.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new(name: &str) -> Self {
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

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    /// Writes `bytes` to `name` under the folder, creating the folders on the way.
    pub(crate) fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.0.join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, bytes).unwrap();
        p
    }

    /// The names of the files in the subfolder `sub` ("" for the folder itself), sorted; none
    /// when it does not exist.
    pub(crate) fn names(&self, sub: &str) -> Vec<String> {
        let mut names: Vec<String> = match fs::read_dir(self.0.join(sub)) {
            Ok(entries) => entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => Vec::new(),
        };
        names.sort();
        names
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A database of `database_type` built at `built` (seconds since 1970) with one network,
/// 5.0.10.0/24: Frankfurt in a City database, AS64504 in an ASN database (a type with "ASN").
pub(crate) fn tiny_db(database_type: &str, built: u64) -> Vec<u8> {
    let record = if database_type.to_lowercase().contains("asn") {
        json!({
            "autonomous_system_number": 64504,
            "autonomous_system_organization": "Transit DE"
        })
    } else {
        frankfurt()
    };
    db_with(database_type, built, &record)
}

/// The City record of Frankfurt, as DB-IP City Lite has it.
pub(crate) fn frankfurt() -> Value {
    json!({
        "city": {"names": {"en": "Frankfurt am Main"}},
        "country": {"iso_code": "DE", "names": {"en": "Germany", "ru": "Германия"}},
        "location": {"latitude": 50.1109, "longitude": 8.6821}
    })
}

/// A database of `database_type` built at `built` with `record` for 5.0.10.0/24.
pub(crate) fn db_with(database_type: &str, built: u64, record: &Value) -> Vec<u8> {
    let mut w = Writer::builder(database_type)
        .ip_version(IpVersion::V6)
        .languages(["en", "ru"])
        .build_epoch(UNIX_EPOCH + Duration::from_secs(built))
        .build();
    let net: IpNet = "5.0.10.0/24".parse().unwrap();
    w.insert(net, record).unwrap();
    w.to_bytes().unwrap()
}
