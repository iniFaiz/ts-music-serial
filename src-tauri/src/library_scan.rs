//! Music library discovery, metadata parsing, and path authorization.

use std::collections::{HashMap, HashSet};
#[cfg(windows)]
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use crate::{library_db as db, library_index, MusicTrack};

const SUPPORTED_EXTS: [&str; 6] = ["mp3", "flac", "wav", "m4a", "ogg", "aac"];

/// `std::fs::canonicalize` returns an extended-length `\\?\` path on Windows.
/// SQLite path identity predates that representation, so storing it verbatim
/// makes one physical file distinct from its legacy `D:\...` spelling. Strip
/// only the namespace marker while retaining canonical symlink/junction
/// resolution and every UTF-16 code unit of the real path.
#[cfg(windows)]
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    const VERBATIM: &[u16] = &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const VERBATIM_UNC: &[u16] = &[
        b'\\' as u16,
        b'\\' as u16,
        b'?' as u16,
        b'\\' as u16,
        b'U' as u16,
        b'N' as u16,
        b'C' as u16,
        b'\\' as u16,
    ];
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    let normalized = if wide.starts_with(VERBATIM_UNC) {
        let mut value = vec![b'\\' as u16, b'\\' as u16];
        value.extend_from_slice(&wide[VERBATIM_UNC.len()..]);
        value
    } else if wide.starts_with(VERBATIM) {
        wide[VERBATIM.len()..].to_vec()
    } else {
        return path;
    };
    PathBuf::from(OsString::from_wide(&normalized))
}

#[cfg(not(windows))]
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    path
}

pub(crate) fn canonicalize_existing_path(path: &Path) -> std::io::Result<PathBuf> {
    fs::canonicalize(path).map(strip_verbatim_prefix)
}

// Parse a ReplayGain gain string like "-6.54 dB" / "+3.2" into decibels.
pub(crate) fn parse_rg_db(s: &str) -> Option<f32> {
    let cleaned = s
        .trim()
        .trim_end_matches(|c: char| c.is_alphabetic() || c.is_whitespace());
    cleaned.trim().parse::<f32>().ok()
}

// Filter supported audio files by extension.
pub(crate) fn is_audio_file(path: &Path) -> bool {
    match path.extension() {
        Some(ext) => {
            let ext_str = ext.to_string_lossy().to_lowercase();
            SUPPORTED_EXTS.contains(&ext_str.as_str())
        }
        None => false,
    }
}

/// Resolve and validate a prospective library root.  Persisting canonical
/// paths makes separator-bound containment checks reliable and prevents a
/// symlink/junction selected as a root from changing its target later.
pub(crate) fn canonicalize_directory(path: &Path) -> Result<PathBuf, String> {
    let canonical = canonicalize_existing_path(path).map_err(|error| {
        format!(
            "Cannot resolve library folder '{}': {error}",
            path.display()
        )
    })?;
    let metadata = fs::metadata(&canonical).map_err(|error| {
        format!(
            "Cannot inspect library folder '{}': {error}",
            path.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "Library root is not a directory: {}",
            path.display()
        ));
    }
    Ok(canonical)
}

// Generic picker-authorised paths (backup files, playlist files, cover images)
// continue to use Tauri's scope. Audio authority is intentionally stricter and
// comes from persisted roots or a native OS transient-file grant below.
pub(crate) fn is_allowed_path<R: Runtime>(app: &AppHandle<R>, path: &Path) -> bool {
    app.asset_protocol_scope().is_allowed(path)
}

pub(crate) fn canonical_roots<R: Runtime>(app: &AppHandle<R>) -> Vec<PathBuf> {
    let Some(database) = app.try_state::<db::Db>() else {
        return Vec::new();
    };
    db::roots(database.inner())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|root| canonicalize_directory(Path::new(&root)).ok())
        .collect()
}

pub(crate) fn path_is_within_roots(path: &Path, roots: &[PathBuf]) -> bool {
    roots
        .iter()
        .any(|root| path == root || path.starts_with(root))
}

