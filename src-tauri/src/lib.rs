use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
#[cfg(target_os = "windows")]
use std::sync::Arc;
use std::time::Duration;

// parking_lot's Mutex never poisons and its guard is returned directly (no
// Result), so there is no lock().unwrap() panic path. Mutex::new is const, so
// it still works for the process-wide statics below.
use parking_lot::Mutex;

use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use tauri::{AppHandle, Emitter, Manager, State};
mod cover_cache;
mod discord;
mod library_db;
mod library_index;
mod library_scan;
mod lyrics;
mod metadata_tags;
mod online_metadata;
mod player;
mod playlist_io;
#[cfg(target_os = "windows")]
mod thumbbar;
mod tray;
mod waveform;
mod window_drag;

#[cfg(test)]
mod ipc_contract_tests;

use library_db as db;

pub(crate) use cover_cache::{cover_cache_dir, cover_cache_key};
use cover_cache::{get_track_cover, get_track_cover_path, get_track_palette};
use library_index::index_library;
use library_scan::{add_library_root, remove_library_root, restore_roots};
pub(crate) use library_scan::{
    compute_fingerprint, is_allowed_audio, is_allowed_path, is_audio_file, parse_metadata,
    parse_rg_db, resolve_allowed_audio,
};
use metadata_tags::{preview_image, write_track_tags};
pub(crate) use player::build_decoder;
use player::{
    compute_track_gain, init_audio_player, list_output_devices, playback_session_intent,
    playback_session_snapshot, player_pause, player_prepare_next, player_resume, player_seek,
    player_set_equalizer, player_set_normalization, player_set_normalization_settings,
    player_set_spectrum_enabled, player_set_transition, player_set_volume, player_spectrum,
    player_status, player_stop, set_output_device, set_wasapi_exclusive, spawn_player_ticker,
    AudioPlayer,
};
use waveform::get_waveform;

// Data sent to the frontend. Also `Deserialize` so a previously-scanned library
// (e.g. from the IndexedDB→SQLite migration) can round-trip back through the DB
// layer (see `db`).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MusicTrack {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: Option<String>,
    pub duration_secs: u64,
    pub date_added: u64,
    pub year: Option<u32>,
    pub track_number: Option<u32>,
    pub has_cover: bool,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    // ReplayGain track gain/peak read from tags (if present), used by the volume
    // normalization feature. `None` when the file carries no ReplayGain tags.
    pub track_gain_db: Option<f32>,
    pub track_peak: Option<f32>,
}

// ---------------------------------------------------------------------------
// "Open with ts-music": files passed on the command line (file association)
// are stashed here until the frontend is ready to consume them. The
// single-instance hook appends the argv of any second launch and pings the
// webview with `open-files-pending`.

struct PendingOpenFiles(Mutex<Vec<String>>);

fn collect_audio_args<I: IntoIterator<Item = String>>(args: I) -> Vec<String> {
    args.into_iter()
        .filter(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .filter(|p| p.is_file() && is_audio_file(p))
        .map(|p| p.to_string_lossy().to_string())
        .collect()
}

#[tauri::command]
fn take_pending_open_files(app: AppHandle, state: State<PendingOpenFiles>) -> Vec<String> {
    let pending: Vec<String> = state.0.lock().drain(..).collect();
    let mut files = Vec::with_capacity(pending.len());
    for f in pending {
        // Grant streaming/cover access to each file individually — opening a
        // song from Explorer must not widen the scope to its whole folder.
        if let Ok(canonical) = library_scan::grant_session_audio(&app, Path::new(&f)) {
            let _ = app.asset_protocol_scope().allow_file(&canonical);
            files.push(canonical.to_string_lossy().to_string());
        }
    }
    files
}

// Parse metadata for audio files that live outside the library (opened via
// file association). Nothing is imported into the DB; the frontend builds a
// transient queue from the returned tracks.
#[tauri::command]
async fn probe_files(app: AppHandle, paths: Vec<String>) -> Result<Vec<MusicTrack>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut tracks = Vec::new();
        for p in paths {
            let pb = PathBuf::from(&p);
            // `take_pending_open_files` grants each OS-provided file explicitly.
            // Never widen the scope from this webview-callable command itself.
            if let Ok(canonical) = resolve_allowed_audio(&app, &pb) {
                if let Some(track) = parse_metadata(&canonical) {
                    tracks.push(track);
                }
            }
        }
        Ok(tracks)
    })
    .await
    .map_err(|e| format!("Probe task failed: {e}"))?
}

