//! Sprint 5.7 — local stats persistence.
//!
//! # Design constraints
//!
//! 1. **Cross-platform paths**: never assume `C:\...` or `/home/...`.
//!    The `directories` crate finds the OS-native config dir:
//!    - Windows: `%APPDATA%\nexus-rar\nexus-compress\stats.db`
//!    - macOS:   `~/Library/Application Support/com.nexus-rar.nexus-compress/stats.db`
//!    - Linux:   `$XDG_CONFIG_HOME/nexus-rar/nexus-compress/stats.db`
//!               (or `~/.config/...`)
//!
//! 2. **Zero system C dependencies**: rusqlite is built with the
//!    `bundled` feature so the SQLite C source is compiled into
//!    our final binary. No need for `libsqlite3-dev`, MSVC build
//!    tools, or any host C compiler. The .exe, .dmg, and .deb
//!    are fully self-contained.
//!
//! # Schema
//!
//! We use a single-row table for the global counters (cheaper
//! than aggregating over a history table for the home dashboard)
//! and a separate `events` table for the per-operation log.
//! Both are SQLite-native and migration-safe: the `init_schema`
//! call uses `CREATE TABLE IF NOT EXISTS` so we can evolve
//! without resetting user data.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Identifier for our `ProjectDirs` lookup. Must be unique per
/// app to avoid colliding with any other Rust app on the system.
const ORG: &str = "nexus-rar";
const APP: &str = "nexus-compress";

/// Schema version, written into the DB on init. We can branch on
/// this in the future if we need to run migrations.
const SCHEMA_VERSION: i32 = 1;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("could not resolve the user data directory on this platform")]
    NoDataDir,
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),
}

pub type DbResult<T> = Result<T, DbError>;

// ─────────────────────────────────────────────────────────────
//  Public data model
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStats {
    /// Total number of files the user has successfully compressed
    /// or extracted since installation.
    pub total_files_processed: u64,
    /// Sum of `(original_size - output_size)` for every compress
    /// op (negative values are clamped to 0). Surfaces in the
    /// home as "Espacio ahorrado".
    pub total_bytes_saved: u64,
    /// Number of completed P2P transfers (sender + receiver
    /// each count once per file).
    pub total_files_shared: u64,
    /// Compression ratio averaged across all compress ops,
    /// weighted by the original size of each file. 0.0 means
    /// "no compressions yet" — the home displays "—".
    pub average_compression_ratio: f32,
    /// Unix epoch seconds of the most recent op of any kind.
    /// 0 means "never". The home renders "Hace 2 min" etc.
    pub last_activity_timestamp: i64,
}

