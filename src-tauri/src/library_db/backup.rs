//! Database backup, restore, root relocation, and missing-file recovery.

use std::path::Path;

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};

use crate::is_allowed_path;

use super::{migrate, Db, SCHEMA};

fn has_backup_extension(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("db" | "tsmback")
    )
}

fn authorize_backup_path<R: Runtime>(app: &AppHandle<R>, path: &Path) -> Result<(), String> {
    if !has_backup_extension(path) {
        return Err("Backup path must end in .db or .tsmback".to_string());
    }
    if !is_allowed_path(app, path) {
        return Err("Backup path was not authorized by the file picker".to_string());
    }
    Ok(())
}

fn validate_backup_source(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err("Backup file does not exist".to_string());
    }
    let check_conn =
        Connection::open(path).map_err(|e| format!("Invalid backup file (cannot open): {e}"))?;
    let has_kv: bool = check_conn
        .prepare("SELECT 1 FROM pragma_table_info('kv')")
        .and_then(|mut statement| statement.exists([]))
        .map_err(|e| format!("Invalid backup file schema: {e}"))?;
    if !has_kv {
        return Err("Backup file is missing required table schema".to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn db_export_backup(app: AppHandle, db: State<Db>, dest: String) -> Result<(), String> {
    let conn = db.0.lock();
    let dest_path = Path::new(&dest);
    authorize_backup_path(&app, dest_path)?;

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
    authorize_backup_path(&app, src_path)?;
    validate_backup_source(src_path)?;

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
    std::fs::copy(src_path, &db_path).map_err(|e| format!("Failed to restore backup file: {e}"))?;

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
        for path in rows.flatten() {
            let exists = Path::new(&path).exists();
            roots.push(RootStatus { path, exists });
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "ts-music-backup-auth-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir(&path).expect("create test directory");
            Self(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn backup_path_requires_supported_extension_and_exact_scope_grant() {
        let app = tauri::test::mock_app();
        let dir = TestDir::new();
        let selected = dir.join("selected.DB");
        let sibling = dir.join("sibling.db");
        let wrong_extension = dir.join("selected.db.exe");
        app.asset_protocol_scope()
            .allow_file(&selected)
            .expect("allow selected backup path");
        app.asset_protocol_scope()
            .allow_file(&wrong_extension)
            .expect("allow wrong-extension path");

        assert!(authorize_backup_path(app.handle(), &selected).is_ok());
        assert!(authorize_backup_path(app.handle(), &sibling).is_err());
        assert_eq!(
            authorize_backup_path(app.handle(), &wrong_extension)
                .expect_err("reject disguised executable"),
            "Backup path must end in .db or .tsmback"
        );
    }

    #[test]
    fn backup_import_rejects_sqlite_files_without_the_app_schema() {
        let dir = TestDir::new();
        let invalid = dir.join("invalid.db");
        let conn = Connection::open(&invalid).expect("create sqlite file");
        conn.execute_batch("CREATE TABLE unrelated (id INTEGER);")
            .expect("create unrelated schema");
        drop(conn);

        assert_eq!(
            validate_backup_source(&invalid).expect_err("reject unrelated database"),
            "Backup file is missing required table schema"
        );
    }

    #[test]
    fn backup_import_accepts_database_with_required_schema() {
        let dir = TestDir::new();
        let valid = dir.join("valid.tsmback");
        let conn = Connection::open(&valid).expect("create sqlite file");
        conn.execute_batch("CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT);")
            .expect("create app marker table");
        drop(conn);

        validate_backup_source(&valid).expect("accept app backup schema");
    }
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

        for (path, title, artist) in rows.flatten() {
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
