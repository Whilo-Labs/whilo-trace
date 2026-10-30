//! Places of the routers for the route map: the databases of places (DB-IP City Lite and ASN
//! Lite, bundled with the program or downloaded by an update), the made-up places of the
//! simulated network, the update check and the download with its `geo` event, and the picture
//! of the map saved for publication. Commands and their JSON: `docs/ui-contract.md`.

use crate::contract::{
    GeoCheckRes, GeoDownload, GeoOffer, GeoStatus, HomeSetting, HopGeo, SaveImageReq, SaveImageRes,
    Settings, SettingsPatch, Trace, TraceHome,
};
use crate::{decode_base64, io, lock, Core};
use std::cell::Cell;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::net::IpAddr;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State};
use whilo_trace_geo::files::{self, Candidate, Layout};
use whilo_trace_geo::update::{self, Offer, Updater};
use whilo_trace_geo::{FakeGeo, GeoDb, GeoError, HopIn, LatLon, Locate, Located};

/// What the geo commands answer on the simulated network: it never reaches DB-IP, and its
/// places are made up, so there is nothing to update or remove.
const UNAVAILABLE: &str = "geo_unavailable";

/// How often the `geo` event tells how a download goes.
const EMIT_EVERY: Duration = Duration::from_millis(250);

/// The error of a download whose thread panicked.
const STOPPED: &str = "the download stopped unexpectedly";

/// The most base64 text a picture may be: far above a picture of the map (2400 x 2160 pixels).
const MAX_PNG_BASE64: usize = 64 << 20;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Pictures of one name in a folder: `name.png`, `name-2.png`, ... up to `name-10000.png`.
const MAX_COPIES: u32 = 10_000;

/// A picture without a usable name.
const IMAGE_NAME: &str = "whilo-trace-map";

/// Where updates come from: DB-IP through `Updater`, or a stand-in in tests, so that no test
/// reaches the network.
pub trait Remote: Send + Sync {
    /// Whether databases newer than `have_month` are out (`Updater::check`).
    fn check(&self, have_month: Option<&str>, now: SystemTime) -> Result<Option<Offer>, GeoError>;

    /// Downloads the databases of `month` into `dir` (`Updater::download`).
    fn download(
        &self,
        month: &str,
        dir: &Path,
        progress: &dyn Fn(u64, Option<u64>),
        cancel: &AtomicBool,
    ) -> Result<(), GeoError>;
}

impl Remote for Updater {
    fn check(&self, have_month: Option<&str>, now: SystemTime) -> Result<Option<Offer>, GeoError> {
        Updater::check(self, have_month, now)
    }

    fn download(
        &self,
        month: &str,
        dir: &Path,
        progress: &dyn Fn(u64, Option<u64>),
        cancel: &AtomicBool,
    ) -> Result<(), GeoError> {
        Updater::download(self, month, dir, progress, cancel)
    }
}

/// Tells the interface how a download goes (the `geo` event).
pub type Emit = Arc<dyn Fn(&GeoDownload) + Send + Sync>;

/// The databases of places, as far as they have been looked at.
enum Db {
    /// Not looked for yet.
    Unknown,
    /// Looked for, not opened yet: the first trace opens them. None: there are none.
    Found(Option<Candidate>),
    Open(Arc<GeoDb>, Candidate),
    /// There, but they do not open.
    Unreadable(Candidate),
}

struct Inner {
    layout: Layout,
    /// None: no checks and no downloads (the simulated network, tests).
    remote: Option<Arc<dyn Remote>>,
    emit: Emit,
    db: Db,
    /// The last download of this run.
    download: Option<GeoDownload>,
    /// A download thread is at work.
    running: bool,
}

impl Inner {
    /// Looks for the databases, and opens what it finds. A set that does not open is
    /// unreadable; nothing here fails.
    fn open(&mut self) {
        if let Db::Unknown = self.db {
            self.db = Db::Found(files::pick(&self.layout));
        }
        if let Db::Found(Some(set)) = &self.db {
            let set = set.clone();
            self.db = match set.open() {
                Ok(db) => Db::Open(Arc::new(db), set),
                // Gone since it was found.
                Err(GeoError::Missing) => Db::Found(None),
                Err(_) => Db::Unreadable(set),
            };
        }
    }
}

/// No database: no address has a place. Own networks are still drawn at home.
struct Nowhere;

impl Locate for Nowhere {
    fn locate(&self, _: IpAddr, _: u8, _: IpAddr) -> Option<Located> {
        None
    }
}

/// The places of the core: which databases are in use, the made-up places of the simulated
/// network, and the update that may be running.
pub struct GeoState {
    simulated: bool,
    /// The made-up places of the simulated network.
    fake: Option<FakeGeo>,
    inner: Mutex<Inner>,
    /// Asks the running download to stop.
    stop: AtomicBool,
    /// One update check at a time: they share the days in state.json.
    checking: Mutex<()>,
}

impl GeoState {
    /// The places for the history at `db_path`. Updates go into `geo-update` next to it; the
    /// bundled databases are looked for in `geo` next to it until setup names the program's
    /// own folder ([`GeoState::wire`]). On the simulated network every place is made up.
    pub fn new(db_path: &Path, simulated: bool) -> Self {
        let data = db_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        GeoState {
            simulated,
            fake: simulated.then(|| FakeGeo::new(whilo_trace_engine::sim::model_rtts)),
            inner: Mutex::new(Inner {
                layout: Layout::new(data.join(files::BUNDLED_DIR), data),
                remote: None,
                emit: Arc::new(|_: &GeoDownload| {}),
                db: Db::Unknown,
                download: None,
                running: false,
            }),
            stop: AtomicBool::new(false),
            checking: Mutex::new(()),
        }
    }

    /// What setup knows: the folder of the bundled databases (none keeps the one next to the
    /// history), where updates come from (none: no checks and no downloads) and who hears how
    /// a download goes. Then deletes what earlier runs left in the update folder.
    pub fn wire(&self, bundled: Option<PathBuf>, remote: Option<Arc<dyn Remote>>, emit: Emit) {
        let mut inner = lock(&self.inner);
        if let Some(dir) = bundled {
            inner.layout.bundled = dir;
        }
        // The simulated network never reaches DB-IP, whatever it is given.
        inner.remote = remote.filter(|_| !self.simulated);
        inner.emit = emit;
        inner.db = Db::Unknown;
        // The update folder is shared with the real network: the simulated one leaves it alone.
        if !self.simulated && !inner.running {
            // Months a reader kept last time (Windows cannot delete a mapped file) and the
            // parts of a download that never finished.
            files::cleanup(&inner.layout);
        }
    }

    /// `Snapshot.geo`. Databases found but not opened yet count as ok: the first trace opens
    /// them and finds out.
    pub fn status(&self) -> GeoStatus {
        if self.simulated {
            return GeoStatus {
                state: "simulated".into(),
                month: None,
                source: Some("simulated".into()),
                download: None,
            };
        }
        let mut inner = lock(&self.inner);
        if let Db::Unknown = inner.db {
            inner.db = Db::Found(files::pick(&inner.layout));
        }
        let (state, set) = match &inner.db {
            Db::Unknown | Db::Found(None) => ("missing", None),
            Db::Found(Some(set)) | Db::Open(_, set) => ("ok", Some(set)),
            Db::Unreadable(set) => ("unreadable", Some(set)),
        };
        GeoStatus {
            state: state.into(),
            month: set.and_then(|s| s.month.clone()),
            source: set.map(|s| s.source.name().into()),
            download: inner.download.clone(),
        }
    }

