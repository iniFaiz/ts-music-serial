use rayon::prelude::*;
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose, Engine as _};
use lofty::picture::MimeType;
use lofty::prelude::*;
use lofty::probe::Probe;
use rodio::{Decoder, OutputStreamBuilder, Sink, Source};
use tauri::{AppHandle, Manager, State};
use walkdir::WalkDir;

const SUPPORTED_EXTS: [&str; 6] = ["mp3", "flac", "wav", "m4a", "ogg", "aac"];
// Cover art is downscaled to this maximum edge (px) before being sent to the
// frontend. Embedded album art is frequently 1000px+/several MB; a thumbnail is
// all the UI ever displays, so this slashes memory use and IPC payload size.
const THUMB_SIZE: u32 = 300;

// Data sent to the frontend.
#[derive(Serialize, Clone, Debug)]
struct MusicTrack {
    path: String,
    title: String,
    artist: String,
    album: String,
    duration_secs: u64,
    date_added: u64,
    year: Option<u32>,
    track_number: Option<u32>,
}

// Filter supported audio files by extension.
fn is_audio_file(path: &Path) -> bool {
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
fn allow_root(app: &AppHandle, path: &str) {
    let _ = app.asset_protocol_scope().allow_directory(path, true);
}

// A path may only be touched by file-reading commands if it is an audio file
// inside one of the directories the user explicitly scanned. This prevents the
// (untrusted) webview from coercing the backend into reading arbitrary files.
fn is_allowed_audio(app: &AppHandle, path: &Path) -> bool {
    is_audio_file(path) && app.asset_protocol_scope().is_allowed(path)
}

// Extract metadata for a single file.
fn parse_metadata(path: &Path) -> Option<MusicTrack> {
    let path_str = path.to_string_lossy().to_string();

    // Date created (falling back to modified) as a unix timestamp.
    let date_added = fs::metadata(path)
        .and_then(|m| m.created().or_else(|_| m.modified()))
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let tagged_file = Probe::open(path).ok()?.read().ok()?;

    let tag = tagged_file.primary_tag().or_else(|| tagged_file.first_tag());
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
    let year = tag.and_then(|t| t.year());
    let track_number = tag.and_then(|t| t.track());
    let duration_secs = properties.duration().as_secs();

    Some(MusicTrack {
        path: path_str,
        title,
        artist,
        album,
        duration_secs,
        date_added,
        year,
        track_number,
    })
}

// Scan a directory tree for audio files and parse their metadata.
#[tauri::command]
async fn scan_music_folder(
    app: AppHandle,
    path: String,
    use_parallelism: bool,
) -> Result<Vec<MusicTrack>, String> {
    println!("Starting scan for: {} (Parallel: {})", path, use_parallelism);
    let start_time = Instant::now();

    // Grant the frontend streaming access to this directory's audio.
    allow_root(&app, &path);

    let tracks: Vec<MusicTrack> = if use_parallelism {
        // jwalk for parallel directory traversal.
        let entries: Vec<_> = jwalk::WalkDir::new(&path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path())
            .collect();

        println!("Found {} files (jwalk). Processing metadata...", entries.len());

        entries
            .into_par_iter()
            .filter(|path| is_audio_file(path))
            .filter_map(|path| parse_metadata(&path))
            .collect()
    } else {
        // walkdir for sequential traversal.
        let entries: Vec<_> = WalkDir::new(&path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path().to_owned())
            .collect();

        println!("Found {} files (walkdir). Processing metadata...", entries.len());

        entries
            .into_iter()
            .filter(|path| is_audio_file(path))
            .filter_map(|path| parse_metadata(&path))
            .collect()
    };

    println!("Scanned {} tracks in {:?}", tracks.len(), start_time.elapsed());

    Ok(tracks)
}

// Re-grant streaming access to previously scanned roots. The frontend persists
// the list of scanned folders and calls this on startup, because the asset
// protocol scope is in-memory and resets each launch.
#[tauri::command]
fn restore_roots(app: AppHandle, roots: Vec<String>) {
    for root in roots {
        allow_root(&app, &root);
    }
}

// Directory where downscaled cover thumbnails are cached on disk.
fn cover_cache_dir(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_cache_dir().ok()?.join("covers");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

// Cache key derived from path + mtime + size, so the thumbnail is invalidated
// automatically if the underlying file changes.
fn cover_cache_key(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut hasher = DefaultHasher::new();
    path.to_string_lossy().hash(&mut hasher);
    mtime.hash(&mut hasher);
    meta.len().hash(&mut hasher);
    Some(format!("{:016x}", hasher.finish()))
}

// Pull the first embedded picture (raw bytes + mime) out of an audio file.
fn extract_cover(path: &Path) -> Option<(Vec<u8>, String)> {
    let tagged_file = Probe::open(path).ok()?.read().ok()?;
    let picture = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag())
        .and_then(|tag| tag.pictures().first())?;

    let mime = match picture.mime_type() {
        Some(MimeType::Png) => "image/png",
        Some(MimeType::Jpeg) => "image/jpeg",
        Some(MimeType::Tiff) => "image/tiff",
        Some(MimeType::Bmp) => "image/bmp",
        Some(MimeType::Gif) => "image/gif",
        _ => "image/jpeg",
    }
    .to_string();

    Some((picture.data().to_vec(), mime))
}

// Decode, downscale and re-encode cover art as a small JPEG thumbnail.
fn make_thumbnail(data: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(data).ok()?;
    let thumb = img.thumbnail(THUMB_SIZE, THUMB_SIZE);
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgb8(thumb.to_rgb8())
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Jpeg)
        .ok()?;
    Some(buf)
}

