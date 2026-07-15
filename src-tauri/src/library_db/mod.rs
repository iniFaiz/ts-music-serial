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
// A single connection guarded by a parking_lot Mutex is enough: SQLite serialises
// writes anyway and our reads are short. The connection is opened once at startup
// and managed as Tauri state.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use parking_lot::Mutex;
use rayon::prelude::*;
use rusqlite::{params, Connection, Row};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};

use crate::MusicTrack;

// Memoized smart-playlist counts, guarded alongside the connection. Keyed by
// playlist id → (library fingerprint, rules JSON, count). The fingerprint is a
// cheap signature of the tracks/stats/favorites tables (see library_fingerprint);
// when it and the rules both match, db_playlists reuses the count instead of
// re-scanning the whole library with smart_eval on every refresh.
#[derive(Default)]
pub struct DbCache {
    smart_counts: Mutex<HashMap<String, (i64, String, i64)>>,
}

pub struct Db(pub Mutex<Connection>, pub DbCache);

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
const TRACK_COLS: &str = "path, title, artist, album, genre, duration_secs, date_added, year, track_number, has_cover, sample_rate, bit_depth, track_gain_db, track_peak";
const TRACK_COLS_T: &str = "t.path, t.title, t.artist, t.album, t.genre, t.duration_secs, t.date_added, t.year, t.track_number, t.has_cover, t.sample_rate, t.bit_depth, t.track_gain_db, t.track_peak";

fn row_to_track(row: &Row) -> rusqlite::Result<MusicTrack> {
    Ok(MusicTrack {
        path: row.get(0)?,
        title: row.get(1)?,
        artist: row.get(2)?,
        album: row.get(3)?,
        genre: row.get(4)?,
        duration_secs: row.get::<_, i64>(5)? as u64,
        date_added: row.get::<_, i64>(6)? as u64,
        year: row.get::<_, Option<i64>>(7)?.map(|v| v as u32),
        track_number: row.get::<_, Option<i64>>(8)?.map(|v| v as u32),
        has_cover: row.get::<_, i64>(9)? != 0,
        sample_rate: row.get::<_, Option<i64>>(10)?.map(|v| v as u32),
        bit_depth: row.get::<_, Option<i64>>(11)?.map(|v| v as u8),
        track_gain_db: row.get::<_, Option<f64>>(12)?.map(|v| v as f32),
        track_peak: row.get::<_, Option<f64>>(13)?.map(|v| v as f32),
    })
}

// ---- Schema / init ----------------------------------------------------------

pub fn init(app: &AppHandle) -> Result<Db, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("no app data dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let conn = Connection::open(dir.join("library.db")).map_err(|e| e.to_string())?;
    conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    migrate(&conn)?;
    Ok(Db(Mutex::new(conn), DbCache::default()))
}

