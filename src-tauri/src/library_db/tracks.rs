//! Track, album, artist, genre, search, and station queries.

use std::collections::{HashMap, HashSet};

use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Connection};
use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::{limits, MusicTrack};

use super::{
    collect_tracks, dir, fts_query, random_u64, row_to_track, row_to_track_at, smart_count,
    smart_eval, validate_smart_request, AlbumRow, ArtistRow, Db, GenreRow, Page, StationSession,
    TRACK_COLS, TRACK_COLS_T,
};

// ---- Track queries ----------------------------------------------------------

fn page_sort_parts(sort_by: &str, prefix: &str) -> Vec<String> {
    let text = |column: &str| format!("COALESCE({prefix}{column}, '') COLLATE NOCASE");
    let number = |column: &str| format!("COALESCE({prefix}{column}, 0)");
    match sort_by {
        "artist" => vec![text("artist"), text("album"), number("track_number")],
        "album" => vec![text("album"), number("track_number")],
        "year" => vec![number("year")],
        "duration" | "duration_secs" => vec![number("duration_secs")],
        "dateAdded" | "date_added" => vec![number("first_seen_at")],
        "track_number" => vec![number("track_number")],
        _ => vec![text("title")],
    }
}

fn collect_track_page(
    conn: &Connection,
    sql: &str,
    params: &[SqlValue],
) -> Result<(Vec<MusicTrack>, Option<i64>), String> {
    let mut statement = conn
        .prepare_cached(sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params_from_iter(params.iter()), |row| {
            Ok((row.get::<_, i64>(0)?, row_to_track_at(row, 1)?))
        })
        .map_err(|error| error.to_string())?;
    let mut tracks = Vec::new();
    let mut next_cursor = None;
    for row in rows {
        let (id, track) = row.map_err(|error| error.to_string())?;
        next_cursor = Some(id);
        tracks.push(track);
    }
    Ok((tracks, next_cursor))
}

#[tauri::command]
pub fn db_tracks_page(
    db: State<Db>,
    sort_by: String,
    order: String,
    search: Option<String>,
    offset: i64,
    cursor: Option<i64>,
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
    let conn = db.read();
    let d = dir(&order);
    let query = search.as_deref().and_then(fts_query);
    let total: i64 = if let Some(query) = query.as_ref() {
        conn.query_row(
            "SELECT COUNT(*) FROM tracks_search WHERE tracks_search MATCH ?1",
            params![query],
            |row| row.get(0),
        )
    } else {
        conn.query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))
    }
    .map_err(|error| error.to_string())?;

    let mut sort_parts = page_sort_parts(&sort_by, "t.");
    sort_parts.push("t.id".to_string());
    let mut cursor_parts = page_sort_parts(&sort_by, "c.");
    cursor_parts.push("c.id".to_string());
    let order_by = sort_parts
        .iter()
        .map(|part| format!("{part} {d}"))
        .collect::<Vec<_>>()
        .join(", ");
    let comparison = if d == "DESC" { "<" } else { ">" };
    let mut params = Vec::<SqlValue>::new();
    let mut sql = format!("SELECT t.id, {TRACK_COLS_T} FROM tracks t");
    if query.is_some() {
        sql.push_str(" JOIN tracks_search f ON f.rowid = t.id");
    }
    sql.push_str(" WHERE ");
    if let Some(query) = query {
        sql.push_str("tracks_search MATCH ?");
        params.push(SqlValue::Text(query));
    } else {
        sql.push_str("1 = 1");
    }
    if let Some(cursor) = cursor {
        sql.push_str(&format!(
            " AND ({}) {comparison} (
                SELECT {} FROM tracks c WHERE c.id = ?
              )",
            sort_parts.join(", "),
            cursor_parts.join(", ")
        ));
        params.push(SqlValue::Integer(cursor));
    }
    sql.push_str(&format!(" ORDER BY {order_by} LIMIT ?"));
    params.push(SqlValue::Integer(limit));
    // Compatibility for older callers/bookmarks. The Vue list uses keyset
    // cursors after page one, so deep pages never pay OFFSET's scan cost.
    if cursor.is_none() && offset > 0 {
        sql.push_str(" OFFSET ?");
        params.push(SqlValue::Integer(offset));
    }
    let (tracks, next_cursor) = collect_track_page(&conn, &sql, &params)?;
    Ok(Page {
        total,
        tracks,
        next_cursor,
    })
}