    /// Places of the hops of a trace and its home, from the minimum round trip of each hop over
    /// the window. The target is the address of the destination hop, else the address the
    /// rounds go to (`row_ip`); without either the trace gets no places.
    pub fn enrich_trace(&self, trace: &mut Trace, row_ip: Option<&str>, settings: &Settings) {
        let dest = trace
            .hops
            .iter()
            .find(|h| h.dest)
            .and_then(|h| h.addr.as_deref());
        let Some(target) = dest.or(row_ip).and_then(|a| a.parse::<IpAddr>().ok()) else {
            return;
        };
        let hops: Vec<HopIn> = trace
            .hops
            .iter()
            .map(|h| HopIn {
                ttl: h.ttl,
                addr: h.addr.as_deref().and_then(|a| a.parse().ok()),
                rtt_min_ms: h.stats.min,
                dest: h.dest,
            })
            .collect();
        let manual = settings.home.map(|h| LatLon::new(h.lat, h.lon));
        let (home, places) = match &self.fake {
            Some(fake) => whilo_trace_geo::enrich(fake, target, &hops, manual, true),
            None => {
                // A clone of the reader: lookups never hold the lock a finished download needs.
                let db = self.reader();
                let loc: &dyn Locate = match &db {
                    Some(db) => db.as_ref(),
                    None => &Nowhere,
                };
                whilo_trace_geo::enrich(loc, target, &hops, manual, false)
            }
        };
        let lang = settings.lang.as_str();
        trace.home = home.map(|h| home_of(h, lang));
        for (hop, place) in trace.hops.iter_mut().zip(places) {
            hop.geo = place.map(|g| hop_geo(g, lang));
        }
    }

    /// The databases, opened the first time a trace needs them; none without readable ones.
    fn reader(&self) -> Option<Arc<GeoDb>> {
        let mut inner = lock(&self.inner);
        inner.open();
        match &inner.db {
            Db::Open(db, _) => Some(db.clone()),
            _ => None,
        }
    }

    /// `geo_check` at `now`, on the local day `today` ("2026-10-28"); `enabled` is the setting.
    /// No request on the simulated network, with the check turned off, after an offer or a
    /// check the same day, or while the databases in use are of the current month; otherwise
    /// one check (`Remote::check`: a HEAD, plus a 1-byte GET when the server does not take
    /// HEAD). Its day is kept before it goes out: answered or not, and even when the program
    /// closes on the way, nothing more is asked that day.
    pub fn check(&self, enabled: bool, now: SystemTime, today: &str) -> GeoCheckRes {
        if self.simulated {
            return answer(None, "simulated");
        }
        if !enabled {
            return answer(None, "disabled");
        }
        let _one = lock(&self.checking);
        let (layout, remote, have) = {
            let mut inner = lock(&self.inner);
            inner.open();
            let have = match &inner.db {
                Db::Open(_, set) => Some(set.month.clone()),
                _ => None,
            };
            (inner.layout.clone(), inner.remote.clone(), have)
        };
        let mut days = files::read_state(&layout);
        if days.last_prompt_day.as_deref() == Some(today) {
            return answer(None, "asked");
        }
        // An offer found today has been made today: that is "asked" above.
        if days.last_check_day.as_deref() == Some(today) {
            return answer(None, "cached");
        }
        let month = have.clone().flatten();
        // A month after the current one comes from a clock that runs behind: nothing newer
        // can be out either.
        if month
            .as_deref()
            .is_some_and(|m| m >= update::current_month(now).as_str())
        {
            return answer(None, "current");
        }
        let Some(remote) = remote else {
            return answer(None, "error");
        };
        // One request a day, answered or not: its day is written before it goes out. A day
        // that could not be written is only asked about again.
        days.last_check_day = Some(today.into());
        let _ = files::write_state(&layout, &days);
        let found = match remote.check(month.as_deref(), now) {
            Ok(found) => found,
            Err(_) => return answer(None, "error"),
        };
        let reason = match &found {
            Some(_) => {
                days.last_prompt_day = Some(today.into());
                let _ = files::write_state(&layout, &days);
                "offer"
            }
            None if have.is_some() => "none",
            // No databases, and none out to download yet.
            None => "missing",
        };
        let offer = found.map(|o| GeoOffer {
            month: o.month,
            bytes: o.city_bytes,
        });
        answer(offer, reason)
    }

    /// Starts downloading the databases of `month` (`geo_download`); the `geo` event and
    /// `Snapshot.geo.download` tell how it goes. One download at a time, and only of a month
    /// newer than the databases in use: on Windows a mapped file cannot be replaced.
    pub fn download(self: &Arc<Self>, month: &str) -> Result<(), String> {
        if self.simulated {
            return Err(UNAVAILABLE.into());
        }
        if update::prev_month(month).is_none() {
            return Err(GeoError::Invalid(format!("not a month: {month:?}")).to_string());
        }
        let mut inner = lock(&self.inner);
        let remote = inner.remote.clone().ok_or(UNAVAILABLE)?;
        if inner.running {
            return Err(GeoError::Busy.to_string());
        }
        inner.open();
        if let Db::Open(
            _,
            Candidate {
                month: Some(have), ..
            },
        ) = &inner.db
        {
            if month <= have.as_str() {
                let why = format!("{month} is not newer than {have}");
                return Err(GeoError::Invalid(why).to_string());
            }
        }
        let start = GeoDownload {
            state: "running".into(),
            month: month.into(),
            received: 0,
            total: None,
            error: None,
        };
        inner.running = true;
        inner.download = Some(start.clone());
        self.stop.store(false, Ordering::SeqCst);
        let (emit, dir) = (inner.emit.clone(), inner.layout.downloaded.clone());
        drop(inner);
        emit(&start);
        let (geo, month) = (self.clone(), month.to_string());
        let spawned = std::thread::Builder::new()
            .name("geo download".into())
            .spawn(move || {
                // A panic on the way ends the download too: otherwise it would stay running,
                // and every later download and removal would be geo_busy.
                let fetch = || geo.fetch(remote.as_ref(), &month, &dir);
                let done = panic::catch_unwind(AssertUnwindSafe(fetch))
                    .unwrap_or_else(|_| Err(GeoError::Io(STOPPED.into())));
                geo.finish(done);
            });
        if let Err(e) = spawned {
            self.finish(Err(GeoError::Io(e.to_string())));
            return Err(io(e));
        }
        Ok(())
    }

    /// The download itself, on its own thread: progress into the state at once, into the
    /// event at most every 250 ms.
    fn fetch(&self, remote: &dyn Remote, month: &str, dir: &Path) -> Result<(), GeoError> {
        let told = Cell::new(Instant::now());
        let progress = |received: u64, total: Option<u64>| {
            let (emit, now) = {
                let mut guard = lock(&self.inner);
                let inner = &mut *guard;
                let Some(d) = inner.download.as_mut() else {
                    return;
                };
                d.received = received;
                d.total = total;
                if told.get().elapsed() < EMIT_EVERY {
                    return;
                }
                told.set(Instant::now());
                (inner.emit.clone(), d.clone())
            };
            emit(&now);
        };
        remote.download(month, dir, &progress, &self.stop)
    }

