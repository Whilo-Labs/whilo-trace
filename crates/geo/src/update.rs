//! Updates of the databases. DB-IP publishes City Lite and ASN Lite once a month, usually on
//! the 1st, under fixed names with the month in them. A check is one HEAD request for the City
//! file of the current month, and no request at all while the databases in use are of that
//! month. A download streams both files through gunzip into parts, makes sure that each part
//! opens as the database it should be, and only then renames them into place; the manifest,
//! written last, makes the new set count.

use crate::db::open_mmdb;
use crate::files::{asn_file, city_file, is_month, write_manifest, Manifest, ManifestFile};
use crate::GeoError;
use flate2::read::MultiGzDecoder;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use ureq::http::{header, Response};
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::typestate::WithoutBody;
use ureq::{Agent, Body, Proxy, RequestBuilder};

/// Where DB-IP keeps its free databases.
pub const DEFAULT_BASE: &str = "https://download.db-ip.com/free/";

/// How the program introduces itself to the server.
pub const USER_AGENT: &str = concat!(
    "Whilo-Trace/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Whilo-Labs/whilo-trace)"
);

/// Setting up a connection, TLS included.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Each read from the network: a download that stalls this long fails.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// A whole check, both requests of the fallback included.
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);

/// How much has to arrive between two reports of a download's progress.
const PROGRESS_STEP: u64 = 256 * 1024;

/// The most bytes of one file a download takes: compressed, as they come, and unpacked.
#[derive(Clone, Copy)]
struct Limits {
    gz: u64,
    db: u64,
}

/// Far above DB-IP's sizes (the City file is about 60 MB, its database about 121 MB): a
/// server that sends more, or a file that unpacks to far more than it is, fails the download
/// instead of filling the disk.
const LIMITS: Limits = Limits {
    gz: 512 << 20,
    db: 1 << 30,
};

/// The month of `now` in UTC, "2026-10": the month in the names of DB-IP's newest files.
pub fn current_month(now: SystemTime) -> String {
    let secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (year, month) = year_month(secs / 86_400);
    format!("{year:04}-{month:02}")
}

/// The month before `month`: "2026-01" gives "2025-12". None when `month` is not one.
pub fn prev_month(month: &str) -> Option<String> {
    if !is_month(month) {
        return None;
    }
    let year: u32 = month[..4].parse().ok()?;
    let m: u32 = month[5..].parse().ok()?;
    Some(match m {
        1 => format!("{:04}-12", year.checked_sub(1)?),
        m => format!("{year:04}-{:02}", m - 1),
    })
}

/// Year and month of the day `days` after 1970-01-01, in the Gregorian calendar (Howard
/// Hinnant's `civil_from_days`, for days on or after the epoch).
fn year_month(days: u64) -> (u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + u64::from(month <= 2), month)
}

/// Newer databases that are out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// Their month, "2026-11".
    pub month: String,
    /// The size of the City file to download (compressed), when the server tells it; the ASN
    /// file adds about 5 MB.
    pub city_bytes: Option<u64>,
}

/// Whether a file is out.
enum Published {
    /// Yes, with its size when the server tells it.
    Yes(Option<u64>),
    /// No (404).
    No,
}

/// Checks for and downloads the databases of a month. Clones share their connections.
#[derive(Debug, Clone)]
pub struct Updater {
    base: String,
    agent: Agent,
}

impl Default for Updater {
    fn default() -> Self {
        Self::new()
    }
}

impl Updater {
    /// From DB-IP, through a proxy when the environment names one (`HTTPS_PROXY` and the like).
    pub fn new() -> Self {
        Self::with_base(DEFAULT_BASE)
    }

    /// From `base`, a folder URL with DB-IP's file names in it, through a proxy when the
    /// environment names one. An `https` base refuses redirects to plain `http`.
    pub fn with_base(base: &str) -> Self {
        Self::build(base, Proxy::try_from_env())
    }

    fn build(base: &str, proxy: Option<Proxy>) -> Self {
        let mut base = base.to_owned();
        if !base.ends_with('/') {
            base.push('/');
        }
        let tls = TlsConfig::builder()
            .provider(TlsProvider::Rustls)
            // The system's certificate store, not a list of roots built into the program.
            .root_certs(RootCerts::PlatformVerifier)
            // ring, set here: ureq built without its own provider panics at the first TLS
            // connection when none is given.
            .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .build();
        let https = base
            .get(..6)
            .is_some_and(|s| s.eq_ignore_ascii_case("https:"));
        let agent = Agent::config_builder()
            .tls_config(tls)
            .proxy(proxy)
            .https_only(https)
            // Statuses are answers here, not errors: 404 means "not out yet".
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_per_read(Some(READ_TIMEOUT))
            .build()
            .new_agent();
        Updater { base, agent }
    }

    /// The folder URL the files come from, ending in `/`.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Whether databases newer than `have_month` are out as of `now`: `have_month` is the month
    /// of the databases in use, none when there are none. The answer is an offer of the current
    /// month, or none when the databases in use are of that month (then no request is sent at
    /// all) or its files are not out yet (404). One HEAD request of the City file; a server that
    /// does not take HEAD (405, 501) then gets one GET of its first byte. Other statuses and
    /// failed connections are errors. The whole check takes at most 20 s.
    pub fn check(
        &self,
        have_month: Option<&str>,
        now: SystemTime,
    ) -> Result<Option<Offer>, GeoError> {
        self.check_within(have_month, now, CHECK_TIMEOUT)
    }

    fn check_within(
        &self,
        have_month: Option<&str>,
        now: SystemTime,
        limit: Duration,
    ) -> Result<Option<Offer>, GeoError> {
        let month = current_month(now);
        // A month later than the current one comes from a clock that runs behind: nothing
        // newer can be out then either.
        if have_month.is_some_and(|have| is_month(have) && have >= month.as_str()) {
            return Ok(None);
        }
        let deadline = Instant::now() + limit;
        Ok(match self.published(&city_file(&month), deadline)? {
            Published::Yes(city_bytes) => Some(Offer { month, city_bytes }),
            Published::No => None,
        })
    }

