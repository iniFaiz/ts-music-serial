//! Playback statistics and derived insight collections.

use rusqlite::params;
use serde::Serialize;
use tauri::State;

use crate::MusicTrack;

use super::{
    collect_tracks, now_ms, ArtistRow, Db, GenreRow, StatRow, StatsSummary, TRACK_COLS,
    TRACK_COLS_T,
};

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
    let sql = format!("SELECT {TRACK_COLS} FROM tracks ORDER BY date_added DESC LIMIT ?1");
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
                 (SELECT path FROM tracks t2 WHERE t2.artist = t.artist AND t2.has_cover = 1 LIMIT 1) AS cover,
                 COALESCE(MAX(s.last_played), 0) AS last_played
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
        conn.query_row(sql, args, |r| r.get(0))
            .map_err(|e| e.to_string())
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