#[tauri::command]
pub fn db_search(db: State<Db>, query: String, limit: i64) -> Result<Vec<MusicTrack>, String> {
    limits::validate_text(&query, "Search query", 512)?;
    if !(1..=100).contains(&limit) {
        return Err("Search limit must be between 1 and 100".to_string());
    }
    let conn = db.read();
    match fts_query(&query) {
        None => Ok(Vec::new()),
        Some(q) => {
            let sql = format!(
                "SELECT {TRACK_COLS_T} FROM tracks t JOIN tracks_search f ON f.rowid = t.id
                 WHERE tracks_search MATCH ?1 ORDER BY rank LIMIT ?2"
            );
            collect_tracks(&conn, &sql, params![q, limit])
        }
    }
}

#[derive(Serialize)]
pub struct GlobalSearchResults {
    request_id: u64,
    songs: Vec<MusicTrack>,
    albums: Vec<AlbumRow>,
    artists: Vec<ArtistRow>,
    genres: Vec<GenreRow>,
}

fn fts_column_query(input: &str, column: &str) -> Option<String> {
    fts_query(input).map(|query| format!("{column}:({query})"))
}

fn like_prefix(input: &str) -> String {
    let escaped = input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("{escaped}%")
}

/// One bounded command-palette search. Each category is ranked natively and
/// capped before it crosses IPC; request_id lets the webview reject stale work.
#[tauri::command]
pub fn db_global_search(
    db: State<Db>,
    query: String,
    song_limit: i64,
    album_limit: i64,
    artist_limit: i64,
    genre_limit: i64,
    request_id: u64,
) -> Result<GlobalSearchResults, String> {
    limits::validate_text(&query, "Global search query", 512)?;
    for (label, value) in [
        ("song", song_limit),
        ("album", album_limit),
        ("artist", artist_limit),
        ("genre", genre_limit),
    ] {
        if !(1..=25).contains(&value) {
            return Err(format!(
                "Global search {label} limit must be between 1 and 25"
            ));
        }
    }
    let conn = db.read();
    global_search(
        &conn,
        &query,
        song_limit,
        album_limit,
        artist_limit,
        genre_limit,
        request_id,
    )
}