    /// Whether `<file>.gz` is out, by `deadline`.
    fn published(&self, file: &str, deadline: Instant) -> Result<Published, GeoError> {
        let url = self.url(file);
        let head = call(self.agent.head(&url), Some(deadline))?;
        match head.status().as_u16() {
            200 => Ok(Published::Yes(content_length(&head))),
            404 => Ok(Published::No),
            405 | 501 => {
                let first = self.agent.get(&url).header(header::RANGE, "bytes=0-0");
                let first = call(first, Some(deadline))?;
                match first.status().as_u16() {
                    // "Content-Range: bytes 0-0/61234567" gives the size of the whole file.
                    206 => Ok(Published::Yes(range_total(&first))),
                    // A server that ignores the range sends the whole file; it is not read.
                    200 => Ok(Published::Yes(content_length(&first))),
                    404 => Ok(Published::No),
                    status => Err(GeoError::Http(status)),
                }
            }
            status => Err(GeoError::Http(status)),
        }
    }

    /// Downloads the City and ASN databases of `month` into the folder `dir`. Each file is
    /// streamed through gunzip into `<name>.part`, and each part has to open as the database it
    /// should be before both are renamed to their final names and `manifest.json` names the
    /// month.
    ///
    /// `progress` hears the bytes received so far and their total, the sum of the two files'
    /// sizes when the server tells them: at the start, then at most every 256 KB, and at the
    /// end. `cancel` is looked at between reads. A file of more than 512 MB, or one that
    /// unpacks to more than 1 GB, is invalid. Cancelling, like any failure, deletes the parts;
    /// the databases already there stay as they were. One download at a time into a folder:
    /// two would write the same parts.
    pub fn download(
        &self,
        month: &str,
        dir: &Path,
        progress: &dyn Fn(u64, Option<u64>),
        cancel: &AtomicBool,
    ) -> Result<(), GeoError> {
        if !is_month(month) {
            return Err(GeoError::Invalid(format!("not a month: {month:?}")));
        }
        fs::create_dir_all(dir)?;
        let city = Part::new(dir, city_file(month));
        let asn = Part::new(dir, asn_file(month));
        let done = self.fetch(month, dir, &city, &asn, progress, cancel);
        if done.is_err() {
            for part in [&city, &asn] {
                let _ = fs::remove_file(&part.temp);
            }
        }
        done
    }

    fn fetch(
        &self,
        month: &str,
        dir: &Path,
        city: &Part,
        asn: &Part,
        progress: &dyn Fn(u64, Option<u64>),
        cancel: &AtomicBool,
    ) -> Result<(), GeoError> {
        // The size of the ASN file first: it completes the total for the progress, and a month
        // without one fails now rather than after the long City download.
        not_cancelled(cancel)?;
        let asn_size = match self.published(&asn.name, Instant::now() + CHECK_TIMEOUT)? {
            Published::Yes(size) => size,
            Published::No => return Err(GeoError::Http(404)),
        };

        not_cancelled(cancel)?;
        let body = self.get(&city.name)?;
        let city_size = content_length(&body);
        let mut meter = Meter {
            progress,
            // The sizes come from the server and may be anything.
            total: city_size.zip(asn_size).and_then(|(c, a)| c.checked_add(a)),
            received: 0,
            reported: 0,
        };
        meter.report();
        let body = body.into_body().into_reader();
        let city_bytes = unpack(body, city, &mut meter, cancel, LIMITS)?;
        check_part(&city.temp, "City")?;

        not_cancelled(cancel)?;
        let body = self.get(&asn.name)?;
        if let Some(size) = content_length(&body) {
            meter.total = city_size.and_then(|c| c.checked_add(size));
        }
        let body = body.into_body().into_reader();
        let asn_bytes = unpack(body, asn, &mut meter, cancel, LIMITS)?;
        check_part(&asn.temp, "ASN")?;

        not_cancelled(cancel)?;
        for part in [city, asn] {
            fs::rename(&part.temp, &part.path).map_err(|e| part.io(e))?;
        }
        let manifest = Manifest {
            month: month.to_owned(),
            city: Some(city.entry(city_bytes)),
            asn: Some(asn.entry(asn_bytes)),
            source: Some(self.base.clone()),
        };
        write_manifest(dir, &manifest)?;
        meter.finish();
        Ok(())
    }

    /// A GET of the whole `<file>.gz`; anything but 200 is an error, and so is a size larger
    /// than a file may be.
    fn get(&self, file: &str) -> Result<Response<Body>, GeoError> {
        let response = call(self.agent.get(self.url(file)), None)?;
        match response.status().as_u16() {
            200 if content_length(&response).is_some_and(|size| size > LIMITS.gz) => {
                Err(too_large(&format!("{file}.gz"), LIMITS.gz))
            }
            200 => Ok(response),
            status => Err(GeoError::Http(status)),
        }
    }

    /// The URL of DB-IP's compressed `file`.
    fn url(&self, file: &str) -> String {
        format!("{}{file}.gz", self.base)
    }
}

/// Sends a request without a body, to be answered by `deadline` when there is one.
fn call(
    request: RequestBuilder<WithoutBody>,
    deadline: Option<Instant>,
) -> Result<Response<Body>, GeoError> {
    let request = match deadline {
        None => request,
        Some(deadline) => {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(GeoError::Net("timeout: global".into()));
            }
            request.config().timeout_global(Some(left)).build()
        }
    };
    request.call().map_err(|e| match e {
        // Without ureq's "io: " in front.
        ureq::Error::Io(e) => GeoError::Net(e.to_string()),
        e => GeoError::Net(e.to_string()),
    })
}

/// The `Content-Length` of a response.
fn content_length(response: &Response<Body>) -> Option<u64> {
    let value = response.headers().get(header::CONTENT_LENGTH)?;
    value.to_str().ok()?.trim().parse().ok()
}

