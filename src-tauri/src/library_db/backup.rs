//! Database backup, restore, root relocation, and missing-file recovery.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OpenFlags, Transaction};
use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use crate::is_allowed_path;
use crate::library_scan::{canonicalize_directory, canonicalize_existing_path};
use crate::security::{ConsentAction, DestructiveConsentState};

use super::{migrate, Db, APPLICATION_ID, SCHEMA, SCHEMA_VERSION};

static TEMP_SERIAL: AtomicU64 = AtomicU64::new(1);

/// A same-directory temporary database. SQLite sidecars are removed with it,
/// including on an early-return error.
struct TemporaryDatabase {
    path: PathBuf,
    preserve: bool,
}

impl TemporaryDatabase {
    fn sibling_of(target: &Path, purpose: &str) -> Result<Self, String> {
        let parent = target
            .parent()
            .ok_or_else(|| "Database path has no parent directory".to_string())?;
        let filename = target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("library.db");
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        for _ in 0..32 {
            let serial = TEMP_SERIAL.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(
                ".{filename}.{purpose}.{}.{nanos}.{serial}.tmp",
                std::process::id()
            ));
            if !path.exists() {
                return Ok(Self {
                    path,
                    preserve: false,
                });
            }
        }
        Err("Could not allocate a temporary database path".to_string())
    }

    fn preserve(&mut self) {
        self.preserve = true;
    }
}

impl Drop for TemporaryDatabase {
    fn drop(&mut self) {
        if self.preserve {
            return;
        }
        let _ = fs::remove_file(&self.path);
        remove_sidecars_best_effort(&self.path);
    }
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn remove_sidecars_best_effort(path: &Path) {
    for suffix in ["-wal", "-shm", "-journal"] {
        let _ = fs::remove_file(sidecar_path(path, suffix));
    }
}

fn remove_stale_sidecars(path: &Path) -> Result<(), String> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = sidecar_path(path, suffix);
        match fs::remove_file(&sidecar) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Failed to remove stale SQLite sidecar '{}': {error}",
                    sidecar.display()
                ));
            }
        }
    }
    Ok(())
}

fn sync_file(path: &Path) -> Result<(), String> {
    // FlushFileBuffers requires a write-capable handle on Windows; File::open
    // creates a read-only handle and fails with ERROR_ACCESS_DENIED even for a
    // file we own. These are same-directory staging/destination files.
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("Failed to flush '{}': {error}", path.display()))
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Database path has no parent directory".to_string())?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("Failed to flush '{}': {error}", parent.display()))
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn replace_existing_file(staging: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{ReplaceFileW, REPLACEFILE_WRITE_THROUGH};

    let target_wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let staging_wide: Vec<u16> = staging.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        ReplaceFileW(
            PCWSTR(target_wide.as_ptr()),
            PCWSTR(staging_wide.as_ptr()),
            PCWSTR::null(),
            REPLACEFILE_WRITE_THROUGH,
            None,
            None,
        )
    }
    .map_err(|error| {
        format!(
            "Failed to atomically replace '{}' with '{}': {error}",
            target.display(),
            staging.display()
        )
    })
}

#[cfg(not(windows))]
fn replace_existing_file(staging: &Path, target: &Path) -> Result<(), String> {
    fs::rename(staging, target).map_err(|error| {
        format!(
            "Failed to atomically replace '{}' with '{}': {error}",
            target.display(),
            staging.display()
        )
    })
}

/// Every caller creates its staging file beside the target, guaranteeing the
/// same filesystem. Windows needs ReplaceFileW for an atomic overwrite; a new
/// target and Unix replacements use same-filesystem rename.
fn atomic_replace(staging: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        replace_existing_file(staging, target)?;
    } else {
        fs::rename(staging, target).map_err(|error| {
            format!(
                "Failed to atomically install '{}' as '{}': {error}",
                staging.display(),
                target.display()
            )
        })?;
    }
    sync_parent(target)
}