#[cfg(test)]
mod file_argument_tests {
    use super::collect_audio_args;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("ts-music-file-args-{}-{nonce}", std::process::id()));
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
    fn command_line_file_collection_accepts_only_existing_audio_files() {
        let dir = TestDir::new();
        let audio = dir.join("song.FLAC");
        let text = dir.join("notes.txt");
        let missing = dir.join("missing.mp3");
        let folder = dir.join("folder.mp3");
        std::fs::write(&audio, b"audio").expect("write audio file");
        std::fs::write(&text, b"text").expect("write text file");
        std::fs::create_dir(&folder).expect("create misleading directory");

        let collected = collect_audio_args([
            "--hidden".to_string(),
            audio.to_string_lossy().to_string(),
            text.to_string_lossy().to_string(),
            missing.to_string_lossy().to_string(),
            folder.to_string_lossy().to_string(),
        ]);

        assert_eq!(collected, vec![audio.to_string_lossy().to_string()]);
    }
}

// Lyrics — local tag / sidecar .lrc, then NetEase → LRCLIB → Musixmatch
// ---------------------------------------------------------------------------

fn lyrics_cache_dir(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_cache_dir().ok()?.join("lyrics");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

// Look for lyrics shipped with the file itself: a sidecar "<name>.lrc" (usually
// hand-synced) takes priority over an embedded lyrics tag.
fn local_lyrics(path: &Path) -> Option<lyrics::Lyrics> {
    let sidecar = path.with_extension("lrc");
    if let Ok(text) = fs::read_to_string(&sidecar) {
        if let Some(l) = lyrics::lyrics_from_text(&text, "Local (.lrc)") {
            return Some(l);
        }
    }
    if let Ok(tagged) = Probe::open(path).and_then(|p| p.read()) {
        let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
        if let Some(text) = tag.and_then(|t| t.get_string(&ItemKey::Lyrics)) {
            if let Some(l) = lyrics::lyrics_from_text(text, "Embedded") {
                return Some(l);
            }
        }
    }
    None
}

// Resolve lyrics through the full pipeline, caching the result (including a
// "not found" sentinel) on disk. `force` bypasses the cache for a manual retry.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn get_lyrics(
    app: AppHandle,
    path: String,
    title: String,
    artist: String,
    album: String,
    duration_secs: u64,
    lyrics_source: String,
    force: bool,
) -> Option<lyrics::Lyrics> {
    let path_buf = resolve_allowed_audio(&app, Path::new(&path)).ok()?;

    if lyrics_source == "none" {
        return None;
    }

    // Disk cache keyed by path+mtime+size+provider. "null" = previously not found.
    // `_v2` schema tag: bumped when LyricLine gained word-level timing + romaji,
    // so stale line-only cache entries are re-fetched instead of reused.
    let cache_file = cover_cache_key(&path_buf).and_then(|k| {
        lyrics_cache_dir(&app).map(|d| d.join(format!("{k}_{lyrics_source}_v2.json")))
    });
    if !force {
        if let Some(cf) = &cache_file {
            if let Ok(data) = fs::read_to_string(cf) {
                if data.trim() == "null" {
                    return None;
                }
                if let Ok(l) = serde_json::from_str::<lyrics::Lyrics>(&data) {
                    // Make sure the cached source matches the requested source
                    let source_matches = match lyrics_source.as_str() {
                        "local" => l.source.to_lowercase() == "local",
                        "lrclib" => l.source.to_lowercase() == "lrclib",
                        "netease" => l.source.to_lowercase() == "netease",
                        "musixmatch" => l.source.to_lowercase() == "musixmatch",
                        _ => true,
                    };
                    if source_matches {
                        let mut loaded_lyrics = l;
                        if loaded_lyrics.source.to_lowercase() == "netease" {
                            loaded_lyrics
                                .lines
                                .retain(|line| !lyrics::is_netease_metadata(line));
                        }
                        // Idempotent: peels parenthetical background vocals into a
                        // secondary tier (covers caches written before this existed).
                        lyrics::apply_background(&mut loaded_lyrics.lines);
                        return Some(loaded_lyrics);
                    }
                }
            }
        }
    }

    let mut result = None;

    if lyrics_source == "local" {
        // 1. Local lyrics (file IO + lofty) on the blocking pool.
        let pb = path_buf.clone();
        result = tauri::async_runtime::spawn_blocking(move || local_lyrics(&pb))
            .await
            .ok()
            .flatten();
    } else {
        // Remote providers
        static HTTP_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
        let client = HTTP_CLIENT.get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .build()
                .unwrap_or_else(|_| reqwest::Client::new())
        });

        if lyrics_source == "lrclib" {
            result = lyrics::from_lrclib(client, &title, &artist, &album, duration_secs).await;
        } else if lyrics_source == "netease" {
            result = lyrics::from_netease(client, &title, &artist).await;
        } else if lyrics_source == "musixmatch" {
            // The user token (if any) lives in the OS credential store, never in
            // the app DB — read it here rather than accepting it from the webview.
            let token = musixmatch_token_get().unwrap_or_default();
            result = lyrics::from_musixmatch(
                client,
                &title,
                &artist,
                &album,
                duration_secs,
                token.trim(),
            )
            .await;
        }
    }

    // Peel parenthetical background vocals into the secondary tier before caching,
    // so the stored JSON already carries the split (and re-reads stay no-ops).
    if let Some(l) = result.as_mut() {
        lyrics::apply_background(&mut l.lines);
    }

    if let Some(cf) = &cache_file {
        let data = match &result {
            Some(l) => serde_json::to_string(l).unwrap_or_else(|_| "null".to_string()),
            None => "null".to_string(),
        };
        let _ = fs::write(cf, data);
    }

    result
}