// Additive schema migrations for databases created before a column existed.
// (CREATE TABLE IF NOT EXISTS never alters an existing table.)
fn migrate(conn: &Connection) -> Result<(), String> {
    let has_fingerprint: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('tracks') WHERE name = 'fingerprint'")
        .and_then(|mut s| s.exists([]))
        .map_err(|e| e.to_string())?;
    if !has_fingerprint {
        // Content fingerprint (size + sampled hash, see crate::compute_fingerprint)
        // used to re-identify a track after its file is moved/renamed so its
        // stats/favorites/playlist memberships survive. '' = hashing failed
        // (unreadable file) — tried, don't retry; NULL = not yet computed.
        conn.execute_batch(
            "ALTER TABLE tracks ADD COLUMN fingerprint TEXT;
             CREATE INDEX IF NOT EXISTS idx_tracks_fp ON tracks(fingerprint);",
        )
        .map_err(|e| e.to_string())?;
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
  year          INTEGER,
  track_number  INTEGER,
  has_cover     INTEGER NOT NULL DEFAULT 0,
  sample_rate   INTEGER,
  bit_depth     INTEGER,
  track_gain_db REAL,
  track_peak    REAL
);
CREATE INDEX IF NOT EXISTS idx_tracks_album  ON tracks(album);
CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
CREATE INDEX IF NOT EXISTS idx_tracks_added  ON tracks(date_added);
-- Default library sort is by title; index it so the ORDER BY is index-served.
CREATE INDEX IF NOT EXISTS idx_tracks_title  ON tracks(title COLLATE NOCASE);

-- Diacritic-insensitive full-text index over the searchable text columns, kept
-- in sync with `tracks` by triggers (external-content FTS5 keyed on tracks.id).
CREATE VIRTUAL TABLE IF NOT EXISTS tracks_fts USING fts5(
  title, artist, album,
  content='tracks', content_rowid='id',
  tokenize="unicode61 remove_diacritics 2"
);
CREATE TRIGGER IF NOT EXISTS tracks_ai AFTER INSERT ON tracks BEGIN
  INSERT INTO tracks_fts(rowid, title, artist, album)
  VALUES (new.id, new.title, new.artist, new.album);
END;
CREATE TRIGGER IF NOT EXISTS tracks_ad AFTER DELETE ON tracks BEGIN
  INSERT INTO tracks_fts(tracks_fts, rowid, title, artist, album)
  VALUES ('delete', old.id, old.title, old.artist, old.album);
END;
CREATE TRIGGER IF NOT EXISTS tracks_au AFTER UPDATE ON tracks BEGIN
  INSERT INTO tracks_fts(tracks_fts, rowid, title, artist, album)
  VALUES ('delete', old.id, old.title, old.artist, old.album);
  INSERT INTO tracks_fts(rowid, title, artist, album)
  VALUES (new.id, new.title, new.artist, new.album);
END;

CREATE TABLE IF NOT EXISTS stats (
  path        TEXT PRIMARY KEY,
  play_count  INTEGER NOT NULL DEFAULT 0,
  last_played INTEGER NOT NULL DEFAULT 0,
  skip_count  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS favorites (
  path     TEXT PRIMARY KEY,
  position INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS roots (path TEXT PRIMARY KEY);

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
  playlist_id TEXT NOT NULL,
  path        TEXT NOT NULL,
  position    INTEGER NOT NULL,
  PRIMARY KEY (playlist_id, path)
);
CREATE INDEX IF NOT EXISTS idx_pl_items ON playlist_items(playlist_id, position);

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

// Whitelist sort keys → SQL so the interpolated ORDER BY can never carry
// user-controlled text. `p` is a table-alias prefix ("" for a plain query, "t."
// when the query JOINs another table — e.g. tracks_fts, which also has
// title/artist/album columns, so the bare names would be ambiguous).
fn sort_col(sort_by: &str, p: &str) -> String {
    match sort_by {
        "artist" => format!("{p}artist COLLATE NOCASE, {p}album COLLATE NOCASE, {p}track_number"),
        "album" => format!("{p}album COLLATE NOCASE, {p}track_number"),
        "year" => format!("{p}year"),
        "duration" | "duration_secs" => format!("{p}duration_secs"),
        "dateAdded" | "date_added" => format!("{p}date_added"),
        "track_number" => format!("{p}track_number"),
        _ => format!("{p}title COLLATE NOCASE"),
    }
}

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
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
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

// ---- Library mutation -------------------------------------------------------

// Insert/update scanned tracks. Returns how many were newly inserted (existing
// rows are refreshed but not counted, matching the old "new tracks" status text).
#[tauri::command]
pub fn db_upsert_tracks(db: State<Db>, tracks: Vec<MusicTrack>) -> Result<usize, String> {
    upsert_tracks(&db, tracks)
}

// Used by the native scanner so metadata can be indexed without crossing IPC.
// Memory usage is bounded by the caller's batch size.
pub(crate) fn upsert_tracks(db: &Db, tracks: Vec<MusicTrack>) -> Result<usize, String> {
    upsert_tracks_with_options(db, tracks, false)
}

// Watcher batches are small and represent actual writes/renames, so refresh
// their fingerprints as well as metadata. Full scans keep existing hashes to
// avoid another 128 KiB of IO for every unchanged file.
pub(crate) fn upsert_changed_tracks(db: &Db, tracks: Vec<MusicTrack>) -> Result<usize, String> {
    upsert_tracks_with_options(db, tracks, true)
}

fn upsert_tracks_with_options(
    db: &Db,
    tracks: Vec<MusicTrack>,
    refresh_fingerprints: bool,
) -> Result<usize, String> {
    // Compute content fingerprints for tracks that don't have one yet (new files,
    // plus pre-fingerprint rows getting backfilled on rescan). Hashing reads
    // ~128 KiB per file, so it happens in parallel and OUTSIDE the connection
    // lock — the UI keeps querying while a big import is being hashed.
    let need_fp: Vec<String> = {
        let conn = db.0.lock();
        let mut has_fingerprint = conn
            .prepare("SELECT fingerprint IS NOT NULL FROM tracks WHERE path = ?1")
            .map_err(|e| e.to_string())?;
        let mut paths = Vec::new();
        for track in &tracks {
            let already_hashed =
                match has_fingerprint.query_row(params![track.path], |row| row.get::<_, bool>(0)) {
                    Ok(value) => value,
                    Err(rusqlite::Error::QueryReturnedNoRows) => false,
                    Err(error) => return Err(error.to_string()),
                };
            if refresh_fingerprints || !already_hashed {
                paths.push(track.path.clone());
            }
        }
        paths
    };
    let fps: HashMap<String, String> = need_fp
        .into_par_iter()
        .map(|p| {
            // '' = tried but unreadable, so the backfill doesn't retry forever.
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
                "INSERT INTO tracks (path, title, artist, album, genre, duration_secs, date_added, year, track_number, has_cover, sample_rate, bit_depth, track_gain_db, track_peak, fingerprint)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
                 ON CONFLICT(path) DO UPDATE SET
                   title=excluded.title, artist=excluded.artist, album=excluded.album,
                   genre=excluded.genre, duration_secs=excluded.duration_secs,
                   date_added=excluded.date_added, year=excluded.year,
                   track_number=excluded.track_number, has_cover=excluded.has_cover,
                   sample_rate=excluded.sample_rate, bit_depth=excluded.bit_depth,
                   track_gain_db=excluded.track_gain_db, track_peak=excluded.track_peak,
                   fingerprint=COALESCE(excluded.fingerprint, fingerprint)",
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

#[tauri::command]
pub fn db_remove_paths(db: State<Db>, paths: Vec<String>) -> Result<(), String> {
    let mut conn = db.0.lock();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for p in &paths {
        tx.execute("DELETE FROM tracks WHERE path = ?1", params![p])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM stats WHERE path = ?1", params![p])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM favorites WHERE path = ?1", params![p])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM playlist_items WHERE path = ?1", params![p])
            .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

// Remove every track whose file no longer exists on disk. Before deleting,
// try to re-identify each missing file among the surviving rows by content
// fingerprint — a moved/renamed file shows up as "old path gone + new path
// just scanned" — and migrate its play stats, favorite flag, playlist
// memberships and original date_added onto the new row instead of losing them.
// Returns the removed (old) paths so the frontend can drop them from the queue.
#[tauri::command]
pub fn db_prune_missing(db: State<Db>) -> Result<Vec<String>, String> {
    let mut conn = db.0.lock();
    let gone: Vec<(String, Option<String>)> = {
        let mut stmt = conn
            .prepare("SELECT path, fingerprint FROM tracks")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })
            .map_err(|e| e.to_string())?;
        rows.filter_map(Result::ok)
            .filter(|(path, _)| !Path::new(path).exists())
            .collect()
    };
    prune_gone_rows(&mut conn, gone)
}

fn prune_gone_rows(
    conn: &mut Connection,
    gone: Vec<(String, Option<String>)>,
) -> Result<Vec<String>, String> {
    if gone.is_empty() {
        return Ok(Vec::new());
    }

    let mut claimed_targets = HashSet::new();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (old_path, fp) in &gone {
        let target = if let Some(fingerprint) = fp.as_ref().filter(|value| !value.is_empty()) {
            let mut stmt = tx
                .prepare("SELECT path FROM tracks WHERE fingerprint = ?1 AND path <> ?2")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![fingerprint, old_path], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(|e| e.to_string())?;
            let found = rows.filter_map(Result::ok).find(|candidate| {
                Path::new(candidate).exists() && !claimed_targets.contains(candidate)
            });
            found
        } else {
            None
        };
        if let Some(new_path) = target {
            claimed_targets.insert(new_path.clone());
            // Merge play stats into the new path (fresh rows normally have none,
            // but a pre-existing row is summed rather than clobbered).
            tx.execute(
                "INSERT INTO stats (path, play_count, last_played, skip_count)
                 SELECT ?1, play_count, last_played, skip_count FROM stats WHERE path = ?2
                 ON CONFLICT(path) DO UPDATE SET
                   play_count  = stats.play_count + excluded.play_count,
                   last_played = MAX(stats.last_played, excluded.last_played),
                   skip_count  = stats.skip_count + excluded.skip_count",
                params![new_path, old_path],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE OR IGNORE favorites SET path = ?1 WHERE path = ?2",
                params![new_path, old_path],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE OR IGNORE playlist_items SET path = ?1 WHERE path = ?2",
                params![new_path, old_path],
            )
            .map_err(|e| e.to_string())?;
            // Keep the original library-add date so a moved file doesn't reappear
            // under "Recently Added".
            tx.execute(
                "UPDATE tracks SET date_added = MIN(
                   date_added,
                   COALESCE((SELECT date_added FROM tracks WHERE path = ?2), date_added)
                 ) WHERE path = ?1",
                params![new_path, old_path],
            )
            .map_err(|e| e.to_string())?;
        }
        // Drop the stale row; cascade like db_remove_paths so nothing dangles.
        // (Migrated rows already moved their stats/favorites/playlist items away;
        // any leftovers here belong to genuinely deleted files.)
        for sql in [
            "DELETE FROM tracks WHERE path = ?1",
            "DELETE FROM stats WHERE path = ?1",
            "DELETE FROM favorites WHERE path = ?1",
            "DELETE FROM playlist_items WHERE path = ?1",
        ] {
            tx.execute(sql, params![old_path])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(gone.into_iter().map(|(p, _)| p).collect())
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
        let mut found: HashMap<String, Option<String>> = HashMap::new();
        let mut stmt = conn
            .prepare(
                "SELECT path, fingerprint FROM tracks
                 WHERE path = ?1
                    OR substr(path, 1, length(?1) + 1) = (?1 || ?2)",
            )
            .map_err(|e| e.to_string())?;
        for path in missing {
            let rows = stmt
                .query_map(params![path, separator], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })
                .map_err(|e| e.to_string())?;
            for (candidate, fingerprint) in rows.filter_map(Result::ok) {
                if !Path::new(&candidate).exists() {
                    found.insert(candidate, fingerprint);
                }
            }
        }
        found.into_iter().collect()
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
           fingerprint=COALESCE(?14, fingerprint)
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
    let conn = db.0.lock();
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
            let conn = db.0.lock();
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
#[tauri::command]
pub fn db_remove_under_root(db: State<Db>, root: String) -> Result<Vec<String>, String> {
    let conn = db.0.lock();
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
    for p in &removed {
        conn.execute("DELETE FROM tracks WHERE path = ?1", params![p])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM stats WHERE path = ?1", params![p])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM favorites WHERE path = ?1", params![p])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM playlist_items WHERE path = ?1", params![p])
            .map_err(|e| e.to_string())?;
    }
    Ok(removed)
}

#[tauri::command]
pub fn db_count(db: State<Db>) -> Result<i64, String> {
    let conn = db.0.lock();
    conn.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))
        .map_err(|e| e.to_string())
}

// Wipe the entire library (tracks, stats, favorites, playlists, roots, recents).
// Settings/playback in `kv` are left intact.
#[tauri::command]
pub fn db_reset(db: State<Db>) -> Result<(), String> {
    let conn = db.0.lock();
    conn.execute_batch(
        "DELETE FROM tracks; DELETE FROM stats; DELETE FROM favorites;
         DELETE FROM playlist_items; DELETE FROM playlists; DELETE FROM roots;
         DELETE FROM recents;",
    )
    .map_err(|e| e.to_string())
}

// ---- Roots ------------------------------------------------------------------

#[tauri::command]
pub fn db_roots(db: State<Db>) -> Result<Vec<String>, String> {
    let conn = db.0.lock();
    let mut stmt = conn
        .prepare("SELECT path FROM roots")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn db_set_roots(db: State<Db>, roots: Vec<String>) -> Result<(), String> {
    let mut conn = db.0.lock();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM roots", [])
        .map_err(|e| e.to_string())?;
    for r in &roots {
        tx.execute("INSERT OR IGNORE INTO roots(path) VALUES (?1)", params![r])
            .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) mod tracks;

pub(crate) mod stats;

pub(crate) mod playlists;

// ---- Key/value (settings + playback state) ---------------------------------

#[tauri::command]
pub fn db_kv_get(db: State<Db>, key: String) -> Result<Option<Value>, String> {
    let conn = db.0.lock();
    let raw: Option<String> = conn
        .query_row("SELECT v FROM kv WHERE k = ?1", params![key], |r| r.get(0))
        .optional_string()?;
    Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
}

#[tauri::command]
pub fn db_kv_set(db: State<Db>, key: String, value: Value) -> Result<(), String> {
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
    // Tracks + roots first (upsert_tracks/set_roots take their own lock).
    db_upsert_tracks(db.clone(), tracks)?;
    db_set_roots(db.clone(), roots)?;

    let mut conn = db.0.lock();
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    if let Some(favs) = state.get("favorites").and_then(|v| v.as_array()) {
        for (i, p) in favs.iter().enumerate() {
            if let Some(path) = p.as_str() {
                tx.execute(
                    "INSERT OR REPLACE INTO favorites(path, position) VALUES (?1, ?2)",
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
                "INSERT OR REPLACE INTO stats(path, play_count, last_played, skip_count) VALUES (?1,?2,?3,?4)",
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
                            "INSERT OR IGNORE INTO playlist_items(playlist_id, path, position) VALUES (?1,?2,?3)",
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
use smart_playlists::{smart_count, smart_eval};

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
    let conn = db.0.lock();

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
        let conn = Connection::open_in_memory().expect("open test database");
        conn.execute_batch(SCHEMA).expect("create test schema");
        migrate(&conn).expect("migrate test schema");
        Db(Mutex::new(conn), DbCache::default())
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
                "INSERT INTO stats(path, play_count, last_played, skip_count)
                 VALUES (?1, 7, 42, 2)",
                params![old_path.to_string_lossy()],
            )
            .expect("insert old stats");

        let removed =
            prune_changed_paths(&db, std::slice::from_ref(&old_path)).expect("prune renamed path");
        assert_eq!(removed, vec![old_path.to_string_lossy().to_string()]);
        let migrated: (i64, i64, i64) =
            db.0.lock()
                .query_row(
                    "SELECT play_count, last_played, skip_count FROM stats WHERE path = ?1",
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
