//! A small log of what went wrong, for a bug report: `error.log` next to the history, with the
//! time in UTC. Panics land here too, since a window app has no console. It holds about a
//! megabyte: past that the old part moves to `error.1.log`, and the one before goes.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const MAX_BYTES: u64 = 1024 * 1024;

static FILE: OnceLock<Mutex<PathBuf>> = OnceLock::new();

/// Logs into `dir/error.log` from now on, panics included.
pub fn init(dir: PathBuf) {
    let _ = fs::create_dir_all(&dir);
    if FILE.set(Mutex::new(dir.join("error.log"))).is_err() {
        return;
    }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log(&format!("panic: {info}"));
        default(info);
    }));
}

/// Writes one line, also to stderr; nothing happens before `init`.
pub fn log(msg: &str) {
    eprintln!("{msg}");
    let Some(file) = FILE.get() else {
        return;
    };
    let path = file.lock().unwrap_or_else(|e| e.into_inner());
    if fs::metadata(&*path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = fs::rename(&*path, path.with_file_name("error.1.log"));
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&*path) {
        let at = whilo_trace_store::report::format_time(whilo_trace_engine::now_ms(), 0);
        let _ = writeln!(f, "{at} UTC  {}", msg.replace('\n', " "));
    }
}