// ---------------------------------------------------------------------------
// Musixmatch token — stored in the OS credential store (never in the app DB)
// ---------------------------------------------------------------------------

const KEYRING_SERVICE: &str = "ts-music";
const KEYRING_USER: &str = "musixmatch_token";

fn musixmatch_token_get() -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .ok()?
        .get_password()
        .ok()
        .filter(|s| !s.trim().is_empty())
}

// Store (or, for an empty string, clear) the Musixmatch user token securely.
#[tauri::command]
fn set_musixmatch_token(token: String) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())?;
    if token.trim().is_empty() {
        // delete_credential errors if none exists; that's fine.
        let _ = entry.delete_credential();
        Ok(())
    } else {
        entry.set_password(token.trim()).map_err(|e| e.to_string())
    }
}

// Whether a token is configured (the value itself is never returned to the UI).
#[tauri::command]
fn musixmatch_token_status() -> bool {
    musixmatch_token_get().is_some()
}

// ---------------------------------------------------------------------------
// Filesystem watching — library auto-update
//
// A single RecommendedWatcher covers all scanned roots. Raw event paths are
// debounced, indexed into SQLite by Rust, then reported to the frontend.
// ---------------------------------------------------------------------------

struct FileWatcher {
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    // Passes relevant paths to the coalescing/indexing thread.
    evt_tx: mpsc::Sender<Vec<PathBuf>>,
}