impl Default for AppStats {
    fn default() -> Self {
        Self {
            total_files_processed: 0,
            total_bytes_saved: 0,
            total_files_shared: 0,
            average_compression_ratio: 0.0,
            last_activity_timestamp: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEvent {
    pub id: i64,
    /// "compress" | "decompress" | "share" | "p2p_recv".
    pub kind: String,
    /// Filename as the user saw it in the UI.
    pub filename: String,
    /// Original size in bytes. 0 for share events where we
    /// don't track the source size separately.
    pub original_bytes: u64,
    /// Compressed / extracted / received size in bytes.
    pub output_bytes: u64,
    /// Unix epoch seconds.
    pub timestamp: i64,
}

// ─────────────────────────────────────────────────────────────
//  Connection lifecycle
// ─────────────────────────────────────────────────────────────

/// Open (or create) the database file at the canonical
/// per-user, per-platform data directory and run the schema
/// migration if needed. The returned `Connection` is `Send +
/// 'static` — wrap it in a `Mutex` (or `parking_lot::Mutex` if
/// you want less overhead) when you store it in Tauri state.
pub fn open() -> DbResult<Connection> {
    let path = db_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(&path)?;
    // Pragmas tuned for a desktop app: WAL gives concurrent
    // reads while a write is in flight; `synchronous=NORMAL`
    // is the recommended balance between safety and latency.
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;",
    )?;
    init_schema(&conn)?;
    Ok(conn)
}

fn db_path() -> DbResult<PathBuf> {
    let dirs = ProjectDirs::from(ORG, ORG, APP).ok_or(DbError::NoDataDir)?;
    Ok(dirs.data_dir().join("stats.db"))
}

/// Compute the directory we use, without opening a connection.
/// Useful for the `reveal in Finder / Explorer` actions and for
/// surfacing a "where is my data" entry in the settings panel.
pub fn data_dir() -> DbResult<PathBuf> {
    let dirs = ProjectDirs::from(ORG, ORG, APP).ok_or(DbError::NoDataDir)?;
    Ok(dirs.data_dir().to_path_buf())
}

fn init_schema(conn: &Connection) -> DbResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS stats (
             id                           INTEGER PRIMARY KEY CHECK (id = 1),
             total_files_processed        INTEGER NOT NULL DEFAULT 0,
             total_bytes_saved             INTEGER NOT NULL DEFAULT 0,
             total_files_shared            INTEGER NOT NULL DEFAULT 0,
             total_compressed_bytes_input  INTEGER NOT NULL DEFAULT 0,
             total_compressed_bytes_output INTEGER NOT NULL DEFAULT 0,
             average_compression_ratio     REAL    NOT NULL DEFAULT 0,
             last_activity_timestamp       INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS events (
             id              INTEGER PRIMARY KEY AUTOINCREMENT,
             kind            TEXT    NOT NULL,
             filename        TEXT    NOT NULL,
             original_bytes  INTEGER NOT NULL DEFAULT 0,
             output_bytes    INTEGER NOT NULL DEFAULT 0,
             timestamp       INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS events_ts ON events(timestamp DESC);",
    )?;
    // Stamp the schema version the first time we see the DB.
    let existing: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if existing.is_none() {
        conn.execute(
            "INSERT INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION.to_string()],
        )?;
    }
    // Seed the singleton stats row on first run.
    let has_stats: Option<i64> = conn
        .query_row("SELECT id FROM stats WHERE id = 1", [], |r| r.get(0))
        .optional()?;
    if has_stats.is_none() {
        conn.execute("INSERT INTO stats(id) VALUES (1)", [])?;
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────
//  Reads
// ─────────────────────────────────────────────────────────────

pub fn get_stats(conn: &Connection) -> DbResult<AppStats> {
    let s = conn.query_row(
        "SELECT total_files_processed,
                total_bytes_saved,
                total_files_shared,
                average_compression_ratio,
                last_activity_timestamp
         FROM stats WHERE id = 1",
        [],
        |r| {
            Ok(AppStats {
                total_files_processed: r.get(0)?,
                total_bytes_saved: r.get(1)?,
                total_files_shared: r.get(2)?,
                average_compression_ratio: r.get(3)?,
                last_activity_timestamp: r.get(4)?,
            })
        },
    )?;
    Ok(s)
}

pub fn list_events(conn: &Connection, limit: u32) -> DbResult<Vec<ActivityEvent>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, filename, original_bytes, output_bytes, timestamp
         FROM events ORDER BY timestamp DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit as i64], |r| {
            Ok(ActivityEvent {
                id: r.get(0)?,
                kind: r.get(1)?,
                filename: r.get(2)?,
                original_bytes: r.get(3)?,
                output_bytes: r.get(4)?,
                timestamp: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ─────────────────────────────────────────────────────────────
//  Writes
// ─────────────────────────────────────────────────────────────

/// Record a successful compression. `input_bytes` and
/// `output_bytes` are the on-disk sizes; we use them to update
/// the running weighted average ratio.
pub fn record_compression(
    conn: &Connection,
    filename: &str,
    input_bytes: u64,
    output_bytes: u64,
) -> DbResult<AppStats> {
    let saved = input_bytes.saturating_sub(output_bytes);
    let now = now_unix();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE stats
         SET total_files_processed        = total_files_processed + 1,
             total_bytes_saved             = total_bytes_saved + ?2,
             total_compressed_bytes_input  = total_compressed_bytes_input + ?3,
             total_compressed_bytes_output = total_compressed_bytes_output + ?4,
             last_activity_timestamp       = ?5
         WHERE id = 1",
        params![1_i64, saved as i64, input_bytes as i64, output_bytes as i64, now as i64],
    )?;
    // Recompute the weighted average.
    let (sum_in, sum_out): (i64, i64) = tx.query_row(
        "SELECT total_compressed_bytes_input, total_compressed_bytes_output FROM stats WHERE id = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let ratio = if sum_in > 0 {
        (sum_out as f32) / (sum_in as f32)
    } else {
        0.0
    };
    tx.execute(
        "UPDATE stats SET average_compression_ratio = ?1 WHERE id = 1",
        params![ratio],
    )?;
    insert_event(&tx, "compress", filename, input_bytes, output_bytes, now)?;
    let stats = get_stats(&tx)?;
    tx.commit()?;
    Ok(stats)
}

/// Record a successful extraction.
pub fn record_decompression(
    conn: &Connection,
    filename: &str,
    input_bytes: u64,
    output_bytes: u64,
) -> DbResult<AppStats> {
    let now = now_unix();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE stats
         SET total_files_processed  = total_files_processed + 1,
             last_activity_timestamp = ?2
         WHERE id = 1",
        params![1_i64, now as i64],
    )?;
    insert_event(&tx, "decompress", filename, input_bytes, output_bytes, now)?;
    let stats = get_stats(&tx)?;
    tx.commit()?;
    Ok(stats)
}

/// Record a successful P2P transfer (sender-side).
pub fn record_share(
    conn: &Connection,
    filename: &str,
    bytes: u64,
) -> DbResult<AppStats> {
    let now = now_unix();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE stats
         SET total_files_shared      = total_files_shared + 1,
             total_files_processed   = total_files_processed + 1,
             last_activity_timestamp  = ?2
         WHERE id = 1",
        params![1_i64, now as i64],
    )?;
    insert_event(&tx, "share", filename, bytes, bytes, now)?;
    let stats = get_stats(&tx)?;
    tx.commit()?;
    Ok(stats)
}

fn insert_event(
    conn: &Connection,
    kind: &str,
    filename: &str,
    original_bytes: u64,
    output_bytes: u64,
    timestamp: i64,
) -> DbResult<()> {
    conn.execute(
        "INSERT INTO events(kind, filename, original_bytes, output_bytes, timestamp)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![kind, filename, original_bytes as i64, output_bytes as i64, timestamp as i64],
    )?;
    Ok(())
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        // Mirror init_schema's CREATE TABLE definitions so the
        // production UPDATE / INSERT statements find every column.
        // We skip the meta + schema_version + WAL pragmas (not
        // needed for in-memory unit tests).
        conn.execute_batch(
            "CREATE TABLE stats (
                 id                           INTEGER PRIMARY KEY CHECK (id = 1),
                 total_files_processed        INTEGER NOT NULL DEFAULT 0,
                 total_bytes_saved             INTEGER NOT NULL DEFAULT 0,
                 total_files_shared            INTEGER NOT NULL DEFAULT 0,
                 total_compressed_bytes_input  INTEGER NOT NULL DEFAULT 0,
                 total_compressed_bytes_output INTEGER NOT NULL DEFAULT 0,
                 average_compression_ratio     REAL    NOT NULL DEFAULT 0,
                 last_activity_timestamp       INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE events (
                 id              INTEGER PRIMARY KEY AUTOINCREMENT,
                 kind            TEXT    NOT NULL,
                 filename        TEXT    NOT NULL,
                 original_bytes  INTEGER NOT NULL DEFAULT 0,
                 output_bytes    INTEGER NOT NULL DEFAULT 0,
                 timestamp       INTEGER NOT NULL
             );
             INSERT INTO stats(id) VALUES (1);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn defaults_are_zero() {
        let conn = mem_db();
        let stats = get_stats(&conn).unwrap();
        assert_eq!(stats.total_files_processed, 0);
        assert_eq!(stats.total_bytes_saved, 0);
        assert_eq!(stats.average_compression_ratio, 0.0);
    }

    #[test]
    fn compression_updates_counters_and_ratio() {
        let conn = mem_db();
        // 1 GB → 600 MB (40% ratio).
        let s = record_compression(&conn, "a.bin", 1_000_000_000, 600_000_000).unwrap();
        assert_eq!(s.total_files_processed, 1);
        assert_eq!(s.total_bytes_saved, 400_000_000);
        assert!((s.average_compression_ratio - 0.6).abs() < 1e-3);

        // Second file: 500 MB → 100 MB.
        let s = record_compression(&conn, "b.bin", 500_000_000, 100_000_000).unwrap();
        assert_eq!(s.total_files_processed, 2);
        assert_eq!(s.total_bytes_saved, 400_000_000 + 400_000_000);
        // Weighted average: (600+100) / (1000+500) = 0.4666...
        assert!((s.average_compression_ratio - (700.0 / 1500.0)).abs() < 1e-3);
    }

    #[test]
    fn share_increments_shared_count() {
        let conn = mem_db();
        let s = record_share(&conn, "movie.mkv", 1_234_567).unwrap();
        assert_eq!(s.total_files_shared, 1);
        assert_eq!(s.total_files_processed, 1);
        assert!(s.last_activity_timestamp > 0);
    }

    #[test]
    fn list_events_returns_newest_first() {
        let conn = mem_db();
        record_compression(&conn, "a", 100, 50).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(1));
        record_share(&conn, "b", 200).unwrap();
        let events = list_events(&conn, 10).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "share");
        assert_eq!(events[1].kind, "compress");
    }
}