// Return album cover art as a base64 data URL (downscaled thumbnail), using a
// disk cache so repeated requests across sessions are cheap.
#[tauri::command]
async fn get_track_cover(app: AppHandle, path: String) -> Result<Option<String>, String> {
    let path_buf = PathBuf::from(&path);
    if !is_allowed_audio(&app, &path_buf) {
        return Err("Path is not within an allowed music folder".to_string());
    }

    let cache = cover_cache_dir(&app);

    // The CPU-bound decode/encode runs on the blocking pool so it never stalls
    // the async runtime's worker threads.
    let result = tauri::async_runtime::spawn_blocking(move || -> Option<String> {
        let key = cover_cache_key(&path_buf);

        // Fast path: serve a previously cached thumbnail.
        if let (Some(dir), Some(k)) = (&cache, &key) {
            if let Ok(bytes) = fs::read(dir.join(format!("{k}.jpg"))) {
                let b64 = general_purpose::STANDARD.encode(&bytes);
                return Some(format!("data:image/jpeg;base64,{b64}"));
            }
        }

        let (raw, raw_mime) = extract_cover(&path_buf)?;

        // Downscale when possible; otherwise fall back to the original bytes.
        match make_thumbnail(&raw) {
            Some(thumb) => {
                if let (Some(dir), Some(k)) = (&cache, &key) {
                    let _ = fs::write(dir.join(format!("{k}.jpg")), &thumb);
                }
                let b64 = general_purpose::STANDARD.encode(&thumb);
                Some(format!("data:image/jpeg;base64,{b64}"))
            }
            None => {
                let b64 = general_purpose::STANDARD.encode(&raw);
                Some(format!("data:{raw_mime};base64,{b64}"))
            }
        }
    })
    .await
    .map_err(|e| format!("Cover task failed: {e}"))?;

    Ok(result)
}

// ---------------------------------------------------------------------------
// Native audio playback
//
// Decoding/playback runs entirely in Rust (rodio + symphonia) rather than the
// webview's <audio> element. This guarantees consistent format support across
// platforms — notably FLAC/AAC, which several webviews (e.g. WebKitGTK) cannot
// decode — and gives precise, reliable seeking.
// ---------------------------------------------------------------------------

struct AudioPlayer {
    // None when no output device is available; playback commands then no-op.
    sink: Option<Arc<Sink>>,
    // Duration of the currently loaded track, in seconds.
    duration: Mutex<f64>,
    // True once a track has been loaded, so an empty sink means "finished"
    // rather than "nothing has played yet".
    active: AtomicBool,
}

#[derive(Serialize)]
struct PlayerStatus {
    position: f64,
    duration: f64,
    playing: bool,
    finished: bool,
}

