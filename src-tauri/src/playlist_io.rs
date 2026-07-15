// M3U / M3U8 playlist import & export.
//
// Export writes a plain #EXTM3U file (with #EXTINF metadata and absolute paths)
// from a playlist's current tracks. Import reads an M3U, resolves its entries
// (relative paths are resolved against the file's own directory), parses the
// metadata of the audio files that actually exist, registers their folders so
// the tracks are streamable, upserts them into the library and returns the
// resolved paths so the frontend can build a playlist from them.

use std::fs;
use std::path::PathBuf;

use tauri::{AppHandle, Runtime, State};

use crate::db::Db;

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
fn norm(s: &str) -> String {
    s.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

fn is_under_any(dir_n: &str, roots: &[String]) -> bool {
    roots.iter().any(|r| {
        let rn = norm(r);
        dir_n == rn || dir_n.starts_with(&format!("{rn}/"))
    })
}

#[tauri::command]
pub fn export_m3u(
    app: AppHandle,
    db: State<Db>,
    dest: String,
    playlist_id: String,
) -> Result<usize, String> {
    let dest_path = PathBuf::from(&dest);
    authorize_playlist_file(&app, &dest_path)?;

    let tracks = crate::db::playlists::db_playlist_tracks(db, playlist_id)?;
    let mut out = String::from("#EXTM3U\n");
    for t in &tracks {
        out.push_str(&format!(
            "#EXTINF:{},{} - {}\n{}\n",
            t.duration_secs, t.artist, t.title, t.path
        ));
    }
    fs::write(&dest_path, out).map_err(|e| e.to_string())?;
    Ok(tracks.len())
}

#[tauri::command]
pub fn import_m3u(app: AppHandle, db: State<Db>, src: String) -> Result<Vec<String>, String> {
    let src_path = PathBuf::from(&src);
    authorize_playlist_file(&app, &src_path)?;
    let content = fs::read_to_string(&src_path).map_err(|e| e.to_string())?;
    let base = src_path.parent().map(|p| p.to_path_buf());

    let mut tracks: Vec<crate::MusicTrack> = Vec::new();
    let mut roots = crate::db::db_roots(db.clone())?;
    let mut added_dirs: Vec<String> = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let raw = PathBuf::from(line);
        let resolved = if raw.is_absolute() {
            raw
        } else if let Some(b) = &base {
            b.join(&raw)
        } else {
            raw
        };

        if !crate::is_audio_file(&resolved) || !resolved.exists() {
            continue;
        }
        // Authorize the containing folder so the track streams and its cover can
        // be read (mirrors what a normal scan does).
        if let Some(dir) = resolved.parent() {
            let dir_s = dir.to_string_lossy().to_string();
            crate::allow_root(&app, &dir_s);
            let dn = norm(&dir_s);
            if !is_under_any(&dn, &roots) && !added_dirs.iter().any(|d| norm(d) == dn) {
                added_dirs.push(dir_s);
            }
        }
        if let Some(t) = crate::parse_metadata(&resolved) {
            tracks.push(t);
        }
    }

    if tracks.is_empty() {
        return Err("No playable audio files found in the M3U".into());
    }
    // Persist any new folders as roots so the tracks stay streamable next launch.
    if !added_dirs.is_empty() {
        roots.extend(added_dirs);
        crate::db::db_set_roots(db.clone(), roots)?;
    }
    crate::db::db_upsert_tracks(db, tracks.clone())?;
    Ok(tracks.iter().map(|t| t.path.clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Manager;

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