/// The size of the whole file from the `Content-Range` of a partial answer:
/// "bytes 0-0/61234567"; none for "bytes 0-0/*".
fn range_total(response: &Response<Body>) -> Option<u64> {
    let value = response
        .headers()
        .get(header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    value.rsplit_once('/')?.1.trim().parse().ok()
}

fn not_cancelled(cancel: &AtomicBool) -> Result<(), GeoError> {
    if cancel.load(Ordering::SeqCst) {
        Err(GeoError::Cancelled)
    } else {
        Ok(())
    }
}

/// One database being downloaded: its final name and the part it is written to first.
struct Part {
    name: String,
    path: PathBuf,
    temp: PathBuf,
}

impl Part {
    fn new(dir: &Path, name: String) -> Self {
        Part {
            path: dir.join(&name),
            temp: dir.join(format!("{name}.part")),
            name,
        }
    }

    fn io(&self, e: io::Error) -> GeoError {
        GeoError::Io(format!("{}: {e}", self.name))
    }

    fn entry(&self, bytes: u64) -> ManifestFile {
        ManifestFile {
            file: self.name.clone(),
            bytes,
            sha1: None,
        }
    }
}

/// Streams a gzip body through gunzip into the part; the size of what it wrote. A file of
/// several gzip members is read whole, as `gzip -d` reads it. A file larger than `limits`
/// allow, as it comes or unpacked, is invalid.
fn unpack(
    body: impl Read,
    part: &Part,
    meter: &mut Meter,
    cancel: &AtomicBool,
    limits: Limits,
) -> Result<u64, GeoError> {
    let before = meter.received;
    let mut gz = MultiGzDecoder::new(Counted {
        inner: body,
        count: 0,
        limit: limits.gz,
        cancel,
        stopped: None,
    });
    let mut out = File::create(&part.temp).map_err(|e| part.io(e))?;
    let mut buf = vec![0; 64 * 1024];
    let mut written = 0u64;
    loop {
        not_cancelled(cancel)?;
        let n = match gz.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(match gz.get_mut().stopped.take() {
                    // The connection broke or stalled.
                    Some(Stopped::Net(why)) => GeoError::Net(why),
                    Some(Stopped::TooLarge) => too_large(&format!("{}.gz", part.name), limits.gz),
                    Some(Stopped::Cancelled) => GeoError::Cancelled,
                    // What came is not gzip, or not all of it.
                    None => GeoError::Invalid(format!("{}.gz: {e}", part.name)),
                });
            }
        };
        written += n as u64;
        if written > limits.db {
            return Err(too_large(&part.name, limits.db));
        }
        out.write_all(&buf[..n]).map_err(|e| part.io(e))?;
        meter.at(before + gz.get_ref().count);
    }
    // The read that found the end can bring the last bytes, gzip's checksum and size.
    meter.at(before + gz.get_ref().count);
    // On disk before the rename makes it count.
    out.sync_all().map_err(|e| part.io(e))?;
    Ok(written)
}

/// `name` is larger than the `limit` of bytes a file may have.
fn too_large(name: &str, limit: u64) -> GeoError {
    GeoError::Invalid(format!("{name}: larger than {limit} bytes"))
}

/// Whether the part opens as a `kind` database. The reader is gone before the part is renamed.
fn check_part(path: &Path, kind: &str) -> Result<(), GeoError> {
    open_mmdb(path, kind).map(drop)
}

/// A response body that counts the bytes coming over the network. It stops at more than
/// `limit` of them, and at the next read once the download is cancelled, also while gunzip
/// reads on without giving anything out. It remembers why it stopped, which tells a broken
/// connection from a broken file.
struct Counted<'a, R> {
    inner: R,
    count: u64,
    limit: u64,
    cancel: &'a AtomicBool,
    stopped: Option<Stopped>,
}

/// Why reading a response body failed, when it was not the file.
enum Stopped {
    /// The network: the connection broke or stalled.
    Net(String),
    /// More than the limit came.
    TooLarge,
    /// The user stopped the download.
    Cancelled,
}

impl<R: Read> Read for Counted<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::SeqCst) {
            self.stopped = Some(Stopped::Cancelled);
            return Err(io::Error::other("cancelled"));
        }
        match self.inner.read(buf) {
            Ok(n) => {
                self.count += n as u64;
                if self.count > self.limit {
                    self.stopped = Some(Stopped::TooLarge);
                    return Err(io::Error::other("too large"));
                }
                Ok(n)
            }
            Err(e) => {
                if e.kind() != io::ErrorKind::Interrupted {
                    self.stopped = Some(Stopped::Net(e.to_string()));
                }
                Err(e)
            }
        }
    }
}

/// The progress of a download, told at the start, at most every [`PROGRESS_STEP`] and at the
/// end.
struct Meter<'a> {
    progress: &'a dyn Fn(u64, Option<u64>),
    total: Option<u64>,
    received: u64,
    reported: u64,
}

