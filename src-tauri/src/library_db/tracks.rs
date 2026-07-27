//! Track, album, artist, genre, search, and station queries.

use rusqlite::{params, params_from_iter};
use serde_json::Value;
use tauri::State;

use crate::{limits, MusicTrack};

use super::{
    collect_tracks, dir, fts_query, row_to_track, smart_eval, sort_col, AlbumRow, ArtistRow, Db,
    Page, TRACK_COLS, TRACK_COLS_T,
};

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
    limits::validate_text(&sort_by, "Track sort", 32)?;
    limits::validate_text(&order, "Track sort order", 8)?;
    if let Some(search) = search.as_deref() {
        limits::validate_text(search, "Track search", 512)?;
    }
    if offset < 0 || limit <= 0 || limit > 1_000 {
        return Err("Invalid track page bounds".to_string());
    }
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
        let sql = format!("SELECT {TRACK_COLS} FROM tracks ORDER BY {sort} {d} LIMIT ?1 OFFSET ?2");
        let tracks = collect_tracks(&conn, &sql, params![limit, offset])?;
        Ok(Page { total, tracks })
    }
}

#[tauri::command]
pub fn db_search(db: State<Db>, query: String, limit: i64) -> Result<Vec<MusicTrack>, String> {
    limits::validate_text(&query, "Search query", 512)?;
    if !(1..=100).contains(&limit) {
        return Err("Search limit must be between 1 and 100".to_string());
    }
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
    limits::validate_paths(&paths, limits::MAX_QUEUE_ENTRIES)?;
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
    Ok(paths
        .iter()
        .filter_map(|p| by_path.get(p).cloned())
        .collect())
}

#[tauri::command]
pub fn db_track(db: State<Db>, path: String) -> Result<Option<MusicTrack>, String> {
    let conn = db.0.lock();
    let sql = format!("SELECT {TRACK_COLS} FROM tracks WHERE path = ?1");
    let mut tracks = collect_tracks(&conn, &sql, params![path])?;
    Ok(tracks.pop())
}

#[tauri::command]
pub fn db_random_track(
    db: State<Db>,
    exclude: Option<String>,
) -> Result<Option<MusicTrack>, String> {
    let conn = db.0.lock();
    let sql = format!("SELECT {TRACK_COLS} FROM tracks WHERE path <> ?1 ORDER BY RANDOM() LIMIT 1");
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
    let sql = "SELECT t.album, MIN(t.artist) AS artist, MAX(t.year) AS year, COUNT(*) AS n,
                 (SELECT path FROM tracks t2 WHERE t2.album = t.album AND t2.has_cover = 1 LIMIT 1) AS cover,
                 COALESCE(MAX(s.last_played), 0) AS last_played,
                 GROUP_CONCAT(DISTINCT t.artist) AS all_artists
               FROM tracks t LEFT JOIN stats s ON s.path = t.path
               WHERE (?1 IS NULL OR t.album LIKE ?1 OR t.artist LIKE ?1)
               GROUP BY t.album ORDER BY t.album COLLATE NOCASE";
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![like], |r| {
            Ok(AlbumRow {
                album: r.get(0)?,
                artist: r.get(1)?,
                year: r.get(2)?,
                track_count: r.get(3)?,
                cover_path: r.get(4)?,
                last_played: r.get(5)?,
                all_artists: r.get(6)?,
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
                 (SELECT path FROM tracks t2 WHERE t2.artist = t.artist AND t2.has_cover = 1 LIMIT 1) AS cover,
                 COALESCE(MAX(s.last_played), 0) AS last_played
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
                last_played: r.get(5)?,
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
pub fn db_station_tracks(
    db: State<Db>,
    kind: String,
    key: String,
) -> Result<Vec<MusicTrack>, String> {
    limits::validate_text(&kind, "Station kind", 16)?;
    limits::validate_text(&key, "Station key", 1_024)?;
    if !matches!(kind.as_str(), "genre" | "artist") {
        return Err("Station kind must be genre or artist".to_string());
    }
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
    limits::validate_json(
        &rules,
        "Smart-playlist rules",
        limits::MAX_RULES_BYTES,
        limits::MAX_JSON_DEPTH,
    )?;
    if let Some(sort_by) = sort_by.as_deref() {
        limits::validate_text(sort_by, "Smart-playlist sort", 32)?;
    }
    if let Some(sort_order) = sort_order.as_deref() {
        if !matches!(sort_order, "asc" | "desc") {
            return Err("Smart-playlist sort order must be asc or desc".to_string());
        }
    }
    if limit.is_some_and(|limit| !(0..=100_000).contains(&limit)) {
        return Err("Smart-playlist limit must be between 0 and 100000".to_string());
    }
    let conn = db.0.lock();
    smart_eval(
        &conn,
        &rules,
        sort_by.as_deref().unwrap_or("none"),
        sort_order.as_deref().unwrap_or("asc"),
        limit.unwrap_or(0),
    )
}
