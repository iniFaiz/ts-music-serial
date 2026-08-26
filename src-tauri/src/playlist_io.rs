// M3U / M3U8 playlist import & export.
//
// Export writes a plain #EXTM3U file (with #EXTINF metadata and absolute paths
// by default, or portable relative paths on request) from a playlist's current
// tracks. Import reads an M3U, resolves its entries (relative paths are
// resolved against the file's own directory), parses the metadata of audio
// files already covered by a trusted root or exact transient grant, upserts
// them into the library and returns the resolved paths so the frontend can
// build a playlist from them. Playlist contents never grant root authority.

use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

use tauri::{AppHandle, Runtime, State};

use crate::{db::Db, limits};

fn authorize_playlist_file<R: Runtime>(
    app: &AppHandle<R>,
    path: &std::path::Path,
) -> Result<(), String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if extension != "m3u" && extension != "m3u8" {
        return Err("Playlist path must end in .m3u or .m3u8".to_string());
    }
    if !crate::is_allowed_path(app, path) {
        return Err("Playlist path was not authorized by the file picker".to_string());
    }
    Ok(())
}

// Normalise a path for prefix comparison (unify separators, drop trailing slash,
// lowercase — Windows paths are case-insensitive).
// Case-insensitive OsStr comparison — Windows paths (including drive letters)
// are case-insensitive; elsewhere exact bytes apply.
#[cfg(windows)]
fn os_components_equal(a: &OsStr, b: &OsStr) -> bool {
    a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
}

#[cfg(not(windows))]
fn os_components_equal(a: &OsStr, b: &OsStr) -> bool {
    a == b
}

fn path_components_equal(a: &Component, b: &Component) -> bool {
    match (a, b) {
        (Component::Normal(a), Component::Normal(b)) => os_components_equal(a, b),
        // Drive letters (`C:` vs `c:`) and other Windows prefixes are
        // case-insensitive too.
        (Component::Prefix(_), Component::Prefix(_)) => {
            os_components_equal(a.as_os_str(), b.as_os_str())
        }
        (a, b) => a.as_os_str() == b.as_os_str(),
    }
}

// Number of leading anchor components (Windows prefix like `C:\` or UNC, plus
// the root separator). Two paths can only be made relative to each other when
// they share the same anchor.
fn anchored_component_count(components: &[Component]) -> usize {
    components
        .iter()
        .take_while(|component| {
            matches!(component, Component::Prefix(_) | Component::RootDir)
        })
        .count()
}