fn global_search(
    conn: &Connection,
    query: &str,
    song_limit: i64,
    album_limit: i64,
    artist_limit: i64,
    genre_limit: i64,
    request_id: u64,
) -> Result<GlobalSearchResults, String> {
    let Some(all_query) = fts_query(query) else {
        return Ok(GlobalSearchResults {
            request_id,
            songs: Vec::new(),
            albums: Vec::new(),
            artists: Vec::new(),
            genres: Vec::new(),
        });
    };
    let exact = query.trim();
    let prefix = like_prefix(exact);

    let songs_sql = format!(
        "SELECT {TRACK_COLS_T}
         FROM tracks t JOIN tracks_search ON tracks_search.rowid = t.id
         WHERE tracks_search MATCH ?1
         ORDER BY
           CASE
             WHEN lower(t.title) = lower(?2) THEN 0
             WHEN lower(t.title) LIKE lower(?3) ESCAPE '\\' THEN 1
             WHEN lower(t.artist) = lower(?2) THEN 2
             WHEN lower(t.artist) LIKE lower(?3) ESCAPE '\\' THEN 3
             WHEN lower(t.album) = lower(?2) THEN 4
             WHEN lower(t.album) LIKE lower(?3) ESCAPE '\\' THEN 5
             ELSE 6
           END,
           bm25(tracks_search, 10.0, 4.0, 2.0, 1.0),
           t.title COLLATE NOCASE
         LIMIT ?4"
    );
    let songs = collect_tracks(
        conn,
        &songs_sql,
        params![all_query, exact, prefix, song_limit],
    )?;

    let album_query = fts_column_query(query, "album").expect("non-empty query has an album query");
    let albums_sql = "WITH matches AS MATERIALIZED (
           SELECT rowid, bm25(tracks_search, 0.0, 0.0, 10.0, 0.0) AS relevance
           FROM tracks_search WHERE tracks_search MATCH ?1
         )
         SELECT t.album, MIN(t.artist), MAX(t.year), COUNT(*),
           (SELECT path FROM tracks cover
            WHERE cover.album = t.album AND cover.has_cover = 1 LIMIT 1),
           COALESCE(MAX(s.last_played), 0),
           GROUP_CONCAT(DISTINCT t.artist)
         FROM matches m
         JOIN tracks t ON t.id = m.rowid
         LEFT JOIN stats s ON s.track_id = t.id
         GROUP BY t.album
         ORDER BY
           CASE
             WHEN lower(t.album) = lower(?2) THEN 0
             WHEN lower(t.album) LIKE lower(?3) ESCAPE '\\' THEN 1
             ELSE 2
           END,
           MIN(m.relevance), t.album COLLATE NOCASE
         LIMIT ?4";
    let mut albums_statement = conn
        .prepare(albums_sql)
        .map_err(|error| error.to_string())?;
    let albums = albums_statement
        .query_map(params![album_query, exact, prefix, album_limit], |row| {
            Ok(AlbumRow {
                album: row.get(0)?,
                artist: row.get(1)?,
                year: row.get(2)?,
                track_count: row.get(3)?,
                cover_path: row.get(4)?,
                last_played: row.get(5)?,
                all_artists: row.get(6)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let artist_query =
        fts_column_query(query, "artist").expect("non-empty query has an artist query");
    let artists_sql = "WITH matches AS MATERIALIZED (
           SELECT rowid, bm25(tracks_search, 0.0, 10.0, 0.0, 0.0) AS relevance
           FROM tracks_search WHERE tracks_search MATCH ?1
         )
         SELECT t.artist, COUNT(*), COUNT(DISTINCT t.album),
           COALESCE(SUM(s.play_count), 0),
           (SELECT path FROM tracks cover
            WHERE cover.artist = t.artist AND cover.has_cover = 1 LIMIT 1),
           COALESCE(MAX(s.last_played), 0)
         FROM matches m
         JOIN tracks t ON t.id = m.rowid
         LEFT JOIN stats s ON s.track_id = t.id
         WHERE t.artist <> ''
         GROUP BY t.artist
         ORDER BY
           CASE
             WHEN lower(t.artist) = lower(?2) THEN 0
             WHEN lower(t.artist) LIKE lower(?3) ESCAPE '\\' THEN 1
             ELSE 2
           END,
           MIN(m.relevance), t.artist COLLATE NOCASE
         LIMIT ?4";
    let mut artists_statement = conn
        .prepare(artists_sql)
        .map_err(|error| error.to_string())?;
    let artists = artists_statement
        .query_map(params![artist_query, exact, prefix, artist_limit], |row| {
            Ok(ArtistRow {
                artist: row.get(0)?,
                track_count: row.get(1)?,
                album_count: row.get(2)?,
                plays: row.get(3)?,
                cover_path: row.get(4)?,
                last_played: row.get(5)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let genre_query = fts_column_query(query, "genre").expect("non-empty query has a genre query");
    let genres_sql = "WITH matches AS MATERIALIZED (
           SELECT rowid, bm25(tracks_search, 0.0, 0.0, 0.0, 10.0) AS relevance
           FROM tracks_search WHERE tracks_search MATCH ?1
         )
         SELECT t.genre, COUNT(*), COALESCE(SUM(s.play_count), 0),
           (SELECT path FROM tracks cover
            WHERE cover.genre = t.genre AND cover.has_cover = 1 LIMIT 1)
         FROM matches m
         JOIN tracks t ON t.id = m.rowid
         LEFT JOIN stats s ON s.track_id = t.id
         WHERE t.genre IS NOT NULL AND t.genre <> ''
         GROUP BY t.genre
         ORDER BY
           CASE
             WHEN lower(t.genre) = lower(?2) THEN 0
             WHEN lower(t.genre) LIKE lower(?3) ESCAPE '\\' THEN 1
             ELSE 2
           END,
           MIN(m.relevance), t.genre COLLATE NOCASE
         LIMIT ?4";
    let mut genres_statement = conn
        .prepare(genres_sql)
        .map_err(|error| error.to_string())?;
    let genres = genres_statement
        .query_map(params![genre_query, exact, prefix, genre_limit], |row| {
            Ok(GenreRow {
                genre: row.get(0)?,
                track_count: row.get(1)?,
                plays: row.get(2)?,
                cover_path: row.get(3)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    Ok(GlobalSearchResults {
        request_id,
        songs,
        albums,
        artists,
        genres,
    })
}

// Hydrate a list of paths into full track objects, preserving input order (used
// to rebuild the play queue and playlist views from stored paths).
#[tauri::command]
pub fn db_tracks_by_paths(db: State<Db>, paths: Vec<String>) -> Result<Vec<MusicTrack>, String> {
    tracks_by_paths(db.inner(), &paths)
}

pub(crate) fn tracks_by_paths(db: &Db, paths: &[String]) -> Result<Vec<MusicTrack>, String> {
    limits::validate_paths(paths, limits::MAX_QUEUE_ENTRIES)?;
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let conn = db.read();
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
    let conn = db.read();
    let sql = format!("SELECT {TRACK_COLS} FROM tracks WHERE path = ?1");
    let mut tracks = collect_tracks(&conn, &sql, params![path])?;
    Ok(tracks.pop())
}

#[tauri::command]
pub fn db_random_track(
    db: State<Db>,
    exclude: Option<String>,
) -> Result<Option<MusicTrack>, String> {
    let conn = db.read();
    random_track(&conn, exclude.as_deref().unwrap_or(""))
}

/// Select uniformly from a caller's current result set without using webview
/// randomness. Only the chosen track is hydrated from SQLite.
#[tauri::command]
pub fn db_random_track_from_paths(
    db: State<Db>,
    paths: Vec<String>,
    exclude: Option<String>,
) -> Result<Option<MusicTrack>, String> {
    limits::validate_paths(&paths, limits::MAX_BATCH_PATHS)?;
    let exclude = exclude.as_deref().unwrap_or("");
    let candidates = paths
        .into_iter()
        .filter(|path| path != exclude)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(None);
    }
    let start = (random_u64() % candidates.len() as u64) as usize;
    let conn = db.read();
    let sql = format!("SELECT {TRACK_COLS} FROM tracks WHERE path = ?1");
    let mut statement = conn.prepare(&sql).map_err(|error| error.to_string())?;
    for offset in 0..candidates.len() {
        let path = &candidates[(start + offset) % candidates.len()];
        let mut rows = statement
            .query_map(params![path], row_to_track)
            .map_err(|error| error.to_string())?;
        if let Some(track) = rows.next() {
            return track.map(Some).map_err(|error| error.to_string());
        }
    }
    Ok(None)
}

/// Native Auto-DJ choice. It samples a bounded random window, avoids recent
/// queue history, then favors related genre/artist/album and long-unplayed
/// tracks. This keeps policy and randomness out of the webview without loading
/// the whole library into memory.
#[tauri::command]
pub fn db_auto_dj_next(
    db: State<Db>,
    current_path: Option<String>,
    recent_paths: Vec<String>,
) -> Result<Option<MusicTrack>, String> {
    limits::validate_paths(&recent_paths, 100)?;
    if let Some(path) = current_path.as_deref() {
        limits::validate_text(path, "Current track path", limits::MAX_PATH_BYTES)?;
    }
    auto_dj_next(&db, current_path.as_deref().unwrap_or(""), &recent_paths)
}

fn auto_dj_next(
    db: &Db,
    current_path: &str,
    recent_paths: &[String],
) -> Result<Option<MusicTrack>, String> {
    let conn = db.read();
    let current = conn
        .query_row(
            "SELECT artist, album, genre FROM tracks WHERE path = ?1",
            params![current_path],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .unwrap_or_default();
    let max_id: i64 = conn
        .query_row("SELECT COALESCE(MAX(id), 0) FROM tracks", [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    if max_id <= 0 {
        return Ok(None);
    }
    let start = (random_u64() % max_id as u64) as i64 + 1;
    let recent = recent_paths
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let sql = format!(
        "SELECT {TRACK_COLS_T}, COALESCE(s.last_played, 0)
         FROM tracks t LEFT JOIN stats s ON s.track_id = t.id
         WHERE t.id >= ?1 AND t.path <> ?2 ORDER BY t.id LIMIT 96"
    );
    let wrapped_sql = format!(
        "SELECT {TRACK_COLS_T}, COALESCE(s.last_played, 0)
         FROM tracks t LEFT JOIN stats s ON s.track_id = t.id
         WHERE t.id < ?1 AND t.path <> ?2 ORDER BY t.id LIMIT 96"
    );
    let mut candidates = Vec::new();
    for query in [&sql, &wrapped_sql] {
        if candidates.len() >= 96 {
            break;
        }
        let mut statement = conn.prepare(query).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![start, current_path], |row| {
                Ok((row_to_track(row)?, row.get::<_, i64>(17)?))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let candidate = row.map_err(|error| error.to_string())?;
            candidates.push(candidate);
            if candidates.len() >= 96 {
                break;
            }
        }
    }
    let non_recent = candidates
        .iter()
        .filter(|(track, _)| !recent.contains(track.path.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    // A small library will eventually have every track in recent history. In
    // that case allow an older repeat instead of stopping Auto-DJ altogether.
    let eligible = if non_recent.is_empty() {
        candidates
    } else {
        non_recent
    };
    let now = super::now_ms();
    Ok(eligible
        .into_iter()
        .max_by_key(|(track, last_played)| {
            let relation = i64::from(
                current
                    .2
                    .as_deref()
                    .is_some_and(|genre| track.genre.as_deref() == Some(genre)),
            ) * 8
                + i64::from(!current.0.is_empty() && track.artist == current.0.as_str()) * 4
                + i64::from(!current.1.is_empty() && track.album == current.1.as_str()) * 2;
            let stale_days = if *last_played <= 0 {
                365
            } else {
                ((now - *last_played).max(0) / 86_400_000).min(365)
            };
            relation * 1_000 + stale_days
        })
        .map(|(track, _)| track))
}

fn random_track(conn: &Connection, exclude: &str) -> Result<Option<MusicTrack>, String> {
    let max_id: i64 = conn
        .query_row("SELECT COALESCE(MAX(id), 0) FROM tracks", [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    if max_id <= 0 {
        return Ok(None);
    }
    let start = (random_u64() % max_id as u64) as i64 + 1;
    let forward = format!(
        "SELECT {TRACK_COLS} FROM tracks
         WHERE id >= ?1 AND path <> ?2 ORDER BY id LIMIT 1"
    );
    let mut tracks = collect_tracks(conn, &forward, params![start, exclude])?;
    if tracks.is_empty() {
        let wrapped = format!(
            "SELECT {TRACK_COLS} FROM tracks
             WHERE id < ?1 AND path <> ?2 ORDER BY id LIMIT 1"
        );
        tracks = collect_tracks(conn, &wrapped, params![start, exclude])?;
    }
    Ok(tracks.pop())
}

// ---- Albums / artists / genres ---------------------------------------------

#[tauri::command]
pub fn db_albums(db: State<Db>, search: Option<String>) -> Result<Vec<AlbumRow>, String> {
    let conn = db.read();
    let like = search
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| super::like_contains(s.trim()));
    let sql = "SELECT t.album, MIN(t.artist) AS artist, MAX(t.year) AS year, COUNT(*) AS n,
                 (SELECT path FROM tracks t2 WHERE t2.album = t.album AND t2.has_cover = 1 LIMIT 1) AS cover,
                 COALESCE(MAX(s.last_played), 0) AS last_played,
                 GROUP_CONCAT(DISTINCT t.artist) AS all_artists
               FROM tracks t LEFT JOIN stats s ON s.track_id = t.id
               WHERE (?1 IS NULL OR t.album LIKE ?1 ESCAPE '^' OR t.artist LIKE ?1 ESCAPE '^')
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
    let conn = db.read();
    let sql = format!(
        "SELECT {TRACK_COLS} FROM tracks WHERE album = ?1 ORDER BY track_number, title COLLATE NOCASE"
    );
    collect_tracks(&conn, &sql, params![album])
}

#[tauri::command]
pub fn db_artists(db: State<Db>, search: Option<String>) -> Result<Vec<ArtistRow>, String> {
    let conn = db.read();
    let like = search
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| super::like_contains(s.trim()));
    let sql = "SELECT t.artist, COUNT(*) AS n, COUNT(DISTINCT t.album) AS albums,
                 COALESCE(SUM(s.play_count), 0) AS plays,
                 (SELECT path FROM tracks t2 WHERE t2.artist = t.artist AND t2.has_cover = 1 LIMIT 1) AS cover,
                 COALESCE(MAX(s.last_played), 0) AS last_played
               FROM tracks t LEFT JOIN stats s ON s.track_id = t.id
               WHERE t.artist <> '' AND (?1 IS NULL OR t.artist LIKE ?1 ESCAPE '^')
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
    let conn = db.read();
    let sql = format!(
        "SELECT {TRACK_COLS} FROM tracks WHERE artist = ?1
         ORDER BY album COLLATE NOCASE, track_number, title COLLATE NOCASE"
    );
    collect_tracks(&conn, &sql, params![artist])
}

#[derive(Serialize)]
pub struct StationBatch {
    session_id: String,
    tracks: Vec<MusicTrack>,
    has_more: bool,
}

fn validate_station_batch_limit(limit: i64) -> Result<usize, String> {
    if !(1..=100).contains(&limit) {
        return Err("Station batch limit must be between 1 and 100".to_string());
    }
    Ok(limit as usize)
}

fn shuffle_station_ids(ids: &mut [i64]) {
    let mut state = random_u64();
    for index in (1..ids.len()).rev() {
        // SplitMix64 gives a fast, well-distributed native shuffle without
        // adding another RNG dependency.
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = state;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^= mixed >> 31;
        ids.swap(index, (mixed % (index as u64 + 1)) as usize);
    }
}

fn hydrate_station_ids(
    db: &Db,
    session_id: String,
    ids: Vec<i64>,
    has_more: bool,
) -> Result<StationBatch, String> {
    if ids.is_empty() {
        return Ok(StationBatch {
            session_id,
            tracks: Vec::new(),
            has_more,
        });
    }
    let conn = db.read();
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!("SELECT {TRACK_COLS}, id FROM tracks WHERE id IN ({placeholders})");
    let mut statement = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params_from_iter(ids.iter()), |row| {
            Ok((row.get::<_, i64>(17)?, row_to_track(row)?))
        })
        .map_err(|error| error.to_string())?;
    let mut by_id = HashMap::new();
    for row in rows {
        let (id, track) = row.map_err(|error| error.to_string())?;
        by_id.insert(id, track);
    }
    let tracks = ids.into_iter().filter_map(|id| by_id.remove(&id)).collect();
    Ok(StationBatch {
        session_id,
        tracks,
        has_more,
    })
}

fn station_next(db: &Db, session_id: String, limit: usize) -> Result<StationBatch, String> {
    let (ids, has_more) = {
        let mut sessions = db.1.station_sessions.lock();
        let session = sessions
            .get_mut(&session_id)
            .ok_or_else(|| "Station session has expired".to_string())?;
        let end = session.cursor.saturating_add(limit).min(session.ids.len());
        let ids = session.ids[session.cursor..end].to_vec();
        session.cursor = end;
        (ids, end < session.ids.len())
    };
    hydrate_station_ids(db, session_id, ids, has_more)
}

/// Build only a shuffled integer-ID plan in Rust, then hydrate the first small
/// window. Full track metadata never crosses IPC all at once.
#[tauri::command]
pub fn db_station_start(
    db: State<Db>,
    kind: String,
    key: String,
    limit: i64,
) -> Result<StationBatch, String> {
    limits::validate_text(&kind, "Station kind", 16)?;
    limits::validate_text(&key, "Station key", 1_024)?;
    let limit = validate_station_batch_limit(limit)?;
    if !matches!(kind.as_str(), "genre" | "artist") {
        return Err("Station kind must be genre or artist".to_string());
    }
    station_start(&db, &kind, &key, limit)
}

fn station_start(db: &Db, kind: &str, key: &str, limit: usize) -> Result<StationBatch, String> {
    let mut ids = {
        let conn = db.read();
        let sql = if kind == "genre" {
            "SELECT id FROM tracks WHERE genre = ?1"
        } else {
            "SELECT id FROM tracks WHERE artist = ?1"
        };
        let mut statement = conn.prepare(sql).map_err(|error| error.to_string())?;
        let collected = statement
            .query_map(params![key], |row| row.get::<_, i64>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        collected
    };
    shuffle_station_ids(&mut ids);

    let session_id = format!("{:016x}{:016x}", random_u64(), random_u64());
    {
        let mut sessions = db.1.station_sessions.lock();
        // Bound native memory and naturally expire the oldest arbitrary session;
        // the UI only keeps one station active.
        if sessions.len() >= 8 {
            if let Some(expired) = sessions.keys().next().cloned() {
                sessions.remove(&expired);
            }
        }
        sessions.insert(session_id.clone(), StationSession { ids, cursor: 0 });
    }
    station_next(db, session_id, limit)
}

#[tauri::command]
pub fn db_station_next(
    db: State<Db>,
    session_id: String,
    limit: i64,
) -> Result<StationBatch, String> {
    limits::validate_text(&session_id, "Station session", 64)?;
    station_next(&db, session_id, validate_station_batch_limit(limit)?)
}

// Whether any track carries a non-empty genre (drives the smart-playlist editor's
// "genre needs reindex" hint).
#[tauri::command]
pub fn db_has_genre(db: State<Db>) -> Result<bool, String> {
    let conn = db.read();
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
    validate_smart_request(
        &rules,
        sort_by.as_deref().unwrap_or("none"),
        sort_order.as_deref().unwrap_or("asc"),
        limit.unwrap_or(0),
    )?;
    let conn = db.read();
    smart_eval(
        &conn,
        &rules,
        sort_by.as_deref().unwrap_or("none"),
        sort_order.as_deref().unwrap_or("asc"),
        limit.unwrap_or(0),
    )
}

/// Count an ad-hoc smart-playlist preview entirely in SQLite. This mirrors
/// db_smart_tracks' rule and limit semantics without materializing track rows.
#[tauri::command]
pub fn db_smart_count(db: State<Db>, rules: Value, limit: Option<i64>) -> Result<i64, String> {
    limits::validate_json(
        &rules,
        "Smart-playlist rules",
        limits::MAX_RULES_BYTES,
        limits::MAX_JSON_DEPTH,
    )?;
    validate_smart_request(&rules, "none", "asc", limit.unwrap_or(0))?;
    let conn = db.read();
    smart_count(&conn, &rules, limit.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use parking_lot::Mutex;
    use rusqlite::{params, Connection};

    use super::*;
    use crate::library_db::{migrate, ConnectionSlot, DbCache, ReadPool, SCHEMA};

    fn database() -> Db {
        let mut connection = Connection::open_in_memory().expect("open test database");
        connection
            .execute_batch(SCHEMA)
            .expect("create test schema");
        migrate(&mut connection).expect("migrate test schema");
        Db(
            Mutex::new(ConnectionSlot::new(connection)),
            DbCache::default(),
            ReadPool::empty(),
        )
    }

    fn insert_track(db: &Db, path: &str, title: &str, artist: &str, album: &str, genre: &str) {
        db.0.lock()
            .execute(
                "INSERT INTO tracks
                 (path, title, artist, album, genre, date_added, first_seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, 1)",
                params![path, title, artist, album, genre],
            )
            .expect("insert search track");
    }

    #[test]
    fn global_search_is_ranked_bounded_and_echoes_request_id() {
        let db = database();
        insert_track(&db, "one", "Blue", "Blue", "Blue", "Blue");
        insert_track(&db, "two", "Blue Sky", "Blue Notes", "Blue Album", "Blues");
        insert_track(
            &db,
            "three",
            "Sky Blue",
            "Deep Blue",
            "Album Blue",
            "Rhythm Blue",
        );

        let connection = db.0.lock();
        let result = global_search(&connection, "Blue", 2, 2, 2, 2, 41).expect("run global search");
        assert_eq!(result.request_id, 41);
        assert_eq!(result.songs.len(), 2);
        assert_eq!(result.songs[0].title, "Blue");
        assert_eq!(result.songs[1].title, "Blue Sky");
        assert_eq!(result.albums.len(), 2);
        assert_eq!(result.albums[0].album, "Blue");
        assert_eq!(result.artists.len(), 2);
        assert_eq!(result.artists[0].artist, "Blue");
        assert_eq!(result.genres.len(), 2);
        assert_eq!(result.genres[0].genre, "Blue");
    }

    #[test]
    fn random_track_uses_rowid_window_and_honors_exclusion() {
        let db = database();
        insert_track(&db, "one", "One", "Artist", "Album", "Genre");
        insert_track(&db, "two", "Two", "Artist", "Album", "Genre");
        let connection = db.0.lock();
        for _ in 0..8 {
            let track = random_track(&connection, "one")
                .expect("select random track")
                .expect("other track exists");
            assert_eq!(track.path, "two");
        }
    }

    #[test]
    fn auto_dj_avoids_recent_paths_and_prefers_related_tracks() {
        let db = database();
        insert_track(&db, "current", "Current", "Artist", "Album", "Rock");
        insert_track(&db, "related", "Related", "Other", "Other Album", "Rock");
        insert_track(&db, "unrelated", "Unrelated", "Else", "Elsewhere", "Jazz");

        let related = auto_dj_next(&db, "current", &[])
            .expect("select auto-dj track")
            .expect("candidate exists");
        assert_eq!(related.path, "related");

        let fallback = auto_dj_next(&db, "current", &["related".to_string()])
            .expect("select non-recent auto-dj track")
            .expect("fallback exists");
        assert_eq!(fallback.path, "unrelated");

        let repeat = auto_dj_next(
            &db,
            "current",
            &["related".to_string(), "unrelated".to_string()],
        )
        .expect("select an older repeat")
        .expect("repeat exists");
        assert_ne!(repeat.path, "current");
    }

    #[test]
    fn station_hydrates_small_non_repeating_batches() {
        let db = database();
        for index in 0..5 {
            insert_track(
                &db,
                &format!("track-{index}"),
                &format!("Track {index}"),
                "Station Artist",
                "Album",
                "Genre",
            );
        }

        let first = station_start(&db, "artist", "Station Artist", 2).expect("start station");
        assert_eq!(first.tracks.len(), 2);
        assert!(first.has_more);
        let second = station_next(&db, first.session_id.clone(), 2).expect("next station batch");
        assert_eq!(second.tracks.len(), 2);
        assert!(second.has_more);
        let third = station_next(&db, first.session_id, 2).expect("final station batch");
        assert_eq!(third.tracks.len(), 1);
        assert!(!third.has_more);

        let paths = first
            .tracks
            .iter()
            .chain(second.tracks.iter())
            .chain(third.tracks.iter())
            .map(|track| track.path.clone())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(paths.len(), 5);
    }
}