// Drain a burst, index its unique paths, and emit once per quiet window.
fn spawn_fs_coalescer(app: AppHandle, rx: mpsc::Receiver<Vec<PathBuf>>) {
    std::thread::spawn(move || {
        while let Ok(first) = rx.recv() {
            // Block until the first event of a burst.
            let mut paths: HashSet<PathBuf> = first.into_iter().collect();
            // Swallow further events until things go quiet for the debounce window.
            loop {
                match rx.recv_timeout(Duration::from_millis(800)) {
                    Ok(more) => paths.extend(more),
                    Err(mpsc::RecvTimeoutError::Timeout) => break,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
            if paths.is_empty() {
                continue;
            }
            match library_index::index_watcher_paths(&app, paths.into_iter().collect()) {
                Ok(summary) => {
                    let _ = app.emit("library-changed", summary);
                }
                Err(error) => eprintln!("Incremental library index failed: {error}"),
            }
        }
    });
}

// (Re)configure the watcher from the persisted root table. Replacing the
// watcher drops the previous one, unwatching the old set. No IPC caller can
// widen this set by supplying paths.
pub(crate) fn reconfigure_watcher(app: &AppHandle) -> Result<(), String> {
    use notify::event::{EventKind, ModifyKind};
    use notify::{RecursiveMode, Watcher};

    let state = app.state::<FileWatcher>();
    let tx = state.evt_tx.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            // Structural changes may contain a directory; other modifications
            // are relevant only when they point at a supported audio file.
            let structural = matches!(
                &event.kind,
                EventKind::Create(_)
                    | EventKind::Remove(_)
                    | EventKind::Modify(ModifyKind::Name(_))
            );
            let content_change = matches!(&event.kind, EventKind::Modify(_));
            if !structural && !content_change {
                return;
            }
            let relevant_paths: Vec<PathBuf> = event
                .paths
                .into_iter()
                .filter(|path| structural || is_audio_file(path))
                .collect();
            if !relevant_paths.is_empty() {
                let _ = tx.send(relevant_paths);
            }
        }
    })
    .map_err(|e| e.to_string())?;

    for root in db::roots(app.state::<db::Db>().inner())? {
        // Missing/offline removable roots remain persisted but cannot be scoped
        // or watched until they exist again.
        let Ok(canonical) = library_scan::canonicalize_directory(Path::new(&root)) else {
            continue;
        };
        watcher
            .watch(&canonical, RecursiveMode::Recursive)
            .map_err(|error| format!("Failed to watch '{}': {error}", canonical.display()))?;
    }

    *state.watcher.lock() = Some(watcher);
    Ok(())
}

#[tauri::command]
fn watch_roots(app: AppHandle) -> Result<(), String> {
    reconfigure_watcher(&app)
}

// ---------------------------------------------------------------------------
// System Media Transport Controls (Windows SMTC)
//
// Surfaces the now-playing track in the Windows volume/media overlay and wires
// up the hardware/keyboard media keys (play/pause/next/prev). The SMTC COM
// object is bound to the main window's HWND and its apartment, so *every* call
// into it is marshalled onto the main thread via run_on_main_thread. Button
// presses are forwarded to the frontend as `media-control` events.
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
struct MediaController(Mutex<Option<souvlaki::MediaControls>>);
// SAFETY: the inner MediaControls is only ever touched on the main thread (the
// thread that created it and pumps the window's message loop). All command
// handlers hop onto that thread before locking the mutex.
#[cfg(target_os = "windows")]
unsafe impl Send for MediaController {}
#[cfg(target_os = "windows")]
unsafe impl Sync for MediaController {}

