//! Music library discovery, metadata parsing, and path authorization.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use tauri::{AppHandle, Manager, Runtime};

use crate::MusicTrack;

const SUPPORTED_EXTS: [&str; 6] = ["mp3", "flac", "wav", "m4a", "ogg", "aac"];

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

// Allow a scanned directory through the asset protocol so the frontend can
// stream its audio files (and so cover extraction is permitted for them).
pub(crate) fn allow_root<R: Runtime>(app: &AppHandle<R>, path: &str) {
    let _ = app.asset_protocol_scope().allow_directory(path, true);
}

// A path may only be touched by file-reading commands if it is an audio file
// inside one of the directories the user explicitly scanned. This prevents the
// (untrusted) webview from coercing the backend into reading arbitrary files.
pub(crate) fn is_allowed_path<R: Runtime>(app: &AppHandle<R>, path: &Path) -> bool {
    app.asset_protocol_scope().is_allowed(path)
}

pub(crate) fn is_allowed_audio<R: Runtime>(app: &AppHandle<R>, path: &Path) -> bool {
    is_audio_file(path) && is_allowed_path(app, path)
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
    let path_str = path.to_string_lossy().to_string();

    // Date created (falling back to modified) as a unix timestamp.
    let date_added = fs::metadata(path)
        .and_then(|m| m.created().or_else(|_| m.modified()))
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let tagged_file = Probe::open(path).ok()?.read().ok()?;

    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());
    let properties = tagged_file.properties();

    let title = tag
        .and_then(|t| t.title().map(|s| s.to_string()))
        .unwrap_or_else(|| {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });
    let artist = tag
        .and_then(|t| t.artist().map(|s| s.to_string()))
        .unwrap_or_else(|| "Unknown Artist".to_string());
    let album = tag
        .and_then(|t| t.album().map(|s| s.to_string()))
        .unwrap_or_else(|| "Unknown Album".to_string());
    // Genre is optional — many files lack it. Trimmed so blank tags become None,
    // which lets the frontend's smart-playlist genre rules ignore them cleanly.
    let genre = tag
        .and_then(|t| t.genre().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty());
    let year = tag.and_then(|t| t.year());
    let track_number = tag.and_then(|t| t.track());
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
    })
}

// Re-grant streaming access to previously scanned roots. The frontend persists
// the list of scanned folders and calls this on startup, because the asset
// protocol scope is in-memory and resets each launch.
#[tauri::command]
pub(crate) fn restore_roots(app: AppHandle, roots: Vec<String>) {
    for root in roots {
        allow_root(&app, &root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn directory_scope_allows_audio_only_inside_that_root() {
        let app = tauri::test::mock_app();
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

        allow_root(app.handle(), library.to_str().expect("utf-8 path"));

        assert!(is_allowed_audio(app.handle(), &allowed));
        assert!(!is_allowed_audio(app.handle(), &wrong_type));
        assert!(!is_allowed_audio(app.handle(), &denied));
    }

    #[test]
    fn single_file_scope_does_not_authorize_its_siblings() {
        let app = tauri::test::mock_app();
        let dir = TestDir::new();
        let selected = dir.join("selected.mp3");
        let sibling = dir.join("private.mp3");
        std::fs::write(&selected, b"selected").expect("write selected file");
        std::fs::write(&sibling, b"private").expect("write sibling file");
        app.asset_protocol_scope()
            .allow_file(&selected)
            .expect("allow selected file");

        assert!(is_allowed_audio(app.handle(), &selected));
        assert!(!is_allowed_audio(app.handle(), &sibling));
    }
}
