//! Database backup, restore, root relocation, and missing-file recovery.

use std::path::Path;

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use super::{migrate, Db, SCHEMA};

#[tauri::command]
pub fn db_export_backup(db: State<Db>, dest: String) -> Result<(), String> {
    let conn = db.0.lock();
    let dest_path = Path::new(&dest);

    // VACUUM INTO fails if the file already exists.
    if dest_path.exists() {
        std::fs::remove_file(dest_path)
            .map_err(|e| format!("Failed to remove existing destination file: {e}"))?;
    }

    conn.execute("VACUUM INTO ?1", params![dest])
        .map_err(|e| format!("Failed to export backup: {e}"))?;

    Ok(())
}

#[derive(Serialize)]
pub struct RootStatus {
    pub path: String,
    pub exists: bool,
}

#[derive(Serialize)]
pub struct ImportBackupResult {
    pub roots: Vec<RootStatus>,
}

#[derive(Serialize)]
pub struct MissingTrackInfo {
    pub title: String,
    pub artist: String,
    pub path: String,
}

#[tauri::command]
pub fn db_import_backup(
    app: AppHandle,
    db: State<Db>,
    src: String,
) -> Result<ImportBackupResult, String> {
    let src_path = Path::new(&src);
    if !src_path.exists() {
        return Err("Backup file does not exist".to_string());
    }

    // Verify it is a valid SQLite database with our schema.
    {
        let check_conn = Connection::open(&src_path)
            .map_err(|e| format!("Invalid backup file (cannot open): {e}"))?;
        let has_kv: bool = check_conn
            .prepare("SELECT 1 FROM pragma_table_info('kv')")
            .and_then(|mut s| s.exists([]))
            .map_err(|e| format!("Invalid backup file schema: {e}"))?;
        if !has_kv {
            return Err("Backup file is missing required table schema".to_string());
        }
    }

    // Resolve the current library.db path.
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("no app data dir: {e}"))?;
    let db_path = dir.join("library.db");
    let wal_path = dir.join("library.db-wal");
    let shm_path = dir.join("library.db-shm");

    // Close the database connection and swap with an in-memory DB to release the file handle.
    let mut conn_lock = db.0.lock();
    let dummy = Connection::open_in_memory().map_err(|e| e.to_string())?;
    let old_conn = std::mem::replace(&mut *conn_lock, dummy);
    drop(old_conn);

    // Delete WAL and SHM files to prevent corruption.
    if wal_path.exists() {
        let _ = std::fs::remove_file(&wal_path);
    }
    if shm_path.exists() {
        let _ = std::fs::remove_file(&shm_path);
    }

    // Copy the backup database file to the current database path.
    std::fs::copy(&src_path, &db_path)
        .map_err(|e| format!("Failed to restore backup file: {e}"))?;

    // Reopen the connection to the new database.
    let new_conn = Connection::open(&db_path).map_err(|e| e.to_string())?;

    // Run schema migrations and PRAGMAs.
    new_conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    migrate(&new_conn)?;

    // Query roots and check if they exist on disk.
    let mut roots = Vec::new();
    {
        let mut stmt = new_conn
            .prepare("SELECT path FROM roots")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        for r in rows {
            if let Ok(path) = r {
                let exists = Path::new(&path).exists();
                roots.push(RootStatus { path, exists });
            }
        }
    }

    // Swap the connection back into the Tauri state.
    *conn_lock = new_conn;

    // Clear the memoized smart playlist count cache.
    db.1.smart_counts.lock().clear();

    Ok(ImportBackupResult { roots })
}

#[tauri::command]
pub fn db_relocate_root(db: State<Db>, old_root: String, new_root: String) -> Result<(), String> {
    let mut conn = db.0.lock();
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    // Perform path relocation replacing the old root prefix with the new root prefix
    tx.execute(
        "UPDATE roots SET path = ?2 || SUBSTR(path, LENGTH(?1) + 1) WHERE path LIKE ?1 || '%'",
        params![old_root, new_root],
    )
    .map_err(|e| e.to_string())?;

    tx.execute(
        "UPDATE tracks SET path = ?2 || SUBSTR(path, LENGTH(?1) + 1) WHERE path LIKE ?1 || '%'",
        params![old_root, new_root],
    )
    .map_err(|e| e.to_string())?;

    tx.execute(
        "UPDATE stats SET path = ?2 || SUBSTR(path, LENGTH(?1) + 1) WHERE path LIKE ?1 || '%'",
        params![old_root, new_root],
    )
    .map_err(|e| e.to_string())?;

    tx.execute(
        "UPDATE favorites SET path = ?2 || SUBSTR(path, LENGTH(?1) + 1) WHERE path LIKE ?1 || '%'",
        params![old_root, new_root],
    )
    .map_err(|e| e.to_string())?;

    tx.execute(
        "UPDATE playlist_items SET path = ?2 || SUBSTR(path, LENGTH(?1) + 1) WHERE path LIKE ?1 || '%'",
        params![old_root, new_root],
    ).map_err(|e| e.to_string())?;

    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn db_prune_and_get_missing(db: State<Db>) -> Result<Vec<MissingTrackInfo>, String> {
    let mut conn = db.0.lock();
    let mut missing = Vec::new();
    let mut to_remove = Vec::new();

    {
        let mut stmt = conn
            .prepare("SELECT path, title, artist FROM tracks")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;

        for row in rows {
            if let Ok((path, title, artist)) = row {
                if !Path::new(&path).exists() {
                    missing.push(MissingTrackInfo {
                        title,
                        artist,
                        path: path.clone(),
                    });
                    to_remove.push(path);
                }
            }
        }
    }

    if !to_remove.is_empty() {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for p in &to_remove {
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
    }

    Ok(missing)
}
