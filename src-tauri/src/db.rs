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

use std::collections::HashMap;
use std::path::Path;

use parking_lot::Mutex;
use rusqlite::{params, params_from_iter, Connection, Row};
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
    Ok(Db(Mutex::new(conn), DbCache::default()))
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
}

#[derive(Serialize)]
pub struct ArtistRow {
    artist: String,
    track_count: i64,
    album_count: i64,
    plays: i64,
    cover_path: Option<String>,
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
    let mut conn = db.0.lock();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut new_count = 0usize;
    {
        let mut exists = tx
            .prepare("SELECT 1 FROM tracks WHERE path = ?1")
            .map_err(|e| e.to_string())?;
        let mut upsert = tx
            .prepare(
                "INSERT INTO tracks (path, title, artist, album, genre, duration_secs, date_added, year, track_number, has_cover, sample_rate, bit_depth, track_gain_db, track_peak)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
                 ON CONFLICT(path) DO UPDATE SET
                   title=excluded.title, artist=excluded.artist, album=excluded.album,
                   genre=excluded.genre, duration_secs=excluded.duration_secs,
                   date_added=excluded.date_added, year=excluded.year,
                   track_number=excluded.track_number, has_cover=excluded.has_cover,
                   sample_rate=excluded.sample_rate, bit_depth=excluded.bit_depth,
                   track_gain_db=excluded.track_gain_db, track_peak=excluded.track_peak",
            )
            .map_err(|e| e.to_string())?;
        for t in &tracks {
            let is_new = !exists
                .exists(params![t.path])
                .map_err(|e| e.to_string())?;
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

// Delete every track whose file no longer exists on disk. Returns the removed
// paths so the frontend can drop them from the queue / current playback.
#[tauri::command]
pub fn db_prune_missing(db: State<Db>) -> Result<Vec<String>, String> {
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
    let gone: Vec<String> = all
        .into_iter()
        .filter(|p| !Path::new(p).exists())
        .collect();
    for p in &gone {
        conn.execute("DELETE FROM tracks WHERE path = ?1", params![p])
            .map_err(|e| e.to_string())?;
    }
    Ok(gone)
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
    tx.execute("DELETE FROM roots", []).map_err(|e| e.to_string())?;
    for r in &roots {
        tx.execute("INSERT OR IGNORE INTO roots(path) VALUES (?1)", params![r])
            .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

// ---- Track queries ----------------------------------------------------------

#[tauri::command]
pub fn db_tracks_page(
    db: State<Db>,
    sort_by: String,
    order: String,
    search: Option<String>,
    offset: i64,
    limit: i64,
) -> Result<Page, String> {
    let conn = db.0.lock();
    let d = dir(&order);
    let query = search.as_deref().and_then(fts_query);

    if let Some(q) = query {
        // JOIN with tracks_fts: qualify the sort columns with `t.` so shared
        // column names (title/artist/album) aren't ambiguous.
        let sort = sort_col(&sort_by, "t.");
        let total: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tracks_fts WHERE tracks_fts MATCH ?1",
                params![q],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let sql = format!(
            "SELECT {TRACK_COLS_T} FROM tracks t JOIN tracks_fts f ON f.rowid = t.id
             WHERE tracks_fts MATCH ?1 ORDER BY {sort} {d} LIMIT ?2 OFFSET ?3"
        );
        let tracks = collect_tracks(&conn, &sql, params![q, limit, offset])?;
        Ok(Page { total, tracks })
    } else {
        let sort = sort_col(&sort_by, "");
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let sql = format!(
            "SELECT {TRACK_COLS} FROM tracks ORDER BY {sort} {d} LIMIT ?1 OFFSET ?2"
        );
        let tracks = collect_tracks(&conn, &sql, params![limit, offset])?;
        Ok(Page { total, tracks })
    }
}

#[tauri::command]
pub fn db_search(db: State<Db>, query: String, limit: i64) -> Result<Vec<MusicTrack>, String> {
    let conn = db.0.lock();
    match fts_query(&query) {
        None => Ok(Vec::new()),
        Some(q) => {
            let sql = format!(
                "SELECT {TRACK_COLS_T} FROM tracks t JOIN tracks_fts f ON f.rowid = t.id
                 WHERE tracks_fts MATCH ?1 ORDER BY rank LIMIT ?2"
            );
            collect_tracks(&conn, &sql, params![q, limit])
        }
    }
}

// Hydrate a list of paths into full track objects, preserving input order (used
// to rebuild the play queue and playlist views from stored paths).
#[tauri::command]
pub fn db_tracks_by_paths(db: State<Db>, paths: Vec<String>) -> Result<Vec<MusicTrack>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let conn = db.0.lock();
    let placeholders = vec!["?"; paths.len()].join(",");
    let sql = format!("SELECT {TRACK_COLS} FROM tracks WHERE path IN ({placeholders})");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params_from_iter(paths.iter()), row_to_track)
        .map_err(|e| e.to_string())?;
    let mut by_path = std::collections::HashMap::new();
    for r in rows {
        let t = r.map_err(|e| e.to_string())?;
        by_path.insert(t.path.clone(), t);
    }
    // Clone (don't remove) so a path repeated in the queue stays repeated.
    Ok(paths.iter().filter_map(|p| by_path.get(p).cloned()).collect())
}

#[tauri::command]
pub fn db_track(db: State<Db>, path: String) -> Result<Option<MusicTrack>, String> {
    let conn = db.0.lock();
    let sql = format!("SELECT {TRACK_COLS} FROM tracks WHERE path = ?1");
    let mut tracks = collect_tracks(&conn, &sql, params![path])?;
    Ok(tracks.pop())
}

#[tauri::command]
pub fn db_random_track(db: State<Db>, exclude: Option<String>) -> Result<Option<MusicTrack>, String> {
    let conn = db.0.lock();
    let sql = format!(
        "SELECT {TRACK_COLS} FROM tracks WHERE path <> ?1 ORDER BY RANDOM() LIMIT 1"
    );
    let mut tracks = collect_tracks(&conn, &sql, params![exclude.unwrap_or_default()])?;
    Ok(tracks.pop())
}

// ---- Albums / artists / genres ---------------------------------------------

#[tauri::command]
pub fn db_albums(db: State<Db>, search: Option<String>) -> Result<Vec<AlbumRow>, String> {
    let conn = db.0.lock();
    let like = search
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| format!("%{}%", s.trim()));
    let sql = "SELECT album, MIN(artist) AS artist, MAX(year) AS year, COUNT(*) AS n,
                 (SELECT path FROM tracks t2 WHERE t2.album = t.album AND t2.has_cover = 1 LIMIT 1) AS cover
               FROM tracks t
               WHERE (?1 IS NULL OR album LIKE ?1 OR artist LIKE ?1)
               GROUP BY album ORDER BY album COLLATE NOCASE";
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![like], |r| {
            Ok(AlbumRow {
                album: r.get(0)?,
                artist: r.get(1)?,
                year: r.get(2)?,
                track_count: r.get(3)?,
                cover_path: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn db_album_tracks(db: State<Db>, album: String) -> Result<Vec<MusicTrack>, String> {
    let conn = db.0.lock();
    let sql = format!(
        "SELECT {TRACK_COLS} FROM tracks WHERE album = ?1 ORDER BY track_number, title COLLATE NOCASE"
    );
    collect_tracks(&conn, &sql, params![album])
}

#[tauri::command]
pub fn db_artists(db: State<Db>, search: Option<String>) -> Result<Vec<ArtistRow>, String> {
    let conn = db.0.lock();
    let like = search
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| format!("%{}%", s.trim()));
    let sql = "SELECT t.artist, COUNT(*) AS n, COUNT(DISTINCT t.album) AS albums,
                 COALESCE(SUM(s.play_count), 0) AS plays,
                 (SELECT path FROM tracks t2 WHERE t2.artist = t.artist AND t2.has_cover = 1 LIMIT 1) AS cover
               FROM tracks t LEFT JOIN stats s ON s.path = t.path
               WHERE t.artist <> '' AND (?1 IS NULL OR t.artist LIKE ?1)
               GROUP BY t.artist ORDER BY t.artist COLLATE NOCASE";
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![like], |r| {
            Ok(ArtistRow {
                artist: r.get(0)?,
                track_count: r.get(1)?,
                album_count: r.get(2)?,
                plays: r.get(3)?,
                cover_path: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn db_artist_tracks(db: State<Db>, artist: String) -> Result<Vec<MusicTrack>, String> {
    let conn = db.0.lock();
    let sql = format!(
        "SELECT {TRACK_COLS} FROM tracks WHERE artist = ?1
         ORDER BY album COLLATE NOCASE, track_number, title COLLATE NOCASE"
    );
    collect_tracks(&conn, &sql, params![artist])
}

// Every track for a "station" (all songs by an artist, or in a genre). The
// frontend shuffles the result into a radio-style queue.
#[tauri::command]
pub fn db_station_tracks(db: State<Db>, kind: String, key: String) -> Result<Vec<MusicTrack>, String> {
    let conn = db.0.lock();
    let sql = if kind == "genre" {
        format!("SELECT {TRACK_COLS} FROM tracks WHERE genre = ?1")
    } else {
        format!("SELECT {TRACK_COLS} FROM tracks WHERE artist = ?1")
    };
    collect_tracks(&conn, &sql, params![key])
}

// Whether any track carries a non-empty genre (drives the smart-playlist editor's
// "genre needs reindex" hint).
#[tauri::command]
pub fn db_has_genre(db: State<Db>) -> Result<bool, String> {
    let conn = db.0.lock();
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM tracks WHERE genre IS NOT NULL AND genre <> '')",
        [],
        |r| r.get::<_, i64>(0).map(|v| v != 0),
    )
    .map_err(|e| e.to_string())
}

// Evaluate ad-hoc smart-playlist rules (used by the editor's live preview before
// the playlist is saved). Same engine as db_playlist_tracks for saved ones.
#[tauri::command]
pub fn db_smart_tracks(
    db: State<Db>,
    rules: Value,
    sort_by: Option<String>,
    sort_order: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<MusicTrack>, String> {
    let conn = db.0.lock();
    smart_eval(
        &conn,
        &rules,
        sort_by.as_deref().unwrap_or("none"),
        sort_order.as_deref().unwrap_or("asc"),
        limit.unwrap_or(0),
    )
}

// ---- Play statistics --------------------------------------------------------

#[tauri::command]
pub fn db_record_play_start(db: State<Db>, path: String) -> Result<(), String> {
    let conn = db.0.lock();
    conn.execute(
        "INSERT INTO stats(path, play_count, last_played, skip_count) VALUES (?1, 0, ?2, 0)
         ON CONFLICT(path) DO UPDATE SET last_played = ?2",
        params![path, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn db_record_play(db: State<Db>, path: String) -> Result<(), String> {
    let conn = db.0.lock();
    conn.execute(
        "INSERT INTO stats(path, play_count, last_played, skip_count) VALUES (?1, 1, ?2, 0)
         ON CONFLICT(path) DO UPDATE SET play_count = play_count + 1, last_played = ?2",
        params![path, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn db_record_skip(db: State<Db>, path: String) -> Result<(), String> {
    let conn = db.0.lock();
    conn.execute(
        "INSERT INTO stats(path, play_count, last_played, skip_count) VALUES (?1, 0, 0, 1)
         ON CONFLICT(path) DO UPDATE SET skip_count = skip_count + 1",
        params![path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn db_stat(db: State<Db>, path: String) -> Result<StatRow, String> {
    let conn = db.0.lock();
    conn.query_row(
        "SELECT play_count, last_played, skip_count FROM stats WHERE path = ?1",
        params![path],
        |r| {
            Ok(StatRow {
                play_count: r.get(0)?,
                last_played: r.get(1)?,
                skip_count: r.get(2)?,
            })
        },
    )
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(StatRow::default()),
        other => Err(other.to_string()),
    })
}

#[tauri::command]
pub fn db_stats_summary(db: State<Db>) -> Result<StatsSummary, String> {
    let conn = db.0.lock();
    conn.query_row(
        "SELECT COALESCE(SUM(s.play_count), 0), COALESCE(SUM(s.play_count * t.duration_secs), 0)
         FROM stats s JOIN tracks t ON t.path = s.path",
        [],
        |r| {
            Ok(StatsSummary {
                total_plays: r.get(0)?,
                total_seconds: r.get(1)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

// ---- Insight collections ----------------------------------------------------

#[tauri::command]
pub fn db_recently_played(db: State<Db>, limit: i64) -> Result<Vec<MusicTrack>, String> {
    let conn = db.0.lock();
    let sql = format!(
        "SELECT {TRACK_COLS_T} FROM tracks t JOIN stats s ON s.path = t.path
         WHERE s.last_played > 0 ORDER BY s.last_played DESC LIMIT ?1"
    );
    collect_tracks(&conn, &sql, params![limit])
}

#[tauri::command]
pub fn db_most_played(db: State<Db>, limit: i64) -> Result<Vec<MusicTrack>, String> {
    let conn = db.0.lock();
    let sql = format!(
        "SELECT {TRACK_COLS_T} FROM tracks t JOIN stats s ON s.path = t.path
         WHERE s.play_count > 0 ORDER BY s.play_count DESC LIMIT ?1"
    );
    collect_tracks(&conn, &sql, params![limit])
}

#[tauri::command]
pub fn db_on_repeat(db: State<Db>, limit: i64) -> Result<Vec<MusicTrack>, String> {
    let conn = db.0.lock();
    let cutoff = now_ms() - 45 * 86_400_000;
    let sql = format!(
        "SELECT {TRACK_COLS_T} FROM tracks t JOIN stats s ON s.path = t.path
         WHERE s.play_count >= 2 AND s.last_played >= ?1
         ORDER BY s.play_count DESC, s.last_played DESC LIMIT ?2"
    );
    collect_tracks(&conn, &sql, params![cutoff, limit])
}

#[tauri::command]
pub fn db_recently_added(db: State<Db>, limit: i64) -> Result<Vec<MusicTrack>, String> {
    let conn = db.0.lock();
    let sql = format!(
        "SELECT {TRACK_COLS} FROM tracks ORDER BY date_added DESC LIMIT ?1"
    );
    collect_tracks(&conn, &sql, params![limit])
}

#[tauri::command]
pub fn db_rediscover(db: State<Db>, limit: i64) -> Result<Vec<MusicTrack>, String> {
    let conn = db.0.lock();
    let cutoff = now_ms() - 60 * 86_400_000;
    let sql = format!(
        "SELECT {TRACK_COLS_T} FROM tracks t JOIN favorites f ON f.path = t.path
         LEFT JOIN stats s ON s.path = t.path
         WHERE COALESCE(s.last_played, 0) = 0 OR s.last_played < ?1
         ORDER BY RANDOM() LIMIT ?2"
    );
    collect_tracks(&conn, &sql, params![cutoff, limit])
}

#[tauri::command]
pub fn db_top_artists(db: State<Db>, limit: i64) -> Result<Vec<ArtistRow>, String> {
    let conn = db.0.lock();
    let sql = "SELECT t.artist, COUNT(*) AS n, COUNT(DISTINCT t.album) AS albums,
                 COALESCE(SUM(s.play_count), 0) AS plays,
                 (SELECT path FROM tracks t2 WHERE t2.artist = t.artist AND t2.has_cover = 1 LIMIT 1) AS cover
               FROM tracks t LEFT JOIN stats s ON s.path = t.path
               WHERE t.artist <> '' AND t.artist <> 'Unknown Artist'
               GROUP BY t.artist ORDER BY plays DESC, n DESC LIMIT ?1";
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![limit], |r| {
            Ok(ArtistRow {
                artist: r.get(0)?,
                track_count: r.get(1)?,
                album_count: r.get(2)?,
                plays: r.get(3)?,
                cover_path: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn db_top_genres(db: State<Db>, limit: i64) -> Result<Vec<GenreRow>, String> {
    let conn = db.0.lock();
    let sql = "SELECT t.genre, COUNT(*) AS n, COALESCE(SUM(s.play_count), 0) AS plays,
                 (SELECT path FROM tracks t2 WHERE t2.genre = t.genre AND t2.has_cover = 1 LIMIT 1) AS cover
               FROM tracks t LEFT JOIN stats s ON s.path = t.path
               WHERE t.genre IS NOT NULL AND t.genre <> ''
               GROUP BY t.genre ORDER BY plays DESC, n DESC LIMIT ?1";
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![limit], |r| {
            Ok(GenreRow {
                genre: r.get(0)?,
                track_count: r.get(1)?,
                plays: r.get(2)?,
                cover_path: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[derive(Serialize)]
pub struct InsightCounts {
    recently_played: i64,
    most_played: i64,
    on_repeat: i64,
    recently_added: i64,
    rediscover: i64,
}

// Cheap COUNTs for the Home "Top Picks" cards, which only need to know which
// collections are non-empty — avoids fetching hundreds of full tracks each on
// every stats change.
#[tauri::command]
pub fn db_insight_counts(db: State<Db>) -> Result<InsightCounts, String> {
    let conn = db.0.lock();
    let cutoff45 = now_ms() - 45 * 86_400_000;
    let cutoff60 = now_ms() - 60 * 86_400_000;
    let one = |sql: &str, args: &[&dyn rusqlite::ToSql]| -> Result<i64, String> {
        conn.query_row(sql, args, |r| r.get(0)).map_err(|e| e.to_string())
    };
    Ok(InsightCounts {
        recently_played: one(
            "SELECT COUNT(*) FROM stats s JOIN tracks t ON t.path = s.path WHERE s.last_played > 0",
            params![],
        )?,
        most_played: one(
            "SELECT COUNT(*) FROM stats s JOIN tracks t ON t.path = s.path WHERE s.play_count > 0",
            params![],
        )?,
        on_repeat: one(
            "SELECT COUNT(*) FROM stats s JOIN tracks t ON t.path = s.path
             WHERE s.play_count >= 2 AND s.last_played >= ?1",
            params![cutoff45],
        )?,
        recently_added: one("SELECT COUNT(*) FROM tracks", params![])?,
        rediscover: one(
            "SELECT COUNT(*) FROM favorites f JOIN tracks t ON t.path = f.path
             LEFT JOIN stats s ON s.path = f.path
             WHERE COALESCE(s.last_played, 0) = 0 OR s.last_played < ?1",
            params![cutoff60],
        )?,
    })
}

// ---- Favorites --------------------------------------------------------------

#[tauri::command]
pub fn db_favorite_paths(db: State<Db>) -> Result<Vec<String>, String> {
    let conn = db.0.lock();
    let mut stmt = conn
        .prepare("SELECT path FROM favorites ORDER BY position")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn db_favorites(db: State<Db>) -> Result<Vec<MusicTrack>, String> {
    let conn = db.0.lock();
    let sql = format!(
        "SELECT {TRACK_COLS_T} FROM tracks t JOIN favorites f ON f.path = t.path ORDER BY f.position"
    );
    collect_tracks(&conn, &sql, params![])
}

// Toggle favorite; returns the new state (true = now favorited).
#[tauri::command]
pub fn db_toggle_favorite(db: State<Db>, path: String) -> Result<bool, String> {
    let conn = db.0.lock();
    let exists: bool = conn
        .query_row("SELECT 1 FROM favorites WHERE path = ?1", params![path], |_| Ok(true))
        .unwrap_or(false);
    if exists {
        conn.execute("DELETE FROM favorites WHERE path = ?1", params![path])
            .map_err(|e| e.to_string())?;
        Ok(false)
    } else {
        let next: i64 = conn
            .query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM favorites", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO favorites(path, position) VALUES (?1, ?2)",
            params![path, next],
        )
        .map_err(|e| e.to_string())?;
        Ok(true)
    }
}

#[tauri::command]
pub fn db_move_favorite(db: State<Db>, from: i64, to: i64) -> Result<(), String> {
    let mut conn = db.0.lock();
    let mut paths: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT path FROM favorites ORDER BY position")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let (from, to) = (from as usize, to as usize);
    if from >= paths.len() || to >= paths.len() {
        return Ok(());
    }
    let item = paths.remove(from);
    paths.insert(to, item);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (i, p) in paths.iter().enumerate() {
        tx.execute(
            "UPDATE favorites SET position = ?1 WHERE path = ?2",
            params![i as i64, p],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

// ---- Playlists (normal + smart share this table) ----------------------------

fn read_playlists(conn: &Connection) -> Result<Vec<PlaylistRow>, String> {
    let mut out = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT p.id, p.name, p.description, p.color, p.cover, p.is_smart, p.rules,
                        p.sort_by, p.sort_order, p.limit_n, p.live_update,
                        (SELECT COUNT(*) FROM playlist_items i WHERE i.playlist_id = p.id) AS n
                 FROM playlists p ORDER BY p.position",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                let rules_str: Option<String> = r.get(6)?;
                Ok(PlaylistRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    description: r.get(2)?,
                    color: r.get(3)?,
                    cover: r.get(4)?,
                    is_smart: r.get::<_, i64>(5)? != 0,
                    rules: rules_str.and_then(|s| serde_json::from_str(&s).ok()),
                    sort_by: r.get(7)?,
                    sort_order: r.get(8)?,
                    limit_n: r.get(9)?,
                    live_update: r.get::<_, Option<i64>>(10)?.unwrap_or(1) != 0,
                    track_count: r.get(11)?,
                })
            })
            .map_err(|e| e.to_string())?;
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
    }
    Ok(out)
}

#[tauri::command]
pub fn db_playlists(db: State<Db>) -> Result<Vec<PlaylistRow>, String> {
    let conn = db.0.lock();
    let mut rows = read_playlists(&conn)?;

    // Smart playlists have no playlist_items; their count is the number of tracks
    // their rules currently match. That's a full-library scan per playlist, so
    // memoize it against a cheap library fingerprint + the rules JSON — only
    // recompute when the library or the rules actually changed.
    let fp = library_fingerprint(&conn);
    let mut cache = db.1.smart_counts.lock();
    for pl in rows.iter_mut() {
        if !pl.is_smart {
            continue;
        }
        let Some(rules) = &pl.rules else { continue };
        let rules_json = rules.to_string();
        if let Some((cfp, crules, count)) = cache.get(&pl.id) {
            if *cfp == fp && *crules == rules_json {
                pl.track_count = *count;
                continue;
            }
        }
        let n = smart_eval(
            &conn,
            rules,
            pl.sort_by.as_deref().unwrap_or("none"),
            pl.sort_order.as_deref().unwrap_or("asc"),
            pl.limit_n.unwrap_or(0),
        )?
        .len() as i64;
        cache.insert(pl.id.clone(), (fp, rules_json, n));
        pl.track_count = n;
    }
    Ok(rows)
}

// Normal playlist → its items in order; smart playlist → evaluated rules.
#[tauri::command]
pub fn db_playlist_tracks(db: State<Db>, id: String) -> Result<Vec<MusicTrack>, String> {
    let conn = db.0.lock();
    let smart: Option<(i64, Option<String>, Option<String>, Option<String>, Option<i64>)> = conn
        .query_row(
            "SELECT is_smart, rules, sort_by, sort_order, limit_n FROM playlists WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional_string()?;
    let Some((is_smart, rules, sort_by, sort_order, limit_n)) = smart else {
        return Ok(Vec::new());
    };
    if is_smart != 0 {
        let rules_val: Value = rules
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        return smart_eval(
            &conn,
            &rules_val,
            sort_by.as_deref().unwrap_or("none"),
            sort_order.as_deref().unwrap_or("asc"),
            limit_n.unwrap_or(0),
        );
    }
    let sql = format!(
        "SELECT {TRACK_COLS_T} FROM playlist_items i JOIN tracks t ON t.path = i.path
         WHERE i.playlist_id = ?1 ORDER BY i.position"
    );
    collect_tracks(&conn, &sql, params![id])
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn db_upsert_playlist(
    db: State<Db>,
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
    live_update: Option<bool>,
) -> Result<(), String> {
    let conn = db.0.lock();
    let rules_str = rules.map(|v| v.to_string());
    // Preserve existing position on update; append to the end on insert.
    let next_pos: i64 = conn
        .query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM playlists", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO playlists (id, name, description, color, cover, position, is_smart, rules, sort_by, sort_order, limit_n, live_update)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
         ON CONFLICT(id) DO UPDATE SET
           name=excluded.name, description=excluded.description, color=excluded.color,
           cover=excluded.cover, is_smart=excluded.is_smart, rules=excluded.rules,
           sort_by=excluded.sort_by, sort_order=excluded.sort_order,
           limit_n=excluded.limit_n, live_update=excluded.live_update",
        params![
            id,
            name,
            description,
            color,
            cover,
            next_pos,
            is_smart as i64,
            rules_str,
            sort_by,
            sort_order,
            limit_n,
            live_update.map(|b| b as i64),
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn db_delete_playlist(db: State<Db>, id: String) -> Result<(), String> {
    let mut conn = db.0.lock();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM playlist_items WHERE playlist_id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM playlists WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn db_move_playlist_order(db: State<Db>, from: i64, to: i64) -> Result<(), String> {
    let mut conn = db.0.lock();
    let mut ids: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT id FROM playlists ORDER BY position")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let (from, to) = (from as usize, to as usize);
    if from >= ids.len() || to >= ids.len() {
        return Ok(());
    }
    let item = ids.remove(from);
    ids.insert(to, item);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (i, id) in ids.iter().enumerate() {
        tx.execute(
            "UPDATE playlists SET position = ?1 WHERE id = ?2",
            params![i as i64, id],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn db_playlist_add(db: State<Db>, id: String, paths: Vec<String>) -> Result<(), String> {
    let mut conn = db.0.lock();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut next: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_items WHERE playlist_id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    for p in &paths {
        let inserted = tx
            .execute(
                "INSERT OR IGNORE INTO playlist_items(playlist_id, path, position) VALUES (?1, ?2, ?3)",
                params![id, p, next],
            )
            .map_err(|e| e.to_string())?;
        if inserted > 0 {
            next += 1;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn db_playlist_remove(db: State<Db>, id: String, path: String) -> Result<(), String> {
    let conn = db.0.lock();
    conn.execute(
        "DELETE FROM playlist_items WHERE playlist_id = ?1 AND path = ?2",
        params![id, path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn db_playlist_move_item(db: State<Db>, id: String, from: i64, to: i64) -> Result<(), String> {
    let mut conn = db.0.lock();
    let mut paths: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT path FROM playlist_items WHERE playlist_id = ?1 ORDER BY position")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let (from, to) = (from as usize, to as usize);
    if from >= paths.len() || to >= paths.len() {
        return Ok(());
    }
    let item = paths.remove(from);
    paths.insert(to, item);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (i, p) in paths.iter().enumerate() {
        tx.execute(
            "UPDATE playlist_items SET position = ?1 WHERE playlist_id = ?2 AND path = ?3",
            params![i as i64, id, p],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

// ---- Recents ----------------------------------------------------------------

#[tauri::command]
pub fn db_recents(db: State<Db>) -> Result<Vec<RecentRow>, String> {
    let conn = db.0.lock();
    let mut stmt = conn
        .prepare("SELECT type, key, ts FROM recents ORDER BY ts DESC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(RecentRow {
                kind: r.get(0)?,
                key: r.get(1)?,
                ts: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn db_record_recent(db: State<Db>, kind: String, key: String) -> Result<(), String> {
    let conn = db.0.lock();
    conn.execute(
        "INSERT INTO recents(type, key, ts) VALUES (?1, ?2, ?3)
         ON CONFLICT(type, key) DO UPDATE SET ts = ?3",
        params![kind, key, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    // Keep only the 40 most recent.
    conn.execute(
        "DELETE FROM recents WHERE (type, key) NOT IN
           (SELECT type, key FROM recents ORDER BY ts DESC LIMIT 40)",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

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
            let id = pl.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
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

// ---- Smart-playlist evaluation (native port of smartPlaylists.js) -----------

// A track plus the extra per-track signals smart rules can test.
struct SmartTrack {
    track: MusicTrack,
    play_count: i64,
    last_played: i64,
    favorite: bool,
}

fn smart_eval(
    conn: &Connection,
    rules: &Value,
    sort_by: &str,
    sort_order: &str,
    limit: i64,
) -> Result<Vec<MusicTrack>, String> {
    // Load every track with its stats + favorite flag in one pass.
    let sql = format!(
        "SELECT {TRACK_COLS_T}, COALESCE(s.play_count, 0), COALESCE(s.last_played, 0),
                CASE WHEN f.path IS NULL THEN 0 ELSE 1 END
         FROM tracks t
         LEFT JOIN stats s ON s.path = t.path
         LEFT JOIN favorites f ON f.path = t.path"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SmartTrack {
                track: row_to_track(r)?,
                play_count: r.get(14)?,
                last_played: r.get(15)?,
                favorite: r.get::<_, i64>(16)? != 0,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut items: Vec<SmartTrack> = Vec::new();
    for r in rows {
        items.push(r.map_err(|e| e.to_string())?);
    }

    let match_all = rules
        .get("match")
        .and_then(|v| v.as_str())
        .map(|s| s != "any")
        .unwrap_or(true);
    let empty = Vec::new();
    let conditions: Vec<&Value> = rules
        .get("conditions")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty)
        .iter()
        .filter(|c| c.get("field").is_some() && c.get("op").is_some())
        .collect();

    let now = now_ms();
    let mut filtered: Vec<SmartTrack> = items
        .into_iter()
        .filter(|it| {
            if conditions.is_empty() {
                return true;
            }
            if match_all {
                conditions.iter().all(|c| match_condition(c, it, now))
            } else {
                conditions.iter().any(|c| match_condition(c, it, now))
            }
        })
        .collect();

    sort_smart(&mut filtered, sort_by, sort_order);

    let mut out: Vec<MusicTrack> = filtered.into_iter().map(|it| it.track).collect();
    if limit > 0 && (out.len() as i64) > limit {
        out.truncate(limit as usize);
    }
    Ok(out)
}

// Comparable numeric value for the number/date sort + comparison ops.
fn field_number(field: &str, it: &SmartTrack) -> f64 {
    match field {
        "year" => it.track.year.unwrap_or(0) as f64,
        "duration" => it.track.duration_secs as f64,
        "playCount" => it.play_count as f64,
        "lastPlayed" => it.last_played as f64,
        "dateAdded" => (it.track.date_added as f64) * 1000.0, // seconds → ms epoch
        _ => 0.0,
    }
}

fn field_text<'a>(field: &str, it: &'a SmartTrack) -> &'a str {
    match field {
        "title" => &it.track.title,
        "artist" => &it.track.artist,
        "album" => &it.track.album,
        "genre" => it.track.genre.as_deref().unwrap_or(""),
        _ => "",
    }
}

fn match_condition(cond: &Value, it: &SmartTrack, now: i64) -> bool {
    let field = cond.get("field").and_then(|v| v.as_str()).unwrap_or("");
    let op = cond.get("op").and_then(|v| v.as_str()).unwrap_or("");
    let val = cond.get("value");

    match field {
        // Text fields.
        "title" | "artist" | "album" | "genre" => {
            let a = field_text(field, it).to_lowercase();
            let b = val
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            match op {
                "contains" => a.contains(&b),
                "notContains" => !a.contains(&b),
                "is" => a == b,
                "isNot" => a != b,
                "startsWith" => a.starts_with(&b),
                "endsWith" => a.ends_with(&b),
                _ => true,
            }
        }
        // Numeric fields (year, duration, playCount).
        "year" | "duration" | "playCount" => {
            let a = field_number(field, it);
            let b = val
                .map(json_to_f64)
                .unwrap_or(0.0);
            match op {
                "is" => a == b,
                "isNot" => a != b,
                "gt" => a > b,
                "lt" => a < b,
                "gte" => a >= b,
                "lte" => a <= b,
                _ => true,
            }
        }
        // Date fields (lastPlayed, dateAdded) — value is a day count.
        "lastPlayed" | "dateAdded" => {
            let a = field_number(field, it); // ms epoch, 0 = never
            let days = val.map(json_to_f64).unwrap_or(0.0);
            let cutoff = now as f64 - days * 86_400_000.0;
            match op {
                "inLast" => a > 0.0 && a >= cutoff,
                "notInLast" => a == 0.0 || a < cutoff,
                "played" => a > 0.0,
                "never" => a == 0.0,
                _ => true,
            }
        }
        // Boolean.
        "favorite" => {
            if op == "isFalse" {
                !it.favorite
            } else {
                it.favorite
            }
        }
        _ => true,
    }
}

fn json_to_f64(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s.trim().parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn sort_smart(items: &mut [SmartTrack], sort_by: &str, sort_order: &str) {
    if sort_by.is_empty() || sort_by == "none" {
        return;
    }
    if sort_by == "random" {
        // Fisher–Yates using a cheap xorshift seeded from the clock.
        let mut seed = now_ms() as u64 | 1;
        let mut rng = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for i in (1..items.len()).rev() {
            let j = (rng() % (i as u64 + 1)) as usize;
            items.swap(i, j);
        }
        return;
    }
    let desc = sort_order.eq_ignore_ascii_case("desc");
    let is_text = matches!(sort_by, "title" | "artist" | "album" | "genre");
    items.sort_by(|a, b| {
        let ord = if is_text {
            field_text(sort_by, a)
                .to_lowercase()
                .cmp(&field_text(sort_by, b).to_lowercase())
        } else {
            field_number(sort_by, a)
                .partial_cmp(&field_number(sort_by, b))
                .unwrap_or(std::cmp::Ordering::Equal)
        };
        if desc {
            ord.reverse()
        } else {
            ord
        }
    });
}

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
