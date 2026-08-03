// Embedded SQLite library database.
//
// This is the source of truth for the music library, play statistics, favorites,
// playlists, smart playlists and recents. It replaces the previous model where
// the whole library lived as a reactive JS array persisted as one JSON blob in
// IndexedDB (which had to deep-clone the entire library on every save and scanned
// O(n) in the webview on every keystroke / grouping).
//
// The webview now drives everything through the query commands below:
//   * full-text search over title/artist/album via FTS5 (diacritic-insensitive),
//   * album/artist grouping via GROUP BY,
//   * smart-playlist evaluation as a native rule pass over the DB,
//   * incremental writes (no whole-library clone).
//
// Writes use one connection guarded by a parking_lot Mutex. Read-heavy commands
// use a small read-only connection pool over WAL so they can run concurrently
// without blocking the writer. The handles are opened once and managed as Tauri
// state.

use std::collections::{HashMap, HashSet};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use parking_lot::{MappedMutexGuard, Mutex, MutexGuard};
use rayon::prelude::*;
use rusqlite::{params, Connection, OpenFlags, Row, Transaction};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};

use crate::{limits, security, MusicTrack};

// Memoized smart-playlist counts, guarded alongside the connection. Keyed by
// playlist id → (library fingerprint, rules JSON, count). The fingerprint is a
// cheap signature of the tracks/stats/favorites tables (see library_fingerprint);
// when it and the rules both match, db_playlists reuses the count instead of
// re-scanning the whole library with smart_eval on every refresh.
#[derive(Default)]
pub struct DbCache {
    smart_counts: Mutex<HashMap<String, (i64, String, i64)>>,
    station_sessions: Mutex<HashMap<String, StationSession>>,
}

pub(super) struct StationSession {
    pub(super) ids: Vec<i64>,
    pub(super) cursor: usize,
}

// The connection normally contains `Some`.  Restore briefly takes ownership
// while holding the mutex so the on-disk file can be replaced on Windows.  An
// explicit empty slot avoids ever installing an unrelated in-memory database;
// no other command can observe the temporary state because the mutex remains
// locked for the whole swap.
pub struct ConnectionSlot(Option<Connection>);

impl ConnectionSlot {
    fn new(connection: Connection) -> Self {
        Self(Some(connection))
    }

    pub(crate) fn take(&mut self) -> Result<Connection, String> {
        self.0
            .take()
            .ok_or_else(|| "Database connection is unavailable".to_string())
    }

    pub(crate) fn install(&mut self, connection: Connection) {
        self.0 = Some(connection);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_none()
    }
}

impl Deref for ConnectionSlot {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        self.0
            .as_ref()
            .expect("database connection slot is only empty while exclusively locked")
    }
}

impl DerefMut for ConnectionSlot {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
            .as_mut()
            .expect("database connection slot is only empty while exclusively locked")
    }
}

pub struct ReadPool {
    path: Option<PathBuf>,
    connections: Vec<Mutex<Option<Connection>>>,
    next: AtomicUsize,
    suspended: AtomicBool,
}

impl ReadPool {
    fn new(path: PathBuf, size: usize) -> Result<Self, String> {
        let connections = (0..size)
            .map(|_| open_read_connection(&path).map(|connection| Mutex::new(Some(connection))))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            path: Some(path),
            connections,
            next: AtomicUsize::new(0),
            suspended: AtomicBool::new(false),
        })
    }

    #[cfg(test)]
    fn empty() -> Self {
        Self {
            path: None,
            connections: Vec::new(),
            next: AtomicUsize::new(0),
            suspended: AtomicBool::new(false),
        }
    }

    pub(crate) fn refresh(&self) -> Result<(), String> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        for connection in &self.connections {
            *connection.lock() = Some(open_read_connection(path)?);
        }
        self.suspended.store(false, Ordering::Release);
        Ok(())
    }

    pub(crate) fn pause(&self) -> ReadPoolPause<'_> {
        self.suspended.store(true, Ordering::Release);
        for connection in &self.connections {
            connection.lock().take();
        }
        ReadPoolPause {
            pool: self,
            active: true,
        }
    }
}

pub enum DbReadGuard<'a> {
    Reader(MappedMutexGuard<'a, Connection>),
    Writer(MutexGuard<'a, ConnectionSlot>),
}

impl Deref for DbReadGuard<'_> {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Reader(connection) => connection,
            Self::Writer(connection) => connection,
        }
    }
}

pub struct Db(pub Mutex<ConnectionSlot>, pub DbCache, pub(crate) ReadPool);

pub(crate) struct ReadPoolPause<'a> {
    pool: &'a ReadPool,
    active: bool,
}

impl ReadPoolPause<'_> {
    pub(crate) fn finish(mut self) -> Result<(), String> {
        self.pool.refresh()?;
        self.active = false;
        Ok(())
    }
}

impl Drop for ReadPoolPause<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self.pool.refresh();
        }
    }
}

impl Db {
    pub(crate) fn read(&self) -> DbReadGuard<'_> {
        if self.2.connections.is_empty() || self.2.suspended.load(Ordering::Acquire) {
            return DbReadGuard::Writer(self.0.lock());
        }
        let index = self.2.next.fetch_add(1, Ordering::Relaxed) % self.2.connections.len();
        let connection = self.2.connections[index].lock();
        if connection.is_none() {
            drop(connection);
            return DbReadGuard::Writer(self.0.lock());
        }
        DbReadGuard::Reader(MutexGuard::map(connection, |slot| {
            slot.as_mut().expect("reader slot checked above")
        }))
    }
}

#[cfg(test)]
impl Db {
    pub(crate) fn from_connection(connection: Connection) -> Self {
        Self(
            Mutex::new(ConnectionSlot::new(connection)),
            DbCache::default(),
            ReadPool::empty(),
        )
    }
}

pub(crate) const APPLICATION_ID: i32 = 0x5453_4D31; // "TSM1"
pub(crate) const SCHEMA_VERSION: i32 = 5;

fn configure_connection(connection: &Connection) -> Result<(), String> {
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
    connection.set_prepared_statement_cache_capacity(128);
    Ok(())
}

fn open_read_connection(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| error.to_string())?;
    configure_connection(&connection)?;
    connection
        .pragma_update(None, "query_only", true)
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