/// Return the canonical audio target only when it is below an exact persisted
/// root or was delivered by a trusted OS file-open/drag operation.  Callers
/// that mutate files should operate on the returned path, not the webview's
/// original spelling, to close symlink/junction escapes.
pub(crate) fn resolve_allowed_audio<R: Runtime>(
    app: &AppHandle<R>,
    path: &Path,
) -> Result<PathBuf, String> {
    if !is_audio_file(path) {
        return Err("Unsupported audio file type".to_string());
    }
    let canonical = canonicalize_existing_path(path)
        .map_err(|error| format!("Cannot resolve audio path '{}': {error}", path.display()))?;
    let metadata = fs::metadata(&canonical)
        .map_err(|error| format!("Cannot inspect audio path '{}': {error}", path.display()))?;
    if !metadata.is_file() || !is_audio_file(&canonical) {
        return Err("Audio path is not a supported regular file".to_string());
    }

    if path_is_within_roots(&canonical, &canonical_roots(app)) {
        return Ok(canonical);
    }
    if app
        .try_state::<LibraryAccessState>()
        .is_some_and(|state| state.session_files.lock().contains(&canonical))
    {
        return Ok(canonical);
    }
    Err("Path is not within an authorised music folder".to_string())
}

pub(crate) fn is_allowed_audio<R: Runtime>(app: &AppHandle<R>, path: &Path) -> bool {
    resolve_allowed_audio(app, path).is_ok()
}

const DROP_GRANT_TTL: Duration = Duration::from_secs(120);
const MAX_DROP_GRANTS: usize = 16;

struct DropGrant {
    paths: Vec<PathBuf>,
    issued_at: Instant,
}

/// Filesystem authority originating outside IPC. Native Tauri drag/drop events
/// create short-lived, single-use grants; file association launches create
/// exact-file session grants. Neither mechanism authorises sibling files.
pub(crate) struct LibraryAccessState {
    drop_grants: Mutex<HashMap<String, DropGrant>>,
    session_files: Mutex<HashSet<PathBuf>>,
    next_grant: AtomicU64,
}

impl LibraryAccessState {
    pub(crate) fn new() -> Self {
        Self {
            drop_grants: Mutex::new(HashMap::new()),
            session_files: Mutex::new(HashSet::new()),
            next_grant: AtomicU64::new(1),
        }
    }

    fn record_drop(&self, paths: &[PathBuf]) -> Option<String> {
        let canonical: Vec<PathBuf> = paths
            .iter()
            .filter_map(|path| canonicalize_existing_path(path).ok())
            .filter(|path| path.is_dir() || (path.is_file() && is_audio_file(path)))
            .collect();
        if canonical.is_empty() {
            return None;
        }

        let mut grants = self.drop_grants.lock();
        grants.retain(|_, grant| grant.issued_at.elapsed() <= DROP_GRANT_TTL);
        if grants.len() >= MAX_DROP_GRANTS {
            if let Some(oldest) = grants
                .iter()
                .min_by_key(|(_, grant)| grant.issued_at)
                .map(|(id, _)| id.clone())
            {
                grants.remove(&oldest);
            }
        }
        let serial = self.next_grant.fetch_add(1, Ordering::Relaxed);
        let token = format!("drop-{serial:x}-{:x}", std::process::id());
        grants.insert(
            token.clone(),
            DropGrant {
                paths: canonical,
                issued_at: Instant::now(),
            },
        );
        Some(token)
    }

    fn consume_drop(&self, token: &str) -> Result<Vec<PathBuf>, String> {
        let mut grants = self.drop_grants.lock();
        grants.retain(|_, grant| grant.issued_at.elapsed() <= DROP_GRANT_TTL);
        grants
            .remove(token)
            .map(|grant| grant.paths)
            .ok_or_else(|| "Drag-and-drop grant is missing, expired, or already used".to_string())
    }