fn copy_to_new_file(source: &Path, destination: &Path) -> Result<(), String> {
    let mut input = File::open(source)
        .map_err(|error| format!("Failed to open backup file '{}': {error}", source.display()))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| {
            format!(
                "Failed to create staging database '{}': {error}",
                destination.display()
            )
        })?;
    io::copy(&mut input, &mut output)
        .map_err(|error| format!("Failed to copy backup into staging: {error}"))?;
    output
        .sync_all()
        .map_err(|error| format!("Failed to flush staged backup: {error}"))
}

fn open_read_only(path: &Path) -> Result<Connection, String> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("Invalid backup file (cannot open read-only): {error}"))
}

fn validate_integrity(connection: &Connection) -> Result<(), String> {
    let result: String = connection
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(|error| format!("Backup integrity check failed: {error}"))?;
    if result.eq_ignore_ascii_case("ok") {
        Ok(())
    } else {
        Err(format!("Backup integrity check failed: {result}"))
    }
}

fn validate_schema_identity(connection: &Connection) -> Result<(), String> {
    let application_id: i32 = connection
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(|error| format!("Invalid backup application id: {error}"))?;
    if application_id != 0 && application_id != APPLICATION_ID {
        return Err("Backup belongs to a different application".to_string());
    }

    let version: i32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| format!("Invalid backup schema version: {error}"))?;
    if version > SCHEMA_VERSION {
        return Err(format!(
            "Backup schema version {version} is newer than supported version {SCHEMA_VERSION}"
        ));
    }

    // Version-zero databases predate application_id/user_version. Requiring the
    // complete set of core tables prevents an arbitrary SQLite file containing
    // only a coincidental `kv` table from being upgraded into an app database.
    const REQUIRED_TABLES: [&str; 9] = [
        "tracks",
        "stats",
        "favorites",
        "roots",
        "playlists",
        "playlist_items",
        "recents",
        "kv",
        "cover_art",
    ];
    let mut statement = connection
        .prepare("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1")
        .map_err(|error| format!("Invalid backup schema: {error}"))?;
    for table in REQUIRED_TABLES {
        let exists = statement
            .exists(params![table])
            .map_err(|error| format!("Invalid backup schema: {error}"))?;
        if !exists {
            return Err(format!("Backup is missing required table '{table}'"));
        }
    }
    Ok(())
}

fn validate_read_only_database(path: &Path) -> Result<(), String> {
    let connection = open_read_only(path)?;
    validate_integrity(&connection)?;
    validate_schema_identity(&connection)
}

fn vacuum_snapshot(connection: &Connection, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        return Err(format!(
            "Refusing to overwrite temporary database '{}'",
            destination.display()
        ));
    }
    connection
        .execute(
            "VACUUM main INTO ?1",
            params![destination.to_string_lossy().as_ref()],
        )
        .map_err(|error| format!("Failed to create SQLite snapshot: {error}"))?;
    sync_file(destination)
}

fn open_application_database(path: &Path) -> Result<Connection, String> {
    let mut connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("Failed to open restored database: {error}"))?;
    connection
        .execute_batch(SCHEMA)
        .map_err(|error| format!("Failed to initialise restored database: {error}"))?;
    migrate(&mut connection)?;
    Ok(connection)
}

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
    validate_read_only_database(path)
}