// Load a track and start playing it, replacing whatever was playing. Returns
// the track duration in seconds.
#[tauri::command]
fn player_load(
    app: AppHandle,
    player: State<AudioPlayer>,
    path: String,
    volume: f64,
) -> Result<f64, String> {
    let path_buf = PathBuf::from(&path);
    if !is_allowed_audio(&app, &path_buf) {
        return Err("Path is not within an allowed music folder".to_string());
    }

    let sink = player.sink.as_ref().ok_or("No audio output device")?;

    // Read the whole file into memory and build a *seekable* decoder. rodio's
    // generic reader wrapper defaults to non-seekable, which makes try_seek fail
    // with SeekError(Unseekable); the builder lets us declare the source
    // seekable and supply its byte length so Symphonia can seek. Loading into a
    // Cursor also removes any disk-read stutter mid-playback.
    let bytes = fs::read(&path_buf).map_err(|e| e.to_string())?;
    let byte_len = bytes.len() as u64;
    let decoder = Decoder::builder()
        .with_data(Cursor::new(bytes))
        .with_byte_len(byte_len)
        .with_seekable(true)
        .build()
        .map_err(|e| e.to_string())?;
    let duration = decoder
        .total_duration()
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);

    sink.clear();
    sink.set_volume(volume.clamp(0.0, 1.0) as f32);
    sink.append(decoder);
    sink.play();

    *player.duration.lock().unwrap() = duration;
    player.active.store(true, Ordering::SeqCst);

    Ok(duration)
}

#[tauri::command]
fn player_pause(player: State<AudioPlayer>) {
    if let Some(sink) = &player.sink {
        sink.pause();
    }
}

#[tauri::command]
fn player_resume(player: State<AudioPlayer>) {
    if let Some(sink) = &player.sink {
        sink.play();
    }
}

#[tauri::command]
fn player_set_volume(player: State<AudioPlayer>, volume: f64) {
    if let Some(sink) = &player.sink {
        sink.set_volume(volume.clamp(0.0, 1.0) as f32);
    }
}

#[tauri::command]
fn player_seek(player: State<AudioPlayer>, position: f64) -> Result<(), String> {
    if let Some(sink) = &player.sink {
        let duration = *player.duration.lock().unwrap();
        // Keep the target inside the track; seeking to/past the end can error.
        let target = if duration > 0.1 {
            position.clamp(0.0, duration - 0.1)
        } else {
            position.max(0.0)
        };
        sink.try_seek(Duration::from_secs_f64(target))
            .map_err(|e| format!("Seek failed: {e:?}"))?;
    }
    Ok(())
}

#[tauri::command]
fn player_stop(player: State<AudioPlayer>) {
    if let Some(sink) = &player.sink {
        sink.clear();
    }
    player.active.store(false, Ordering::SeqCst);
}

#[tauri::command]
fn player_status(player: State<AudioPlayer>) -> PlayerStatus {
    match &player.sink {
        Some(sink) => {
            let empty = sink.empty();
            PlayerStatus {
                position: sink.get_pos().as_secs_f64(),
                duration: *player.duration.lock().unwrap(),
                playing: !sink.is_paused() && !empty,
                finished: player.active.load(Ordering::SeqCst) && empty,
            }
        }
        None => PlayerStatus {
            position: 0.0,
            duration: 0.0,
            playing: false,
            finished: false,
        },
    }
}

// Build the audio player. The OutputStream is `!Send`, so it lives on a
// dedicated thread that parks for the app's lifetime; only the (Send) Sink is
// shared back to the command handlers.
fn init_audio_player() -> AudioPlayer {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        match OutputStreamBuilder::open_default_stream() {
            Ok(stream) => {
                let sink = Sink::connect_new(stream.mixer());
                let _ = tx.send(Some(Arc::new(sink)));
                // Keep the output stream alive so audio output stays open.
                let _keep = stream;
                loop {
                    std::thread::sleep(Duration::from_secs(3600));
                }
            }
            Err(_) => {
                let _ = tx.send(None);
            }
        }
    });

    AudioPlayer {
        sink: rx.recv().unwrap_or(None),
        duration: Mutex::new(0.0),
        active: AtomicBool::new(false),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(init_audio_player())
        .invoke_handler(tauri::generate_handler![
            scan_music_folder,
            get_track_cover,
            restore_roots,
            player_load,
            player_pause,
            player_resume,
            player_set_volume,
            player_seek,
            player_stop,
            player_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