    fn grant_session_file(&self, path: &Path) -> Result<PathBuf, String> {
        let canonical = canonicalize_existing_path(path)
            .map_err(|error| format!("Cannot resolve dropped audio file: {error}"))?;
        if !canonical.is_file() || !is_audio_file(&canonical) {
            return Err("Transient grant is not a supported audio file".to_string());
        }
        self.session_files.lock().insert(canonical.clone());
        Ok(canonical)
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DropGrantNotice {
    pub(crate) grant_id: String,
}

pub(crate) fn record_native_drop<R: Runtime>(
    app: &AppHandle<R>,
    paths: &[PathBuf],
) -> Option<DropGrantNotice> {
    app.try_state::<LibraryAccessState>()
        .and_then(|state| state.record_drop(paths))
        .map(|grant_id| DropGrantNotice { grant_id })
}

pub(crate) fn consume_native_drop(app: &AppHandle, token: &str) -> Result<Vec<PathBuf>, String> {
    app.state::<LibraryAccessState>().consume_drop(token)
}

pub(crate) fn grant_session_audio<R: Runtime>(
    app: &AppHandle<R>,
    path: &Path,
) -> Result<PathBuf, String> {
    app.state::<LibraryAccessState>().grant_session_file(path)
}

// Content fingerprint used to re-identify a track after it is moved or renamed
// (so its stats/favorites/playlist memberships can be migrated, see
// db_prune_missing): file size + MD5 over three sampled windows — head 64 KiB,
// 32 KiB from the middle, tail 32 KiB. At most ~128 KiB of IO per file, yet two
// different files only collide if they agree on size AND all three regions.
// (MD5 is fine here — this is dedup/identity, not a security boundary.)
pub(crate) fn compute_fingerprint(path: &Path) -> Option<String> {
    use md5::{Digest, Md5};
    use std::io::{Read, Seek, SeekFrom};

    let mut f = fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut hasher = Md5::new();
    hasher.update(len.to_le_bytes());

    fn read_window(f: &mut fs::File, buf: &mut [u8], pos: u64, want: usize) -> Option<usize> {
        f.seek(SeekFrom::Start(pos)).ok()?;
        let mut taken = 0usize;
        while taken < want {
            let n = f.read(&mut buf[taken..want]).ok()?;
            if n == 0 {
                break;
            }
            taken += n;
        }
        Some(taken)
    }

    let mut buf = vec![0u8; 64 * 1024];
    let head = read_window(&mut f, &mut buf, 0, 64 * 1024)?;
    hasher.update(&buf[..head]);
    // Middle + tail only add signal beyond what the head already covered.
    if len > 64 * 1024 {
        let mid = read_window(&mut f, &mut buf, len / 2, 32 * 1024)?;
        hasher.update(&buf[..mid]);
        let tail_start = len.saturating_sub(32 * 1024).max(64 * 1024);
        let tail = read_window(&mut f, &mut buf, tail_start, 32 * 1024)?;
        hasher.update(&buf[..tail]);
    }
    Some(format!("{len}:{:x}", hasher.finalize()))
}

// Extract metadata for a single file.
pub(crate) fn parse_metadata(path: &Path) -> Option<MusicTrack> {
    fn bounded_tag(mut value: String, max_bytes: usize) -> String {
        if value.len() <= max_bytes {
            return value;
        }
        let mut end = max_bytes;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
        value
    }

    let path_str = path.to_string_lossy().to_string();

    let file_metadata = fs::metadata(path).ok()?;
    let file_size = file_metadata.len();
    let mtime_ns = file_metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0);
    // This is the time the application first sees a new row, not a filesystem
    // timestamp. The upsert deliberately preserves it on later tag/file edits.
    let date_added = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    let tagged_file = Probe::open(path).ok()?.read().ok()?;

    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());
    let properties = tagged_file.properties();