// Build a target path relative to `base` (both absolute), for portable M3U
// entries that keep working when the playlist file travels together with the
// audio files. Returns None — the caller falls back to the absolute path —
// when no portable form exists, most notably when the two paths live under
// different anchors (e.g. C:\ vs D:\ on Windows).
fn make_relative(base: &Path, target: &Path) -> Option<PathBuf> {
    let base: Vec<Component> = base.components().collect();
    let target: Vec<Component> = target.components().collect();

    if anchored_component_count(&base) != anchored_component_count(&target) {
        return None;
    }

    let common = base.len().min(target.len());
    let mut shared = 0;
    while shared < common && path_components_equal(&base[shared], &target[shared]) {
        shared += 1;
    }

    // Bail out when either path diverges inside its anchor (e.g. `C:\` vs
    // `D:\`, two different UNC shares) — there is no portable relative form.
    if shared < anchored_component_count(&base) || shared < anchored_component_count(&target) {
        return None;
    }

    let mut out = PathBuf::new();
    for _ in shared..base.len() {
        out.push("..");
    }
    for component in &target[shared..] {
        match component {
            Component::Normal(segment) => out.push(segment),
            // CurDir/ParentDir inside the remainder cannot be expressed
            // faithfully without normalizing; fall back to absolute.
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        return None;
    }
    Some(out)
}

// Both commands touch the filesystem (and import parses full audio tags per
// entry), so they run off the main thread.
#[tauri::command(async)]
pub fn export_m3u(
    app: AppHandle,
    db: State<Db>,
    dest: String,
    playlist_id: String,
    relative_paths: Option<bool>,
) -> Result<usize, String> {
    limits::validate_text(&playlist_id, "Playlist ID", 128)?;
    let dest_path = PathBuf::from(&dest);
    authorize_playlist_file(&app, &dest_path)?;

    let tracks = crate::db::playlists::db_playlist_tracks(db, playlist_id)?;
    let write_relative = relative_paths.unwrap_or(false);
    let base_dir = dest_path.parent().map(Path::to_path_buf);

    let mut out = String::from("#EXTM3U\n");
    for t in &tracks {
        // Relative mode is best-effort: entries outside the destination's
        // subtree (different drive, unrelated mount) stay absolute so they
        // still resolve on this machine.
        let entry_path: PathBuf = if write_relative {
            base_dir
                .as_deref()
                .and_then(|base| make_relative(base, Path::new(&t.path)))
                .unwrap_or_else(|| PathBuf::from(&t.path))
        } else {
            PathBuf::from(&t.path)
        };
        out.push_str(&format!(
            "#EXTINF:{},{} - {}\n{}\n",
            t.duration_secs,
            t.artist,
            t.title,
            entry_path.to_string_lossy()
        ));
    }
    fs::write(&dest_path, out).map_err(|e| e.to_string())?;
    Ok(tracks.len())
}

#[tauri::command(async)]
pub fn import_m3u(app: AppHandle, db: State<Db>, src: String) -> Result<Vec<String>, String> {
    let src_path = PathBuf::from(&src);
    authorize_playlist_file(&app, &src_path)?;
    let metadata = fs::metadata(&src_path).map_err(|error| error.to_string())?;
    if metadata.len() > limits::MAX_M3U_BYTES {
        return Err(format!(
            "Playlist file is too large (max {} MB)",
            limits::MAX_M3U_BYTES / 1024 / 1024
        ));
    }
    let content = fs::read_to_string(&src_path).map_err(|e| e.to_string())?;
    let base = src_path.parent().map(|p| p.to_path_buf());

    let mut tracks: Vec<crate::MusicTrack> = Vec::new();
    for (index, line) in content.lines().enumerate() {
        if index >= limits::MAX_M3U_LINES {
            return Err(format!(
                "Playlist contains too many lines (max {})",
                limits::MAX_M3U_LINES
            ));
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        limits::validate_text(line, "Playlist entry", limits::MAX_PATH_BYTES)?;
        let raw = PathBuf::from(line);
        let resolved = if raw.is_absolute() {
            raw
        } else if let Some(b) = &base {
            b.join(&raw)
        } else {
            raw
        };

        // Playlist contents are data, not authority. Entries must already be
        // covered by a persisted library root or an exact OS session grant.
        let Ok(canonical) = crate::resolve_allowed_audio(&app, &resolved) else {
            continue;
        };
        if let Some(t) = crate::parse_metadata(&canonical) {
            tracks.push(t);
        }
    }

    if tracks.is_empty() {
        return Err("No playable audio files found in the M3U".into());
    }
    crate::db::db_upsert_tracks(db, tracks.clone())?;
    Ok(tracks.iter().map(|t| t.path.clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Manager;

    fn relative(base: &str, target: &str) -> Option<String> {
        make_relative(Path::new(base), Path::new(target)).map(|p| p.to_string_lossy().into_owned())
    }

    #[cfg(windows)]
    #[test]
    fn relative_paths_stay_within_the_same_drive_anchor() {
        assert_eq!(relative(r"C:\Music\Lists", r"C:\Music\Songs\a.mp3"), Some(r"..\Songs\a.mp3".into()));
        assert_eq!(relative(r"C:\Music", r"C:\Music\a.mp3"), Some("a.mp3".into()));
        assert_eq!(relative(r"C:\Music\A\B", r"C:\Music\x.mp3"), Some(r"..\..\x.mp3".into()));
        // Case-insensitive comparison across drive letters and directories.
        assert_eq!(relative(r"c:\music\Lists", r"C:\MUSIC\a.flac"), Some(r"..\a.flac".into()));
    }

    #[cfg(windows)]
    #[test]
    fn relative_paths_reject_cross_drive_targets() {
        assert_eq!(relative(r"C:\Music", r"D:\Songs\a.mp3"), None);
        // UNC shares and drive paths are different anchors even when both
        // contribute a prefix+root pair.
        assert_eq!(relative(r"C:\Music", r"\\server\share\a.mp3"), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn relative_paths_work_for_posix_absolute_paths() {
        assert_eq!(relative("/home/u/lists", "/home/u/songs/a.mp3"), Some("../songs/a.mp3".into()));
        assert_eq!(relative("/home/u/lists/deep", "/home/u/x.mp3"), Some("../../x.mp3".into()));
        assert_eq!(relative("/home/u", "/home/u/a.mp3"), Some("a.mp3".into()));
    }

    #[test]
    fn playlist_file_authorization_is_extension_and_scope_bound() {
        let app = tauri::test::mock_app();
        let base =
            std::env::temp_dir().join(format!("ts-music-playlist-auth-{}", std::process::id()));
        std::fs::create_dir_all(&base).expect("create test directory");
        let selected = base.join("selected.m3u8");
        let sibling = base.join("sibling.m3u8");
        let disguised = base.join("playlist.m3u8.exe");
        app.asset_protocol_scope()
            .allow_file(&selected)
            .expect("allow selected playlist");
        app.asset_protocol_scope()
            .allow_file(&disguised)
            .expect("allow disguised file");

        assert!(authorize_playlist_file(app.handle(), &selected).is_ok());
        assert!(authorize_playlist_file(app.handle(), &sibling).is_err());
        assert_eq!(
            authorize_playlist_file(app.handle(), &disguised)
                .expect_err("reject unsupported extension"),
            "Playlist path must end in .m3u or .m3u8"
        );

        let _ = std::fs::remove_dir_all(base);
    }
}
