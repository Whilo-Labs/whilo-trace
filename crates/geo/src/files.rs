//! Where the databases live. There are two folders, and they are never the same one: the
//! bundled folder that comes with the program (`<resources>/geo`, read-only) and the folder of
//! downloaded updates (`<data folder>/geo-update`). Each holds a City and an ASN database and a
//! `manifest.json` with their release month; the downloaded folder also keeps `state.json`, the
//! days of the last update check and of the last offer. Of the two sets, the newer month is
//! used.

use crate::db::open_mmdb;
use crate::update::current_month;
use crate::{GeoDb, GeoError};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, UNIX_EPOCH};

/// The bundled folder's name among the program's resources.
pub const BUNDLED_DIR: &str = "geo";

/// The downloaded folder's name in the data folder. It is not [`BUNDLED_DIR`]: the per-user
/// installer on Windows puts the program into its data folder (`%LOCALAPPDATA%\Whilo Trace`),
/// and an update must never write into the bundled set.
pub const UPDATE_DIR: &str = "geo-update";

/// The bundled City database.
pub const BUNDLED_CITY: &str = "dbip-city-lite.mmdb";

/// The bundled ASN database.
pub const BUNDLED_ASN: &str = "dbip-asn-lite.mmdb";

/// The month and the files of a set, in either folder.
pub const MANIFEST: &str = "manifest.json";

/// The days of the last update check and the last offer, in the downloaded folder.
pub const STATE: &str = "state.json";

/// The downloaded City database of `month` ("2026-10"): DB-IP's file name without `.gz`.
pub fn city_file(month: &str) -> String {
    format!("dbip-city-lite-{month}.mmdb")
}

/// The downloaded ASN database of `month`.
pub fn asn_file(month: &str) -> String {
    format!("dbip-asn-lite-{month}.mmdb")
}

/// The two folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// The bundled databases: `<resources>/geo`, or a developer's copy of them.
    pub bundled: PathBuf,
    /// The downloaded databases: `<data folder>/geo-update`.
    pub downloaded: PathBuf,
}

impl Layout {
    /// The bundled folder `bundled`, and the downloaded folder in the data folder `data_dir`.
    pub fn new(bundled: impl Into<PathBuf>, data_dir: &Path) -> Self {
        Layout {
            bundled: bundled.into(),
            downloaded: data_dir.join(UPDATE_DIR),
        }
    }
}

/// Where a set of databases comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// It came with the program.
    Bundled,
    /// An update downloaded it.
    Downloaded,
}

impl Source {
    /// The name in the interface contract.
    pub fn name(self) -> &'static str {
        match self {
            Source::Bundled => "bundled",
            Source::Downloaded => "downloaded",
        }
    }
}

/// The set of databases to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub source: Source,
    /// The release month, "2026-10"; none for a bundled copy whose month cannot be read.
    pub month: Option<String>,
    pub city: PathBuf,
    /// None for a bundled copy without the ASN database.
    pub asn: Option<PathBuf>,
}

impl Candidate {
    /// Opens the databases, with the month attached.
    pub fn open(&self) -> Result<GeoDb, GeoError> {
        Ok(GeoDb::open(&self.city, self.asn.as_deref())?.with_month(self.month.clone()))
    }
}

/// `manifest.json`: `{"month":"2026-10","city":{"file":"…","bytes":N,"sha1":"…"},"asn":{…},
/// "source":"https://download.db-ip.com/free/"}`. Only the month is needed to read one; the
/// rest describes the files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// The release month, as in DB-IP's file names.
    pub month: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<ManifestFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asn: Option<ManifestFile>,
    /// Where the files were downloaded from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// One database file of a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    pub file: String,
    /// The size of the unpacked file.
    pub bytes: u64,
    /// SHA-1 of the unpacked file, as DB-IP publishes it; the program itself does not compute
    /// it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
}

/// The manifest in `dir`; none when there is none, it does not parse or its month is not one.
pub fn read_manifest(dir: &Path) -> Option<Manifest> {
    let bytes = fs::read(dir.join(MANIFEST)).ok()?;
    serde_json::from_slice::<Manifest>(&bytes)
        .ok()
        .filter(|m| is_month(&m.month))
}