#[cfg(target_os = "windows")]
fn init_media_controls(app: &AppHandle) {
    use souvlaki::{MediaControlEvent, MediaControls, PlatformConfig, SeekDirection};
    use tauri::Emitter;

    let window = match app.get_webview_window("main") {
        Some(w) => w,
        None => return,
    };
    let hwnd = match window.hwnd() {
        // Convert the platform HWND to the raw pointer souvlaki expects. The
        // double cast tolerates either an isize- or pointer-shaped HWND field.
        Ok(h) => h.0 as isize as *mut std::ffi::c_void,
        Err(_) => return,
    };

    let config = PlatformConfig {
        dbus_name: "ts-music",
        display_name: "ts-music",
        hwnd: Some(hwnd),
    };

    let mut controls = match MediaControls::new(config) {
        Ok(c) => c,
        Err(_) => return,
    };

    let app_handle = app.clone();
    let _ = controls.attach(move |event: MediaControlEvent| {
        let (action, position): (&str, Option<f64>) = match event {
            MediaControlEvent::Play => ("play", None),
            MediaControlEvent::Pause => ("pause", None),
            MediaControlEvent::Toggle => ("toggle", None),
            MediaControlEvent::Next => ("next", None),
            MediaControlEvent::Previous => ("previous", None),
            MediaControlEvent::Stop => ("stop", None),
            MediaControlEvent::SetPosition(p) => ("seek", Some(p.0.as_secs_f64())),
            MediaControlEvent::Seek(SeekDirection::Forward) => ("seek_forward", None),
            MediaControlEvent::Seek(SeekDirection::Backward) => ("seek_backward", None),
            _ => return,
        };
        let _ = app_handle.emit(
            "media-control",
            serde_json::json!({ "action": action, "position": position }),
        );
    });

    if let Some(controller) = app.try_state::<Arc<MediaController>>() {
        *controller.0.lock() = Some(controls);
    }
}

#[cfg(target_os = "windows")]
#[tauri::command]
fn smtc_set_metadata(
    app: AppHandle,
    controller: State<Arc<MediaController>>,
    title: String,
    artist: String,
    album: String,
    duration: f64,
    path: String,
) {
    // Reuse the on-disk cover thumbnail (already generated for the UI) as the
    // SMTC artwork, if it exists. We never decode here — no art is fine.
    let cover_uri = cover_cache_dir(&app).and_then(|dir| {
        let key = cover_cache_key(Path::new(&path))?;
        let file = dir.join(format!("{key}.jpg"));
        if file.exists() {
            Some(format!("file://{}", file.display()))
        } else {
            None
        }
    });

    let arc = controller.inner().clone();
    let _ = app.run_on_main_thread(move || {
        let mut guard = arc.0.lock();
        if let Some(controls) = guard.as_mut() {
            let metadata = souvlaki::MediaMetadata {
                title: Some(&title),
                artist: Some(&artist),
                album: Some(&album),
                cover_url: cover_uri.as_deref(),
                duration: Some(Duration::from_secs_f64(duration.max(0.0))),
            };
            let _ = controls.set_metadata(metadata);
        }
    });
}

#[cfg(target_os = "windows")]
#[tauri::command]
fn smtc_set_playback(
    app: AppHandle,
    controller: State<Arc<MediaController>>,
    playing: bool,
    position: f64,
) {
    let arc = controller.inner().clone();
    let _ = app.run_on_main_thread(move || {
        let mut guard = arc.0.lock();
        if let Some(controls) = guard.as_mut() {
            let progress = Some(souvlaki::MediaPosition(Duration::from_secs_f64(
                position.max(0.0),
            )));
            let state = if playing {
                souvlaki::MediaPlayback::Playing { progress }
            } else {
                souvlaki::MediaPlayback::Paused { progress }
            };
            let _ = controls.set_playback(state);
        }
    });

    // Keep the taskbar thumbnail's Play/Pause button in sync with playback.
    thumbbar::set_playing(&app, playing);
}

