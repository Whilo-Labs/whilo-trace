//! Local history in SQLite: targets, rounds and one row per hop per round, plus what the window
//! keeps between runs: row order, pinned hops, settings and recent inputs.

pub mod buckets;
pub mod report;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;
use whilo_trace_engine::{now_ms, HopSample, ProbeKind, ReplyKind, RoundRecord};

pub use rusqlite::Error;
pub type Result<T> = rusqlite::Result<T>;

/// Written to `PRAGMA user_version` once the database has every table below.
pub const SCHEMA_VERSION: i64 = 1;

/// Seconds between rounds of a target nobody chose an interval for.
pub const DEFAULT_INTERVAL_MS: i64 = 2500;

/// Recent inputs kept for the add field.
pub const RECENT_MAX: usize = 10;

/// Version 0, the prototype's tables, kept as they were. A new database starts here too and goes
/// through the same migrations as an old one.
const SCHEMA_V0: &str = "
CREATE TABLE IF NOT EXISTS targets (
    id         INTEGER PRIMARY KEY,
    host       TEXT    NOT NULL UNIQUE,
    active     INTEGER NOT NULL DEFAULT 1,
    created_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS rounds (
    id        INTEGER PRIMARY KEY,
    target_id INTEGER NOT NULL REFERENCES targets(id),
    ts_ms     INTEGER NOT NULL,
    dst       TEXT    NOT NULL,
    path_len  INTEGER NOT NULL,
    reached   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS rounds_by_target_time ON rounds(target_id, ts_ms);
CREATE TABLE IF NOT EXISTS hops (
    round_id INTEGER NOT NULL REFERENCES rounds(id),
    ttl      INTEGER NOT NULL,
    kind     INTEGER NOT NULL,
    addr     TEXT,
    rtt_us   INTEGER,
    PRIMARY KEY (round_id, ttl)
) WITHOUT ROWID;
";

/// Version 1: order, interval, pause and probe type per target, probe type per round, pinned hops
/// (one order for targets and pinned hops), settings and recent inputs. `ADD COLUMN` with a default
/// does not rewrite old rows, so this is quick on a big history.
const MIGRATE_V1: &str = "
ALTER TABLE targets ADD COLUMN position    INTEGER NOT NULL DEFAULT 0;
ALTER TABLE targets ADD COLUMN interval_ms INTEGER NOT NULL DEFAULT 2500;
ALTER TABLE targets ADD COLUMN paused      INTEGER NOT NULL DEFAULT 0;
ALTER TABLE targets ADD COLUMN probe       TEXT    NOT NULL DEFAULT 'icmp';
UPDATE targets SET position = id;
ALTER TABLE rounds ADD COLUMN probe TEXT NOT NULL DEFAULT 'icmp';
CREATE TABLE IF NOT EXISTS pinned (
    id         INTEGER PRIMARY KEY,
    target_id  INTEGER NOT NULL REFERENCES targets(id),
    ttl        INTEGER NOT NULL,
    probe      TEXT    NOT NULL DEFAULT 'icmp',
    addr       TEXT,
    position   INTEGER NOT NULL,
    created_ms INTEGER NOT NULL,
    UNIQUE (target_id, ttl, probe)
);
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS recent (
    host    TEXT PRIMARY KEY COLLATE NOCASE,
    used_ms INTEGER NOT NULL
);
";

#[derive(Debug, Clone, Serialize)]
pub struct TargetRow {
    pub id: i64,
    pub host: String,
    pub active: bool,
    pub created_ms: i64,
    /// Place on the main screen; targets and pinned hops share one order.
    pub position: i64,
    pub interval_ms: i64,
    pub paused: bool,
    pub probe: String,
}

/// A hop of a target shown as its own row on the main screen.
#[derive(Debug, Clone, Serialize)]
pub struct PinRow {
    pub id: i64,
    pub target_id: i64,
    pub ttl: u8,
    pub probe: String,
    /// Who answered at this TTL when it was pinned; a label while there is no data.
    pub addr: Option<String>,
    pub position: i64,
}

/// A row of the main screen, for `set_order`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKey {
    Target(i64),
    Pin(i64),
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens or creates the database and brings it to [`SCHEMA_VERSION`] without losing history.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        // The app and the CLI may write at the same time.
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))?;
        conn.execute_batch("PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;")?;
        migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn schema_version(&self) -> Result<i64> {
        self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))
    }

    /// Adds an ICMP target, or switches an old one back on. Returns its id.
    pub fn add_target(&self, host: &str) -> Result<i64> {
        self.add_target_with(host, ProbeKind::Icmp.as_str(), None)
    }

    /// Adds a target at the end of the main screen. A host seen before (any case) keeps its id and
    /// history: an active one is returned as it is, a removed one comes back at the end, running.
    pub fn add_target_with(
        &self,
        host: &str,
        probe: &str,
        interval_ms: Option<i64>,
    ) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        let found: Option<(i64, bool)> = tx
            .query_row(
                "SELECT id, active FROM targets WHERE host = ?1 COLLATE NOCASE ORDER BY id LIMIT 1",
                [host],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let id = match found {
            Some((id, true)) => id,
            Some((id, false)) => {
                let pos = next_position(&tx)?;
                tx.execute(
                    "UPDATE targets SET active = 1, paused = 0, position = ?2, probe = ?3,
                     interval_ms = COALESCE(?4, interval_ms) WHERE id = ?1",
                    params![id, pos, probe, interval_ms],
                )?;
                id
            }
            None => {
                let pos = next_position(&tx)?;
                tx.execute(
                    "INSERT INTO targets (host, active, created_ms, position, interval_ms, paused, probe)
                     VALUES (?1, 1, ?2, ?3, ?4, 0, ?5)",
                    params![
                        host,
                        now_ms(),
                        pos,
                        interval_ms.unwrap_or(DEFAULT_INTERVAL_MS),
                        probe
                    ],
                )?;
                tx.last_insert_rowid()
            }
        };
        tx.commit()?;
        Ok(id)
    }

    /// Stops a target without deleting its history.
    pub fn set_active(&self, id: i64, active: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE targets SET active = ?2 WHERE id = ?1",
            params![id, active],
        )?;
        Ok(())
    }

    /// Takes a target off the main screen with its pinned hops; its rounds stay.
    pub fn remove_target(&self, id: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM pinned WHERE target_id = ?1", [id])?;
        tx.execute("UPDATE targets SET active = 0 WHERE id = ?1", [id])?;
        tx.commit()
    }

    pub fn set_paused(&self, id: i64, paused: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE targets SET paused = ?2 WHERE id = ?1",
            params![id, paused],
        )?;
        Ok(())
    }

    pub fn set_interval(&self, id: i64, interval_ms: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE targets SET interval_ms = ?2 WHERE id = ?1",
            params![id, interval_ms],
        )?;
        Ok(())
    }

    /// Every target, active or not, in screen order.
    pub fn targets(&self) -> Result<Vec<TargetRow>> {
        let mut st = self.conn.prepare(
            "SELECT id, host, active, created_ms, position, interval_ms, paused, probe
             FROM targets ORDER BY position, id",
        )?;
        let rows = st.query_map([], |r| {
            Ok(TargetRow {
                id: r.get(0)?,
                host: r.get(1)?,
                active: r.get(2)?,
                created_ms: r.get(3)?,
                position: r.get(4)?,
                interval_ms: r.get(5)?,
                paused: r.get(6)?,
                probe: r.get(7)?,
            })
        })?;
        rows.collect()
    }

    pub fn find_target(&self, host: &str) -> Result<Option<TargetRow>> {
        Ok(self
            .targets()?
            .into_iter()
            .find(|t| t.host.eq_ignore_ascii_case(host)))
    }

    /// Pins a hop at the end of the main screen; the same hop pinned twice keeps its id.
    pub fn pin_hop(&self, target_id: i64, ttl: u8, probe: &str, addr: Option<&str>) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        let found: Option<i64> = tx
            .query_row(
                "SELECT id FROM pinned WHERE target_id = ?1 AND ttl = ?2 AND probe = ?3",
                params![target_id, ttl, probe],
                |r| r.get(0),
            )
            .optional()?;
        let id = match found {
            Some(id) => id,
            None => {
                let pos = next_position(&tx)?;
                tx.execute(
                    "INSERT INTO pinned (target_id, ttl, probe, addr, position, created_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![target_id, ttl, probe, addr, pos, now_ms()],
                )?;
                tx.last_insert_rowid()
            }
        };
        tx.commit()?;
        Ok(id)
    }

    /// Returns false when there was no such pinned hop.
    pub fn unpin_hop(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM pinned WHERE id = ?1", [id])?
            > 0)
    }

    /// Pinned hops of active targets, in screen order.
    pub fn pins(&self) -> Result<Vec<PinRow>> {
        let mut st = self.conn.prepare(
            "SELECT p.id, p.target_id, p.ttl, p.probe, p.addr, p.position
             FROM pinned p JOIN targets t ON t.id = p.target_id
             WHERE t.active = 1 ORDER BY p.position, p.id",
        )?;
        let rows = st.query_map([], |r| {
            Ok(PinRow {
                id: r.get(0)?,
                target_id: r.get(1)?,
                ttl: r.get(2)?,
                probe: r.get(3)?,
                addr: r.get(4)?,
                position: r.get(5)?,
            })
        })?;
        rows.collect()
    }

    /// Puts the listed rows first, in this order; rows not listed follow in their old order.
    pub fn set_order(&self, order: &[RowKey]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let mut all: Vec<(i64, RowKey)> = Vec::new();
        {
            let mut st = tx.prepare(
                "SELECT position, id FROM targets WHERE active = 1 ORDER BY position, id",
            )?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, RowKey::Target(r.get(1)?))))?;
            for row in rows {
                all.push(row?);
            }
            let mut st = tx.prepare("SELECT position, id FROM pinned ORDER BY position, id")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, RowKey::Pin(r.get(1)?))))?;
            for row in rows {
                all.push(row?);
            }
        }
        all.sort_by_key(|(pos, _)| *pos);
        let known: Vec<RowKey> = all.iter().map(|(_, k)| *k).collect();
        let mut final_order: Vec<RowKey> = Vec::new();
        for key in order {
            if known.contains(key) && !final_order.contains(key) {
                final_order.push(*key);
            }
        }
        for key in known {
            if !final_order.contains(&key) {
                final_order.push(key);
            }
        }
        for (pos, key) in final_order.iter().enumerate() {
            match key {
                RowKey::Target(id) => tx.execute(
                    "UPDATE targets SET position = ?2 WHERE id = ?1",
                    params![id, pos as i64],
                )?,
                RowKey::Pin(id) => tx.execute(
                    "UPDATE pinned SET position = ?2 WHERE id = ?1",
                    params![id, pos as i64],
                )?,
            };
        }
        tx.commit()
    }

    /// Saved settings as key and JSON text; the caller fills defaults for missing keys.
    pub fn settings(&self) -> Result<Vec<(String, String)>> {
        let mut st = self
            .conn
            .prepare("SELECT key, value FROM settings ORDER BY key")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Remembers a host typed into the add field; keeps the last [`RECENT_MAX`].
    pub fn add_recent(&self, host: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        // Strictly increasing, so two inputs in the same millisecond keep their order.
        tx.execute(
            "INSERT INTO recent (host, used_ms)
             VALUES (?1, MAX(?2, (SELECT COALESCE(MAX(used_ms), 0) + 1 FROM recent)))
             ON CONFLICT(host) DO UPDATE SET host = excluded.host, used_ms = excluded.used_ms",
            params![host, now_ms()],
        )?;
        tx.execute(
            "DELETE FROM recent WHERE host NOT IN
             (SELECT host FROM recent ORDER BY used_ms DESC LIMIT ?1)",
            [RECENT_MAX as i64],
        )?;
        tx.commit()
    }

    /// Recent inputs, newest first.
    pub fn recent(&self) -> Result<Vec<String>> {
        let mut st = self
            .conn
            .prepare("SELECT host FROM recent ORDER BY used_ms DESC LIMIT ?1")?;
        let rows = st.query_map([RECENT_MAX as i64], |r| r.get(0))?;
        rows.collect()
    }

    pub fn insert_round(&mut self, target_id: i64, round: &RoundRecord) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO rounds (target_id, ts_ms, dst, path_len, reached, probe)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                target_id,
                round.ts_ms,
                round.dst.to_string(),
                round.path_len,
                round.reached,
                round.probe.as_str()
            ],
        )?;
        let round_id = tx.last_insert_rowid();
        {
            let mut st = tx.prepare_cached(
                "INSERT INTO hops (round_id, ttl, kind, addr, rtt_us) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for h in &round.hops {
                st.execute(params![
                    round_id,
                    h.ttl,
                    h.kind.code(),
                    h.addr.map(|a| a.to_string()),
                    h.rtt_us
                ])?;
            }
        }
        tx.commit()
    }

    /// Rounds of one target with `from_ms <= ts_ms < to_ms`, oldest first.
    pub fn load_rounds(
        &self,
        target_id: i64,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<RoundRecord>> {
        let mut st = self.conn.prepare_cached(
            "SELECT r.id, r.ts_ms, r.dst, r.path_len, r.reached, r.probe, h.ttl, h.kind, h.addr, h.rtt_us
             FROM rounds r LEFT JOIN hops h ON h.round_id = r.id
             WHERE r.target_id = ?1 AND r.ts_ms >= ?2 AND r.ts_ms < ?3
             ORDER BY r.ts_ms, r.id, h.ttl",
        )?;
        let mut rows = st.query(params![target_id, from_ms, to_ms])?;
        let mut out: Vec<RoundRecord> = Vec::new();
        let mut current: Option<i64> = None;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            if current != Some(id) {
                current = Some(id);
                let dst: String = row.get(2)?;
                let probe: String = row.get(5)?;
                out.push(RoundRecord {
                    ts_ms: row.get(1)?,
                    probe: ProbeKind::parse(&probe).unwrap_or_default(),
                    dst: dst.parse().unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
                    path_len: row.get(3)?,
                    reached: row.get(4)?,
                    hops: Vec::new(),
                });
            }
            if let Some(ttl) = row.get::<_, Option<u8>>(6)? {
                let addr: Option<String> = row.get(8)?;
                out.last_mut().unwrap().hops.push(HopSample {
                    ttl,
                    kind: ReplyKind::from_code(row.get(7)?),
                    addr: addr.and_then(|a| a.parse().ok()),
                    rtt_us: row.get(9)?,
                });
            }
        }
        Ok(out)
    }

    /// Start times of the rounds that did not reach the destination, oldest first.
    pub fn lost_times(&self, target_id: i64, from_ms: i64, to_ms: i64) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare_cached(
            "SELECT ts_ms FROM rounds
             WHERE target_id = ?1 AND ts_ms >= ?2 AND ts_ms < ?3 AND reached = 0
             ORDER BY ts_ms",
        )?;
        let rows = st.query_map(params![target_id, from_ms, to_ms], |r| r.get(0))?;
        rows.collect()
    }
}

/// Next free place at the end of the main screen.
fn next_position(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(p), -1) + 1 FROM (
             SELECT MAX(position) AS p FROM targets WHERE active = 1
             UNION ALL SELECT MAX(position) FROM pinned)",
        [],
        |r| r.get(0),
    )
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version >= SCHEMA_VERSION {
        return Ok(());
    }
    // IMMEDIATE takes the write lock first: a second process opening the same old file waits here
    // and then finds the work done.
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let steps = (|| {
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(SCHEMA_V0)?;
            conn.execute_batch(MIGRATE_V1)?;
        }
        conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))
    })();
    match steps {
        Ok(()) => conn.execute_batch("COMMIT"),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Per-user data folder shared by the GUI and the CLI.
pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(dir).join("Whilo Trace");
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join("Library/Application Support/Whilo Trace");
    }
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(dir).join("whilo-trace");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local/share/whilo-trace");
    }
    PathBuf::from(".")
}

pub fn default_db_path() -> PathBuf {
    data_dir().join("history.db")
}

#[cfg(test)]
mod tests;