// Cheap signature that changes whenever the tracks / stats / favorites that a
// smart playlist can match change (track added/removed, played, skipped, or
// (un)favorited). Weighted so distinct states rarely collide.
fn library_fingerprint(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT (SELECT COUNT(*) FROM tracks)
              + (SELECT COALESCE(MAX(last_played), 0) FROM stats)
              + (SELECT COALESCE(SUM(play_count), 0) FROM stats) * 7
              + (SELECT COALESCE(SUM(skip_count), 0) FROM stats) * 13
              + (SELECT COUNT(*) FROM favorites) * 1000003",
        [],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

// Column list shared by every "give me tracks" query, in the order row_to_track
// expects. `_T` is the alias-qualified variant for queries that JOIN `stats`
// (which also has a `path` column, so the bare name would be ambiguous).
const TRACK_COLS: &str = "path, title, artist, album, genre, duration_secs, first_seen_at, year, track_number, has_cover, sample_rate, bit_depth, track_gain_db, track_peak, file_size, mtime_ns, file_id";
const TRACK_COLS_T: &str = "t.path, t.title, t.artist, t.album, t.genre, t.duration_secs, t.first_seen_at, t.year, t.track_number, t.has_cover, t.sample_rate, t.bit_depth, t.track_gain_db, t.track_peak, t.file_size, t.mtime_ns, t.file_id";

fn row_to_track(row: &Row) -> rusqlite::Result<MusicTrack> {
    row_to_track_at(row, 0)
}

fn row_to_track_at(row: &Row, offset: usize) -> rusqlite::Result<MusicTrack> {
    Ok(MusicTrack {
        path: row.get(offset)?,
        title: row.get(offset + 1)?,
        artist: row.get(offset + 2)?,
        album: row.get(offset + 3)?,
        genre: row.get(offset + 4)?,
        duration_secs: row.get::<_, i64>(offset + 5)? as u64,
        date_added: row.get::<_, i64>(offset + 6)? as u64,
        year: row.get::<_, Option<i64>>(offset + 7)?.map(|v| v as u32),
        track_number: row.get::<_, Option<i64>>(offset + 8)?.map(|v| v as u32),
        has_cover: row.get::<_, i64>(offset + 9)? != 0,
        sample_rate: row.get::<_, Option<i64>>(offset + 10)?.map(|v| v as u32),
        bit_depth: row.get::<_, Option<i64>>(offset + 11)?.map(|v| v as u8),
        track_gain_db: row.get::<_, Option<f64>>(offset + 12)?.map(|v| v as f32),
        track_peak: row.get::<_, Option<f64>>(offset + 13)?.map(|v| v as f32),
        file_size: row.get::<_, Option<i64>>(offset + 14)?.unwrap_or(0).max(0) as u64,
        mtime_ns: row.get::<_, Option<i64>>(offset + 15)?.unwrap_or(0),
        file_id: row.get(offset + 16)?,
    })
}

// ---- Schema / init ----------------------------------------------------------

pub fn init(app: &AppHandle) -> Result<Db, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("no app data dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("library.db");
    let mut conn = Connection::open(&path).map_err(|e| e.to_string())?;
    configure_connection(&conn)?;
    conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    migrate(&mut conn)?;
    let read_pool = ReadPool::new(path, 3)?;
    Ok(Db(
        Mutex::new(ConnectionSlot::new(conn)),
        DbCache::default(),
        read_pool,
    ))
}

// Additive schema migrations for databases created before a column existed.
// (CREATE TABLE IF NOT EXISTS never alters an existing table.)
fn strip_windows_verbatim_path(path: &str) -> Option<String> {
    if let Some(relative) = path.strip_prefix(r"\\?\UNC\") {
        Some(format!(r"\\{relative}"))
    } else {
        path.strip_prefix(r"\\?\").map(str::to_string)
    }
}

fn normalize_json_paths(value: &mut Value) -> bool {
    match value {
        Value::String(path) => {
            let Some(normalized) = strip_windows_verbatim_path(path) else {
                return false;
            };
            *path = normalized;
            true
        }
        Value::Array(values) => {
            let mut changed = false;
            for value in values {
                changed |= normalize_json_paths(value);
            }
            changed
        }
        Value::Object(values) => {
            let mut changed = false;
            for value in values.values_mut() {
                changed |= normalize_json_paths(value);
            }
            changed
        }
        _ => false,
    }
}

/// Schema v2 repairs the Windows path identity regression where canonicalized
/// paths (`\\?\D:\...`) and legacy paths (`D:\...`) represented the same file
/// as two SQLite rows. Related state is merged before the duplicate track is
/// removed so plays, skips, favorites, and playlist membership survive.
fn normalize_verbatim_paths(tx: &Transaction<'_>) -> Result<(), String> {
    let paths = {
        let mut statement = tx
            .prepare(
                "SELECT path FROM tracks
                 UNION SELECT path FROM stats
                 UNION SELECT path FROM favorites
                 UNION SELECT path FROM playlist_items
                 UNION SELECT path FROM roots
                 UNION SELECT path FROM pending_roots",
            )
            .map_err(|error| error.to_string())?;
        let collected = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        collected
    };

    for source in paths {
        let Some(target) = strip_windows_verbatim_path(&source) else {
            continue;
        };

        tx.execute(
            "INSERT INTO stats(path, play_count, last_played, skip_count)
             SELECT ?2, play_count, last_played, skip_count FROM stats WHERE path = ?1
             ON CONFLICT(path) DO UPDATE SET
               play_count = stats.play_count + excluded.play_count,
               last_played = MAX(stats.last_played, excluded.last_played),
               skip_count = stats.skip_count + excluded.skip_count",
            params![source, target],
        )
        .map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM stats WHERE path = ?1", params![source])
            .map_err(|error| error.to_string())?;

        tx.execute(
            "INSERT INTO favorites(path, position)
             SELECT ?2, position FROM favorites WHERE path = ?1
             ON CONFLICT(path) DO UPDATE SET position = MIN(favorites.position, excluded.position)",
            params![source, target],
        )
        .map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM favorites WHERE path = ?1", params![source])
            .map_err(|error| error.to_string())?;

        tx.execute(
            "INSERT INTO playlist_items(playlist_id, path, position)
             SELECT playlist_id, ?2, position FROM playlist_items WHERE path = ?1
             ON CONFLICT(playlist_id, path) DO UPDATE SET
               position = MIN(playlist_items.position, excluded.position)",
            params![source, target],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            "DELETE FROM playlist_items WHERE path = ?1",
            params![source],
        )
        .map_err(|error| error.to_string())?;

        let target_exists = tx
            .prepare("SELECT 1 FROM tracks WHERE path = ?1")
            .and_then(|mut statement| statement.exists(params![target]))
            .map_err(|error| error.to_string())?;
        if target_exists {
            tx.execute(
                "UPDATE tracks SET
                   fingerprint = COALESCE(
                     fingerprint,
                     (SELECT fingerprint FROM tracks WHERE path = ?2)
                   ),
                   first_seen_at = MIN(
                     first_seen_at,
                     COALESCE((SELECT first_seen_at FROM tracks WHERE path = ?2), first_seen_at)
                   )
                 WHERE path = ?1",
                params![target, source],
            )
            .map_err(|error| error.to_string())?;
            tx.execute("DELETE FROM tracks WHERE path = ?1", params![source])
                .map_err(|error| error.to_string())?;
        } else {
            tx.execute(
                "UPDATE tracks SET path = ?2 WHERE path = ?1",
                params![source, target],
            )
            .map_err(|error| error.to_string())?;
        }

        for table in ["roots", "pending_roots"] {
            let insert = format!(
                "INSERT OR IGNORE INTO {table}(path) SELECT ?2 WHERE EXISTS(
                    SELECT 1 FROM {table} WHERE path = ?1
                 )"
            );
            let delete = format!("DELETE FROM {table} WHERE path = ?1");
            tx.execute(&insert, params![source, target])
                .map_err(|error| error.to_string())?;
            tx.execute(&delete, params![source])
                .map_err(|error| error.to_string())?;
        }
    }

    let kv_rows = {
        let mut statement = tx
            .prepare("SELECT k, v FROM kv")
            .map_err(|error| error.to_string())?;
        let collected = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        collected
    };
    for (key, encoded) in kv_rows {
        let Ok(mut value) = serde_json::from_str::<Value>(&encoded) else {
            continue;
        };
        if normalize_json_paths(&mut value) {
            tx.execute(
                "UPDATE kv SET v = ?2 WHERE k = ?1",
                params![key, value.to_string()],
            )
            .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn rebuild_search_index(tx: &Transaction<'_>) -> Result<(), String> {
    for (stage, sql) in [
        (
            "drop legacy triggers",
            "DROP TRIGGER IF EXISTS tracks_ai;
             DROP TRIGGER IF EXISTS tracks_ad;
             DROP TRIGGER IF EXISTS tracks_au;",
        ),
        (
            "create genre index",
            "CREATE VIRTUAL TABLE IF NOT EXISTS tracks_search USING fts5(
           title, artist, album, genre,
           content='tracks', content_rowid='id',
           tokenize=\"unicode61 remove_diacritics 2\"
         );",
        ),
        (
            "create sync triggers",
            "CREATE TRIGGER tracks_ai AFTER INSERT ON tracks BEGIN
           INSERT INTO tracks_search(rowid, title, artist, album, genre)
           VALUES (new.id, new.title, new.artist, new.album, new.genre);
         END;
         CREATE TRIGGER tracks_ad AFTER DELETE ON tracks BEGIN
           INSERT INTO tracks_search(tracks_search, rowid, title, artist, album, genre)
           VALUES ('delete', old.id, old.title, old.artist, old.album, old.genre);
         END;
         CREATE TRIGGER tracks_au AFTER UPDATE ON tracks BEGIN
           INSERT INTO tracks_search(tracks_search, rowid, title, artist, album, genre)
           VALUES ('delete', old.id, old.title, old.artist, old.album, old.genre);
           INSERT INTO tracks_search(rowid, title, artist, album, genre)
           VALUES (new.id, new.title, new.artist, new.album, new.genre);
         END;",
        ),
        (
            "populate index",
            "INSERT INTO tracks_search(tracks_search) VALUES ('rebuild');",
        ),
    ] {
        tx.execute_batch(sql).map_err(|error| {
            format!("Failed to rebuild library search index ({stage}): {error}")
        })?;
    }
    Ok(())
}

fn migrate_stable_track_ids(tx: &Transaction<'_>) -> Result<(), String> {
    tx.execute_batch(
        "CREATE TABLE stats_v4 (
           track_id    INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
           play_count  INTEGER NOT NULL DEFAULT 0,
           last_played INTEGER NOT NULL DEFAULT 0,
           skip_count  INTEGER NOT NULL DEFAULT 0
         );
         INSERT INTO stats_v4(track_id, play_count, last_played, skip_count)
         SELECT t.id, s.play_count, s.last_played, s.skip_count
         FROM stats s JOIN tracks t ON t.path = s.path;

         CREATE TABLE favorites_v4 (
           track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
           position INTEGER NOT NULL DEFAULT 0
         );
         INSERT INTO favorites_v4(track_id, position)
         SELECT t.id, f.position FROM favorites f JOIN tracks t ON t.path = f.path;

         CREATE TABLE playlist_items_v4 (
           id          INTEGER PRIMARY KEY,
           playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
           track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
           position    INTEGER NOT NULL,
           UNIQUE(playlist_id, track_id)
         );
         INSERT INTO playlist_items_v4(playlist_id, track_id, position)
         SELECT i.playlist_id, t.id, i.position
         FROM playlist_items i JOIN tracks t ON t.path = i.path;

         DROP TABLE playlist_items;
         DROP TABLE favorites;
         DROP TABLE stats;
         ALTER TABLE stats_v4 RENAME TO stats;
         ALTER TABLE favorites_v4 RENAME TO favorites;
         ALTER TABLE playlist_items_v4 RENAME TO playlist_items;
         CREATE INDEX idx_pl_items ON playlist_items(playlist_id, position);
         CREATE INDEX idx_stats_last_played ON stats(last_played);
         CREATE INDEX idx_favorites_position ON favorites(position);",
    )
    .map_err(|error| format!("Failed to migrate stable track identities: {error}"))
}

fn migrate(conn: &mut Connection) -> Result<(), String> {
    let application_id: i32 = conn
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(|e| format!("Failed to read database application id: {e}"))?;
    if application_id != 0 && application_id != APPLICATION_ID {
        return Err("Database belongs to a different application".to_string());
    }

    let version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| format!("Failed to read database schema version: {e}"))?;
    if version > SCHEMA_VERSION {
        return Err(format!(
            "Database schema version {version} is newer than supported version {SCHEMA_VERSION}"
        ));
    }

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let has_fingerprint: bool = tx
        .prepare("SELECT 1 FROM pragma_table_info('tracks') WHERE name = 'fingerprint'")
        .and_then(|mut s| s.exists([]))
        .map_err(|e| e.to_string())?;
    if !has_fingerprint {
        // Content fingerprint (size + sampled hash, see crate::compute_fingerprint)
        // used to re-identify a track after its file is moved/renamed so its
        // stats/favorites/playlist memberships survive. '' = hashing failed
        // (unreadable file) — tried, don't retry; NULL = not yet computed.
        tx.execute_batch(
            "ALTER TABLE tracks ADD COLUMN fingerprint TEXT;
             CREATE INDEX IF NOT EXISTS idx_tracks_fp ON tracks(fingerprint);",
        )
        .map_err(|e| e.to_string())?;
    } else {
        tx.execute_batch("CREATE INDEX IF NOT EXISTS idx_tracks_fp ON tracks(fingerprint);")
            .map_err(|e| e.to_string())?;
    }
    let has_file_size: bool = tx
        .prepare("SELECT 1 FROM pragma_table_info('tracks') WHERE name = 'file_size'")
        .and_then(|mut statement| statement.exists([]))
        .map_err(|error| error.to_string())?;
    if !has_file_size {
        tx.execute_batch("ALTER TABLE tracks ADD COLUMN file_size INTEGER;")
            .map_err(|error| error.to_string())?;
    }
    let has_mtime_ns: bool = tx
        .prepare("SELECT 1 FROM pragma_table_info('tracks') WHERE name = 'mtime_ns'")
        .and_then(|mut statement| statement.exists([]))
        .map_err(|error| error.to_string())?;
    if !has_mtime_ns {
        tx.execute_batch("ALTER TABLE tracks ADD COLUMN mtime_ns INTEGER;")
            .map_err(|error| error.to_string())?;
    }
    let has_file_id: bool = tx
        .prepare("SELECT 1 FROM pragma_table_info('tracks') WHERE name = 'file_id'")
        .and_then(|mut statement| statement.exists([]))
        .map_err(|error| error.to_string())?;
    if !has_file_id {
        tx.execute_batch(
            "ALTER TABLE tracks ADD COLUMN file_id TEXT;
             CREATE INDEX IF NOT EXISTS idx_tracks_file_id ON tracks(file_id);",
        )
        .map_err(|error| error.to_string())?;
    } else {
        tx.execute_batch("CREATE INDEX IF NOT EXISTS idx_tracks_file_id ON tracks(file_id);")
            .map_err(|error| error.to_string())?;
    }
    let has_first_seen_at: bool = tx
        .prepare("SELECT 1 FROM pragma_table_info('tracks') WHERE name = 'first_seen_at'")
        .and_then(|mut statement| statement.exists([]))
        .map_err(|error| error.to_string())?;
    if !has_first_seen_at {
        tx.execute_batch(
            "ALTER TABLE tracks ADD COLUMN first_seen_at INTEGER NOT NULL DEFAULT 0;
             UPDATE tracks SET first_seen_at = date_added WHERE first_seen_at = 0;",
        )
        .map_err(|error| error.to_string())?;
    }
    tx.execute_batch("CREATE INDEX IF NOT EXISTS idx_tracks_first_seen ON tracks(first_seen_at);")
        .map_err(|error| error.to_string())?;
    let relations_use_path: bool = tx
        .prepare("SELECT 1 FROM pragma_table_info('stats') WHERE name = 'path'")
        .and_then(|mut statement| statement.exists([]))
        .map_err(|error| error.to_string())?;
    if version < 2 && relations_use_path {
        normalize_verbatim_paths(&tx)?;
    }
    if version < 3 {
        rebuild_search_index(&tx)?;
    }
    if relations_use_path {
        migrate_stable_track_ids(&tx)?;
    }
    tx.pragma_update(None, "application_id", APPLICATION_ID)
        .map_err(|e| format!("Failed to set database application id: {e}"))?;
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|e| format!("Failed to set database schema version: {e}"))?;
    tx.commit().map_err(|e| e.to_string())?;
    if version < 3 {
        // Dropping an external-content FTS5 table inside the same transaction
        // that creates/rebuilds its replacement can fail at commit on SQLite.
        // The old v2 index is unreferenced once the new triggers are committed,
        // so reclaim it safely afterward.
        conn.execute_batch("DROP TABLE IF EXISTS tracks_fts;")
            .map_err(|error| format!("Failed to remove legacy search index: {error}"))?;
    }
    Ok(())
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS tracks (
  id            INTEGER PRIMARY KEY,
  path          TEXT NOT NULL UNIQUE,
  title         TEXT NOT NULL DEFAULT '',
  artist        TEXT NOT NULL DEFAULT '',
  album         TEXT NOT NULL DEFAULT '',
  genre         TEXT,
  duration_secs INTEGER NOT NULL DEFAULT 0,
  date_added    INTEGER NOT NULL DEFAULT 0,
  first_seen_at INTEGER NOT NULL DEFAULT 0,
  year          INTEGER,
  track_number  INTEGER,
  has_cover     INTEGER NOT NULL DEFAULT 0,
  sample_rate   INTEGER,
  bit_depth     INTEGER,
  track_gain_db REAL,
  track_peak    REAL,
  fingerprint   TEXT,
  file_size     INTEGER,
  mtime_ns      INTEGER,
  file_id       TEXT
);
CREATE INDEX IF NOT EXISTS idx_tracks_album  ON tracks(album);
CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
CREATE INDEX IF NOT EXISTS idx_tracks_added  ON tracks(date_added);
-- Default library sort is by title; index it so the ORDER BY is index-served.
CREATE INDEX IF NOT EXISTS idx_tracks_title  ON tracks(title COLLATE NOCASE);

-- Diacritic-insensitive full-text index over the searchable text columns, kept
-- in sync with `tracks` by triggers (external-content FTS5 keyed on tracks.id).
CREATE VIRTUAL TABLE IF NOT EXISTS tracks_search USING fts5(
  title, artist, album, genre,
  content='tracks', content_rowid='id',
  tokenize="unicode61 remove_diacritics 2"
);
CREATE TRIGGER IF NOT EXISTS tracks_ai AFTER INSERT ON tracks BEGIN
  INSERT INTO tracks_search(rowid, title, artist, album, genre)
  VALUES (new.id, new.title, new.artist, new.album, new.genre);
END;
CREATE TRIGGER IF NOT EXISTS tracks_ad AFTER DELETE ON tracks BEGIN
  INSERT INTO tracks_search(tracks_search, rowid, title, artist, album, genre)
  VALUES ('delete', old.id, old.title, old.artist, old.album, old.genre);
END;
CREATE TRIGGER IF NOT EXISTS tracks_au AFTER UPDATE ON tracks BEGIN
  INSERT INTO tracks_search(tracks_search, rowid, title, artist, album, genre)
  VALUES ('delete', old.id, old.title, old.artist, old.album, old.genre);
  INSERT INTO tracks_search(rowid, title, artist, album, genre)
  VALUES (new.id, new.title, new.artist, new.album, new.genre);
END;

CREATE TABLE IF NOT EXISTS stats (
  track_id    INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
  play_count  INTEGER NOT NULL DEFAULT 0,
  last_played INTEGER NOT NULL DEFAULT 0,
  skip_count  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS favorites (
  track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
  position INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS roots (path TEXT PRIMARY KEY);
-- Roots restored from a backup are data, not filesystem authority. They remain
-- pending until the user confirms a replacement through the native picker.
CREATE TABLE IF NOT EXISTS pending_roots (path TEXT PRIMARY KEY);

CREATE TABLE IF NOT EXISTS playlists (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL DEFAULT '',
  description TEXT NOT NULL DEFAULT '',
  color       TEXT,
  cover       TEXT,
  position    INTEGER NOT NULL DEFAULT 0,
  is_smart    INTEGER NOT NULL DEFAULT 0,
  rules       TEXT,
  sort_by     TEXT,
  sort_order  TEXT,
  limit_n     INTEGER,
  live_update INTEGER
);

CREATE TABLE IF NOT EXISTS playlist_items (
  id          INTEGER PRIMARY KEY,
  playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
  track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  position    INTEGER NOT NULL,
  UNIQUE (playlist_id, track_id)
);
CREATE INDEX IF NOT EXISTS idx_pl_items ON playlist_items(playlist_id, position);
CREATE INDEX IF NOT EXISTS idx_stats_last_played ON stats(last_played);
CREATE INDEX IF NOT EXISTS idx_favorites_position ON favorites(position);

CREATE TABLE IF NOT EXISTS recents (
  type TEXT NOT NULL,
  key  TEXT NOT NULL,
  ts   INTEGER NOT NULL,
  PRIMARY KEY (type, key)
);

CREATE TABLE IF NOT EXISTS kv (k TEXT PRIMARY KEY, v TEXT);
CREATE TABLE IF NOT EXISTS cover_art (
  album   TEXT NOT NULL,
  artist  TEXT NOT NULL,
  bytes   BLOB NOT NULL,
  PRIMARY KEY (album, artist)
);
"#;

// ---- Helpers ----------------------------------------------------------------

fn dir(order: &str) -> &'static str {
    if order.eq_ignore_ascii_case("desc") {
        "DESC"
    } else {
        "ASC"
    }
}

// Turn free-text into an FTS5 prefix query: each whitespace token becomes a
// quoted prefix term (implicit AND). Quoting neutralises FTS5 syntax chars.
fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"*", t.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}

fn collect_tracks(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Vec<MusicTrack>, String> {
    let mut stmt = conn.prepare_cached(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params, row_to_track)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

// ---- Result shapes ----------------------------------------------------------

#[derive(Serialize)]
pub struct Page {
    total: i64,
    tracks: Vec<MusicTrack>,
    next_cursor: Option<i64>,
}

#[derive(Serialize)]
pub struct AlbumRow {
    album: String,
    artist: String,
    year: Option<i64>,
    track_count: i64,
    cover_path: Option<String>,
    last_played: i64,
    all_artists: Option<String>,
}

#[derive(Serialize)]
pub struct ArtistRow {
    artist: String,
    track_count: i64,
    album_count: i64,
    plays: i64,
    cover_path: Option<String>,
    last_played: i64,
}

#[derive(Serialize)]
pub struct GenreRow {
    genre: String,
    track_count: i64,
    plays: i64,
    cover_path: Option<String>,
}

#[derive(Serialize, Default)]
pub struct StatRow {
    play_count: i64,
    last_played: i64,
    skip_count: i64,
}

#[derive(Serialize)]
pub struct StatsSummary {
    total_plays: i64,
    total_seconds: i64,
}

#[derive(Serialize)]
pub struct PlaylistRow {
    id: String,
    name: String,
    description: String,
    color: Option<String>,
    cover: Option<String>,
    is_smart: bool,
    rules: Option<Value>,
    sort_by: Option<String>,
    sort_order: Option<String>,
    limit_n: Option<i64>,
    live_update: bool,
    track_count: i64,
}

#[derive(Serialize)]
pub struct RecentRow {
    #[serde(rename = "type")]
    kind: String,
    key: String,
    ts: i64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn random_u64() -> u64 {
    let mut bytes = [0_u8; 8];
    if getrandom::fill(&mut bytes).is_ok() {
        u64::from_le_bytes(bytes)
    } else {
        // OS randomness is expected to succeed; this fallback only preserves
        // non-security-sensitive playback behavior on an unusual platform.
        now_ms() as u64 ^ (std::process::id() as u64).rotate_left(17)
    }
}

// ---- Library mutation -------------------------------------------------------

// Insert/update scanned tracks. Returns how many were newly inserted (existing
// rows are refreshed but not counted, matching the old "new tracks" status text).
pub fn db_upsert_tracks(db: State<Db>, tracks: Vec<MusicTrack>) -> Result<usize, String> {
    upsert_tracks(&db, tracks)
}

// Used by the native scanner so metadata can be indexed without crossing IPC.
// Memory usage is bounded by the caller's batch size.
pub(crate) fn upsert_tracks(db: &Db, tracks: Vec<MusicTrack>) -> Result<usize, String> {
    upsert_tracks_with_options(db, tracks)
}

fn upsert_tracks_with_options(db: &Db, tracks: Vec<MusicTrack>) -> Result<usize, String> {
    // The scanner only passes new or signature-changed files. Hashing reads
    // ~128 KiB per file, so it runs in parallel and outside the connection lock.
    let need_fp: Vec<String> = tracks.iter().map(|track| track.path.clone()).collect();
    let fps: HashMap<String, String> = need_fp
        .into_par_iter()
        .map(|p| {
            // '' records an attempted but unreadable fingerprint.
            let fp = crate::compute_fingerprint(Path::new(&p)).unwrap_or_default();
            (p, fp)
        })
        .collect();

    let mut conn = db.0.lock();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut new_count = 0usize;
    {
        let mut exists = tx
            .prepare("SELECT 1 FROM tracks WHERE path = ?1")
            .map_err(|e| e.to_string())?;
        let mut upsert = tx
            .prepare(
                "INSERT INTO tracks (path, title, artist, album, genre, duration_secs, date_added, first_seen_at, year, track_number, has_cover, sample_rate, bit_depth, track_gain_db, track_peak, fingerprint, file_size, mtime_ns, file_id)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
                 ON CONFLICT(path) DO UPDATE SET
                   title=excluded.title, artist=excluded.artist, album=excluded.album,
                   genre=excluded.genre, duration_secs=excluded.duration_secs,
                   year=excluded.year, track_number=excluded.track_number,
                   has_cover=excluded.has_cover,
                   sample_rate=excluded.sample_rate, bit_depth=excluded.bit_depth,
                   track_gain_db=excluded.track_gain_db, track_peak=excluded.track_peak,
                   fingerprint=excluded.fingerprint, file_size=excluded.file_size,
                   mtime_ns=excluded.mtime_ns, file_id=excluded.file_id",
            )
            .map_err(|e| e.to_string())?;
        for t in &tracks {
            let is_new = !exists.exists(params![t.path]).map_err(|e| e.to_string())?;
            upsert
                .execute(params![
                    t.path,
                    t.title,
                    t.artist,
                    t.album,
                    t.genre,
                    t.duration_secs as i64,
                    t.date_added as i64,
                    t.year.map(|v| v as i64),
                    t.track_number.map(|v| v as i64),
                    t.has_cover as i64,
                    t.sample_rate.map(|v| v as i64),
                    t.bit_depth.map(|v| v as i64),
                    t.track_gain_db.map(|v| v as f64),
                    t.track_peak.map(|v| v as f64),
                    fps.get(&t.path),
                    t.file_size.min(i64::MAX as u64) as i64,
                    t.mtime_ns,
                    t.file_id,
                ])
                .map_err(|e| e.to_string())?;
            if is_new {
                new_count += 1;
            }
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(new_count)
}

/// Select only candidates whose cheap filesystem signature differs from the
/// indexed row. NULL signatures are the one-time migration/backfill case.
pub(crate) fn paths_requiring_metadata(
    db: &Db,
    candidates: &[(std::path::PathBuf, i64, i64, Option<String>)],
) -> Result<Vec<std::path::PathBuf>, String> {
    let conn = db.read();
    let mut statement = conn
        .prepare("SELECT file_size, mtime_ns, file_id FROM tracks WHERE path = ?1")
        .map_err(|error| error.to_string())?;
    let mut changed = Vec::new();
    for (path, file_size, mtime_ns, file_id) in candidates {
        let indexed = statement.query_row(params![path.to_string_lossy()], |row| {
            Ok((
                row.get::<_, Option<i64>>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        });
        match indexed {
            Ok((Some(indexed_size), Some(indexed_mtime), indexed_file_id))
                if indexed_size == *file_size
                    && indexed_mtime == *mtime_ns
                    && indexed_file_id.as_ref() == file_id.as_ref() => {}
            Ok(_) | Err(rusqlite::Error::QueryReturnedNoRows) => changed.push(path.clone()),
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(changed)
}

pub(crate) fn remove_paths(db: &Db, paths: &[String]) -> Result<(), String> {
    limits::validate_paths(paths, limits::MAX_BATCH_PATHS)?;
    let mut conn = db.0.lock();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for p in paths {
        tx.execute("DELETE FROM tracks WHERE path = ?1", params![p])
            .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn db_remove_paths(
    app: AppHandle,
    db: State<Db>,
    consent: State<security::DestructiveConsentState>,
    paths: Vec<String>,
    consent_token: String,
) -> Result<(), String> {
    limits::validate_paths(&paths, limits::MAX_BATCH_PATHS)?;
    let canonical = paths
        .iter()
        .map(|path| {
            crate::resolve_allowed_audio(&app, Path::new(path))
                .map(|path| path.to_string_lossy().to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    for path in &canonical {
        if tracks::db_track(db.clone(), path.clone())?.is_none() {
            return Err("Track is not present in the library".to_string());
        }
    }
    for path in &canonical {
        consent.consume(
            &consent_token,
            security::ConsentAction::RemoveLibraryTracks,
            Some(path),
        )?;
    }
    remove_paths(db.inner(), &canonical)
}

// Remove every track whose file no longer exists on disk. Before deleting,
// try to re-identify each missing file among the surviving rows by content
// fingerprint — a moved/renamed file shows up as "old path gone + new path
// just scanned" — and migrate its play stats, favorite flag, playlist
// memberships and original date_added onto the new row instead of losing them.
// Returns the removed (old) paths so the frontend can drop them from the queue.
pub fn db_prune_missing(db: State<Db>) -> Result<Vec<String>, String> {
    let mut conn = db.0.lock();
    let gone: Vec<(String, Option<String>, Option<String>)> = {
        let mut stmt = conn
            .prepare("SELECT path, fingerprint, file_id FROM tracks")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.filter_map(Result::ok)
            .filter(|(path, _, _)| !Path::new(path).exists())
            .collect()
    };
    prune_gone_rows(&mut conn, gone)
}

fn prune_gone_rows(
    conn: &mut Connection,
    gone: Vec<(String, Option<String>, Option<String>)>,
) -> Result<Vec<String>, String> {
    if gone.is_empty() {
        return Ok(Vec::new());
    }

    let mut claimed_targets = HashSet::new();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (old_path, fp, file_id) in &gone {
        let by_file_id = if let Some(file_id) = file_id.as_ref().filter(|value| !value.is_empty()) {
            let mut statement = tx
                .prepare("SELECT id, path FROM tracks WHERE file_id = ?1 AND path <> ?2")
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map(params![file_id, old_path], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|error| error.to_string())?;
            let found = rows.filter_map(Result::ok).find(|(_, candidate)| {
                Path::new(candidate).exists() && !claimed_targets.contains(candidate)
            });
            found
        } else {
            None
        };
        let target = if by_file_id.is_some() {
            by_file_id
        } else if let Some(fingerprint) = fp.as_ref().filter(|value| !value.is_empty()) {
            let mut stmt = tx
                .prepare("SELECT id, path FROM tracks WHERE fingerprint = ?1 AND path <> ?2")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![fingerprint, old_path], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?;
            let found = rows.filter_map(Result::ok).find(|(_, candidate)| {
                Path::new(candidate).exists() && !claimed_targets.contains(candidate)
            });
            found
        } else {
            None
        };
        if let Some((new_id, new_path)) = target {
            claimed_targets.insert(new_path.clone());
            let old_id: i64 = tx
                .query_row(
                    "SELECT id FROM tracks WHERE path = ?1",
                    params![old_path],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;

            // Preserve the original row identity. A scanner may already have
            // inserted a temporary row for the new path, so merge any state it
            // accumulated, copy its fresh metadata, delete it, then mutate the
            // old row's path in place.
            tx.execute(
                "INSERT INTO stats (track_id, play_count, last_played, skip_count)
                 SELECT ?1, play_count, last_played, skip_count FROM stats WHERE track_id = ?2
                 ON CONFLICT(track_id) DO UPDATE SET
                   play_count  = stats.play_count + excluded.play_count,
                   last_played = MAX(stats.last_played, excluded.last_played),
                   skip_count  = stats.skip_count + excluded.skip_count",
                params![old_id, new_id],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO favorites(track_id, position)
                 SELECT ?1, position FROM favorites WHERE track_id = ?2
                 ON CONFLICT(track_id) DO UPDATE SET
                   position = MIN(favorites.position, excluded.position)",
                params![old_id, new_id],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO playlist_items(playlist_id, track_id, position)
                 SELECT playlist_id, ?1, position FROM playlist_items WHERE track_id = ?2
                 ON CONFLICT(playlist_id, track_id) DO UPDATE SET
                   position = MIN(playlist_items.position, excluded.position)",
                params![old_id, new_id],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE tracks AS old SET
                   (title, artist, album, genre, duration_secs, year, track_number,
                    has_cover, sample_rate, bit_depth, track_gain_db, track_peak,
                    fingerprint, file_size, mtime_ns, file_id) =
                   (SELECT title, artist, album, genre, duration_secs, year, track_number,
                           has_cover, sample_rate, bit_depth, track_gain_db, track_peak,
                           fingerprint, file_size, mtime_ns, file_id
                    FROM tracks WHERE id = ?2),
                   first_seen_at = MIN(
                     old.first_seen_at,
                     COALESCE((SELECT first_seen_at FROM tracks WHERE id = ?2), old.first_seen_at)
                   ),
                   date_added = MIN(
                     old.date_added,
                     COALESCE((SELECT date_added FROM tracks WHERE id = ?2), old.date_added)
                   )
                 WHERE old.id = ?1",
                params![old_id, new_id],
            )
            .map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM tracks WHERE id = ?1", params![new_id])
                .map_err(|error| error.to_string())?;
            tx.execute(
                "UPDATE tracks SET path = ?2 WHERE id = ?1",
                params![old_id, new_path],
            )
            .map_err(|error| error.to_string())?;
        } else {
            tx.execute("DELETE FROM tracks WHERE path = ?1", params![old_path])
                .map_err(|error| error.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(gone.into_iter().map(|(path, _, _)| path).collect())
}

// Watcher removals are path-scoped: query only the missing file or subtree
// named by notify instead of checking every track in the library.
pub(crate) fn prune_changed_paths(
    db: &Db,
    changed: &[std::path::PathBuf],
) -> Result<Vec<String>, String> {
    let missing: Vec<String> = changed
        .iter()
        .filter(|path| !path.exists())
        .map(|path| {
            path.to_string_lossy()
                .trim_end_matches(['/', '\\'])
                .to_string()
        })
        .filter(|path| !path.is_empty())
        .collect();
    if missing.is_empty() {
        return Ok(Vec::new());
    }

    let mut conn = db.0.lock();
    let gone = {
        let separator = std::path::MAIN_SEPARATOR.to_string();
        let mut found: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
        let mut stmt = conn
            .prepare(
                "SELECT path, fingerprint, file_id FROM tracks
                 WHERE path = ?1
                    OR substr(path, 1, length(?1) + 1) = (?1 || ?2)",
            )
            .map_err(|e| e.to_string())?;
        for path in missing {
            let rows = stmt
                .query_map(params![path, separator], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            for (candidate, fingerprint, file_id) in rows.filter_map(Result::ok) {
                if !Path::new(&candidate).exists() {
                    found.insert(candidate, (fingerprint, file_id));
                }
            }
        }
        found
            .into_iter()
            .map(|(path, (fingerprint, file_id))| (path, fingerprint, file_id))
            .collect()
    };
    prune_gone_rows(&mut conn, gone)
}

// Overwrite one track row with freshly re-parsed metadata (used after the
// in-app tag editor writes a file). date_added is intentionally left alone —
// editing tags must not resurface the track under "Recently Added". The FTS
// index follows via the tracks_au trigger. A path not in the library (e.g. a
// file opened via Explorer but never imported) simply updates 0 rows.
pub(crate) fn reindex_track(
    db: &Db,
    t: &MusicTrack,
    fingerprint: Option<&str>,
) -> Result<(), String> {
    let conn = db.0.lock();
    conn.execute(
        "UPDATE tracks SET
           title=?2, artist=?3, album=?4, genre=?5, duration_secs=?6, year=?7,
           track_number=?8, has_cover=?9, sample_rate=?10, bit_depth=?11,
           track_gain_db=?12, track_peak=?13,
           fingerprint=COALESCE(?14, fingerprint), file_size=?15, mtime_ns=?16,
           file_id=?17
         WHERE path=?1",
        params![
            t.path,
            t.title,
            t.artist,
            t.album,
            t.genre,
            t.duration_secs as i64,
            t.year.map(|v| v as i64),
            t.track_number.map(|v| v as i64),
            t.has_cover as i64,
            t.sample_rate.map(|v| v as i64),
            t.bit_depth.map(|v| v as i64),
            t.track_gain_db.map(|v| v as f64),
            t.track_peak.map(|v| v as f64),
            fingerprint,
            t.file_size.min(i64::MAX as u64) as i64,
            t.mtime_ns,
            t.file_id,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// Paths available to the opt-in online metadata importer. Keeping this query in
// the DB module avoids exposing a "load the whole library" command to the
// webview; the importer still re-checks the real file tags before doing network
// work, because display fallbacks such as "Unknown Artist" are not real tags.
pub(crate) fn all_track_paths(db: &Db) -> Result<Vec<String>, String> {
    let conn = db.read();
    let mut stmt = conn
        .prepare("SELECT path FROM tracks ORDER BY path")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

// One-time background backfill for libraries that predate the fingerprint
// column. Batches keep each lock hold short and the hashing itself runs with
// the lock released, so the UI never stalls behind it.
pub(crate) fn backfill_fingerprints(app: &AppHandle) {
    loop {
        let batch: Vec<String> = {
            let db = app.state::<Db>();
            let conn = db.read();
            let Ok(mut stmt) =
                conn.prepare("SELECT path FROM tracks WHERE fingerprint IS NULL LIMIT 48")
            else {
                return;
            };
            let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else {
                return;
            };
            rows.filter_map(|r| r.ok()).collect()
        };
        if batch.is_empty() {
            return;
        }
        let fps: Vec<(String, String)> = batch
            .into_par_iter()
            .map(|p| {
                // '' = unreadable; recorded so this row isn't re-selected forever.
                let fp = crate::compute_fingerprint(Path::new(&p)).unwrap_or_default();
                (p, fp)
            })
            .collect();
        {
            let db = app.state::<Db>();
            let conn = db.0.lock();
            for (p, fp) in fps {
                let _ = conn.execute(
                    "UPDATE tracks SET fingerprint = ?2 WHERE path = ?1 AND fingerprint IS NULL",
                    params![p, fp],
                );
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

// Delete every track whose path lives under `root` (case-insensitive, slash-
// normalised prefix match). Returns the removed paths so the frontend can drop
// them from the queue / current playback. Also cascades to stats/favorites/
// playlist items so nothing dangles.
pub fn db_remove_under_root(db: State<Db>, root: String) -> Result<Vec<String>, String> {
    let mut conn = db.0.lock();
    let all: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT path FROM tracks")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let norm = |s: &str| s.replace('\\', "/").to_lowercase();
    let root_n = norm(&root);
    let root_prefix = format!("{}/", root_n.trim_end_matches('/'));
    let removed: Vec<String> = all
        .into_iter()
        .filter(|p| {
            let pn = norm(p);
            pn == root_n || pn.starts_with(&root_prefix)
        })
        .collect();
    let transaction = conn.transaction().map_err(|e| e.to_string())?;
    for p in &removed {
        transaction
            .execute("DELETE FROM tracks WHERE path = ?1", params![p])
            .map_err(|e| e.to_string())?;
    }
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(removed)
}

#[tauri::command]
pub fn db_count(db: State<Db>) -> Result<i64, String> {
    let conn = db.read();
    conn.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))
        .map_err(|e| e.to_string())
}

// Wipe the entire library (tracks, stats, favorites, playlists, roots, recents).
// Settings/playback in `kv` are left intact.
#[tauri::command]
pub fn db_reset(
    db: State<Db>,
    consent: State<security::DestructiveConsentState>,
    consent_token: String,
) -> Result<(), String> {
    consent.consume(&consent_token, security::ConsentAction::ResetLibrary, None)?;
    {
        let conn = db.0.lock();
        conn.execute_batch(
            "DELETE FROM tracks; DELETE FROM stats; DELETE FROM favorites;
             DELETE FROM playlist_items; DELETE FROM playlists; DELETE FROM roots;
             DELETE FROM pending_roots; DELETE FROM recents; DELETE FROM cover_art;",
        )
        .map_err(|e| e.to_string())?;
    }
    db.1.smart_counts.lock().clear();
    db.1.station_sessions.lock().clear();
    Ok(())
}

// ---- Roots ------------------------------------------------------------------

#[tauri::command]
pub fn db_roots(db: State<Db>) -> Result<Vec<String>, String> {
    roots(db.inner())
}

/// Read the persisted library roots from trusted Rust code.  Mutating this
/// table is deliberately not exposed as a raw IPC command: a webview must not
/// be able to turn an arbitrary path into an authorised library root.
pub(crate) fn roots(db: &Db) -> Result<Vec<String>, String> {
    let conn = db.read();
    let mut stmt = conn
        .prepare("SELECT path FROM roots ORDER BY path")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub(crate) fn replace_roots(db: &Db, roots: &[String]) -> Result<(), String> {
    let mut conn = db.0.lock();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM roots", [])
        .map_err(|e| e.to_string())?;
    for r in roots {
        tx.execute("INSERT OR IGNORE INTO roots(path) VALUES (?1)", params![r])
            .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) fn insert_roots(db: &Db, roots: &[String]) -> Result<(), String> {
    let mut conn = db.0.lock();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for root in roots {
        tx.execute(
            "INSERT OR IGNORE INTO roots(path) VALUES (?1)",
            params![root],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

pub(crate) fn delete_root(db: &Db, root: &str) -> Result<bool, String> {
    let conn = db.0.lock();
    let changed = conn
        .execute("DELETE FROM roots WHERE path = ?1", params![root])
        .map_err(|e| e.to_string())?;
    Ok(changed == 1)
}

pub(crate) mod tracks;

pub(crate) mod stats;

pub(crate) mod playlists;

// ---- Key/value (settings + playback state) ---------------------------------

#[tauri::command]
pub fn db_kv_get(db: State<Db>, key: String) -> Result<Option<Value>, String> {
    if !matches!(key.as_str(), "settings" | "playback") {
        return Err("Unsupported settings key".to_string());
    }
    kv_get(db.inner(), &key)
}

pub(crate) fn kv_get(db: &Db, key: &str) -> Result<Option<Value>, String> {
    let conn = db.read();
    let raw: Option<String> = conn
        .query_row("SELECT v FROM kv WHERE k = ?1", params![key], |r| r.get(0))
        .optional_string()?;
    Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
}

#[tauri::command]
pub fn db_kv_set(db: State<Db>, key: String, value: Value) -> Result<(), String> {
    if !matches!(key.as_str(), "settings" | "playback") {
        return Err("Unsupported settings key".to_string());
    }
    kv_set(db.inner(), &key, &value, limits::MAX_KV_BYTES)
}

pub(crate) fn kv_set(db: &Db, key: &str, value: &Value, max_bytes: usize) -> Result<(), String> {
    limits::validate_json(value, "Settings value", max_bytes, 12)?;
    let conn = db.0.lock();
    conn.execute(
        "INSERT INTO kv(k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = ?2",
        params![key, value.to_string()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) mod backup;

// ---- One-time migration from the legacy IndexedDB blob ----------------------

// Seed the database from the old IndexedDB state. Called once by the frontend
// when the DB is empty but legacy IndexedDB data exists. `state` is the old
// `app_state` object (favorites/playlists/stats/recents/settings/playback).
#[tauri::command]
pub fn db_import(
    db: State<Db>,
    tracks: Vec<MusicTrack>,
    roots: Vec<String>,
    state: Value,
) -> Result<(), String> {
    {
        let conn = db.read();
        let has_existing: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM tracks
                    UNION ALL SELECT 1 FROM playlists
                    UNION ALL SELECT 1 FROM favorites
                    UNION ALL SELECT 1 FROM stats
                    UNION ALL SELECT 1 FROM roots
                )",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if has_existing {
            return Err("Legacy import is allowed only into a new empty library".to_string());
        }
    }
    if tracks.len() > 200_000 {
        return Err("Legacy import contains too many tracks (max 200000)".to_string());
    }
    limits::validate_paths(&roots, 256)?;
    limits::validate_json(
        &state,
        "Legacy state",
        16 * 1024 * 1024,
        limits::MAX_JSON_DEPTH,
    )?;
    for track in &tracks {
        limits::validate_text(&track.path, "Track path", limits::MAX_PATH_BYTES)?;
        limits::validate_text(&track.title, "Track title", 1_024)?;
        limits::validate_text(&track.artist, "Track artist", 1_024)?;
        limits::validate_text(&track.album, "Track album", 1_024)?;
        if let Some(genre) = track.genre.as_deref() {
            limits::validate_text(genre, "Track genre", 256)?;
        }
    }
    // Legacy roots came from webview-controlled IndexedDB/localStorage.  They
    // are intentionally not restored as filesystem authority; the user can
    // re-authorise those folders once through the native folder picker.
    db_upsert_tracks(db.clone(), tracks)?;

    let mut conn = db.0.lock();
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    if let Some(favs) = state.get("favorites").and_then(|v| v.as_array()) {
        for (i, p) in favs.iter().enumerate() {
            if let Some(path) = p.as_str() {
                tx.execute(
                    "INSERT OR REPLACE INTO favorites(track_id, position)
                     SELECT id, ?2 FROM tracks WHERE path = ?1",
                    params![path, i as i64],
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }

    if let Some(stats) = state.get("stats").and_then(|v| v.as_object()) {
        for (path, st) in stats {
            let pc = st.get("playCount").and_then(|v| v.as_i64()).unwrap_or(0);
            let lp = st.get("lastPlayed").and_then(|v| v.as_i64()).unwrap_or(0);
            let sc = st.get("skipCount").and_then(|v| v.as_i64()).unwrap_or(0);
            tx.execute(
                "INSERT OR REPLACE INTO stats(track_id, play_count, last_played, skip_count)
                 SELECT id, ?2, ?3, ?4 FROM tracks WHERE path = ?1",
                params![path, pc, lp, sc],
            )
            .map_err(|e| e.to_string())?;
        }
    }

    if let Some(pls) = state.get("playlists").and_then(|v| v.as_array()) {
        for (pos, pl) in pls.iter().enumerate() {
            let id = pl
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                continue;
            }
            let is_smart = pl.get("rules").map(|r| !r.is_null()).unwrap_or(false);
            let rules_str = pl.get("rules").map(|r| r.to_string());
            tx.execute(
                "INSERT OR REPLACE INTO playlists (id, name, description, color, cover, position, is_smart, rules, sort_by, sort_order, limit_n, live_update)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    id,
                    pl.get("name").and_then(|v| v.as_str()).unwrap_or("Playlist"),
                    pl.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                    pl.get("color").and_then(|v| v.as_str()),
                    pl.get("cover").and_then(|v| v.as_str()),
                    pos as i64,
                    is_smart as i64,
                    rules_str,
                    pl.get("sortBy").and_then(|v| v.as_str()),
                    pl.get("sortOrder").and_then(|v| v.as_str()),
                    pl.get("limit").and_then(|v| v.as_i64()),
                    pl.get("liveUpdate").and_then(|v| v.as_bool()).map(|b| b as i64),
                ],
            )
            .map_err(|e| e.to_string())?;
            if let Some(paths) = pl.get("paths").and_then(|v| v.as_array()) {
                for (i, p) in paths.iter().enumerate() {
                    if let Some(path) = p.as_str() {
                        tx.execute(
                            "INSERT OR IGNORE INTO playlist_items(playlist_id, track_id, position)
                             SELECT ?1, id, ?3 FROM tracks WHERE path = ?2",
                            params![id, path, i as i64],
                        )
                        .map_err(|e| e.to_string())?;
                    }
                }
            }
        }
    }

    if let Some(recents) = state.get("recents").and_then(|v| v.as_array()) {
        for r in recents {
            let (Some(kind), Some(key)) = (
                r.get("type").and_then(|v| v.as_str()),
                r.get("key").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let ts = r.get("ts").and_then(|v| v.as_i64()).unwrap_or(0);
            tx.execute(
                "INSERT OR REPLACE INTO recents(type, key, ts) VALUES (?1,?2,?3)",
                params![kind, key, ts],
            )
            .map_err(|e| e.to_string())?;
        }
    }

    if let Some(settings) = state.get("settings") {
        tx.execute(
            "INSERT OR REPLACE INTO kv(k, v) VALUES ('settings', ?1)",
            params![settings.to_string()],
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(playback) = state.get("playback") {
        tx.execute(
            "INSERT OR REPLACE INTO kv(k, v) VALUES ('playback', ?1)",
            params![playback.to_string()],
        )
        .map_err(|e| e.to_string())?;
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

mod smart_playlists;
use smart_playlists::{smart_count, smart_eval, validate_smart_request};

// Small helper: turn "no rows" into Ok(None) for optional single-row reads.
trait OptionalString<T> {
    fn optional_string(self) -> Result<Option<T>, String>;
}

impl<T> OptionalString<T> for rusqlite::Result<T> {
    fn optional_string(self) -> Result<Option<T>, String> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}

pub fn db_get_cover_art(db: &Db, path: &str) -> Option<(String, String, Vec<u8>)> {
    let conn = db.read();

    // First find album and artist for the path
    let (album, artist): (String, String) = conn
        .query_row(
            "SELECT album, artist FROM tracks WHERE path = ?1",
            params![path],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;

    // Look up cover bytes in cover_art table
    let bytes: Vec<u8> = conn
        .query_row(
            "SELECT bytes FROM cover_art WHERE album = ?1 AND artist = ?2",
            params![album, artist],
            |r| r.get(0),
        )
        .ok()?;

    Some((album, artist, bytes))
}

// Insert cover art bytes into SQLite
pub fn db_save_cover_art(db: &Db, album: &str, artist: &str, bytes: &[u8]) -> Result<(), String> {
    let conn = db.0.lock();
    conn.execute(
        "INSERT OR REPLACE INTO cover_art(album, artist, bytes) VALUES (?1, ?2, ?3)",
        params![album, artist, bytes],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod indexing_tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn memory_db() -> Db {
        let mut conn = Connection::open_in_memory().expect("open test database");
        conn.execute_batch(SCHEMA).expect("create test schema");
        migrate(&mut conn).expect("migrate test schema");
        Db(
            Mutex::new(ConnectionSlot::new(conn)),
            DbCache::default(),
            ReadPool::empty(),
        )
    }

    fn unique_temp_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("ts-music-index-{}-{nonce}", std::process::id()))
    }

    fn insert_track(db: &Db, path: &Path, fingerprint: &str) {
        db.0.lock()
            .execute(
                "INSERT INTO tracks(path, title, artist, album, fingerprint)
                 VALUES (?1, 'Song', 'Artist', 'Album', ?2)",
                params![path.to_string_lossy(), fingerprint],
            )
            .expect("insert test track");
    }

    fn music_track(path: &str, first_seen_at: u64, file_size: u64, mtime_ns: i64) -> MusicTrack {
        MusicTrack {
            path: path.to_string(),
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            genre: Some("Genre".to_string()),
            duration_secs: 180,
            date_added: first_seen_at,
            year: Some(2026),
            track_number: Some(1),
            has_cover: false,
            sample_rate: Some(44_100),
            bit_depth: Some(16),
            track_gain_db: None,
            track_peak: None,
            file_size,
            mtime_ns,
            file_id: Some("test-file-id".to_string()),
        }
    }

    #[test]
    fn filesystem_signatures_skip_unchanged_candidates() {
        let db = memory_db();
        db.0.lock()
            .execute(
                "INSERT INTO tracks
                (path, title, artist, album, file_size, mtime_ns, file_id)
                 VALUES ('same.mp3', 'Same', 'Artist', 'Album', 100, 200, 'same-id')",
                [],
            )
            .expect("insert signature row");
        let candidates = vec![
            (PathBuf::from("same.mp3"), 100, 200, Some("same-id".into())),
            (
                PathBuf::from("changed.mp3"),
                101,
                201,
                Some("changed-id".into()),
            ),
        ];
        let changed = paths_requiring_metadata(&db, &candidates).expect("compare signatures");
        assert_eq!(changed, vec![PathBuf::from("changed.mp3")]);

        let changed_signature = vec![(PathBuf::from("same.mp3"), 100, 201, Some("same-id".into()))];
        assert_eq!(
            paths_requiring_metadata(&db, &changed_signature).expect("detect changed mtime"),
            vec![PathBuf::from("same.mp3")]
        );

        let replaced_same_size_and_mtime = vec![(
            PathBuf::from("same.mp3"),
            100,
            200,
            Some("replacement-id".into()),
        )];
        assert_eq!(
            paths_requiring_metadata(&db, &replaced_same_size_and_mtime)
                .expect("detect replaced file identity"),
            vec![PathBuf::from("same.mp3")]
        );
    }

    #[test]
    fn metadata_refresh_preserves_first_seen_time() {
        let db = memory_db();
        upsert_tracks(&db, vec![music_track("song.mp3", 100, 10, 20)]).expect("insert track");
        upsert_tracks(&db, vec![music_track("song.mp3", 999, 11, 21)]).expect("refresh track");
        let (first_seen_at, file_size, mtime_ns): (i64, i64, i64) =
            db.0.lock()
                .query_row(
                    "SELECT first_seen_at, file_size, mtime_ns FROM tracks WHERE path = 'song.mp3'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .expect("read refreshed track");
        assert_eq!(first_seen_at, 100);
        assert_eq!((file_size, mtime_ns), (11, 21));
    }

    #[test]
    fn schema_v3_migrates_signatures_first_seen_and_genre_fts() {
        let mut connection = Connection::open_in_memory().expect("open v2 database");
        connection
            .execute_batch(
                "CREATE TABLE tracks (
                   id INTEGER PRIMARY KEY,
                   path TEXT NOT NULL UNIQUE,
                   title TEXT NOT NULL DEFAULT '',
                   artist TEXT NOT NULL DEFAULT '',
                   album TEXT NOT NULL DEFAULT '',
                   genre TEXT,
                   duration_secs INTEGER NOT NULL DEFAULT 0,
                   date_added INTEGER NOT NULL DEFAULT 0,
                   year INTEGER,
                   track_number INTEGER,
                   has_cover INTEGER NOT NULL DEFAULT 0,
                   sample_rate INTEGER,
                   bit_depth INTEGER,
                   track_gain_db REAL,
                   track_peak REAL,
                   fingerprint TEXT
                 );
                 INSERT INTO tracks
                   (path, title, artist, album, genre, date_added)
                 VALUES ('legacy.mp3', 'Legacy', 'Artist', 'Album', 'Rock', 123);
                 CREATE VIRTUAL TABLE tracks_fts USING fts5(
                   title, artist, album,
                   content='tracks', content_rowid='id',
                   tokenize=\"unicode61 remove_diacritics 2\"
                 );
                 INSERT INTO tracks_fts(tracks_fts) VALUES ('rebuild');
                 CREATE TRIGGER tracks_ai AFTER INSERT ON tracks BEGIN
                   INSERT INTO tracks_fts(rowid, title, artist, album)
                   VALUES (new.id, new.title, new.artist, new.album);
                 END;
                 CREATE TRIGGER tracks_ad AFTER DELETE ON tracks BEGIN
                   INSERT INTO tracks_fts(tracks_fts, rowid, title, artist, album)
                   VALUES ('delete', old.id, old.title, old.artist, old.album);
                 END;
                 CREATE TRIGGER tracks_au AFTER UPDATE ON tracks BEGIN
                   INSERT INTO tracks_fts(tracks_fts, rowid, title, artist, album)
                   VALUES ('delete', old.id, old.title, old.artist, old.album);
                   INSERT INTO tracks_fts(rowid, title, artist, album)
                   VALUES (new.id, new.title, new.artist, new.album);
                 END;",
            )
            .expect("create legacy tracks table");
        connection
            .pragma_update(None, "application_id", APPLICATION_ID)
            .expect("set application id");
        connection
            .pragma_update(None, "user_version", 2)
            .expect("mark schema v2");

        // init() executes the idempotent base schema before additive migration.
        connection
            .execute_batch(SCHEMA)
            .expect("base schema remains compatible with v2 tables");
        migrate(&mut connection).expect("migrate v2 database");

        let migrated: (i64, Option<i64>, Option<i64>) = connection
            .query_row(
                "SELECT first_seen_at, file_size, mtime_ns FROM tracks WHERE path = 'legacy.mp3'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read migrated signature columns");
        assert_eq!(migrated, (123, None, None));
        let genre_matches: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM tracks_search WHERE tracks_search MATCH 'genre:(\"Rock\"*)'",
                [],
                |row| row.get(0),
            )
            .expect("query migrated genre FTS");
        assert_eq!(genre_matches, 1);
    }

    #[test]
    fn migration_merges_windows_verbatim_path_duplicates() {
        let mut connection = Connection::open_in_memory().expect("open migration database");
        connection
            .execute_batch(SCHEMA)
            .expect("create test schema");
        connection
            .execute_batch(
                "DROP TABLE playlist_items;
                 DROP TABLE favorites;
                 DROP TABLE stats;
                 CREATE TABLE stats (
                   path TEXT PRIMARY KEY,
                   play_count INTEGER NOT NULL DEFAULT 0,
                   last_played INTEGER NOT NULL DEFAULT 0,
                   skip_count INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TABLE favorites (
                   path TEXT PRIMARY KEY,
                   position INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TABLE playlist_items (
                   playlist_id TEXT NOT NULL,
                   path TEXT NOT NULL,
                   position INTEGER NOT NULL,
                   PRIMARY KEY (playlist_id, path)
                 );",
            )
            .expect("restore v1 path-based relationships");
        connection
            .pragma_update(None, "user_version", 1)
            .expect("mark schema v1");
        let legacy = r"D:\Music\song.flac";
        let verbatim = r"\\?\D:\Music\song.flac";
        let legacy_root = r"D:\Music";
        let verbatim_root = r"\\?\D:\Music";

        for (path, title) in [(legacy, "Legacy"), (verbatim, "Canonical")] {
            connection
                .execute(
                    "INSERT INTO tracks(path, title, artist, album) VALUES (?1, ?2, 'Artist', 'Album')",
                    params![path, title],
                )
                .expect("insert duplicate track spelling");
        }
        for (path, plays, last, skips) in [(legacy, 2, 10, 1), (verbatim, 5, 20, 2)] {
            connection
                .execute(
                    "INSERT INTO stats(path, play_count, last_played, skip_count) VALUES (?1, ?2, ?3, ?4)",
                    params![path, plays, last, skips],
                )
                .expect("insert duplicate stats");
        }
        connection
            .execute(
                "INSERT INTO favorites(path, position) VALUES (?1, 7), (?2, 2)",
                params![legacy, verbatim],
            )
            .expect("insert duplicate favorites");
        connection
            .execute(
                "INSERT INTO playlists(id, name) VALUES ('playlist', 'Playlist')",
                [],
            )
            .expect("insert playlist");
        connection
            .execute(
                "INSERT INTO playlist_items(playlist_id, path, position)
                 VALUES ('playlist', ?1, 4), ('playlist', ?2, 1)",
                params![legacy, verbatim],
            )
            .expect("insert duplicate playlist entries");
        connection
            .execute(
                "INSERT INTO roots(path) VALUES (?1)",
                params![verbatim_root],
            )
            .expect("insert verbatim root");
        connection
            .execute(
                "INSERT INTO kv(k, v) VALUES ('playback', ?1)",
                params![serde_json::json!({
                    "songPath": verbatim,
                    "queuePaths": [verbatim]
                })
                .to_string()],
            )
            .expect("insert playback state");

        migrate(&mut connection).expect("run v2 migration");

        let track_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))
            .expect("count merged tracks");
        let stats: (i64, i64, i64) = connection
            .query_row(
                "SELECT s.play_count, s.last_played, s.skip_count
                 FROM stats s JOIN tracks t ON t.id = s.track_id WHERE t.path = ?1",
                params![legacy],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read merged stats");
        let favorite_position: i64 = connection
            .query_row(
                "SELECT f.position FROM favorites f
                 JOIN tracks t ON t.id = f.track_id WHERE t.path = ?1",
                params![legacy],
                |row| row.get(0),
            )
            .expect("read merged favorite");
        let playlist_position: i64 = connection
            .query_row(
                "SELECT i.position FROM playlist_items i
                 JOIN tracks t ON t.id = i.track_id
                 WHERE i.playlist_id = 'playlist' AND t.path = ?1",
                params![legacy],
                |row| row.get(0),
            )
            .expect("read merged playlist item");
        let root_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM roots WHERE path = ?1",
                params![legacy_root],
                |row| row.get(0),
            )
            .expect("read normalized root");
        let playback: String = connection
            .query_row("SELECT v FROM kv WHERE k = 'playback'", [], |row| {
                row.get(0)
            })
            .expect("read normalized playback state");
        let playback: Value =
            serde_json::from_str(&playback).expect("parse normalized playback state");

        assert_eq!(track_count, 1);
        assert_eq!(stats, (7, 20, 3));
        assert_eq!(favorite_position, 2);
        assert_eq!(playlist_position, 1);
        assert_eq!(root_count, 1);
        assert_eq!(playback["songPath"], legacy);
        assert_eq!(playback["queuePaths"][0], legacy);
    }

    #[test]
    fn changed_path_pruning_preserves_stats_across_a_rename() {
        let db = memory_db();
        let dir = unique_temp_dir();
        std::fs::create_dir(&dir).expect("create test directory");
        let old_path = dir.join("old.flac");
        let new_path = dir.join("new.flac");
        std::fs::write(&new_path, b"renamed audio placeholder").expect("create surviving file");

        insert_track(&db, &old_path, "same-fingerprint");
        insert_track(&db, &new_path, "same-fingerprint");
        db.0.lock()
            .execute(
                "INSERT INTO stats(track_id, play_count, last_played, skip_count)
                 SELECT id, 7, 42, 2 FROM tracks WHERE path = ?1",
                params![old_path.to_string_lossy()],
            )
            .expect("insert old stats");

        let removed =
            prune_changed_paths(&db, std::slice::from_ref(&old_path)).expect("prune renamed path");
        assert_eq!(removed, vec![old_path.to_string_lossy().to_string()]);
        let migrated: (i64, i64, i64) =
            db.0.lock()
                .query_row(
                    "SELECT s.play_count, s.last_played, s.skip_count
                     FROM stats s JOIN tracks t ON t.id = s.track_id WHERE t.path = ?1",
                    params![new_path.to_string_lossy()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .expect("stats migrated to new path");
        assert_eq!(migrated, (7, 42, 2));

        std::fs::remove_file(new_path).expect("remove test file");
        std::fs::remove_dir(dir).expect("remove test directory");
    }

    #[test]
    fn changed_directory_pruning_is_limited_to_that_subtree() {
        let db = memory_db();
        let base = unique_temp_dir();
        let removed_path = base.join("removed").join("song.flac");
        let unrelated_path = base.join("other").join("keep.flac");
        insert_track(&db, &removed_path, "removed-fingerprint");
        insert_track(&db, &unrelated_path, "unrelated-fingerprint");

        let changed_dir = base.join("removed");
        let removed = prune_changed_paths(&db, std::slice::from_ref(&changed_dir))
            .expect("prune changed subtree");
        assert_eq!(removed, vec![removed_path.to_string_lossy().to_string()]);
        let unrelated_count: i64 =
            db.0.lock()
                .query_row(
                    "SELECT COUNT(*) FROM tracks WHERE path = ?1",
                    params![unrelated_path.to_string_lossy()],
                    |row| row.get(0),
                )
                .expect("query unrelated row");
        assert_eq!(unrelated_count, 1);
    }
}