// Non-Windows stubs so the frontend can call these unconditionally.
#[cfg(not(target_os = "windows"))]
#[tauri::command]
fn smtc_set_metadata(
    _title: String,
    _artist: String,
    _album: String,
    _duration: f64,
    _path: String,
) {
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
fn smtc_set_playback(_playing: bool, _position: f64) {}

#[tauri::command]
fn player_show_in_folder(app: AppHandle, path: String) -> Result<(), String> {
    use std::process::Command;
    let path_buf = resolve_allowed_audio(&app, Path::new(&path))?;

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let mut path_win = path_buf.to_string_lossy().replace('/', "\\");
        if path_win.starts_with(r"\\?\UNC\") {
            path_win = format!(r"\\{}", &path_win[8..]);
        } else if path_win.starts_with(r"\\?\") {
            path_win = path_win[4..].to_string();
        }
        Command::new("explorer")
            .raw_arg(format!(r#"/select,"{}""#, path_win))
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg("-R")
            .arg(&path_buf)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(parent) = path_buf.parent() {
            Command::new("xdg-open")
                .arg(parent)
                .spawn()
                .map_err(|e| e.to_string())?;
        } else {
            return Err("Parent directory not found".to_string());
        }
    }

    Ok(())
}

#[tauri::command]
fn player_delete_file(app: AppHandle, db: State<db::Db>, path: String) -> Result<(), String> {
    let path_buf = resolve_allowed_audio(&app, Path::new(&path))?;
    let canonical = path_buf.to_string_lossy().to_string();
    if db::tracks::db_track(db, canonical)?.is_none() {
        return Err("File is not an indexed library track".to_string());
    }
    fs::remove_file(&path_buf).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Channel feeding the filesystem-watch coalescer (spawned in setup).
    let (fs_tx, fs_rx) = mpsc::channel::<Vec<PathBuf>>();
    let fs_rx = Mutex::new(Some(fs_rx));

    let audio = init_audio_player();
    // The exclusive engine reuses the main player's EQ / spectrum / volume /
    // normalization state so those features behave identically in exclusive mode.
    #[cfg(target_os = "windows")]
    let exclusive_player = player::init_exclusive_player(&audio);

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        // Must be the FIRST plugin: a second launch (e.g. double-clicking an
        // associated audio file in Explorer) focuses this instance and forwards
        // its argv here instead of opening a second window that would fight
        // over the SQLite database.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            tray::show_main_window(app);
            let files = collect_audio_args(argv.into_iter().skip(1));
            if !files.is_empty() {
                if let Some(st) = app.try_state::<PendingOpenFiles>() {
                    st.0.lock().extend(files);
                }
                let _ = app.emit("open-files-pending", ());
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(audio)
        .manage(discord::DiscordState::new())
        .manage(tray::TrayState::new())
        // Audio files passed to the very first launch (file association).
        .manage(PendingOpenFiles(Mutex::new(collect_audio_args(
            std::env::args().skip(1),
        ))))
        .manage(FileWatcher {
            watcher: Mutex::new(None),
            evt_tx: fs_tx,
        })
        .manage(library_index::LibraryIndexState::new())
        .manage(library_scan::LibraryAccessState::new())
        // Close-to-tray: while the setting is enabled, closing the main window
        // hides it instead of quitting; Quit lives in the tray menu.
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                if let Some(st) = window.app_handle().try_state::<tray::TrayState>() {
                    if st.close_to_tray() {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            }
            tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. })
                if window.label() == "main" =>
            {
                if let Some(grant) = library_scan::record_native_drop(window.app_handle(), paths) {
                    let _ = window.emit("library-drop-grant", grant);
                }
            }
            _ => {}
        });

    #[cfg(target_os = "windows")]
    {
        builder = builder
            .manage(Arc::new(MediaController(Mutex::new(None))))
            .manage(thumbbar::ThumbbarController::new())
            .manage(exclusive_player);
    }

    builder
        .setup(move |_app| {
            // Open the SQLite library database (source of truth for tracks, stats,
            // playlists, favorites, recents). Managed here so every db_* command
            // can reach it via State<Db>.
            let database = db::init(_app.handle())?;
            _app.manage(database);

            // Backfill content fingerprints for rows that predate the column,
            // in the background. Delayed a bit so it never competes with the
            // startup burst of queries.
            {
                let handle = _app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(6));
                    let state = handle.state::<library_index::LibraryIndexState>();
                    let _job = state.job.lock();
                    db::backfill_fingerprints(&handle);
                });
            }

            // Allow the cover-thumbnail cache dir through the asset protocol so
            // the webview can load cached covers by path (convertFileSrc) instead
            // of receiving them base64-encoded over IPC.
            if let Some(dir) = cover_cache_dir(_app.handle()) {
                let _ = _app.asset_protocol_scope().allow_directory(&dir, false);
            }

            #[cfg(target_os = "windows")]
            {
                init_media_controls(_app.handle());
                thumbbar::init(_app.handle());
            }
            // Start the debounced filesystem-change → library-changed pump.
            if let Some(rx) = fs_rx.lock().take() {
                spawn_fs_coalescer(_app.handle().clone(), rx);
            }

            // Rebuild filesystem authority only from SQLite. The frontend does
            // not provide roots during startup or watcher configuration.
            restore_roots(_app.handle().clone())?;

            // Spawn the audio player tick thread for crossfade/gapless transitions
            let player = _app.state::<AudioPlayer>().inner().clone();
            spawn_player_ticker(_app.handle().clone(), player);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            add_library_root,
            remove_library_root,
            index_library,
            get_track_cover,
            get_track_cover_path,
            get_track_palette,
            get_waveform,
            restore_roots,
            window_drag::start_window_drag,
            player_prepare_next,
            player_pause,
            player_resume,
            player_set_volume,
            player_seek,
            player_stop,
            player_status,
            playback_session_snapshot,
            playback_session_intent,
            player_spectrum,
            player_set_spectrum_enabled,
            player_set_equalizer,
            list_output_devices,
            set_output_device,
            player_set_normalization,
            compute_track_gain,
            get_lyrics,
            set_musixmatch_token,
            musixmatch_token_status,
            playlist_io::export_m3u,
            playlist_io::import_m3u,
            watch_roots,
            smtc_set_metadata,
            smtc_set_playback,
            player_show_in_folder,
            player_delete_file,
            write_track_tags,
            preview_image,
            probe_files,
            take_pending_open_files,
            tray::set_close_to_tray,
            player_set_transition,
            player_set_normalization_settings,
            set_wasapi_exclusive,
            discord::discord_set_enabled,
            discord::discord_update,
            discord::discord_clear,
            discord::discord_cover_art,
            online_metadata::import_online_metadata,
            online_metadata::cancel_online_metadata,
            db::db_import,
            db::db_remove_paths,
            db::db_count,
            db::db_reset,
            db::db_roots,
            db::tracks::db_tracks_page,
            db::tracks::db_search,
            db::tracks::db_tracks_by_paths,
            db::tracks::db_track,
            db::tracks::db_random_track,
            db::tracks::db_albums,
            db::tracks::db_album_tracks,
            db::tracks::db_artists,
            db::tracks::db_artist_tracks,
            db::tracks::db_station_tracks,
            db::tracks::db_has_genre,
            db::tracks::db_smart_tracks,
            db::stats::db_record_play_start,
            db::stats::db_record_play,
            db::stats::db_record_skip,
            db::stats::db_stat,
            db::stats::db_stats_summary,
            db::stats::db_recently_played,
            db::stats::db_most_played,
            db::stats::db_on_repeat,
            db::stats::db_recently_added,
            db::stats::db_rediscover,
            db::stats::db_top_artists,
            db::stats::db_top_genres,
            db::stats::db_genres,
            db::stats::db_insight_counts,
            db::playlists::db_favorite_paths,
            db::playlists::db_favorites,
            db::playlists::db_toggle_favorite,
            db::playlists::db_move_favorite,
            db::playlists::db_playlists,
            db::playlists::db_playlist_tracks,
            db::playlists::db_upsert_playlist,
            db::playlists::db_delete_playlist,
            db::playlists::db_move_playlist_order,
            db::playlists::db_playlist_add,
            db::playlists::db_playlist_remove,
            db::playlists::db_playlist_move_item,
            db::playlists::db_recents,
            db::playlists::db_record_recent,
            db::db_kv_get,
            db::db_kv_set,
            db::backup::db_export_backup,
            db::backup::db_import_backup,
            db::backup::db_relocate_root,
            db::backup::db_prune_and_get_missing
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