    let title = bounded_tag(
        tag.and_then(|t| t.title().map(|s| s.to_string()))
            .unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            }),
        1_024,
    );
    let artist = bounded_tag(
        tag.and_then(|t| t.artist().map(|s| s.to_string()))
            .unwrap_or_else(|| "Unknown Artist".to_string()),
        1_024,
    );
    let album = bounded_tag(
        tag.and_then(|t| t.album().map(|s| s.to_string()))
            .unwrap_or_else(|| "Unknown Album".to_string()),
        1_024,
    );
    // Genre is optional — many files lack it. Trimmed so blank tags become None,
    // which lets the frontend's smart-playlist genre rules ignore them cleanly.
    let genre = tag
        .and_then(|t| t.genre().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .map(|genre| bounded_tag(genre, 256));
    let year = tag.and_then(|t| t.year()).filter(|year| *year <= 9_999);
    let track_number = tag
        .and_then(|t| t.track())
        .filter(|track| (1..=9_999).contains(track));
    let duration_secs = properties.duration().as_secs();
    let has_cover = tag.as_ref().is_some_and(|t| !t.pictures().is_empty());

    let sample_rate = properties.sample_rate();
    let bit_depth = properties.bit_depth();

    let track_gain_db = tag
        .and_then(|t| t.get_string(&ItemKey::ReplayGainTrackGain))
        .and_then(parse_rg_db);
    let track_peak = tag
        .and_then(|t| t.get_string(&ItemKey::ReplayGainTrackPeak))
        .and_then(|s| s.trim().parse::<f32>().ok());

    Some(MusicTrack {
        path: path_str,
        title,
        artist,
        album,
        genre,
        duration_secs,
        date_added,
        year,
        track_number,
        has_cover,
        sample_rate,
        bit_depth,
        track_gain_db,
        track_peak,
        file_size,
        mtime_ns,
    })
}

fn canonical_root_strings(paths: impl IntoIterator<Item = PathBuf>) -> Vec<String> {
    let mut roots = Vec::new();
    for path in paths {
        let value = path.to_string_lossy().to_string();
        if !roots.contains(&value) {
            roots.push(value);
        }
    }
    roots
}

/// Register trusted roots as one logical operation.  The database is updated
/// first and the watcher is rebuilt from that database state. Library roots are
/// deliberately never added to the webview asset-protocol scope; playback and
/// metadata access stay behind path-authorized Rust commands.
pub(crate) fn register_library_roots(
    app: &AppHandle,
    roots: &[PathBuf],
) -> Result<Vec<String>, String> {
    let canonical: Vec<PathBuf> = roots
        .iter()
        .map(|root| canonicalize_directory(root))
        .collect::<Result<_, _>>()?;
    let new_roots = canonical_root_strings(canonical.clone());
    let database = app.state::<db::Db>();
    let previous = db::roots(database.inner())?;

    db::insert_roots(database.inner(), &new_roots)?;
    if let Err(error) = crate::reconfigure_watcher(app) {
        let _ = db::replace_roots(database.inner(), &previous);
        let _ = crate::reconfigure_watcher(app);
        return Err(error);
    }
    Ok(new_roots)
}

/// Open the native picker inside Rust. The webview supplies no path, so a
/// forged invoke cannot register an arbitrary directory. Selection, canonical
/// validation, DB persistence, media scope, watcher setup, and initial scan all
/// remain on the trusted side of the IPC boundary.
#[tauri::command]
pub(crate) async fn add_library_root(
    app: AppHandle,
    use_parallelism: bool,
) -> Result<Option<library_index::IndexSummary>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(selected) = app
            .dialog()
            .file()
            .set_title("Add music folder")
            .blocking_pick_folder()
        else {
            return Ok(None);
        };
        let selected = selected
            .into_path()
            .map_err(|error| format!("Folder picker did not return a local path: {error}"))?;
        let root = canonicalize_directory(&selected)?;
        register_library_roots(&app, std::slice::from_ref(&root))?;
        let mut summary = library_index::index_authorized_paths(
            &app,
            vec![root.clone()],
            use_parallelism,
            false,
        )?;
        summary.roots = canonical_root_strings([root]);
        Ok(Some(summary))
    })
    .await
    .map_err(|error| format!("Add-library-root task failed: {error}"))?
}