#[tauri::command]
pub fn db_export_backup(app: AppHandle, db: State<Db>, dest: String) -> Result<(), String> {
    let dest_path = Path::new(&dest);
    authorize_backup_path(&app, dest_path)?;
    let staging = TemporaryDatabase::sibling_of(dest_path, "export")?;

    {
        // VACUUM INTO uses SQLite's online snapshot machinery, so committed WAL
        // content is included without copying a live database file directly.
        let connection = db.0.lock();
        vacuum_snapshot(&connection, &staging.path)?;
    }
    validate_read_only_database(&staging.path)?;
    atomic_replace(&staging.path, dest_path)
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

fn staged_restore(
    source: &Path,
    database_path: &Path,
) -> Result<(TemporaryDatabase, Vec<RootStatus>), String> {
    let staging = TemporaryDatabase::sibling_of(database_path, "restore")?;
    copy_to_new_file(source, &staging.path)?;

    // The first open is strictly read-only and occurs before any schema DDL or
    // migration can mutate attacker-controlled input.
    validate_read_only_database(&staging.path)?;

    let mut staged = Connection::open(&staging.path)
        .map_err(|error| format!("Failed to open staging database: {error}"))?;
    staged
        .execute_batch(SCHEMA)
        .map_err(|error| format!("Failed to prepare staging schema: {error}"))?;
    migrate(&mut staged).map_err(|error| format!("Failed to migrate staging database: {error}"))?;

    // Paths carried by a backup are application data, not proof of user-granted
    // filesystem authority. Keep them pending until relocate_root confirms a
    // picker-scoped destination.
    staged
        .execute_batch(
            "BEGIN IMMEDIATE;
             INSERT OR IGNORE INTO pending_roots(path) SELECT path FROM roots;
             DELETE FROM roots;
             COMMIT;",
        )
        .map_err(|error| format!("Failed to quarantine restored roots: {error}"))?;

    validate_integrity(&staged)?;
    let roots = query_root_statuses(&staged, "pending_roots")?;

    // Leave the staging database as one self-contained main file. This prevents
    // either its own or the old active database's WAL from being replayed after
    // the filename swap.
    staged
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode = DELETE;")
        .map_err(|error| format!("Failed to finalise staging database: {error}"))?;
    staged.close().map_err(|(_, error)| {
        format!("Failed to close staging database before replacement: {error}")
    })?;
    sync_file(&staging.path)?;
    remove_stale_sidecars(&staging.path)?;

    // Re-open read-only after migrations and journal finalisation; this catches
    // schema/migration or flush failures before the active file is touched.
    validate_read_only_database(&staging.path)?;
    Ok((staging, roots))
}

fn query_root_statuses(connection: &Connection, table: &str) -> Result<Vec<RootStatus>, String> {
    let sql = match table {
        "roots" => "SELECT path FROM roots ORDER BY path",
        "pending_roots" => "SELECT path FROM pending_roots ORDER BY path",
        _ => return Err("Invalid root table".to_string()),
    };
    let mut statement = connection.prepare(sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    let mut roots = Vec::new();
    for row in rows {
        let path = row.map_err(|error| error.to_string())?;
        roots.push(RootStatus {
            exists: Path::new(&path).is_dir(),
            path,
        });
    }
    Ok(roots)
}

fn restore_from_path(
    db: &Db,
    source: &Path,
    database_path: &Path,
) -> Result<ImportBackupResult, String> {
    let (staging, roots) = staged_restore(source, database_path)?;
    let mut rollback = TemporaryDatabase::sibling_of(database_path, "rollback")?;
    let rollback_swap = TemporaryDatabase::sibling_of(database_path, "rollback-swap")?;
    let reader_pause = db.2.pause();
    let mut slot = db.0.lock();

    // Capture a consistent rollback image while the current connection is live.
    // This includes committed WAL frames and never copies library.db directly.
    vacuum_snapshot(&slot, &rollback.path)?;
    validate_read_only_database(&rollback.path)?;
    // Keep one closed copy for the filename rollback and one already-open,
    // verified connection as a last-resort fallback. Even a filesystem failure
    // during rollback can therefore never leave the managed slot empty.
    copy_to_new_file(&rollback.path, &rollback_swap.path)?;
    validate_read_only_database(&rollback_swap.path)?;
    let mut emergency_connection = Some(open_application_database(&rollback.path)?);
    slot.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .map_err(|error| format!("Failed to checkpoint active database: {error}"))?;

    let replacement_result = (|| -> Result<(), String> {
        let active = slot.take()?;
        if let Err((active, error)) = active.close() {
            slot.install(active);
            return Err(format!(
                "Failed to close active database before restore: {error}"
            ));
        }

        remove_stale_sidecars(database_path)?;
        atomic_replace(&staging.path, database_path)?;
        let restored = open_application_database(database_path)?;
        slot.install(restored);
        Ok(())
    })();

    if let Err(replacement_error) = replacement_result {
        if slot.is_empty() {
            // Once the old handle has closed, every failure path restores the
            // verified online snapshot before returning. A failed open can have
            // created sidecars, so they are removed before the rollback swap.
            let cleanup_error = remove_stale_sidecars(database_path).err();
            let rollback_error = if cleanup_error.is_some() {
                Some("Stale SQLite sidecars could not be removed safely".to_string())
            } else if rollback_swap.path.exists() {
                atomic_replace(&rollback_swap.path, database_path).err()
            } else {
                Some("Verified rollback swap file is missing".to_string())
            };
            let reopen = if rollback_error.is_none() {
                open_application_database(database_path)
            } else {
                Err("Rollback file replacement did not complete".to_string())
            };
            if let Ok(previous) = reopen {
                slot.install(previous);
                return Err(format!(
                    "{replacement_error}; previous database was restored"
                ));
            }

            let fallback = emergency_connection
                .take()
                .expect("verified emergency connection exists until recovery completes");
            slot.install(fallback);
            rollback.preserve();
            return Err(format!(
                "{replacement_error}; active filename rollback failed (cleanup: {}; replace: {}), so the verified previous database remains active at '{}'",
                cleanup_error.as_deref().unwrap_or("ok"),
                rollback_error.as_deref().unwrap_or("reopen failed"),
                rollback.path.display()
            ));
        }
        return Err(replacement_error);
    }

    db.1.smart_counts.lock().clear();
    db.1.station_sessions.lock().clear();
    drop(slot);
    reader_pause.finish()?;
    Ok(ImportBackupResult { roots })
}

#[tauri::command]
pub fn db_import_backup(
    app: AppHandle,
    db: State<Db>,
    consent: State<DestructiveConsentState>,
    src: String,
    consent_token: String,
) -> Result<ImportBackupResult, String> {
    let src_path = Path::new(&src);
    authorize_backup_path(&app, src_path)?;
    validate_backup_source(src_path)?;
    let canonical = canonicalize_existing_path(src_path)
        .map_err(|error| format!("Backup file is unavailable: {error}"))?;
    consent.consume(
        &consent_token,
        ConsentAction::ImportBackup,
        Some(canonical.to_string_lossy().as_ref()),
    )?;

    // Resolve the current library.db path.
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("no app data dir: {e}"))?;
    restore_from_path(db.inner(), src_path, &dir.join("library.db"))
}

#[tauri::command]
pub async fn db_relocate_root(app: AppHandle, old_root: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(selected) = app
            .dialog()
            .file()
            .set_title("Authorize restored music folder")
            .blocking_pick_folder()
        else {
            return Ok(None);
        };
        let selected = selected
            .into_path()
            .map_err(|error| format!("Folder picker did not return a local path: {error}"))?;
        let canonical_new = canonicalize_directory(&selected)?;
        let canonical_string = canonical_new.to_string_lossy().to_string();
        let database = app.state::<Db>();
        {
            let mut connection = database.0.lock();
            relocate_root(&mut connection, &old_root, &canonical_new)?;
        }
        database.1.smart_counts.lock().clear();
        database.1.station_sessions.lock().clear();

        // The webview never supplies the destination. Only the directory
        // returned by the Rust-side picker can become a trusted root. It is
        // intentionally not exposed through the asset protocol.
        crate::reconfigure_watcher(&app)?;
        Ok(Some(canonical_string))
    })
    .await
    .map_err(|error| format!("Root relocation task failed: {error}"))?
}