    /// Ends a download. New databases replace the old ones, whose reader goes first so that
    /// their files can be deleted; then the event tells how it ended.
    fn finish(&self, done: Result<(), GeoError>) {
        let mut guard = lock(&self.inner);
        let inner = &mut *guard;
        let done = done.and_then(|()| {
            inner.db = Db::Unknown;
            inner.open();
            match inner.db {
                Db::Open(..) => Ok(()),
                _ => Err(GeoError::Invalid(
                    "the downloaded databases do not open".into(),
                )),
            }
        });
        if done.is_ok() {
            // The months before it; a file a snapshot still reads goes at the next start.
            files::cleanup(&inner.layout);
        }
        let (state, error) = match done {
            Ok(()) => ("done", None),
            Err(GeoError::Cancelled) => ("cancelled", Some(GeoError::Cancelled.to_string())),
            Err(e) => ("failed", Some(e.to_string())),
        };
        inner.running = false;
        let last = inner.download.as_mut().map(|d| {
            d.state = state.into();
            d.error = error;
            d.clone()
        });
        let emit = inner.emit.clone();
        drop(guard);
        if let Some(last) = last {
            emit(&last);
        }
    }

    /// Asks the running download to stop (`geo_cancel`); it ends as "cancelled".
    pub fn cancel(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// Deletes the downloaded databases and goes back to the bundled ones (`geo_remove`). The
    /// days of the last check and offer stay: an update offered or checked today is not offered
    /// again today.
    pub fn remove(&self) -> Result<(), String> {
        if self.simulated {
            return Err(UNAVAILABLE.into());
        }
        let mut inner = lock(&self.inner);
        if inner.running {
            return Err(GeoError::Busy.to_string());
        }
        // The reader goes first: on Windows a mapped file cannot be deleted.
        inner.db = Db::Unknown;
        inner.download = None;
        let dir = inner.layout.downloaded.clone();
        // Without its manifest the downloaded set no longer counts.
        match fs::remove_file(dir.join(files::MANIFEST)) {
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(io(e)),
            _ => {}
        }
        // A file a snapshot still reads stays until the next start cleans up.
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let ours = name.starts_with("dbip-")
                || (name.starts_with(files::MANIFEST) && name.ends_with(".tmp"));
            if ours {
                let _ = fs::remove_file(entry.path());
            }
        }
        Ok(())
    }
}

fn answer(offer: Option<GeoOffer>, reason: &str) -> GeoCheckRes {
    GeoCheckRes {
        offer,
        reason: reason.into(),
    }
}

/// The country name in the interface language; Russian falls back to English.
fn country(lang: &str, en: Option<String>, ru: Option<String>) -> Option<String> {
    if lang == "en" {
        en
    } else {
        ru.or(en)
    }
}

fn home_of(h: whilo_trace_geo::Home, lang: &str) -> TraceHome {
    TraceHome {
        lat: h.at.lat,
        lon: h.at.lon,
        country: country(lang, h.country_en, h.country_ru),
        city: h.city,
        cc: h.cc,
        source: if h.manual { "manual" } else { "auto" }.into(),
        ttl: h.ttl,
    }
}

fn hop_geo(g: whilo_trace_geo::HopGeo, lang: &str) -> HopGeo {
    HopGeo {
        level: g.level.name().into(),
        special: g.special.map(|s| s.name().into()),
        country: country(lang, g.country_en, g.country_ru),
        city: g.city,
        region: g.region,
        cc: g.cc,
        lat: g.lat,
        lon: g.lon,
        asn: g.asn,
        as_org: g.as_org,
        dist_km: g.dist_km,
        fiber_ms: g.fiber_ms,
        vacuum_ms: g.vacuum_ms,
        rtt_ms: g.rtt_ms,
        extra_ms: g.extra_ms,
        verdict: g.verdict.map(|v| v.name().into()),
    }
}

/// The settings of the map in a patch, as `apply_settings` takes the others: whether to check
/// for newer databases, and the home put by hand (null clears it; one off the globe is
/// ignored, a good one rounded to 0.1°).
pub fn apply_settings(
    s: &mut Settings,
    p: &SettingsPatch,
    changed: &mut Vec<(&'static str, String)>,
) {
    if let Some(v) = p.geo_check {
        s.geo_check = v;
        changed.push(("geoCheck", v.to_string()));
    }
    match p.home {
        Some(None) => {
            s.home = None;
            changed.push(("home", "null".into()));
        }
        Some(Some(h)) => {
            if let Some(h) = on_globe(h) {
                s.home = Some(h);
                let json = serde_json::json!({ "lat": h.lat, "lon": h.lon });
                changed.push(("home", json.to_string()));
            }
        }
        None => {}
    }
}

/// A saved setting of the map, for `load_settings`: other keys are not the map's.
pub fn load_setting(p: &mut SettingsPatch, key: &str, v: &serde_json::Value) {
    match key {
        "geoCheck" => p.geo_check = v.as_bool(),
        "home" => p.home = Some(serde_json::from_value(v.clone()).ok()),
        _ => {}
    }
}

/// A home on the globe, rounded to 0.1°; none for one off it.
fn on_globe(h: HomeSetting) -> Option<HomeSetting> {
    let fits =
        h.lat.is_finite() && h.lon.is_finite() && h.lat.abs() <= 90.0 && h.lon.abs() <= 180.0;
    // Adding zero turns -0.0 into 0.0.
    let tenth = |x: f64| (x * 10.0).round() / 10.0 + 0.0;
    fits.then(|| HomeSetting {
        lat: tenth(h.lat),
        lon: tenth(h.lon),
    })
}

/// The folder of the bundled databases: the one `WHILO_TRACE_GEO_DIR` names (a developer's
/// copy; Linux and macOS builds run from `.target` find no resources), else `geo` among the
/// program's resources.
pub fn bundled_dir(env: Option<OsString>, resources: Option<PathBuf>) -> Option<PathBuf> {
    env.filter(|v| !v.to_string_lossy().trim().is_empty())
        .map(PathBuf::from)
        .or(resources)
}

/// Setup: the bundled databases, updates from DB-IP (never on the simulated network) and the
/// `geo` event.
pub fn wire_app(app: &tauri::App, geo: &GeoState) {
    let resources = app
        .path()
        .resolve(files::BUNDLED_DIR, BaseDirectory::Resource)
        .ok();
    let bundled = bundled_dir(std::env::var_os("WHILO_TRACE_GEO_DIR"), resources);
    let remote = (!geo.simulated).then(|| Arc::new(Updater::new()) as Arc<dyn Remote>);
    let handle = app.handle().clone();
    let emit: Emit = Arc::new(move |d: &GeoDownload| {
        let _ = handle.emit("geo", d);
    });
    geo.wire(bundled, remote, emit);
}

/// Today in the local time zone, "2026-10-28": update checks count days as the user sees them.
fn local_today() -> String {
    let (year, month, day) = local_date().unwrap_or_else(|| utc_date(SystemTime::now()));
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(windows)]
fn local_date() -> Option<(u32, u32, u32)> {
    #[repr(C)]
    #[derive(Default)]
    struct WinTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetLocalTime(time: *mut WinTime);
    }
    let mut t = WinTime::default();
    // SAFETY: GetLocalTime only fills the SYSTEMTIME it is given, and it cannot fail.
    unsafe { GetLocalTime(&mut t) };
    Some((t.year.into(), t.month.into(), t.day.into()))
}

