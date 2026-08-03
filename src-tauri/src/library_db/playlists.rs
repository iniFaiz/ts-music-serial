//! Favorites, regular/smart playlists, and recently opened collections.

use rusqlite::{params, Connection};
use serde_json::Value;
use tauri::State;

use crate::{limits, MusicTrack};

use super::{
    collect_tracks, library_fingerprint, now_ms, smart_count, smart_eval, validate_smart_request,
    Db, OptionalString, PlaylistRow, RecentRow, TRACK_COLS_T,
};

type SmartPlaylistDefinition = (
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

// ---- Favorites --------------------------------------------------------------

#[tauri::command]
pub fn db_favorite_paths(db: State<Db>) -> Result<Vec<String>, String> {
    let conn = db.read();
    let mut stmt = conn
        .prepare(
            "SELECT t.path FROM favorites f
             JOIN tracks t ON t.id = f.track_id ORDER BY f.position",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn db_favorites(db: State<Db>) -> Result<Vec<MusicTrack>, String> {
    let conn = db.read();
    let sql = format!(
        "SELECT {TRACK_COLS_T} FROM tracks t JOIN favorites f ON f.track_id = t.id ORDER BY f.position"
    );
    collect_tracks(&conn, &sql, params![])
}

// Toggle favorite; returns the new state (true = now favorited).
#[tauri::command]
pub fn db_toggle_favorite(db: State<Db>, path: String) -> Result<bool, String> {
    let conn = db.0.lock();
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM favorites f JOIN tracks t ON t.id = f.track_id WHERE t.path = ?1",
            params![path],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if exists {
        conn.execute(
            "DELETE FROM favorites WHERE track_id = (SELECT id FROM tracks WHERE path = ?1)",
            params![path],
        )
        .map_err(|e| e.to_string())?;
        Ok(false)
    } else {
        let next: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM favorites",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO favorites(track_id, position)
             SELECT id, ?2 FROM tracks WHERE path = ?1",
            params![path, next],
        )
        .map_err(|e| e.to_string())?;
        Ok(true)
    }
}

#[tauri::command]
pub fn db_move_favorite(db: State<Db>, from: i64, to: i64) -> Result<(), String> {
    let mut conn = db.0.lock();
    let mut track_ids: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT track_id FROM favorites ORDER BY position")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let (from, to) = (from as usize, to as usize);
    if from >= track_ids.len() || to >= track_ids.len() {
        return Ok(());
    }
    let item = track_ids.remove(from);
    track_ids.insert(to, item);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (i, track_id) in track_ids.iter().enumerate() {
        tx.execute(
            "UPDATE favorites SET position = ?1 WHERE track_id = ?2",
            params![i as i64, track_id],
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
    let conn = db.read();
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
        let n = smart_count(&conn, rules, pl.limit_n.unwrap_or(0))?;
        cache.insert(pl.id.clone(), (fp, rules_json, n));
        pl.track_count = n;
    }
    Ok(rows)
}

// Normal playlist → its items in order; smart playlist → evaluated rules.
#[tauri::command]
pub fn db_playlist_tracks(db: State<Db>, id: String) -> Result<Vec<MusicTrack>, String> {
    let conn = db.read();
    let smart: Option<SmartPlaylistDefinition> = conn
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
        "SELECT {TRACK_COLS_T} FROM playlist_items i JOIN tracks t ON t.id = i.track_id
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
    limits::validate_text(&id, "Playlist ID", 128)?;
    limits::validate_text(&name, "Playlist name", 200)?;
    limits::validate_text(&description, "Playlist description", 2_000)?;
    if name.trim().is_empty() {
        return Err("Playlist name cannot be empty".to_string());
    }
    if let Some(color) = color.as_deref() {
        limits::validate_text(color, "Playlist color", 32)?;
    }
    if let Some(cover) = cover.as_deref() {
        crate::playlist_cover::validate_playlist_cover_data_url(cover)?;
    }
    if let Some(rules) = rules.as_ref() {
        limits::validate_json(
            rules,
            "Smart-playlist rules",
            limits::MAX_RULES_BYTES,
            limits::MAX_JSON_DEPTH,
        )?;
    }
    if let Some(sort_by) = sort_by.as_deref() {
        limits::validate_text(sort_by, "Smart-playlist sort", 32)?;
    }
    if let Some(sort_order) = sort_order.as_deref() {
        if !matches!(sort_order, "asc" | "desc") {
            return Err("Smart-playlist sort order must be asc or desc".to_string());
        }
    }
    if is_smart {
        let rules = rules
            .as_ref()
            .ok_or_else(|| "Smart playlists require a rules object".to_string())?;
        validate_smart_request(
            rules,
            sort_by.as_deref().unwrap_or("none"),
            sort_order.as_deref().unwrap_or("asc"),
            limit_n.unwrap_or(0),
        )?;
    } else if limit_n.is_some_and(|limit| !(0..=100_000).contains(&limit)) {
        return Err("Playlist limit must be between 0 and 100000".to_string());
    }
    let conn = db.0.lock();
    let rules_str = rules.map(|v| v.to_string());
    // Preserve existing position on update; append to the end on insert.
    let next_pos: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM playlists",
            [],
            |r| r.get(0),
        )
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
pub fn db_delete_playlist(
    db: State<Db>,
    consent: State<crate::security::DestructiveConsentState>,
    id: String,
    consent_token: String,
) -> Result<(), String> {
    limits::validate_text(&id, "Playlist ID", 128)?;
    consent.consume(
        &consent_token,
        crate::security::ConsentAction::DeletePlaylist,
        Some(&id),
    )?;
    let mut conn = db.0.lock();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM playlist_items WHERE playlist_id = ?1",
        params![id],
    )
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
    limits::validate_text(&id, "Playlist ID", 128)?;
    limits::validate_paths(&paths, limits::MAX_BATCH_PATHS)?;
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
                "INSERT OR IGNORE INTO playlist_items(playlist_id, track_id, position)
                 SELECT ?1, id, ?3 FROM tracks WHERE path = ?2",
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
    limits::validate_text(&id, "Playlist ID", 128)?;
    limits::validate_text(&path, "Track path", limits::MAX_PATH_BYTES)?;
    let conn = db.0.lock();
    conn.execute(
        "DELETE FROM playlist_items
         WHERE playlist_id = ?1
           AND track_id = (SELECT id FROM tracks WHERE path = ?2)",
        params![id, path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn db_playlist_move_item(db: State<Db>, id: String, from: i64, to: i64) -> Result<(), String> {
    limits::validate_text(&id, "Playlist ID", 128)?;
    let mut conn = db.0.lock();
    let mut item_ids: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT id FROM playlist_items WHERE playlist_id = ?1 ORDER BY position")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![id], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let (from, to) = (from as usize, to as usize);
    if from >= item_ids.len() || to >= item_ids.len() {
        return Ok(());
    }
    let item = item_ids.remove(from);
    item_ids.insert(to, item);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (i, item_id) in item_ids.iter().enumerate() {
        tx.execute(
            "UPDATE playlist_items SET position = ?1 WHERE id = ?2",
            params![i as i64, item_id],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

// ---- Recents ----------------------------------------------------------------

#[tauri::command]
pub fn db_recents(db: State<Db>) -> Result<Vec<RecentRow>, String> {
    let conn = db.read();
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