fn normalized_path(path: &str) -> String {
    #[cfg(windows)]
    let value = path.replace('\\', "/");
    #[cfg(not(windows))]
    let value = path.to_string();
    let trimmed = if value.len() > 1 {
        value.trim_end_matches('/').to_string()
    } else {
        value
    };
    #[cfg(windows)]
    {
        trimmed.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        trimmed
    }
}

fn same_physical_root(left: &str, right: &Path) -> bool {
    let right_string = right.to_string_lossy();
    if normalized_path(left) == normalized_path(&right_string) {
        return true;
    }
    canonicalize_directory(Path::new(left))
        .map(|canonical| normalized_path(&canonical.to_string_lossy()))
        .is_ok_and(|canonical| canonical == normalized_path(&right_string))
}

fn relocated_path(path: &str, old_root: &str, new_root: &Path) -> Option<String> {
    #[cfg(windows)]
    let path_normalized = path.replace('\\', "/");
    #[cfg(not(windows))]
    let path_normalized = path.to_string();
    #[cfg(windows)]
    let root_normalized = old_root.replace('\\', "/");
    #[cfg(not(windows))]
    let root_normalized = old_root.to_string();
    let root_normalized = if root_normalized.len() > 1 {
        root_normalized.trim_end_matches('/')
    } else {
        root_normalized.as_str()
    };
    let path_key = normalized_path(&path_normalized);
    let root_key = normalized_path(root_normalized);
    let relative = if path_key == root_key {
        ""
    } else {
        let prefix = if root_key == "/" {
            "/".to_string()
        } else {
            format!("{root_key}/")
        };
        if !path_key.starts_with(&prefix) {
            return None;
        }
        let start = if root_normalized == "/" {
            1
        } else {
            root_normalized.len() + 1
        };
        &path_normalized[start..]
    };
    let relocated = if relative.is_empty() {
        new_root.to_owned()
    } else {
        let components: Vec<_> = relative
            .split('/')
            .filter(|component| !component.is_empty())
            .collect();
        if components
            .iter()
            .any(|component| *component == "." || *component == "..")
        {
            return None;
        }
        components
            .into_iter()
            .fold(new_root.to_owned(), |path, component| path.join(component))
    };
    Some(relocated.to_string_lossy().to_string())
}