#[cfg(unix)]
fn local_date() -> Option<(u32, u32, u32)> {
    let now: libc::time_t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs()
        .try_into()
        .ok()?;
    // SAFETY: localtime_r only reads `now` and fills `tm`, both ours; all zeros is a valid
    // `tm` (a null `tm_zone` where there is one).
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return None;
        }
        tm
    };
    Some((
        u32::try_from(tm.tm_year + 1900).ok()?,
        u32::try_from(tm.tm_mon + 1).ok()?,
        u32::try_from(tm.tm_mday).ok()?,
    ))
}

#[cfg(not(any(windows, unix)))]
fn local_date() -> Option<(u32, u32, u32)> {
    None
}

/// The UTC date of `t`, for a system that does not tell its local one (Howard Hinnant's
/// `civil_from_days`).
fn utc_date(t: SystemTime) -> (u32, u32, u32) {
    let days = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400);
    let z = days + 719_468;
    let (era, doe) = (z / 146_097, z % 146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year as u32, month as u32, day as u32)
}

/// The bytes of a PNG picture sent as base64 or as a `data:` URL; none for anything else, or
/// for more text than a picture of the map can be.
pub fn png_of(text: &str) -> Option<Vec<u8>> {
    if text.len() > MAX_PNG_BASE64 {
        return None;
    }
    decode_base64(text).filter(|b| b.starts_with(PNG_SIGNATURE))
}

/// The file name of a picture without `.png`: what a file name should not have becomes `_`,
/// and it is never empty, hidden or a device name of Windows.
fn image_stem(name: &str) -> String {
    let name = name.trim();
    let name = match name
        .len()
        .checked_sub(4)
        .and_then(|i| name.get(i..).map(|e| (i, e)))
    {
        Some((i, ext)) if ext.eq_ignore_ascii_case(".png") => &name[..i],
        _ => name,
    };
    let safe: String = name
        .chars()
        .take(120)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = safe.trim_matches(|c| c == '.' || c == '_');
    if stem.is_empty() {
        return IMAGE_NAME.into();
    }
    let base = stem
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let device = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit());
    if device {
        format!("_{stem}")
    } else {
        stem.into()
    }
}

/// Saves a picture of the map into `req.dir`, or into `<docs>/Whilo Trace` without one. A file
/// of that name is never overwritten: the picture gets `-2`, `-3` and so on instead.
pub fn save_image_in(req: &SaveImageReq, docs: Option<PathBuf>) -> Result<SaveImageRes, String> {
    let png = png_of(&req.png_base64).ok_or_else(|| io("not a PNG picture"))?;
    let dir = match req.dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => docs.ok_or("io: no Documents folder")?.join("Whilo Trace"),
    };
    fs::create_dir_all(&dir).map_err(io)?;
    let stem = image_stem(&req.name);
    for k in 1..=MAX_COPIES {
        let name = match k {
            1 => format!("{stem}.png"),
            k => format!("{stem}-{k}.png"),
        };
        let path = dir.join(name);
        // Only a new file: two saves at once never write into one.
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io(e)),
        };
        if let Err(e) = file.write_all(&png) {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(io(e));
        }
        return Ok(SaveImageRes {
            path: path.display().to_string(),
        });
    }
    Err(io(format!("{MAX_COPIES} pictures named {stem} already")))
}

/// Is a newer database of places out? The interface asks when the map opens; at most one
/// request a day goes out, answered or not.
#[tauri::command]
pub async fn geo_check(core: State<'_, Arc<Core>>) -> Result<GeoCheckRes, String> {
    let geo = core.geo.clone();
    let enabled = lock(&core.settings).geo_check;
    // Up to 20 s on the network: not on a worker of the async runtime.
    tauri::async_runtime::spawn_blocking(move || {
        geo.check(enabled, SystemTime::now(), &local_today())
    })
    .await
    .map_err(io)
}

/// Starts downloading the databases of `month`; returns at once.
#[tauri::command]
pub fn geo_download(core: State<'_, Arc<Core>>, month: String) -> Result<(), String> {
    core.geo.download(&month)
}

#[tauri::command]
pub fn geo_cancel(core: State<'_, Arc<Core>>) {
    core.geo.cancel();
}

#[tauri::command]
pub fn geo_remove(core: State<'_, Arc<Core>>) -> Result<(), String> {
    core.geo.remove()
}

/// Saves the picture of the map the interface drew for publication.
#[tauri::command(async)]
pub fn save_image(
    app: AppHandle,
    dir: Option<String>,
    name: String,
    png_base64: String,
) -> Result<SaveImageRes, String> {
    let req = SaveImageReq {
        dir,
        name,
        png_base64,
    };
    save_image_in(&req, app.path().document_dir().ok())
}

/// Tiny databases, updates without the network and folders of their own, for the tests of the
/// app.
#[cfg(test)]
pub(crate) mod testing {
    use super::{Emit, Remote};
    use crate::contract::GeoDownload;
    use mmdb_writer::ipnet::IpNet;
    use mmdb_writer::{IpVersion, Writer};
    use serde_json::{json, Value};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use whilo_trace_geo::files::{asn_file, city_file, BUNDLED_ASN, BUNDLED_CITY, MANIFEST};
    use whilo_trace_geo::update::Offer;
    use whilo_trace_geo::GeoError;

    /// 2026-10-28 12:00 UTC.
    pub const NOW_SECS: u64 = 1_793_188_800;