impl Meter<'_> {
    fn at(&mut self, received: u64) {
        self.received = received;
        if received >= self.reported + PROGRESS_STEP {
            self.report();
        }
    }

    fn report(&mut self) {
        self.reported = self.received;
        (self.progress)(self.received, self.total);
    }

    fn finish(&mut self) {
        if self.received != self.reported {
            self.report();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::{pick, read_manifest, Layout, Source, MANIFEST};
    use crate::testing::{db_with, frankfurt, tiny_db, TempDir};
    use crate::Locate;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;
    use std::thread;

    /// 2026-10-01 00:00 UTC.
    const BUILT: u64 = 1_790_812_800;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    /// 2026-10-28 12:00 UTC.
    fn now() -> SystemTime {
        at(1_793_188_800)
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    /// A request as the server saw it; header names in lower case.
    #[derive(Debug, Clone)]
    struct Seen {
        method: String,
        path: String,
        headers: Vec<(String, String)>,
    }

    impl Seen {
        fn header(&self, name: &str) -> Option<&str> {
            let found = self.headers.iter().find(|(k, _)| k == name);
            found.map(|(_, v)| v.as_str())
        }

        fn line(&self) -> String {
            format!("{} {}", self.method, self.path)
        }
    }

    /// What the server answers to one request.
    struct Answer {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        /// Only this much of the body goes out before the connection closes.
        cut: Option<usize>,
        /// A pause before the answer.
        wait: Duration,
        /// The connection closes without an answer.
        hang_up: bool,
    }

    impl Answer {
        fn status(status: u16) -> Self {
            Answer {
                status,
                headers: Vec::new(),
                body: Vec::new(),
                cut: None,
                wait: Duration::ZERO,
                hang_up: false,
            }
        }

        fn file(body: Vec<u8>) -> Self {
            Answer {
                body,
                ..Answer::status(200)
            }
        }

        fn hang_up() -> Self {
            Answer {
                hang_up: true,
                ..Answer::status(200)
            }
        }

        fn with(mut self, name: &str, value: impl ToString) -> Self {
            self.headers.push((name.to_owned(), value.to_string()));
            self
        }

        fn cut(mut self, sent: usize) -> Self {
            self.cut = Some(sent);
            self
        }

        fn after(mut self, wait: Duration) -> Self {
            self.wait = wait;
            self
        }
    }

    /// A plain HTTP/1.1 server on 127.0.0.1 for one test. It answers each connection once,
    /// through `answer`, and closes it; it counts the connections and keeps the requests.
    struct Server {
        addr: SocketAddr,
        connections: Arc<AtomicUsize>,
        seen: Arc<Mutex<Vec<Seen>>>,
        stop: Arc<AtomicBool>,
    }

    impl Server {
        fn start(answer: impl Fn(&Seen) -> Answer + Send + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let server = Server {
                addr: listener.local_addr().unwrap(),
                connections: Arc::default(),
                seen: Arc::default(),
                stop: Arc::default(),
            };
            let (connections, seen, stop) = (
                server.connections.clone(),
                server.seen.clone(),
                server.stop.clone(),
            );
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    connections.fetch_add(1, Ordering::SeqCst);
                    // A client that hangs up early is no failure of the server.
                    let _ = serve(stream, &answer, &seen);
                }
            });
            server
        }

        fn base(&self) -> String {
            format!("http://{}/free/", self.addr)
        }

        /// An updater that goes to this server directly, whatever proxy the environment
        /// names.
        fn updater(&self) -> Updater {
            Updater::build(&self.base(), None)
        }

        fn connections(&self) -> usize {
            self.connections.load(Ordering::SeqCst)
        }

        fn seen(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }

        fn lines(&self) -> Vec<String> {
            self.seen().iter().map(Seen::line).collect()
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            // Wakes the server up to see that.
            let _ = TcpStream::connect(self.addr);
        }
    }

    fn serve(
        mut stream: TcpStream,
        answer: &dyn Fn(&Seen) -> Answer,
        seen: &Mutex<Vec<Seen>>,
    ) -> io::Result<()> {
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let mut head = Vec::new();
        let mut chunk = [0; 4096];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                return Ok(());
            }
            if head.is_empty() && !chunk[0].is_ascii_uppercase() {
                // Not HTTP: noted by its first byte (0x16 opens a TLS handshake), and hung up on.
                let method = format!("<{:#04x}>", chunk[0]);
                seen.lock().unwrap().push(Seen {
                    method,
                    path: String::new(),
                    headers: Vec::new(),
                });
                return Ok(());
            }
            head.extend_from_slice(&chunk[..n]);
        }
        let text = String::from_utf8_lossy(&head).into_owned();
        let mut lines = text.split("\r\n");
        let mut first = lines.next().unwrap_or_default().split(' ');
        let request = Seen {
            method: first.next().unwrap_or_default().to_owned(),
            path: first.next().unwrap_or_default().to_owned(),
            headers: lines
                .filter_map(|l| l.split_once(':'))
                .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
                .collect(),
        };
        seen.lock().unwrap().push(request.clone());
        let a = answer(&request);
        thread::sleep(a.wait);
        if a.hang_up {
            return Ok(());
        }
        let mut out = format!("HTTP/1.1 {} {}\r\n", a.status, reason(a.status));
        out.push_str("Connection: close\r\n");
        if !a.headers.iter().any(|(k, _)| k == "Content-Length") {
            out.push_str(&format!("Content-Length: {}\r\n", a.body.len()));
        }
        for (k, v) in &a.headers {
            out.push_str(&format!("{k}: {v}\r\n"));
        }
        out.push_str("\r\n");
        stream.write_all(out.as_bytes())?;
        if request.method != "HEAD" {
            stream.write_all(&a.body[..a.cut.unwrap_or(a.body.len())])?;
        }
        stream.flush()
    }

    fn reason(status: u16) -> &'static str {
        match status {
            200 => "OK",
            206 => "Partial Content",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            500 => "Internal Server Error",
            501 => "Not Implemented",
            503 => "Service Unavailable",
            _ => "Something",
        }
    }

    /// DB-IP as these tests have it: `files` under /free/. HEAD answers with the size, or with
    /// the status `head` when that is not 200 (a server without HEAD); GET sends the whole
    /// file, or its first byte for `Range: bytes=0-0`; anything else is not found.
    fn dbip(files: Vec<(String, Vec<u8>)>, head: u16) -> impl Fn(&Seen) -> Answer + Send {
        move |r: &Seen| {
            if r.method == "HEAD" && head != 200 {
                return Answer::status(head);
            }
            let name = r.path.strip_prefix("/free/").unwrap_or_default();
            let Some((_, body)) = files.iter().find(|(n, _)| n == name) else {
                return Answer::status(404);
            };
            if r.method == "GET" && r.header("range") == Some("bytes=0-0") {
                let first = Answer {
                    body: body[..1].to_vec(),
                    ..Answer::status(206)
                };
                return first.with("Content-Range", format!("bytes 0-0/{}", body.len()));
            }
            Answer::file(body.clone())
        }
    }

    /// The files of 2026-11 under DB-IP's names.
    fn month_files(city_gz: Vec<u8>, asn_gz: Vec<u8>) -> Vec<(String, Vec<u8>)> {
        vec![
            ("dbip-city-lite-2026-11.mmdb.gz".into(), city_gz),
            ("dbip-asn-lite-2026-11.mmdb.gz".into(), asn_gz),
        ]
    }

    fn city_db() -> Vec<u8> {
        tiny_db("DBIP-City-Lite", BUILT)
    }

    fn asn_db() -> Vec<u8> {
        tiny_db("DBIP-ASN-Lite (compat=GeoLite2-ASN)", BUILT)
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), Compression::default());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    /// `n` bytes that do not compress.
    fn noise(n: usize) -> Vec<u8> {
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 32) as u8
            })
            .collect()
    }

    /// The progress reports of a download: bytes received and their total.
    type Heard = Vec<(u64, Option<u64>)>;

    /// Downloads 2026-11 from `server` into `folder`: the progress reports and the outcome.
    fn fetch_month(
        server: &Server,
        folder: &Path,
        cancel: &AtomicBool,
    ) -> (Heard, Result<(), GeoError>) {
        let heard = Mutex::new(Vec::new());
        let progress = |received: u64, total: Option<u64>| {
            heard.lock().unwrap().push((received, total));
        };
        let done = server
            .updater()
            .download("2026-11", folder, &progress, cancel);
        (heard.into_inner().unwrap(), done)
    }

    #[test]
    fn months_follow_the_calendar_in_utc() {
        for (secs, month) in [
            (0, "1970-01"),
            // 2000-02-29 23:59:59: a century that is a leap year.
            (951_868_799, "2000-02"),
            (951_868_800, "2000-03"),
            (1_709_208_000, "2024-02"),
            (1_790_812_799, "2026-09"),
            (1_790_812_800, "2026-10"),
            (1_793_188_800, "2026-10"),
            (1_798_761_599, "2026-12"),
            (1_798_761_600, "2027-01"),
            // 2100 is no leap year.
            (4_107_542_399, "2100-02"),
            (4_107_542_400, "2100-03"),
        ] {
            assert_eq!(current_month(at(secs)), month, "{secs}");
        }
        // A clock before 1970 is wrong; the first month stands in.
        let before = UNIX_EPOCH - Duration::from_secs(86_400);
        assert_eq!(current_month(before), "1970-01");

        for (month, prev) in [
            ("2026-10", Some("2026-09")),
            ("2026-01", Some("2025-12")),
            ("2000-03", Some("2000-02")),
            ("0000-01", None),
            ("2026-13", None),
            ("October", None),
        ] {
            assert_eq!(prev_month(month).as_deref(), prev, "{month}");
        }
    }

    #[test]
    fn updaters_go_to_dbip_unless_told_otherwise() {
        // The app checks on one thread and downloads on another.
        fn shared<T: Send + Sync + Clone>() {}
        shared::<Updater>();
        let dbip = Updater::new();
        assert_eq!(dbip.base(), DEFAULT_BASE);
        assert_eq!(Updater::default().base(), DEFAULT_BASE);
        let config = dbip.agent.config();
        assert!(config.https_only());
        assert!(!config.http_status_as_error());
        assert_eq!(config.timeouts().connect, Some(CONNECT_TIMEOUT));
        assert_eq!(config.timeout_per_read(), Some(READ_TIMEOUT));
        // A whole download may take minutes: only a check has a time limit.
        assert_eq!(config.timeouts().global, None);

        let local = Updater::with_base("http://127.0.0.1:9/free");
        assert_eq!(local.base(), "http://127.0.0.1:9/free/");
        assert!(!local.agent.config().https_only());
        assert_eq!(
            local.url("dbip-city-lite-2026-10.mmdb"),
            "http://127.0.0.1:9/free/dbip-city-lite-2026-10.mmdb.gz"
        );
        assert_eq!(
            USER_AGENT,
            format!(
                "Whilo-Trace/{} (+https://github.com/Whilo-Labs/whilo-trace)",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn no_request_while_the_databases_are_of_the_current_month() {
        let server = Server::start(|_: &Seen| Answer::status(200));
        let u = server.updater();
        assert_eq!(u.check(Some("2026-10"), now()), Ok(None));
        // A clock that runs behind the databases.
        assert_eq!(u.check(Some("2026-11"), now()), Ok(None));
        assert_eq!(server.connections(), 0);
        assert!(server.seen().is_empty());
    }

    #[test]
    fn a_month_that_is_out_is_offered_with_the_size_of_its_city_file() {
        let files = vec![("dbip-city-lite-2026-10.mmdb.gz".to_owned(), vec![7; 1234])];
        let server = Server::start(dbip(files, 200));
        let u = server.updater();
        let offer = Ok(Some(Offer {
            month: "2026-10".into(),
            city_bytes: Some(1234),
        }));
        assert_eq!(u.check(Some("2026-09"), now()), offer);
        // No databases at all: the current month too. A month that is not one counts as none.
        assert_eq!(u.check(None, now()), offer);
        assert_eq!(u.check(Some("someday"), now()), offer);

        assert_eq!(server.connections(), 3);
        for s in server.seen() {
            assert_eq!(s.line(), "HEAD /free/dbip-city-lite-2026-10.mmdb.gz");
            assert_eq!(s.header("user-agent"), Some(USER_AGENT));
            assert_eq!(s.header("range"), None);
            // The .gz file is taken as it is, never unpacked on the way.
            assert_eq!(s.header("accept-encoding"), None);
        }
    }

    #[test]
    fn a_month_not_out_yet_is_no_offer() {
        let server = Server::start(dbip(Vec::new(), 200));
        assert_eq!(server.updater().check(Some("2026-09"), now()), Ok(None));
        assert_eq!(
            server.lines(),
            ["HEAD /free/dbip-city-lite-2026-10.mmdb.gz"]
        );
    }

    #[test]
    fn servers_without_head_get_a_request_for_one_byte() {
        let files = vec![("dbip-city-lite-2026-10.mmdb.gz".to_owned(), vec![7; 61_234])];
        for head in [405, 501] {
            let server = Server::start(dbip(files.clone(), head));
            assert_eq!(
                server.updater().check(None, now()),
                Ok(Some(Offer {
                    month: "2026-10".into(),
                    city_bytes: Some(61_234),
                }))
            );
            let seen = server.seen();
            assert_eq!(
                seen.iter().map(Seen::line).collect::<Vec<_>>(),
                [
                    "HEAD /free/dbip-city-lite-2026-10.mmdb.gz",
                    "GET /free/dbip-city-lite-2026-10.mmdb.gz"
                ]
            );
            assert_eq!(seen[1].header("range"), Some("bytes=0-0"));
            assert_eq!(server.connections(), 2);
        }

        // Not out yet.
        let server = Server::start(dbip(Vec::new(), 405));
        assert_eq!(server.updater().check(None, now()), Ok(None));
        assert_eq!(server.connections(), 2);

        // A server that ignores the range sends the whole file, which is not read.
        let server = Server::start(|r: &Seen| match r.method.as_str() {
            "HEAD" => Answer::status(405),
            _ => Answer::file(vec![7; 500_000]),
        });
        let offer = server.updater().check(None, now()).unwrap().unwrap();
        assert_eq!(offer.city_bytes, Some(500_000));

        // A size the server does not know.
        let server = Server::start(|r: &Seen| match r.method.as_str() {
            "HEAD" => Answer::status(501),
            _ => Answer {
                body: vec![0x1f],
                ..Answer::status(206)
            }
            .with("Content-Range", "bytes 0-0/*"),
        });
        let offer = server.updater().check(None, now()).unwrap().unwrap();
        assert_eq!((offer.month.as_str(), offer.city_bytes), ("2026-10", None));
    }

    #[test]
    fn other_answers_and_failed_connections_are_errors() {
        let server = Server::start(|_: &Seen| Answer::status(503));
        assert_eq!(
            server.updater().check(None, now()),
            Err(GeoError::Http(503))
        );

        let server = Server::start(|r: &Seen| match r.method.as_str() {
            "HEAD" => Answer::status(405),
            _ => Answer::status(403),
        });
        assert_eq!(
            server.updater().check(None, now()),
            Err(GeoError::Http(403))
        );

        let server = Server::start(|_: &Seen| Answer::hang_up());
        let e = server.updater().check(None, now()).unwrap_err();
        assert!(matches!(e, GeoError::Net(_)), "{e}");

        // HTTPS: ring and the system's certificates are set up, so a server that does not
        // speak TLS gets a TLS handshake and makes an error, not a panic.
        let https = Updater::build(&format!("https://{}/free/", server.addr), None);
        let e = https.check(None, now()).unwrap_err();
        assert!(matches!(e, GeoError::Net(_)), "{e}");
        assert_eq!(server.seen().last().unwrap().method, "<0x16>");
    }

    #[test]
    fn a_whole_check_has_a_time_limit() {
        // No HEAD, after a while, and slower still with the range request, which only gets
        // what is left of the limit.
        let server = Server::start(|r: &Seen| match r.method.as_str() {
            "HEAD" => Answer::status(405).after(Duration::from_secs(1)),
            _ => Answer::status(404).after(Duration::from_secs(3)),
        });
        let started = Instant::now();
        let limit = Duration::from_millis(1500);
        let e = server
            .updater()
            .check_within(None, now(), limit)
            .unwrap_err();
        assert!(
            matches!(&e, GeoError::Net(why) if why.contains("timeout")),
            "{e}"
        );
        // About 1.5 s; a limit for each request would make it 1 s + 1.5 s.
        let took = started.elapsed();
        assert!(took < Duration::from_millis(2200), "{took:?}");
        assert_eq!(server.connections(), 2);
    }

    #[test]
    fn a_download_unpacks_checks_and_renames_both_files() {
        let dir = TempDir::new("download");
        let folder = dir.path().join("geo-update");
        let (city, asn) = (city_db(), asn_db());
        let (city_gz, asn_gz) = (gzip(&city), gzip(&asn));
        let total = (city_gz.len() + asn_gz.len()) as u64;
        let server = Server::start(dbip(month_files(city_gz, asn_gz), 200));

        let (heard, done) = fetch_month(&server, &folder, &AtomicBool::new(false));
        done.unwrap();

        // The size of the ASN file, then the two files.
        assert_eq!(
            server.lines(),
            [
                "HEAD /free/dbip-asn-lite-2026-11.mmdb.gz",
                "GET /free/dbip-city-lite-2026-11.mmdb.gz",
                "GET /free/dbip-asn-lite-2026-11.mmdb.gz"
            ]
        );
        assert_eq!(server.connections(), 3);
        assert!(server
            .seen()
            .iter()
            .all(|s| s.header("user-agent") == Some(USER_AGENT)));
        // From nothing to all of it, the total known from the start.
        assert_eq!(heard, [(0, Some(total)), (total, Some(total))]);

        // The unpacked files under their final names, and the manifest naming the month.
        assert_eq!(
            dir.names("geo-update"),
            [
                "dbip-asn-lite-2026-11.mmdb",
                "dbip-city-lite-2026-11.mmdb",
                MANIFEST
            ]
        );
        assert_eq!(
            fs::read(folder.join("dbip-city-lite-2026-11.mmdb")).unwrap(),
            city
        );
        assert_eq!(
            fs::read(folder.join("dbip-asn-lite-2026-11.mmdb")).unwrap(),
            asn
        );
        assert_eq!(
            read_manifest(&folder),
            Some(Manifest {
                month: "2026-11".into(),
                city: Some(ManifestFile {
                    file: "dbip-city-lite-2026-11.mmdb".into(),
                    bytes: city.len() as u64,
                    sha1: None,
                }),
                asn: Some(ManifestFile {
                    file: "dbip-asn-lite-2026-11.mmdb".into(),
                    bytes: asn.len() as u64,
                    sha1: None,
                }),
                source: Some(server.base()),
            })
        );

        // It is the set to use now, and it finds places.
        let layout = Layout {
            bundled: dir.path().join("bundled"),
            downloaded: folder,
        };
        let set = pick(&layout).unwrap();
        assert_eq!(
            (set.source, set.month.as_deref()),
            (Source::Downloaded, Some("2026-11"))
        );
        let db = set.open().unwrap();
        let found = db.locate(ip("8.8.8.8"), 5, ip("5.0.10.1")).unwrap();
        assert_eq!(found.city.as_deref(), Some("Frankfurt am Main"));
        assert_eq!(found.asn, Some(64504));
    }

    #[test]
    fn a_large_download_tells_its_progress_on_the_way() {
        let dir = TempDir::new("large");
        let folder = dir.path().join("geo-update");
        // A City database with 2 MB of letters that compress badly, sent as three gzip
        // members (which `gzip -d` reads as one file), by a server without HEAD.
        let padding: String = noise(2_000_000)
            .iter()
            .map(|b| char::from(b'a' + b % 26))
            .collect();
        let mut record = frankfurt();
        record["padding"] = padding.into();
        let city = db_with("DBIP-City-Lite", BUILT, &record);
        let third = city.len() / 3;
        let city_gz: Vec<u8> = [&city[..third], &city[third..2 * third], &city[2 * third..]]
            .iter()
            .flat_map(|part| gzip(part))
            .collect();
        let asn_gz = gzip(&asn_db());
        let total = (city_gz.len() + asn_gz.len()) as u64;
        let server = Server::start(dbip(month_files(city_gz, asn_gz), 405));

        let (heard, done) = fetch_month(&server, &folder, &AtomicBool::new(false));
        done.unwrap();
        assert_eq!(
            fs::read(folder.join("dbip-city-lite-2026-11.mmdb")).unwrap(),
            city
        );
        assert!(total > 3 * PROGRESS_STEP, "{total}");
        // The size of the ASN file came from its first byte.
        assert_eq!(
            server.lines()[..2],
            [
                "HEAD /free/dbip-asn-lite-2026-11.mmdb.gz",
                "GET /free/dbip-asn-lite-2026-11.mmdb.gz"
            ]
        );
        assert_eq!(heard.first(), Some(&(0, Some(total))));
        assert_eq!(heard.last(), Some(&(total, Some(total))));
        assert!(heard.len() >= 4, "{heard:?}");
        for pair in heard[..heard.len() - 1].windows(2) {
            assert!(pair[1].0 >= pair[0].0 + PROGRESS_STEP, "{heard:?}");
        }
    }

    #[test]
    fn progress_is_told_at_most_every_256_kb() {
        let heard = Mutex::new(Vec::new());
        let progress = |received: u64, _: Option<u64>| heard.lock().unwrap().push(received);
        let mut meter = Meter {
            progress: &progress,
            total: Some(1_000_000),
            received: 0,
            reported: 0,
        };
        meter.report();
        for received in [1000, 200_000, 262_144, 262_145, 500_000, 524_288, 999_999] {
            meter.at(received);
        }
        meter.at(1_000_000);
        meter.finish();
        meter.finish();
        assert_eq!(
            heard.into_inner().unwrap(),
            [0, 262_144, 524_288, 999_999, 1_000_000]
        );
    }

    /// Unpacks `body` into a part in a folder of its own, within `limits`: the outcome and
    /// the progress reports, the last one made at the end.
    fn unpack_alone(
        body: impl Read,
        cancel: &AtomicBool,
        limits: Limits,
    ) -> (Result<u64, GeoError>, Heard) {
        let dir = TempDir::new("unpack");
        let part = Part::new(dir.path(), "dbip-city-lite-2026-11.mmdb".into());
        let heard = Mutex::new(Vec::new());
        let progress = |received: u64, total: Option<u64>| {
            heard.lock().unwrap().push((received, total));
        };
        let mut meter = Meter {
            progress: &progress,
            total: None,
            received: 0,
            reported: 0,
        };
        let done = unpack(body, &part, &mut meter, cancel, limits);
        meter.finish();
        (done, heard.into_inner().unwrap())
    }

    #[test]
    fn the_end_of_a_file_counts_when_it_comes_on_its_own() {
        let city = city_db();
        let gz = gzip(&city);
        // Over a network, the last read can bring gzip's last 8 bytes (checksum and size)
        // alone, after all they close is unpacked.
        let (most, end) = gz.split_at(gz.len() - 8);
        let (done, heard) = unpack_alone(most.chain(end), &AtomicBool::new(false), LIMITS);
        assert_eq!(done, Ok(city.len() as u64));
        assert_eq!(heard, [(gz.len() as u64, None)]);
    }

    /// `len` bytes of a gzip file that gunzip takes in without giving anything out: a
    /// header, then empty deflate blocks, none of them the last.
    fn blank(len: usize) -> Vec<u8> {
        // Gzip, deflate, no flags, no time, no extra flags, an unknown system.
        let mut gz = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
        // A stored block of no bytes that is not the last: its header, LEN 0 and NLEN !0.
        let block = [0, 0, 0, 0xff, 0xff];
        gz.extend(block.iter().cycle().take(len - gz.len()));
        gz
    }

    /// Reads `inner` and sets `cancel` once `after` bytes have come through: the user
    /// pressing Cancel on the way.
    struct CancelAfter<'a, R> {
        inner: R,
        after: u64,
        cancel: &'a AtomicBool,
    }

    impl<R: Read> Read for CancelAfter<'_, R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.after = self.after.saturating_sub(n as u64);
            if self.after == 0 {
                self.cancel.store(true, Ordering::SeqCst);
            }
            Ok(n)
        }
    }

    #[test]
    fn unpacking_stops_at_the_limits_and_when_cancelled() {
        let small = Limits {
            gz: 64 << 10,
            db: 64 << 10,
        };
        // A file that goes on and on: gunzip reads it within one call, without giving
        // anything out, until more than the limit has come.
        let blank = blank(1 << 20);
        let (done, _) = unpack_alone(&blank[..], &AtomicBool::new(false), small);
        assert_eq!(
            done,
            Err(GeoError::Invalid(
                "dbip-city-lite-2026-11.mmdb.gz: larger than 65536 bytes".into()
            ))
        );

        // Cancelled during that call, once 100 KB have come.
        let cancel = AtomicBool::new(false);
        let body = CancelAfter {
            inner: &blank[..],
            after: 100 << 10,
            cancel: &cancel,
        };
        let (done, _) = unpack_alone(body, &cancel, LIMITS);
        assert_eq!(done, Err(GeoError::Cancelled));

        // A small file that unpacks to far more.
        let bomb = gzip(&vec![0; 1 << 20]);
        assert!(bomb.len() < 64 << 10, "{}", bomb.len());
        let (done, _) = unpack_alone(&bomb[..], &AtomicBool::new(false), small);
        assert_eq!(
            done,
            Err(GeoError::Invalid(
                "dbip-city-lite-2026-11.mmdb: larger than 65536 bytes".into()
            ))
        );
    }

    #[test]
    fn sizes_from_the_server_cannot_break_a_download() {
        let (city_gz, asn_gz) = (gzip(&city_db()), gzip(&asn_db()));
        let total = (city_gz.len() + asn_gz.len()) as u64;
        for line in [
            "HEAD /free/dbip-asn-lite-2026-11.mmdb.gz",
            "GET /free/dbip-city-lite-2026-11.mmdb.gz",
            "GET /free/dbip-asn-lite-2026-11.mmdb.gz",
        ] {
            let dir = TempDir::new("size");
            let folder = dir.path().join("geo-update");
            let site = dbip(month_files(city_gz.clone(), asn_gz.clone()), 200);
            // The largest size there is: added to another, it overflows.
            let server = Server::start(move |r: &Seen| {
                if r.line() == line {
                    site(r).with("Content-Length", u64::MAX)
                } else {
                    site(r)
                }
            });
            let (heard, done) = fetch_month(&server, &folder, &AtomicBool::new(false));
            match line.strip_prefix("GET /free/") {
                // The size from HEAD only goes into the total, which waits for the size the
                // ASN file comes with.
                None => {
                    done.unwrap();
                    assert_eq!(heard, [(0, None), (total, Some(total))]);
                }
                Some(file) => {
                    let why = format!("{file}: larger than 536870912 bytes");
                    assert_eq!(done, Err(GeoError::Invalid(why)), "{line}");
                    assert!(dir.names("geo-update").is_empty(), "{line}");
                }
            }
        }
    }

    #[test]
    fn a_month_without_its_asn_file_fails_before_the_city_download() {
        let dir = TempDir::new("no-asn");
        let folder = dir.path().join("geo-update");
        let files = vec![(
            "dbip-city-lite-2026-11.mmdb.gz".to_owned(),
            gzip(&city_db()),
        )];
        let server = Server::start(dbip(files, 200));
        let (heard, done) = fetch_month(&server, &folder, &AtomicBool::new(false));
        assert_eq!(done, Err(GeoError::Http(404)));
        assert_eq!(server.lines(), ["HEAD /free/dbip-asn-lite-2026-11.mmdb.gz"]);
        assert!(heard.is_empty());
        assert!(dir.names("geo-update").is_empty());
    }

    #[test]
    fn the_files_have_to_come_whole() {
        let city_gz = gzip(&city_db());
        let asn_gz = gzip(&asn_db());
        let mut flipped = city_gz.clone();
        let middle = flipped.len() / 2;
        flipped[middle] ^= 0xff;
        let shortened = city_gz[..city_gz.len() - 20].to_vec();
        let half = city_gz.len() / 2;
        type Site = Box<dyn Fn(&Seen) -> Answer + Send>;
        let cases: Vec<(&str, Site)> = vec![
            (
                "damaged",
                Box::new(dbip(month_files(flipped, asn_gz.clone()), 200)),
            ),
            (
                "not gzip",
                Box::new(dbip(month_files(city_db(), asn_gz.clone()), 200)),
            ),
            (
                "a gzip file cut short",
                Box::new(dbip(month_files(shortened, asn_gz.clone()), 200)),
            ),
            ("a connection cut short", {
                let site = dbip(month_files(city_gz.clone(), asn_gz.clone()), 200);
                Box::new(move |r: &Seen| match r.line().as_str() {
                    "GET /free/dbip-city-lite-2026-11.mmdb.gz" => site(r).cut(half),
                    _ => site(r),
                })
            }),
            ("a server error", {
                let site = dbip(month_files(city_gz.clone(), asn_gz.clone()), 200);
                Box::new(move |r: &Seen| match r.line().as_str() {
                    "GET /free/dbip-asn-lite-2026-11.mmdb.gz" => Answer::status(500),
                    _ => site(r),
                })
            }),
        ];
        for (case, answer) in cases {
            let dir = TempDir::new("broken");
            let folder = dir.path().join("geo-update");
            // The month in use before the download.
            dir.file("geo-update/dbip-city-lite-2026-10.mmdb", &city_db());
            dir.file("geo-update/dbip-asn-lite-2026-10.mmdb", &asn_db());
            dir.file("geo-update/manifest.json", br#"{"month":"2026-10"}"#);
            let server = Server::start(answer);

            let (_, done) = fetch_month(&server, &folder, &AtomicBool::new(false));
            let e = done.unwrap_err();
            let fits = match case {
                "a connection cut short" => matches!(e, GeoError::Net(_)),
                "a server error" => e == GeoError::Http(500),
                _ => matches!(&e, GeoError::Invalid(why)
                    if why.starts_with("dbip-city-lite-2026-11.mmdb.gz: ")),
            };
            assert!(fits, "{case}: {e}");
            // No parts, and the month in use as it was.
            assert_eq!(
                dir.names("geo-update"),
                [
                    "dbip-asn-lite-2026-10.mmdb",
                    "dbip-city-lite-2026-10.mmdb",
                    MANIFEST
                ],
                "{case}"
            );
            assert_eq!(read_manifest(&folder).unwrap().month, "2026-10", "{case}");
        }
    }

    #[test]
    fn a_database_of_another_type_is_refused() {
        let city_gz = gzip(&city_db());
        let asn_gz = gzip(&asn_db());
        for (case, files, why) in [
            (
                "ASN as City",
                month_files(asn_gz.clone(), asn_gz.clone()),
                "dbip-city-lite-2026-11.mmdb.part: the database type is \
                 \"DBIP-ASN-Lite (compat=GeoLite2-ASN)\", expected City",
            ),
            (
                "City as ASN",
                month_files(city_gz.clone(), city_gz.clone()),
                "dbip-asn-lite-2026-11.mmdb.part: the database type is \
                 \"DBIP-City-Lite\", expected ASN",
            ),
        ] {
            let dir = TempDir::new("type");
            let folder = dir.path().join("geo-update");
            let server = Server::start(dbip(files, 200));
            let (_, done) = fetch_month(&server, &folder, &AtomicBool::new(false));
            assert_eq!(done, Err(GeoError::Invalid(why.into())), "{case}");
            assert!(dir.names("geo-update").is_empty(), "{case}");
        }

        // A gzip of something that is no database at all.
        let dir = TempDir::new("type");
        let folder = dir.path().join("geo-update");
        let server = Server::start(dbip(month_files(gzip(b"hello"), asn_gz), 200));
        let (_, done) = fetch_month(&server, &folder, &AtomicBool::new(false));
        let e = done.unwrap_err();
        assert!(
            matches!(&e, GeoError::Invalid(why)
                if why.starts_with("dbip-city-lite-2026-11.mmdb.part: ")),
            "{e}"
        );
        assert!(dir.names("geo-update").is_empty());
    }

    #[test]
    fn cancelling_deletes_the_parts() {
        let big = gzip(&noise(1 << 20));
        for (case, files) in [
            (
                "during the City file",
                month_files(big.clone(), gzip(&asn_db())),
            ),
            (
                "during the ASN file",
                month_files(gzip(&city_db()), big.clone()),
            ),
        ] {
            let dir = TempDir::new("cancel");
            let folder = dir.path().join("geo-update");
            let server = Server::start(dbip(files, 200));
            let cancel = AtomicBool::new(false);
            let heard = Mutex::new(Vec::new());
            // The user cancels once a quarter of a megabyte is in.
            let progress = |received: u64, total: Option<u64>| {
                heard.lock().unwrap().push(received);
                if received >= PROGRESS_STEP {
                    assert!(received < total.unwrap(), "{case}");
                    cancel.store(true, Ordering::SeqCst);
                }
            };
            let done = server
                .updater()
                .download("2026-11", &folder, &progress, &cancel);
            assert_eq!(done, Err(GeoError::Cancelled), "{case}");
            assert!(dir.names("geo-update").is_empty(), "{case}");
            assert_eq!(heard.into_inner().unwrap().len(), 2, "{case}");
        }

        // Cancelled before it starts: not even a connection.
        let dir = TempDir::new("cancel");
        let server = Server::start(dbip(Vec::new(), 200));
        let folder = dir.path().join("geo-update");
        let (heard, done) = fetch_month(&server, &folder, &AtomicBool::new(true));
        assert_eq!(done, Err(GeoError::Cancelled));
        assert!(heard.is_empty());
        assert_eq!(server.connections(), 0);
    }

    #[test]
    fn only_months_are_downloaded() {
        let dir = TempDir::new("month");
        let server = Server::start(dbip(Vec::new(), 200));
        let nothing = |_: u64, _: Option<u64>| {};
        let done =
            server
                .updater()
                .download("../2026-11", dir.path(), &nothing, &AtomicBool::new(false));
        assert_eq!(
            done,
            Err(GeoError::Invalid("not a month: \"../2026-11\"".into()))
        );
        assert_eq!(server.connections(), 0);
    }
}