/// Writes the manifest into `dir`, through a temporary file, so that a reader never sees half
/// of one.
pub fn write_manifest(dir: &Path, manifest: &Manifest) -> Result<(), GeoError> {
    write_json(&dir.join(MANIFEST), manifest)
}

/// `state.json`: what the downloaded folder remembers between runs. Days are local dates,
/// "2026-10-28", as the caller gives them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// The last day a check sent its request to the server, answered or not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_check_day: Option<String>,
    /// The last day the user was offered an update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt_day: Option<String>,
}

/// The state in the downloaded folder; empty when there is none or it does not parse.
pub fn read_state(layout: &Layout) -> State {
    fs::read(layout.downloaded.join(STATE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Saves the state into the downloaded folder, creating the folder when needed.
pub fn write_state(layout: &Layout, state: &State) -> Result<(), GeoError> {
    fs::create_dir_all(&layout.downloaded)?;
    write_json(&layout.downloaded.join(STATE), state)
}

/// The set to use: the newer month of the bundled and the downloaded set, the downloaded one
/// on a tie. A downloaded set counts only with both files and a manifest that reads; a bundled
/// one needs the City file, and takes its month from the manifest or, in a developer's copy
/// without one, from the City file's build time (UTC). None when there is neither.
pub fn pick(layout: &Layout) -> Option<Candidate> {
    let bundled = bundled_set(&layout.bundled);
    let downloaded = downloaded_set(&layout.downloaded);
    match (bundled, downloaded) {
        // A bundled set of an unknown month (None) is older than any downloaded one.
        (Some(b), Some(d)) if b.month > d.month => Some(b),
        (b, d) => d.or(b),
    }
}

fn bundled_set(dir: &Path) -> Option<Candidate> {
    let city = dir.join(BUNDLED_CITY);
    if !city.is_file() {
        return None;
    }
    let asn = Some(dir.join(BUNDLED_ASN)).filter(|p| p.is_file());
    let month = match read_manifest(dir) {
        Some(m) => Some(m.month),
        None => built_month(&city),
    };
    Some(Candidate {
        source: Source::Bundled,
        month,
        city,
        asn,
    })
}

fn downloaded_set(dir: &Path) -> Option<Candidate> {
    let month = read_manifest(dir)?.month;
    let city = dir.join(city_file(&month));
    let asn = dir.join(asn_file(&month));
    (city.is_file() && asn.is_file()).then_some(Candidate {
        source: Source::Downloaded,
        month: Some(month),
        city,
        asn: Some(asn),
    })
}

/// The UTC month of the build time of the City database at `path`.
fn built_month(path: &Path) -> Option<String> {
    let reader = open_mmdb(path, "City").ok()?;
    let built = UNIX_EPOCH + Duration::from_secs(reader.metadata().build_epoch);
    Some(current_month(built))
}

/// Deletes from the downloaded folder what is no longer in use: the databases of other months
/// than the set [`pick`] chooses, all of them (with their manifest) when it chooses the bundled
/// set, and the parts and temporary files of writes that never finished. Other files stay.
///
/// Call it only while no download runs, and after dropping the reader of a set that is no
/// longer wanted. A file that cannot be deleted (on Windows, one that is still mapped) stays
/// for the next call.
pub fn cleanup(layout: &Layout) {
    let keep = match pick(layout) {
        Some(c) if c.source == Source::Downloaded => c.month,
        _ => None,
    };
    let Ok(entries) = fs::read_dir(&layout.downloaded) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let stale = match month_of_file(&name) {
            Some(month) => keep.as_deref() != Some(month),
            None => name.ends_with(".part") || name.ends_with(".tmp"),
        };
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
    if keep.is_none() {
        let _ = fs::remove_file(layout.downloaded.join(MANIFEST));
    }
}

/// The month in the name of a downloaded database: "dbip-asn-lite-2026-10.mmdb" is of
/// "2026-10".
fn month_of_file(name: &str) -> Option<&str> {
    let rest = name
        .strip_prefix("dbip-city-lite-")
        .or_else(|| name.strip_prefix("dbip-asn-lite-"))?;
    let month = rest.strip_suffix(".mmdb")?;
    is_month(month).then_some(month)
}

/// A month the way DB-IP writes it: "2026-10".
pub(crate) fn is_month(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 7
        && b[4] == b'-'
        && b[..4].iter().chain(&b[5..]).all(u8::is_ascii_digit)
        && (1..=12).contains(&(u32::from(b[5] - b'0') * 10 + u32::from(b[6] - b'0')))
}

/// Writes `value` as JSON to `path` through a temporary file of its own, so that a reader sees
/// the old file or the new one, never half of one, even with two writers at once.
fn write_json(path: &Path, value: &impl Serialize) -> Result<(), GeoError> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let bytes = serde_json::to_vec(value).map_err(|e| GeoError::Io(e.to_string()))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temp = path.with_file_name(format!(
        "{name}.{}-{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let written = File::create(&temp)
        .and_then(|mut f| f.write_all(&bytes).and_then(|()| f.sync_all()))
        .and_then(|()| fs::rename(&temp, path));
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written.map_err(|e: io::Error| GeoError::Io(format!("{name}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{tiny_db, TempDir};
    use crate::Locate;
    use std::net::IpAddr;

    /// 2026-09-15 00:00 UTC.
    const SEPT_15: u64 = 1_789_430_400;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    /// A layout inside `dir`: `bundled/` and `data/geo-update/`.
    fn layout(dir: &TempDir) -> Layout {
        Layout::new(dir.path().join("bundled"), &dir.path().join("data"))
    }

    fn manifest(month: &str, city: &str, asn: &str) -> Manifest {
        Manifest {
            month: month.into(),
            city: Some(ManifestFile {
                file: city.into(),
                bytes: 4,
                sha1: None,
            }),
            asn: Some(ManifestFile {
                file: asn.into(),
                bytes: 4,
                sha1: None,
            }),
            source: Some("https://download.db-ip.com/free/".into()),
        }
    }

    /// A bundled set: real databases, and a manifest of `month` unless it is none.
    fn bundle(dir: &TempDir, month: Option<&str>) {
        dir.file(
            "bundled/dbip-city-lite.mmdb",
            &tiny_db("DBIP-City-Lite", SEPT_15),
        );
        dir.file(
            "bundled/dbip-asn-lite.mmdb",
            &tiny_db("DBIP-ASN-Lite (compat=GeoLite2-ASN)", SEPT_15),
        );
        if let Some(month) = month {
            let m = manifest(month, BUNDLED_CITY, BUNDLED_ASN);
            write_manifest(&dir.path().join("bundled"), &m).unwrap();
        }
    }

    /// The databases of a downloaded month, without a manifest.
    fn download_files(dir: &TempDir, month: &str) {
        let name = |f: String| format!("data/geo-update/{f}");
        dir.file(&name(city_file(month)), &tiny_db("DBIP-City-Lite", SEPT_15));
        dir.file(&name(asn_file(month)), &tiny_db("DBIP-ASN-Lite", SEPT_15));
    }

    /// A downloaded set of `month`: its databases and the manifest naming it.
    fn download(dir: &TempDir, month: &str) {
        download_files(dir, month);
        let m = manifest(month, &city_file(month), &asn_file(month));
        write_manifest(&layout(dir).downloaded, &m).unwrap();
    }

    fn chosen(layout: &Layout) -> Option<(Source, Option<String>)> {
        pick(layout).map(|c| (c.source, c.month))
    }

    #[test]
    fn the_folders_never_meet() {
        // The per-user installer on Windows: the resources are in the data folder.
        let data = Path::new("C:/Users/u/AppData/Local/Whilo Trace");
        let l = Layout::new(data.join(BUNDLED_DIR), data);
        assert_eq!(l.bundled, data.join("geo"));
        assert_eq!(l.downloaded, data.join("geo-update"));
        assert_ne!(l.bundled, l.downloaded);
        assert_eq!(city_file("2026-10"), "dbip-city-lite-2026-10.mmdb");
        assert_eq!(asn_file("2026-10"), "dbip-asn-lite-2026-10.mmdb");
    }

    #[test]
    fn manifests_read_back_as_written() {
        let dir = TempDir::new("manifest");
        let m = manifest(
            "2026-11",
            "dbip-city-lite-2026-11.mmdb",
            "dbip-asn-lite-2026-11.mmdb",
        );
        write_manifest(dir.path(), &m).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(MANIFEST)).unwrap(),
            r#"{"month":"2026-11","city":{"file":"dbip-city-lite-2026-11.mmdb","bytes":4},"asn":{"file":"dbip-asn-lite-2026-11.mmdb","bytes":4},"source":"https://download.db-ip.com/free/"}"#
        );
        assert_eq!(read_manifest(dir.path()), Some(m));
        // No temporary file is left behind.
        assert_eq!(dir.names(""), [MANIFEST]);

        // Pretty-printed, with SHA-1 sums and a field this program does not know.
        dir.file(
            MANIFEST,
            br#"{
              "month": "2026-10",
              "city": {"file": "dbip-city-lite.mmdb", "bytes": 126974321, "sha1": "df17bd24390108ed1d24dcb7718dc60f6b9abd4f"},
              "asn": {"file": "dbip-asn-lite.mmdb", "bytes": 9541234, "sha1": "bc76615cd8f99b7b3a878803b1c84cbeda65769a"},
              "source": "https://download.db-ip.com/free/",
              "fetched": "2026-10-02T06:00:00Z"
            }"#,
        );
        let m = read_manifest(dir.path()).unwrap();
        assert_eq!(m.month, "2026-10");
        let city = m.city.unwrap();
        assert_eq!(
            (city.file.as_str(), city.bytes),
            ("dbip-city-lite.mmdb", 126_974_321)
        );
        assert_eq!(
            city.sha1.as_deref(),
            Some("df17bd24390108ed1d24dcb7718dc60f6b9abd4f")
        );
        // Only the month is needed.
        dir.file(MANIFEST, br#"{"month":"2026-10"}"#);
        assert_eq!(read_manifest(dir.path()).unwrap().city, None);

        for broken in [
            &b""[..],
            b"{",
            br#"{"month":"October"}"#,
            br#"{"month":"2026-13"}"#,
            br#"{"month":"2026-00"}"#,
            br#"{"month":"2026-1"}"#,
            br#"{"month":202610}"#,
            br#"{"city":{"file":"x","bytes":1}}"#,
        ] {
            dir.file(MANIFEST, broken);
            assert_eq!(
                read_manifest(dir.path()),
                None,
                "{}",
                String::from_utf8_lossy(broken)
            );
        }
        assert_eq!(read_manifest(&dir.path().join("nowhere")), None);
    }

    #[test]
    fn the_state_reads_back_as_written() {
        let dir = TempDir::new("state");
        let l = layout(&dir);
        // Nothing yet: not even the folder.
        assert_eq!(read_state(&l), State::default());
        let s = State {
            last_check_day: Some("2026-10-28".into()),
            last_prompt_day: Some("2026-10-27".into()),
        };
        write_state(&l, &s).unwrap();
        assert_eq!(
            fs::read_to_string(l.downloaded.join(STATE)).unwrap(),
            r#"{"lastCheckDay":"2026-10-28","lastPromptDay":"2026-10-27"}"#
        );
        assert_eq!(read_state(&l), s);
        // Overwritten in place, leaving no temporary file.
        let checked = State {
            last_check_day: Some("2026-10-29".into()),
            last_prompt_day: None,
        };
        write_state(&l, &checked).unwrap();
        assert_eq!(read_state(&l), checked);
        assert_eq!(dir.names("data/geo-update"), [STATE]);

        dir.file(
            "data/geo-update/state.json",
            br#"{"lastPromptDay":"2026-10-28","x":1}"#,
        );
        assert_eq!(
            read_state(&l).last_prompt_day.as_deref(),
            Some("2026-10-28")
        );
        for broken in [&b"nonsense"[..], br#"{"lastCheckDay":5}"#] {
            dir.file("data/geo-update/state.json", broken);
            assert_eq!(read_state(&l), State::default());
        }
    }

    #[test]
    fn the_newer_month_wins_and_a_tie_goes_to_the_download() {
        let dir = TempDir::new("pick");
        let l = layout(&dir);
        assert_eq!(pick(&l), None);

        bundle(&dir, Some("2026-10"));
        let only = pick(&l).unwrap();
        assert_eq!(
            only,
            Candidate {
                source: Source::Bundled,
                month: Some("2026-10".into()),
                city: l.bundled.join(BUNDLED_CITY),
                asn: Some(l.bundled.join(BUNDLED_ASN)),
            }
        );

        download(&dir, "2026-11");
        let newer = pick(&l).unwrap();
        assert_eq!(
            newer,
            Candidate {
                source: Source::Downloaded,
                month: Some("2026-11".into()),
                city: l.downloaded.join("dbip-city-lite-2026-11.mmdb"),
                asn: Some(l.downloaded.join("dbip-asn-lite-2026-11.mmdb")),
            }
        );

        // A program update that bundles a newer month.
        bundle(&dir, Some("2026-12"));
        assert_eq!(chosen(&l), Some((Source::Bundled, Some("2026-12".into()))));

        // The same month in both: the download.
        bundle(&dir, Some("2026-11"));
        assert_eq!(
            chosen(&l),
            Some((Source::Downloaded, Some("2026-11".into())))
        );

        // Without a bundled set at all.
        fs::remove_dir_all(&l.bundled).unwrap();
        assert_eq!(
            chosen(&l),
            Some((Source::Downloaded, Some("2026-11".into())))
        );
    }

    #[test]
    fn a_downloaded_set_needs_both_files_and_its_manifest() {
        let dir = TempDir::new("incomplete");
        let l = layout(&dir);
        bundle(&dir, Some("2026-10"));
        download(&dir, "2026-11");
        let bundled = Some((Source::Bundled, Some("2026-10".into())));

        fs::remove_file(l.downloaded.join("dbip-asn-lite-2026-11.mmdb")).unwrap();
        assert_eq!(chosen(&l), bundled);
        download(&dir, "2026-11");
        fs::remove_file(l.downloaded.join("dbip-city-lite-2026-11.mmdb")).unwrap();
        assert_eq!(chosen(&l), bundled);

        // The files are there, the manifest is not, or does not read, or names another month.
        download_files(&dir, "2026-11");
        fs::remove_file(l.downloaded.join(MANIFEST)).unwrap();
        assert_eq!(chosen(&l), bundled);
        dir.file("data/geo-update/manifest.json", b"{\"month\":");
        assert_eq!(chosen(&l), bundled);
        dir.file("data/geo-update/manifest.json", br#"{"month":"2026-12"}"#);
        assert_eq!(chosen(&l), bundled);
        dir.file("data/geo-update/manifest.json", br#"{"month":"2026-11"}"#);
        assert_eq!(
            chosen(&l),
            Some((Source::Downloaded, Some("2026-11".into())))
        );
    }

    #[test]
    fn a_bundled_copy_without_a_manifest_is_of_its_build_month() {
        let dir = TempDir::new("dev-copy");
        let l = layout(&dir);
        bundle(&dir, None);
        let c = pick(&l).unwrap();
        assert_eq!(
            (c.source, c.month.as_deref()),
            (Source::Bundled, Some("2026-09"))
        );
        // It opens with that month, and finds places.
        let db = c.open().unwrap();
        assert_eq!(db.info().month.as_deref(), Some("2026-09"));
        let found = db.locate(ip("8.8.8.8"), 5, ip("5.0.10.1")).unwrap();
        assert_eq!(found.city.as_deref(), Some("Frankfurt am Main"));
        assert_eq!(found.asn, Some(64504));
        drop(db);

        // Without the ASN database, places still come.
        fs::remove_file(l.bundled.join(BUNDLED_ASN)).unwrap();
        let c = pick(&l).unwrap();
        assert_eq!(c.asn, None);
        let found = c.open().unwrap().locate(ip("8.8.8.8"), 5, ip("5.0.10.1"));
        assert_eq!(found.unwrap().asn, None);

        // A City file whose month cannot be read still counts, as older than any download.
        dir.file("bundled/dbip-city-lite.mmdb", b"not a database");
        assert_eq!(chosen(&l), Some((Source::Bundled, None)));
        assert!(matches!(
            pick(&l).unwrap().open(),
            Err(GeoError::Invalid(_))
        ));
        download(&dir, "2026-01");
        assert_eq!(
            chosen(&l),
            Some((Source::Downloaded, Some("2026-01".into())))
        );

        // An ASN file alone is no set.
        fs::remove_file(l.bundled.join(BUNDLED_CITY)).unwrap();
        dir.file(
            "bundled/dbip-asn-lite.mmdb",
            &tiny_db("DBIP-ASN-Lite", SEPT_15),
        );
        fs::remove_dir_all(&l.downloaded).unwrap();
        assert_eq!(pick(&l), None);
    }

    #[test]
    fn cleanup_keeps_only_the_month_in_use() {
        let dir = TempDir::new("cleanup");
        let l = layout(&dir);
        bundle(&dir, Some("2026-10"));
        download_files(&dir, "2026-09");
        dir.file("data/geo-update/dbip-city-lite-2026-10.mmdb", b"half a set");
        download(&dir, "2026-11");
        for leftover in [
            "dbip-city-lite-2026-12.mmdb.part",
            "manifest.json.4242-0.tmp",
            "notes.txt",
            "dbip-city-lite-2026-13.mmdb",
        ] {
            dir.file(&format!("data/geo-update/{leftover}"), b"x");
        }
        write_state(
            &l,
            &State {
                last_check_day: Some("2026-11-02".into()),
                last_prompt_day: None,
            },
        )
        .unwrap();

        cleanup(&l);
        assert_eq!(
            dir.names("data/geo-update"),
            [
                "dbip-asn-lite-2026-11.mmdb",
                "dbip-city-lite-2026-11.mmdb",
                // Not a month: not a database of this program.
                "dbip-city-lite-2026-13.mmdb",
                MANIFEST,
                "notes.txt",
                STATE,
            ]
        );
        assert_eq!(
            chosen(&l),
            Some((Source::Downloaded, Some("2026-11".into())))
        );
        // The bundled folder is never touched.
        assert_eq!(dir.names("bundled").len(), 3);

        // A newer bundled month makes the whole download stale; the state stays.
        bundle(&dir, Some("2026-12"));
        cleanup(&l);
        assert_eq!(
            dir.names("data/geo-update"),
            ["dbip-city-lite-2026-13.mmdb", "notes.txt", STATE]
        );
        assert_eq!(read_state(&l).last_check_day.as_deref(), Some("2026-11-02"));

        // Nothing to clean, or no folder at all.
        cleanup(&l);
        fs::remove_dir_all(&l.downloaded).unwrap();
        cleanup(&l);
        assert!(!l.downloaded.exists());
    }

    #[test]
    fn a_database_still_open_is_deleted_by_a_later_cleanup() {
        let dir = TempDir::new("cleanup-open");
        let l = layout(&dir);
        download(&dir, "2026-10");
        let old = pick(&l).unwrap().open().unwrap();
        download(&dir, "2026-11");
        // On Windows the mapped files cannot be deleted yet; elsewhere they go, and the open
        // database keeps its pages. Either way the cleanup carries on.
        cleanup(&l);
        assert!(old.locate(ip("8.8.8.8"), 5, ip("5.0.10.1")).is_some());
        drop(old);
        cleanup(&l);
        assert_eq!(
            dir.names("data/geo-update"),
            [
                "dbip-asn-lite-2026-11.mmdb",
                "dbip-city-lite-2026-11.mmdb",
                MANIFEST
            ]
        );
    }

    #[test]
    fn months_are_written_as_in_dbip_file_names() {
        for good in ["2026-10", "2026-01", "2026-12", "1999-09"] {
            assert!(is_month(good), "{good}");
        }
        for bad in [
            "2026-13",
            "2026-00",
            "2026-1",
            "26-10",
            "2026/10",
            "2026-1x",
            "",
            "２０２６-10",
        ] {
            assert!(!is_month(bad), "{bad}");
        }
        assert_eq!(
            month_of_file("dbip-city-lite-2026-10.mmdb"),
            Some("2026-10")
        );
        assert_eq!(month_of_file("dbip-asn-lite-2026-10.mmdb"), Some("2026-10"));
        for other in [
            "dbip-city-lite.mmdb",
            "dbip-city-lite-2026-10.mmdb.part",
            "dbip-city-lite-2026-10.mmdb.gz",
            "dbip-country-lite-2026-10.mmdb",
        ] {
            assert_eq!(month_of_file(other), None, "{other}");
        }
    }
}