/// Rebuild watcher state only from SQLite. Existing roots are upgraded to
/// canonical spellings. Missing/offline roots remain persisted (for removable
/// drives) but are not watched or indexed until they resolve again.
#[tauri::command]
pub(crate) fn restore_roots(app: AppHandle) -> Result<Vec<String>, String> {
    let database = app.state::<db::Db>();
    let previous = db::roots(database.inner())?;
    let mut canonical = Vec::new();
    let mut roots = Vec::new();
    for root in &previous {
        if let Ok(resolved) = canonicalize_directory(Path::new(root)) {
            let value = resolved.to_string_lossy().to_string();
            if !roots.contains(&value) {
                roots.push(value);
                canonical.push(resolved);
            }
        } else if !roots.contains(root) {
            roots.push(root.clone());
        }
    }
    db::replace_roots(database.inner(), &roots)?;
    if let Err(error) = crate::reconfigure_watcher(&app) {
        let _ = db::replace_roots(database.inner(), &previous);
        let _ = crate::reconfigure_watcher(&app);
        return Err(error);
    }
    Ok(roots)
}

/// Removing is destructive, but it can only target an exact root that already
/// exists in the trusted root table; this command cannot introduce authority.
#[tauri::command]
pub(crate) fn remove_library_root(
    app: AppHandle,
    consent: State<'_, crate::security::DestructiveConsentState>,
    root: String,
    consent_token: String,
) -> Result<Vec<String>, String> {
    let database = app.state::<db::Db>();
    let stored = db::roots(database.inner())?;
    let requested_canonical = canonicalize_directory(Path::new(&root)).ok();
    let existing = stored
        .iter()
        .find(|candidate| {
            candidate.as_str() == root
                || requested_canonical.as_ref().is_some_and(|requested| {
                    canonicalize_directory(Path::new(candidate))
                        .is_ok_and(|saved| saved == *requested)
                })
        })
        .cloned()
        .ok_or_else(|| "Library root is not registered".to_string())?;

    consent.consume(
        &consent_token,
        crate::security::ConsentAction::RemoveLibraryRoot,
        Some(&existing),
    )?;
    if !db::delete_root(database.inner(), &existing)? {
        return Err("Library root is not registered".to_string());
    }
    if let Err(error) = crate::reconfigure_watcher(&app) {
        let _ = db::insert_roots(database.inner(), std::slice::from_ref(&existing));
        let _ = crate::reconfigure_watcher(&app);
        return Err(error);
    }

    match db::db_remove_under_root(app.state::<db::Db>(), existing.clone()) {
        Ok(removed) => Ok(removed),
        Err(error) => {
            let _ = db::insert_roots(database.inner(), std::slice::from_ref(&existing));
            let _ = crate::reconfigure_watcher(&app);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tauri::Manager;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("ts-music-path-auth-{}-{nonce}", std::process::id()));
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
    fn audio_extension_allowlist_is_case_insensitive() {
        assert!(is_audio_file(Path::new("track.FLAC")));
        assert!(is_audio_file(Path::new("track.m4a")));
        assert!(!is_audio_file(Path::new("cover.png")));
        assert!(!is_audio_file(Path::new("no-extension")));
    }

    #[cfg(windows)]
    #[test]
    fn canonical_storage_path_omits_windows_verbatim_namespace() {
        let dir = TestDir::new();
        let canonical = canonicalize_existing_path(&dir.0).expect("canonicalize test directory");
        assert!(!canonical.to_string_lossy().starts_with(r"\\?\"));
    }

    #[test]
    fn persisted_root_allows_audio_only_inside_that_root() {
        let app = tauri::test::mock_app();
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute("CREATE TABLE roots(path TEXT PRIMARY KEY)", [])
            .expect("create roots table");
        app.manage(db::Db::from_connection(conn));
        app.manage(LibraryAccessState::new());
        let dir = TestDir::new();
        let library = dir.join("library");
        let outside = dir.join("outside");
        std::fs::create_dir_all(library.join("nested")).expect("create library");
        std::fs::create_dir(&outside).expect("create outside directory");
        let allowed = library.join("nested").join("song.flac");
        let wrong_type = library.join("nested").join("notes.txt");
        let denied = outside.join("song.flac");
        std::fs::write(&allowed, b"audio").expect("write allowed file");
        std::fs::write(&wrong_type, b"text").expect("write wrong type");
        std::fs::write(&denied, b"audio").expect("write denied file");

        let canonical = canonicalize_directory(&library).expect("canonical library");
        db::insert_roots(
            app.state::<db::Db>().inner(),
            &[canonical.to_string_lossy().to_string()],
        )
        .expect("persist root");

        assert!(is_allowed_audio(app.handle(), &allowed));
        assert!(!is_allowed_audio(app.handle(), &wrong_type));
        assert!(!is_allowed_audio(app.handle(), &denied));
    }

    #[test]
    fn os_file_grant_does_not_authorize_its_siblings() {
        let app = tauri::test::mock_app();
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute("CREATE TABLE roots(path TEXT PRIMARY KEY)", [])
            .expect("create roots table");
        app.manage(db::Db::from_connection(conn));
        app.manage(LibraryAccessState::new());
        let dir = TestDir::new();
        let selected = dir.join("selected.mp3");
        let sibling = dir.join("private.mp3");
        std::fs::write(&selected, b"selected").expect("write selected file");
        std::fs::write(&sibling, b"private").expect("write sibling file");
        grant_session_audio(app.handle(), &selected).expect("grant selected file");

        assert!(is_allowed_audio(app.handle(), &selected));
        assert!(!is_allowed_audio(app.handle(), &sibling));
    }

    #[test]
    fn asset_scope_alone_is_not_audio_authority() {
        let app = tauri::test::mock_app();
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute("CREATE TABLE roots(path TEXT PRIMARY KEY)", [])
            .expect("create roots table");
        app.manage(db::Db::from_connection(conn));
        app.manage(LibraryAccessState::new());
        let dir = TestDir::new();
        let selected = dir.join("dialog-selected.mp3");
        std::fs::write(&selected, b"audio").expect("write audio file");
        app.asset_protocol_scope()
            .allow_file(&selected)
            .expect("scope selected file");

        assert!(!is_allowed_audio(app.handle(), &selected));
    }

    #[test]
    fn drop_grants_are_single_use() {
        let state = LibraryAccessState::new();
        let dir = TestDir::new();
        let selected = dir.join("selected.flac");
        std::fs::write(&selected, b"audio").expect("write selected file");
        let token = state
            .record_drop(std::slice::from_ref(&selected))
            .expect("issue drop grant");
        assert_eq!(state.consume_drop(&token).expect("consume").len(), 1);
        assert!(state.consume_drop(&token).is_err());
    }

    #[test]
    fn component_prefix_does_not_escape_root() {
        let dir = TestDir::new();
        let music = dir.join("Music");
        let music_two = dir.join("Music2");
        std::fs::create_dir_all(&music).expect("create Music");
        std::fs::create_dir_all(&music_two).expect("create Music2");
        let roots = vec![canonicalize_directory(&music).expect("canonical Music")];
        let other = canonicalize_directory(&music_two).expect("canonical Music2");
        assert!(!path_is_within_roots(&other, &roots));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_resolves_outside_persisted_root() {
        use std::os::unix::fs::symlink;

        let dir = TestDir::new();
        let library = dir.join("library");
        let outside = dir.join("outside");
        std::fs::create_dir_all(&library).expect("create library");
        std::fs::create_dir_all(&outside).expect("create outside");
        let secret = outside.join("secret.flac");
        std::fs::write(&secret, b"audio").expect("write outside file");
        symlink(&outside, library.join("linked")).expect("create directory symlink");

        let canonical_root = canonicalize_directory(&library).expect("canonical root");
        let escaped = std::fs::canonicalize(library.join("linked/secret.flac"))
            .expect("canonical escaped file");
        assert!(!path_is_within_roots(&escaped, &[canonical_root]));
    }
}