    pub fn now() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(NOW_SECS)
    }

    /// A folder of its own for one test, removed at the end. Declare it before anything that
    /// opens a database in it: a mapped file cannot be deleted on Windows.
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new(name: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir().join(format!(
                "whilo-trace-app-geo-{}-{name}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The names of the files in `dir`, sorted; none when it does not exist.
    pub fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn db(database_type: &str, records: &[(&str, Value)]) -> Vec<u8> {
        let mut w = Writer::builder(database_type)
            .ip_version(IpVersion::V6)
            .languages(["en", "ru"])
            .build_epoch(UNIX_EPOCH + Duration::from_secs(NOW_SECS))
            .build();
        for (net, record) in records {
            let net: IpNet = net.parse().unwrap();
            w.insert(net, record).unwrap();
        }
        w.to_bytes().unwrap()
    }

    fn city(name: &str, region: &str, country: [&str; 3], at: (f64, f64)) -> Value {
        let [cc, en, ru] = country;
        json!({
            "city": {"names": {"en": name}},
            "subdivisions": [{"names": {"en": region}}],
            "country": {"iso_code": cc, "names": {"en": en, "ru": ru}},
            "location": {"latitude": at.0, "longitude": at.1}
        })
    }

    /// The City database of the test route: the provider in Baku (5.0.20.0/24), Frankfurt
    /// (5.0.10.0/24), a router known only by its country (5.0.30.0/24) and Mountain View
    /// (5.0.40.0/24).
    pub fn city_db() -> Vec<u8> {
        let bulgaria = json!({
            "country": {"iso_code": "BG", "names": {"en": "Bulgaria", "ru": "Болгария"}}
        });
        db(
            "DBIP-City-Lite",
            &[
                (
                    "5.0.20.0/24",
                    city(
                        "Baku",
                        "Baku",
                        ["AZ", "Azerbaijan", "Азербайджан"],
                        (40.4093, 49.8671),
                    ),
                ),
                (
                    "5.0.10.0/24",
                    city(
                        "Frankfurt am Main",
                        "Hesse",
                        ["DE", "Germany", "Германия"],
                        (50.1109, 8.6821),
                    ),
                ),
                ("5.0.30.0/24", bulgaria),
                (
                    "5.0.40.0/24",
                    city(
                        "Mountain View",
                        "California",
                        ["US", "United States", "США"],
                        (37.3861, -122.0839),
                    ),
                ),
            ],
        )
    }

    /// The ASN database of the test route.
    pub fn asn_db() -> Vec<u8> {
        let asn = |n: u32, org: &str| json!({"autonomous_system_number": n, "autonomous_system_organization": org});
        db(
            "DBIP-ASN-Lite (compat=GeoLite2-ASN)",
            &[
                ("5.0.20.0/24", asn(64500, "Demo provider")),
                ("5.0.10.0/24", asn(64504, "Transit DE")),
                ("5.0.40.0/24", asn(15169, "Google LLC")),
            ],
        )
    }

    fn manifest(dir: &Path, month: &str) {
        fs::write(dir.join(MANIFEST), format!("{{\"month\":\"{month}\"}}")).unwrap();
    }

    /// A bundled set in `dir`: both databases, and a manifest of `month` unless it is none.
    pub fn bundle(dir: &Path, month: Option<&str>) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(BUNDLED_CITY), city_db()).unwrap();
        fs::write(dir.join(BUNDLED_ASN), asn_db()).unwrap();
        if let Some(month) = month {
            manifest(dir, month);
        }
    }

    /// A downloaded set of `month` in the update folder `dir`, as an update leaves it.
    pub fn downloaded(dir: &Path, month: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(city_file(month)), city_db()).unwrap();
        fs::write(dir.join(asn_file(month)), asn_db()).unwrap();
        manifest(dir, month);
    }

    /// Updates without the network: a check answers what it is told to, a download writes the
    /// databases of the test route. Remembers what it was asked.
    #[derive(Default)]
    pub struct FakeRemote {
        /// The month in use at every check.
        checks: Mutex<Vec<Option<String>>>,
        answer: Mutex<Option<Result<Option<Offer>, GeoError>>>,
        downloads: AtomicUsize,
        fail: Mutex<Option<GeoError>>,
        /// A download waits for its cancel.
        pub hold: AtomicBool,
        /// A check panics once it is noted (the program closing on the way), a download after
        /// its first progress.
        pub panic: AtomicBool,
    }

    impl FakeRemote {
        pub fn checks(&self) -> Vec<Option<String>> {
            self.checks.lock().unwrap().clone()
        }

        pub fn downloads(&self) -> usize {
            self.downloads.load(Ordering::SeqCst)
        }

        /// What the next checks answer.
        pub fn answer(&self, a: Result<Option<Offer>, GeoError>) {
            *self.answer.lock().unwrap() = Some(a);
        }

        /// How the next downloads fail; none: they work.
        pub fn fail(&self, e: Option<GeoError>) {
            *self.fail.lock().unwrap() = e;
        }
    }

    impl Remote for FakeRemote {
        fn check(
            &self,
            have_month: Option<&str>,
            _now: SystemTime,
        ) -> Result<Option<Offer>, GeoError> {
            self.checks
                .lock()
                .unwrap()
                .push(have_month.map(String::from));
            if self.panic.load(Ordering::SeqCst) {
                panic!("the program closed on the way");
            }
            self.answer.lock().unwrap().clone().unwrap_or(Ok(None))
        }

        fn download(
            &self,
            month: &str,
            dir: &Path,
            progress: &dyn Fn(u64, Option<u64>),
            cancel: &AtomicBool,
        ) -> Result<(), GeoError> {
            self.downloads.fetch_add(1, Ordering::SeqCst);
            progress(0, Some(200));
            if self.panic.load(Ordering::SeqCst) {
                panic!("a download that breaks down");
            }
            if self.hold.load(Ordering::SeqCst) {
                let t0 = Instant::now();
                while !cancel.load(Ordering::SeqCst) {
                    assert!(t0.elapsed() < Duration::from_secs(20), "never cancelled");
                    std::thread::sleep(Duration::from_millis(5));
                }
                return Err(GeoError::Cancelled);
            }
            if let Some(e) = self.fail.lock().unwrap().clone() {
                return Err(e);
            }
            progress(100, Some(200));
            downloaded(dir, month);
            progress(200, Some(200));
            Ok(())
        }
    }

    /// An emitter that keeps what it hears.
    pub fn listener() -> (Emit, Arc<Mutex<Vec<GeoDownload>>>) {
        let heard: Arc<Mutex<Vec<GeoDownload>>> = Arc::default();
        let keep = heard.clone();
        let emit: Emit = Arc::new(move |d: &GeoDownload| keep.lock().unwrap().push(d.clone()));
        (emit, heard)
    }

    pub fn quiet() -> Emit {
        Arc::new(|_: &GeoDownload| {})
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    const DAY: &str = "2026-10-28";

    fn layout(dir: &TempDir) -> Layout {
        Layout::new(dir.path().join("geo"), dir.path())
    }

    fn history(dir: &TempDir) -> PathBuf {
        dir.path().join("history.db")
    }

    /// Places for a history in `dir`, with updates from `remote`.
    fn geo_in(dir: &TempDir, remote: &Arc<FakeRemote>, emit: Emit) -> Arc<GeoState> {
        let geo = Arc::new(GeoState::new(&history(dir), false));
        geo.wire(None, Some(remote.clone() as Arc<dyn Remote>), emit);
        geo
    }

    fn offer(month: &str, bytes: Option<u64>) -> Offer {
        Offer {
            month: month.into(),
            city_bytes: bytes,
        }
    }

    /// The reason of an answer without an offer, or of an offer.
    fn reason(res: GeoCheckRes) -> String {
        assert!(res.offer.is_some() == (res.reason == "offer"), "{res:?}");
        res.reason
    }

    /// The last download, once it has ended.
    fn ended(geo: &GeoState) -> GeoDownload {
        let t0 = Instant::now();
        loop {
            let d = geo.status().download.expect("no download");
            if d.state != "running" {
                return d;
            }
            assert!(t0.elapsed() < Duration::from_secs(20), "still running");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// What the event told, once it has told the end: the download thread tells it just after
    /// the snapshot shows it.
    fn told_end(heard: &Mutex<Vec<GeoDownload>>) -> Vec<GeoDownload> {
        let t0 = Instant::now();
        loop {
            let told = heard.lock().unwrap().clone();
            if told.last().is_some_and(|d| d.state != "running") {
                return told;
            }
            assert!(t0.elapsed() < Duration::from_secs(20), "no end told");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// State, source and month of `Snapshot.geo`.
    fn shown(geo: &GeoState) -> (String, Option<String>, Option<String>) {
        let s = geo.status();
        (s.state, s.source, s.month)
    }

    fn set(state: &str, source: &str, month: &str) -> (String, Option<String>, Option<String>) {
        (state.into(), Some(source.into()), Some(month.into()))
    }

    #[test]
    fn a_check_asks_the_network_at_most_once_a_day() {
        let dir = TempDir::new("check");
        bundle(&dir.path().join("geo"), Some("2026-09"));
        let remote = Arc::new(FakeRemote::default());
        let geo = geo_in(&dir, &remote, quiet());
        let check = |enabled: bool, day: &str| geo.check(enabled, now(), day);

        // Turned off in the settings: nothing is asked, nothing remembered.
        assert_eq!(reason(check(false, DAY)), "disabled");
        assert!(remote.checks().is_empty());
        assert_eq!(files::read_state(&layout(&dir)), files::State::default());

        // A newer month is out: offered once that day.
        remote.answer(Ok(Some(offer("2026-10", Some(61_000_000)))));
        assert_eq!(
            check(true, DAY),
            GeoCheckRes {
                offer: Some(GeoOffer {
                    month: "2026-10".into(),
                    bytes: Some(61_000_000),
                }),
                reason: "offer".into(),
            }
        );
        assert_eq!(remote.checks(), [Some("2026-09".to_string())]);
        let days = files::read_state(&layout(&dir));
        assert_eq!(days.last_check_day.as_deref(), Some(DAY));
        assert_eq!(days.last_prompt_day.as_deref(), Some(DAY));
        assert_eq!(reason(check(true, DAY)), "asked");
        assert_eq!(remote.checks().len(), 1);

        // The next day nothing new is out; asked again that day, the answer is kept.
        remote.answer(Ok(None));
        assert_eq!(reason(check(true, "2026-10-29")), "none");
        assert_eq!(reason(check(true, "2026-10-29")), "cached");
        assert_eq!(remote.checks().len(), 2);
        let days = files::read_state(&layout(&dir));
        assert_eq!(days.last_check_day.as_deref(), Some("2026-10-29"));
        assert_eq!(days.last_prompt_day.as_deref(), Some(DAY));

        // A check that failed counts too: nothing more is asked that day.
        remote.answer(Err(GeoError::Net("timeout: global".into())));
        assert_eq!(reason(check(true, "2026-10-30")), "error");
        assert_eq!(reason(check(true, "2026-10-30")), "cached");
        assert_eq!(remote.checks().len(), 3);
        let days = files::read_state(&layout(&dir));
        assert_eq!(days.last_check_day.as_deref(), Some("2026-10-30"));
        assert_eq!(days.last_prompt_day.as_deref(), Some(DAY));

        // The next day asks again; a server error counts the same.
        remote.answer(Err(GeoError::Http(503)));
        assert_eq!(reason(check(true, "2026-10-31")), "error");
        assert_eq!(reason(check(true, "2026-10-31")), "cached");
        assert_eq!(remote.checks().len(), 4);
    }

    #[test]
    fn a_check_the_program_closed_on_is_not_sent_again_that_day() {
        let dir = TempDir::new("closed");
        bundle(&dir.path().join("geo"), Some("2026-09"));
        let remote = Arc::new(FakeRemote::default());
        remote.panic.store(true, Ordering::SeqCst);
        let geo = geo_in(&dir, &remote, quiet());
        let closed = panic::catch_unwind(AssertUnwindSafe(|| geo.check(true, now(), DAY)));
        assert!(closed.is_err());
        // Started again the same day: the day was kept before the request went out.
        remote.panic.store(false, Ordering::SeqCst);
        let geo = geo_in(&dir, &remote, quiet());
        assert_eq!(reason(geo.check(true, now(), DAY)), "cached");
        assert_eq!(remote.checks().len(), 1);
    }

    #[test]
    fn no_request_while_the_databases_are_of_the_current_month() {
        // 2026-11 in October: a clock that runs behind.
        for month in ["2026-10", "2026-11"] {
            let dir = TempDir::new("current");
            bundle(&dir.path().join("geo"), Some(month));
            let remote = Arc::new(FakeRemote::default());
            remote.answer(Ok(Some(offer("2026-10", None))));
            let geo = geo_in(&dir, &remote, quiet());
            assert_eq!(reason(geo.check(true, now(), DAY)), "current", "{month}");
            assert!(remote.checks().is_empty(), "{month}");
            // Nothing asked, nothing remembered.
            assert_eq!(files::read_state(&layout(&dir)), files::State::default());
        }
    }

    #[test]
    fn without_databases_the_check_offers_them() {
        let dir = TempDir::new("none");
        let remote = Arc::new(FakeRemote::default());
        let geo = geo_in(&dir, &remote, quiet());
        assert_eq!(shown(&geo), ("missing".into(), None, None));
        // Not out yet this month: nothing to offer.
        assert_eq!(reason(geo.check(true, now(), DAY)), "missing");
        remote.answer(Ok(Some(offer("2026-10", Some(61_000_000)))));
        assert_eq!(reason(geo.check(true, now(), "2026-10-29")), "offer");
        assert_eq!(remote.checks(), [None, None]);

        // Databases that do not open count as none.
        let dir = TempDir::new("broken");
        let bundled = dir.path().join("geo");
        fs::create_dir_all(&bundled).unwrap();
        fs::write(bundled.join(files::BUNDLED_CITY), b"not a database").unwrap();
        fs::write(bundled.join(files::MANIFEST), br#"{"month":"2026-10"}"#).unwrap();
        let remote = Arc::new(FakeRemote::default());
        let geo = geo_in(&dir, &remote, quiet());
        // Found, not opened yet.
        assert_eq!(shown(&geo), set("ok", "bundled", "2026-10"));
        assert_eq!(reason(geo.check(true, now(), DAY)), "missing");
        assert_eq!(remote.checks(), [None]);
        assert_eq!(shown(&geo), set("unreadable", "bundled", "2026-10"));
    }

    #[test]
    fn the_simulated_network_and_an_unwired_core_never_reach_the_network() {
        let dir = TempDir::new("offline");
        bundle(&dir.path().join("geo"), Some("2026-09"));
        let remote = Arc::new(FakeRemote::default());
        remote.answer(Ok(Some(offer("2026-10", None))));
        let sim = Arc::new(GeoState::new(&history(&dir), true));
        sim.wire(None, Some(remote.clone() as Arc<dyn Remote>), quiet());
        assert_eq!(reason(sim.check(true, now(), DAY)), "simulated");
        assert_eq!(reason(sim.check(false, now(), DAY)), "simulated");
        assert_eq!(sim.download("2026-10").unwrap_err(), "geo_unavailable");
        assert_eq!(sim.remove().unwrap_err(), "geo_unavailable");
        assert_eq!(
            sim.status(),
            GeoStatus {
                state: "simulated".into(),
                month: None,
                source: Some("simulated".into()),
                download: None,
            }
        );
        assert!(remote.checks().is_empty() && remote.downloads() == 0);
        // The real databases next to the history stay as they are.
        assert_eq!(names(&dir.path().join("geo")).len(), 3);

        // Not wired by setup: no checks, no downloads.
        let geo = Arc::new(GeoState::new(&history(&dir), false));
        assert_eq!(reason(geo.check(true, now(), DAY)), "error");
        assert_eq!(geo.download("2026-10").unwrap_err(), "geo_unavailable");
        // Neither remembers a day: no request went out.
        assert_eq!(files::read_state(&layout(&dir)), files::State::default());
    }

    #[test]
    fn a_download_replaces_the_databases_and_tells_how_it_went() {
        let dir = TempDir::new("download");
        bundle(&dir.path().join("geo"), Some("2026-08"));
        let update = dir.path().join("geo-update");
        downloaded(&update, "2026-09");
        let remote = Arc::new(FakeRemote::default());
        let (emit, heard) = listener();
        let geo = geo_in(&dir, &remote, emit);
        assert!(geo.reader().is_some());
        assert_eq!(shown(&geo), set("ok", "downloaded", "2026-09"));

        // Not a month, or not newer than the databases in use.
        assert_eq!(
            geo.download("2026-13").unwrap_err(),
            "geo_invalid: not a month: \"2026-13\""
        );
        for month in ["2026-09", "2026-08"] {
            assert_eq!(
                geo.download(month).unwrap_err(),
                format!("geo_invalid: {month} is not newer than 2026-09")
            );
        }
        assert_eq!(remote.downloads(), 0);
        assert!(heard.lock().unwrap().is_empty());

        geo.download("2026-10").unwrap();
        let done = GeoDownload {
            state: "done".into(),
            month: "2026-10".into(),
            received: 200,
            total: Some(200),
            error: None,
        };
        assert_eq!(ended(&geo), done);
        assert_eq!(shown(&geo), set("ok", "downloaded", "2026-10"));
        // The event says when it starts and when it ends, as the snapshot does.
        let heard = told_end(&heard);
        let start = GeoDownload {
            state: "running".into(),
            received: 0,
            total: None,
            ..done.clone()
        };
        assert_eq!(heard.first(), Some(&start));
        assert_eq!(heard.last(), Some(&done));
        // The month before goes with its reader; the bundled databases stay.
        assert_eq!(
            names(&update),
            [
                "dbip-asn-lite-2026-10.mmdb",
                "dbip-city-lite-2026-10.mmdb",
                "manifest.json"
            ]
        );
        assert_eq!(names(&dir.path().join("geo")).len(), 3);
        // The new databases find places.
        let db = geo.reader().unwrap();
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let found = db.locate(ip("8.8.8.8"), 3, ip("5.0.10.1")).unwrap();
        assert_eq!(found.city.as_deref(), Some("Frankfurt am Main"));
    }

    #[test]
    fn one_download_at_a_time_and_cancel_stops_it() {
        let dir = TempDir::new("cancel");
        bundle(&dir.path().join("geo"), Some("2026-09"));
        let remote = Arc::new(FakeRemote::default());
        let geo = geo_in(&dir, &remote, quiet());
        remote.hold.store(true, Ordering::SeqCst);
        geo.download("2026-10").unwrap();
        assert_eq!(geo.download("2026-10").unwrap_err(), "geo_busy");
        assert_eq!(geo.remove().unwrap_err(), "geo_busy");
        assert_eq!(geo.status().download.unwrap().state, "running");
        geo.cancel();
        let last = ended(&geo);
        assert_eq!(
            (last.state.as_str(), last.error.as_deref()),
            ("cancelled", Some("geo_cancelled"))
        );
        assert_eq!(shown(&geo), set("ok", "bundled", "2026-09"));

        // A download that fails says why and changes nothing.
        remote.hold.store(false, Ordering::SeqCst);
        remote.fail(Some(GeoError::Http(404)));
        geo.download("2026-10").unwrap();
        let last = ended(&geo);
        assert_eq!(
            (last.state.as_str(), last.error.as_deref()),
            ("failed", Some("http: 404"))
        );
        assert_eq!(shown(&geo), set("ok", "bundled", "2026-09"));

        // The next one may start, and works.
        remote.fail(None);
        geo.download("2026-10").unwrap();
        assert_eq!(ended(&geo).state, "done");
        assert_eq!(shown(&geo), set("ok", "downloaded", "2026-10"));
        assert_eq!(remote.downloads(), 3);
    }

    #[test]
    fn a_download_that_panics_still_ends() {
        let dir = TempDir::new("panic");
        bundle(&dir.path().join("geo"), Some("2026-09"));
        let remote = Arc::new(FakeRemote::default());
        let (emit, heard) = listener();
        let geo = geo_in(&dir, &remote, emit);
        remote.panic.store(true, Ordering::SeqCst);
        geo.download("2026-10").unwrap();
        let last = ended(&geo);
        assert_eq!(
            (last.state.as_str(), last.error.as_deref()),
            ("failed", Some("io: the download stopped unexpectedly"))
        );
        assert_eq!(told_end(&heard).last(), Some(&last));
        assert_eq!(shown(&geo), set("ok", "bundled", "2026-09"));

        // Not left busy: the next download starts, and works.
        remote.panic.store(false, Ordering::SeqCst);
        geo.download("2026-10").unwrap();
        assert_eq!(ended(&geo).state, "done");
        assert_eq!(shown(&geo), set("ok", "downloaded", "2026-10"));
        geo.remove().unwrap();
        assert_eq!(remote.downloads(), 2);
    }

    #[test]
    fn removing_the_download_goes_back_to_the_bundled_databases_and_keeps_the_days() {
        let dir = TempDir::new("remove");
        bundle(&dir.path().join("geo"), Some("2026-09"));
        let update = dir.path().join("geo-update");
        downloaded(&update, "2026-10");
        let days = files::State {
            last_check_day: Some(DAY.into()),
            last_prompt_day: Some(DAY.into()),
        };
        files::write_state(&layout(&dir), &days).unwrap();
        let remote = Arc::new(FakeRemote::default());
        let geo = geo_in(&dir, &remote, quiet());
        assert!(geo.reader().is_some());
        assert_eq!(shown(&geo), set("ok", "downloaded", "2026-10"));

        geo.remove().unwrap();
        assert_eq!(names(&update), [files::STATE]);
        assert_eq!(files::read_state(&layout(&dir)), days);
        assert_eq!(shown(&geo), set("ok", "bundled", "2026-09"));
        assert!(geo.reader().is_some());
        // Nothing left to remove: fine too.
        geo.remove().unwrap();
        assert_eq!(names(&dir.path().join("geo")).len(), 3);
        // Offered today: the update just removed is not offered again today.
        remote.answer(Ok(Some(offer("2026-10", None))));
        assert_eq!(reason(geo.check(true, now(), DAY)), "asked");
        assert!(remote.checks().is_empty());
    }

    #[test]
    fn pictures_are_saved_under_a_free_name() {
        let dir = TempDir::new("image");
        // The PNG signature and a little more.
        let png = b"\x89PNG\r\n\x1a\nmap".to_vec();
        let url = "data:image/png;base64,iVBORw0KGgptYXA=";
        let docs = dir.path().join("Documents");
        let picked = dir.path().join("picked");
        let save = |to: Option<&Path>, name: &str, text: &str| {
            let req = SaveImageReq {
                dir: to.map(|d| d.display().to_string()),
                name: name.into(),
                png_base64: text.into(),
            };
            save_image_in(&req, Some(docs.clone())).map(|r| PathBuf::from(r.path))
        };
        let name = "whilo-trace-map-8.8.8.8-20261028-1530";
        let to = Some(picked.as_path());
        let first = save(to, name, url).unwrap();
        assert_eq!(first, picked.join(format!("{name}.png")));
        assert_eq!(fs::read(&first).unwrap(), png);
        // The same name again, with .png or without: the next free one.
        assert_eq!(
            save(to, &format!("{name}.png"), url).unwrap(),
            picked.join(format!("{name}-2.png"))
        );
        assert_eq!(
            save(to, name, url).unwrap(),
            picked.join(format!("{name}-3.png"))
        );
        assert_eq!(fs::read(&first).unwrap(), png, "never overwritten");
        // Without a folder: Documents/Whilo Trace, as for reports. Plain base64 works too.
        let docs_dir = docs.join("Whilo Trace");
        assert_eq!(
            save(None, name, "iVBORw0KGgptYXA=").unwrap(),
            docs_dir.join(format!("{name}.png"))
        );
        assert_eq!(
            save(Some(Path::new("  ")), "x", url).unwrap(),
            docs_dir.join("x.png")
        );
        // Not a PNG picture: nothing is written.
        for text in ["data:image/png;base64,aGk=", "not base64!", ""] {
            let e = save(to, "bad", text).unwrap_err();
            assert!(e.starts_with("io: "), "{e}");
        }
        assert_eq!(names(&picked).len(), 3);
        // No folder for the default.
        let req = SaveImageReq {
            dir: None,
            name: "x".into(),
            png_base64: url.into(),
        };
        assert!(save_image_in(&req, None).unwrap_err().starts_with("io: "));
    }

    #[test]
    fn picture_names_stay_in_their_folder() {
        for (name, stem) in [
            (
                "whilo-trace-map-one.one.one.one",
                "whilo-trace-map-one.one.one.one",
            ),
            ("map.PNG", "map"),
            ("../../etc/passwd", "etc_passwd"),
            ("..\\..\\Windows\\win.ini", "Windows_win.ini"),
            ("C:\\Users\\x", "C__Users_x"),
            ("a b:c*d?e\"f<g>h|i", "a_b_c_d_e_f_g_h_i"),
            (".hidden", "hidden"),
            ("trailing.", "trailing"),
            ("", IMAGE_NAME),
            ("   ", IMAGE_NAME),
            ("Карта маршрута", IMAGE_NAME),
            ("..", IMAGE_NAME),
            (".png", IMAGE_NAME),
            ("CON", "_CON"),
            ("nul.map", "_nul.map"),
            ("com7", "_com7"),
            ("LPT1x", "LPT1x"),
            ("console", "console"),
        ] {
            assert_eq!(image_stem(name), stem, "{name:?}");
        }
        assert_eq!(image_stem(&"x".repeat(500)).len(), 120);
    }

    #[test]
    fn the_home_is_kept_cleared_or_set_on_the_globe() {
        let patch = |json: &str| serde_json::from_str::<SettingsPatch>(json).unwrap();
        assert_eq!(patch("{}").home, None);
        assert_eq!(patch(r#"{"home":null}"#).home, Some(None));
        assert_eq!(
            patch(r#"{"home":{"lat":40.4093,"lon":49.8671}}"#).home,
            Some(Some(HomeSetting {
                lat: 40.4093,
                lon: 49.8671
            }))
        );
        assert_eq!(patch(r#"{"geoCheck":false}"#).geo_check, Some(false));

        let apply = |s: &mut Settings, json: &str| {
            let mut changed = Vec::new();
            apply_settings(s, &patch(json), &mut changed);
            changed
        };
        let mut s = Settings::default();
        assert!(s.geo_check && s.home.is_none());
        let baku = r#"{"home":{"lat":40.4093,"lon":49.8671},"geoCheck":false}"#;
        assert_eq!(
            apply(&mut s, baku),
            [
                ("geoCheck", "false".to_string()),
                ("home", r#"{"lat":40.4,"lon":49.9}"#.to_string())
            ]
        );
        let rounded = Some(HomeSetting {
            lat: 40.4,
            lon: 49.9,
        });
        assert_eq!(s.home, rounded);
        assert!(!s.geo_check);
        // Off the globe: ignored.
        for bad in [
            r#"{"lat":90.5,"lon":0}"#,
            r#"{"lat":0,"lon":-180.2}"#,
            r#"{"lat":1e300,"lon":0}"#,
        ] {
            assert!(
                apply(&mut s, &format!(r#"{{"home":{bad}}}"#)).is_empty(),
                "{bad}"
            );
        }
        assert_eq!(s.home, rounded);
        // The edges of the globe count, and no negative zero comes out.
        apply(&mut s, r#"{"home":{"lat":-90,"lon":180}}"#);
        assert_eq!(
            s.home,
            Some(HomeSetting {
                lat: -90.0,
                lon: 180.0
            })
        );
        apply(&mut s, r#"{"home":{"lat":-0.04,"lon":-0.01}}"#);
        let h = s.home.unwrap();
        assert!(h.lat == 0.0 && h.lat.is_sign_positive() && h.lon.is_sign_positive());
        // No key leaves the home; null clears it.
        assert!(apply(&mut s, "{}").is_empty());
        assert!(s.home.is_some());
        assert_eq!(
            apply(&mut s, r#"{"home":null}"#),
            [("home", "null".to_string())]
        );
        assert_eq!(s.home, None);
    }

    #[test]
    fn days_are_calendar_dates() {
        assert_eq!(utc_date(now()), (2026, 10, 28));
        for (secs, date) in [
            (0, (1970, 1, 1)),
            // A century that is a leap year.
            (951_782_400, (2000, 2, 29)),
            (951_868_800, (2000, 3, 1)),
            (1_798_761_599, (2026, 12, 31)),
            (1_798_761_600, (2027, 1, 1)),
        ] {
            let t = SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
            assert_eq!(utc_date(t), date, "{secs}");
        }
        let today = local_today();
        let b = today.as_bytes();
        assert!(
            b.len() == 10
                && b[4] == b'-'
                && b[7] == b'-'
                && b.iter()
                    .enumerate()
                    .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit()),
            "{today}"
        );
        #[cfg(any(windows, unix))]
        assert!(local_date().is_some());
    }

    #[test]
    fn a_developer_may_name_the_bundled_folder() {
        let res = Some(PathBuf::from("resources").join("geo"));
        assert_eq!(bundled_dir(None, res.clone()), res);
        assert_eq!(bundled_dir(Some("".into()), res.clone()), res);
        assert_eq!(bundled_dir(Some("  ".into()), res.clone()), res);
        assert_eq!(
            bundled_dir(Some("D:/dev/geo".into()), res),
            Some(PathBuf::from("D:/dev/geo"))
        );
        assert_eq!(bundled_dir(None, None), None);
    }
}