#[derive(Clone)]
struct PathMove {
    old: String,
    new: String,
    temporary: String,
}

fn table_moves(
    transaction: &Transaction<'_>,
    old_root: &str,
    new_root: &Path,
) -> Result<Vec<PathMove>, String> {
    let mut statement = transaction
        .prepare("SELECT path FROM tracks")
        .map_err(|error| error.to_string())?;
    let paths = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    build_moves(paths, old_root, new_root, "tracks")
}

fn build_moves(
    paths: Vec<String>,
    old_root: &str,
    new_root: &Path,
    namespace: &str,
) -> Result<Vec<PathMove>, String> {
    let existing: HashSet<String> = paths.iter().map(|path| normalized_path(path)).collect();
    let mut moves = Vec::new();
    let mut targets = HashSet::new();
    for (index, old) in paths.iter().enumerate() {
        let Some(new) = relocated_path(old, old_root, new_root) else {
            continue;
        };
        let target_key = normalized_path(&new);
        if !targets.insert(target_key.clone()) {
            return Err(format!(
                "Relocation would create duplicate paths in {namespace}: {new}"
            ));
        }
        let old_key = normalized_path(old);
        if existing.contains(&target_key)
            && !paths.iter().any(|candidate| {
                normalized_path(candidate) == target_key
                    && relocated_path(candidate, old_root, new_root).is_some()
            })
            && target_key != old_key
        {
            return Err(format!(
                "Relocation target already exists in {namespace}: {new}"
            ));
        }
        moves.push(PathMove {
            old: old.clone(),
            new,
            temporary: format!(
                "__ts_music_relocate_{}_{}_{}_{}__",
                std::process::id(),
                TEMP_SERIAL.fetch_add(1, Ordering::Relaxed),
                namespace,
                index
            ),
        });
    }
    Ok(moves)
}

fn apply_moves(transaction: &Transaction<'_>, moves: &[PathMove]) -> Result<(), String> {
    let sql = "UPDATE tracks SET path = ?2 WHERE path = ?1";
    for path_move in moves {
        transaction
            .execute(sql, params![path_move.old, path_move.temporary])
            .map_err(|error| error.to_string())?;
    }
    for path_move in moves {
        transaction
            .execute(sql, params![path_move.temporary, path_move.new])
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn relocate_root(
    connection: &mut Connection,
    old_root: &str,
    new_root: &Path,
) -> Result<(), String> {
    if !Path::new(old_root).is_absolute() {
        return Err("Stored root is not an absolute path".to_string());
    }
    let new_root_string = new_root.to_string_lossy().to_string();
    let same_root = same_physical_root(old_root, new_root);

    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let trusted: bool = transaction
        .prepare("SELECT 1 FROM roots WHERE path = ?1")
        .and_then(|mut statement| statement.exists(params![old_root]))
        .map_err(|error| error.to_string())?;
    let pending: bool = transaction
        .prepare("SELECT 1 FROM pending_roots WHERE path = ?1")
        .and_then(|mut statement| statement.exists(params![old_root]))
        .map_err(|error| error.to_string())?;
    if !trusted && !pending {
        return Err("Old root is not an exact stored library root".to_string());
    }

    let mut roots_statement = transaction
        .prepare("SELECT path FROM roots UNION ALL SELECT path FROM pending_roots")
        .map_err(|error| error.to_string())?;
    let stored_roots = roots_statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(roots_statement);

    if same_root {
        // Confirming an existing pending root is an authorization transition,
        // not a path relocation. Preserve every track/stat path byte-for-byte.
        if pending {
            transaction
                .execute(
                    "DELETE FROM pending_roots WHERE path = ?1",
                    params![old_root],
                )
                .map_err(|error| error.to_string())?;
            let already_trusted = transaction
                .prepare("SELECT path FROM roots")
                .and_then(|mut statement| {
                    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
                    for row in rows {
                        if same_physical_root(&row?, new_root) {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                })
                .map_err(|error| error.to_string())?;
            if !already_trusted {
                transaction
                    .execute(
                        "INSERT INTO roots(path) VALUES (?1)",
                        params![new_root_string],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        return transaction.commit().map_err(|error| error.to_string());
    }

    if stored_roots
        .iter()
        .any(|root| root != old_root && same_physical_root(root, new_root))
    {
        return Err("New root collides with another stored library root".to_string());
    }

    // Relationships use tracks.id, so only the mutable path attribute moves.
    // No LIKE is used, so '%' and '_' remain literal and Music never matches Music2.
    let track_moves = table_moves(&transaction, old_root, new_root)?;
    apply_moves(&transaction, &track_moves)?;

    transaction
        .execute(
            "DELETE FROM pending_roots WHERE path = ?1",
            params![old_root],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute("DELETE FROM roots WHERE path = ?1", params![old_root])
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "INSERT INTO roots(path) VALUES (?1)",
            params![new_root_string],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn app_connection(path: &Path) -> Connection {
        let mut connection = Connection::open(path).expect("open app database");
        connection.execute_batch(SCHEMA).expect("create app schema");
        migrate(&mut connection).expect("migrate app schema");
        connection
    }

    fn insert_track(connection: &Connection, path: &Path) {
        connection
            .execute(
                "INSERT INTO tracks(path, title, artist, album) VALUES (?1, 'Song', 'Artist', 'Album')",
                params![path.to_string_lossy().as_ref()],
            )
            .expect("insert track");
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

        let error = validate_backup_source(&invalid).expect_err("reject unrelated database");
        assert!(error.contains("missing required table 'tracks'"), "{error}");
    }

    #[test]
    fn backup_import_accepts_database_with_required_schema() {
        let dir = TestDir::new();
        let valid = dir.join("valid.tsmback");
        drop(app_connection(&valid));

        validate_backup_source(&valid).expect("accept app backup schema");
    }

    #[test]
    fn backup_import_rejects_newer_schema_versions() {
        let dir = TestDir::new();
        let future = dir.join("future.db");
        let connection = app_connection(&future);
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .expect("set future schema version");
        drop(connection);

        let error = validate_backup_source(&future).expect_err("reject future backup");
        assert!(error.contains("newer than supported"), "{error}");
    }

    #[test]
    fn staged_restore_migrates_and_quarantines_backup_roots() {
        let dir = TestDir::new();
        let source = dir.join("source.db");
        let target = dir.join("library.db");
        let source_connection = app_connection(&source);
        let imported_root = dir.join("from-backup");
        source_connection
            .execute(
                "INSERT INTO roots(path) VALUES (?1)",
                params![imported_root.to_string_lossy().as_ref()],
            )
            .expect("insert source root");
        drop(source_connection);

        let (staging, statuses) = staged_restore(&source, &target).expect("prepare restore");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].path, imported_root.to_string_lossy());
        let staged = open_read_only(&staging.path).expect("open staged database");
        let trusted_count: i64 = staged
            .query_row("SELECT COUNT(*) FROM roots", [], |row| row.get(0))
            .expect("count trusted roots");
        let pending_count: i64 = staged
            .query_row("SELECT COUNT(*) FROM pending_roots", [], |row| row.get(0))
            .expect("count pending roots");
        let version: i32 = staged
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read schema version");
        assert_eq!((trusted_count, pending_count), (0, 1));
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn restore_replaces_active_database_from_consistent_snapshots() {
        let dir = TestDir::new();
        let active_path = dir.join("library.db");
        let source_path = dir.join("source.db");
        let active = app_connection(&active_path);
        active
            .execute("INSERT INTO kv(k, v) VALUES ('marker', '\"old\"')", [])
            .expect("insert active marker");
        let source = app_connection(&source_path);
        source
            .execute("INSERT INTO kv(k, v) VALUES ('marker', '\"new\"')", [])
            .expect("insert source marker");
        drop(source);
        let database = Db::from_connection(active);

        restore_from_path(&database, &source_path, &active_path).expect("restore database");
        let marker: String = database
            .0
            .lock()
            .query_row("SELECT v FROM kv WHERE k = 'marker'", [], |row| row.get(0))
            .expect("read restored marker");
        assert_eq!(marker, "\"new\"");
        validate_read_only_database(&active_path).expect("restored main file remains valid");
        drop(database);
    }

    #[test]
    fn relocation_is_separator_bound_and_treats_like_wildcards_literally() {
        let dir = TestDir::new();
        let old_root = dir.join("%_Music");
        let sibling_root = dir.join("%_Music2");
        let destination = dir.join("Moved");
        fs::create_dir(&destination).expect("create destination");
        let canonical_destination = canonicalize_directory(&destination).expect("canonical target");
        let old_track = old_root.join("album").join("song.flac");
        let sibling_track = sibling_root.join("keep.flac");

        let mut connection = Connection::open_in_memory().expect("open database");
        connection.execute_batch(SCHEMA).expect("create schema");
        migrate(&mut connection).expect("migrate schema");
        connection
            .execute(
                "INSERT INTO pending_roots(path) VALUES (?1)",
                params![old_root.to_string_lossy().as_ref()],
            )
            .expect("insert pending root");
        insert_track(&connection, &old_track);
        insert_track(&connection, &sibling_track);
        connection
            .execute(
                "INSERT INTO stats(track_id, play_count)
                 SELECT id, 4 FROM tracks WHERE path = ?1",
                params![old_track.to_string_lossy().as_ref()],
            )
            .expect("insert stats");

        relocate_root(
            &mut connection,
            old_root.to_string_lossy().as_ref(),
            &canonical_destination,
        )
        .expect("relocate root");

        let moved_track = canonical_destination.join("album").join("song.flac");
        let moved_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM tracks WHERE path = ?1",
                params![moved_track.to_string_lossy().as_ref()],
                |row| row.get(0),
            )
            .expect("query moved track");
        let sibling_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM tracks WHERE path = ?1",
                params![sibling_track.to_string_lossy().as_ref()],
                |row| row.get(0),
            )
            .expect("query sibling track");
        let trusted_root: String = connection
            .query_row("SELECT path FROM roots", [], |row| row.get(0))
            .expect("query trusted root");
        assert_eq!((moved_count, sibling_count), (1, 1));
        assert_eq!(trusted_root, canonical_destination.to_string_lossy());
    }

    #[test]
    fn confirming_same_pending_root_promotes_authority_without_rewriting_paths() {
        let dir = TestDir::new();
        let selected_root = dir.join("ExistingMusic");
        fs::create_dir(&selected_root).expect("create selected root");
        let canonical_root = canonicalize_directory(&selected_root).expect("canonical root");
        let stored_track = selected_root.join("song.flac");

        let mut connection = Connection::open_in_memory().expect("open database");
        connection.execute_batch(SCHEMA).expect("create schema");
        migrate(&mut connection).expect("migrate schema");
        connection
            .execute(
                "INSERT INTO pending_roots(path) VALUES (?1)",
                params![selected_root.to_string_lossy().as_ref()],
            )
            .expect("insert pending root");
        insert_track(&connection, &stored_track);

        relocate_root(
            &mut connection,
            selected_root.to_string_lossy().as_ref(),
            &canonical_root,
        )
        .expect("promote pending root");

        let pending_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM pending_roots", [], |row| row.get(0))
            .expect("count pending roots");
        let trusted_root: String = connection
            .query_row("SELECT path FROM roots", [], |row| row.get(0))
            .expect("read trusted root");
        let unchanged_track: String = connection
            .query_row("SELECT path FROM tracks", [], |row| row.get(0))
            .expect("read track path");
        assert_eq!(pending_count, 0);
        assert_eq!(trusted_root, canonical_root.to_string_lossy());
        assert_eq!(unchanged_track, stored_track.to_string_lossy());
    }

    #[test]
    fn relocation_collision_preflight_rolls_back_every_table() {
        let dir = TestDir::new();
        let old_root = dir.join("Music");
        let destination = dir.join("Moved");
        fs::create_dir(&destination).expect("create destination");
        let canonical_destination = canonicalize_directory(&destination).expect("canonical target");
        let old_track = old_root.join("song.flac");
        let colliding_track = canonical_destination.join("song.flac");

        let mut connection = Connection::open_in_memory().expect("open database");
        connection.execute_batch(SCHEMA).expect("create schema");
        migrate(&mut connection).expect("migrate schema");
        connection
            .execute(
                "INSERT INTO roots(path) VALUES (?1)",
                params![old_root.to_string_lossy().as_ref()],
            )
            .expect("insert root");
        insert_track(&connection, &old_track);
        insert_track(&connection, &colliding_track);

        let error = relocate_root(
            &mut connection,
            old_root.to_string_lossy().as_ref(),
            &canonical_destination,
        )
        .expect_err("reject collision");
        assert!(error.contains("target already exists"), "{error}");
        let old_track_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM tracks WHERE path = ?1",
                params![old_track.to_string_lossy().as_ref()],
                |row| row.get(0),
            )
            .expect("old track remains");
        let old_root_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM roots WHERE path = ?1",
                params![old_root.to_string_lossy().as_ref()],
                |row| row.get(0),
            )
            .expect("old root remains");
        assert_eq!((old_track_count, old_root_count), (1, 1));
    }
}

#[tauri::command]
pub fn db_prune_and_get_missing(db: State<Db>) -> Result<Vec<MissingTrackInfo>, String> {
    let mut conn = db.0.lock();
    let mut missing = Vec::new();
    let mut to_remove = Vec::new();
    let pending_roots: Vec<String> = {
        let mut statement = conn
            .prepare("SELECT path FROM pending_roots")
            .map_err(|error| error.to_string())?;
        let roots = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        roots
    };

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
            let remains_untrusted = pending_roots.iter().any(|root| {
                Path::new(root).is_absolute()
                    && relocated_path(&path, root, Path::new(root)).is_some()
            });
            if remains_untrusted || !Path::new(&path).exists() {
                missing.push(MissingTrackInfo {
                    title,
                    artist,
                    path: path.clone(),
                });
                to_remove.push(path);
            }
        }
    }

    if !to_remove.is_empty() || !pending_roots.is_empty() {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for p in &to_remove {
            tx.execute("DELETE FROM tracks WHERE path = ?1", params![p])
                .map_err(|e| e.to_string())?;
        }
        tx.execute("DELETE FROM pending_roots", [])
            .map_err(|error| error.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }

    Ok(missing)
}
