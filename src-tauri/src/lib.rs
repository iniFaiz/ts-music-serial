use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// parking_lot's Mutex never poisons and its guard is returned directly (no
// Result), so there is no lock().unwrap() panic path. Mutex::new is const, so
// it still works for the process-wide statics below.
use parking_lot::Mutex;

use base64::{engine::general_purpose, Engine as _};
use lofty::picture::MimeType;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use rodio::source::SeekError;
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use tauri::{AppHandle, Emitter, Manager, State};
use walkdir::WalkDir;

mod db;
mod discord;
mod lyrics;
mod playlist_io;
#[cfg(target_os = "windows")]
mod thumbbar;
#[cfg(target_os = "windows")]
mod exclusive;

const SUPPORTED_EXTS: [&str; 6] = ["mp3", "flac", "wav", "m4a", "ogg", "aac"];
// Cover art is downscaled to this maximum edge (px) before being sent to the
// frontend. Embedded album art is frequently 1000px+/several MB; a thumbnail is
// all the UI ever displays, so this slashes memory use and IPC payload size.
const THUMB_SIZE: u32 = 300;
// Number of amplitude buckets ("bars") in a precomputed waveform. Enough detail
// for a seek bar without bloating the cache/IPC (stored as one byte per bar).
const WAVEFORM_BUCKETS: usize = 400;

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

// Parse a ReplayGain gain string like "-6.54 dB" / "+3.2" into decibels.
fn parse_rg_db(s: &str) -> Option<f32> {
    let cleaned = s.trim().trim_end_matches(|c: char| c.is_alphabetic() || c.is_whitespace());
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
pub(crate) fn allow_root(app: &AppHandle, path: &str) {
    let _ = app.asset_protocol_scope().allow_directory(path, true);
}

// A path may only be touched by file-reading commands if it is an audio file
// inside one of the directories the user explicitly scanned. This prevents the
// (untrusted) webview from coercing the backend into reading arbitrary files.
fn is_allowed_audio(app: &AppHandle, path: &Path) -> bool {
    is_audio_file(path) && app.asset_protocol_scope().is_allowed(path)
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
    // Genre is optional — many files lack it. Trimmed so blank tags become None,
    // which lets the frontend's smart-playlist genre rules ignore them cleanly.
    let genre = tag
        .and_then(|t| t.genre().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty());
    let year = tag.and_then(|t| t.year());
    let track_number = tag.and_then(|t| t.track());
    let duration_secs = properties.duration().as_secs();
    let has_cover = tag.as_ref().map_or(false, |t| !t.pictures().is_empty());

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

// Scan a heterogeneous list of paths (files and/or folders), e.g. from a
// drag-and-drop onto the window. Folders are walked recursively; lone audio
// files are parsed directly. Each touched directory is granted streaming scope.
#[tauri::command]
async fn scan_paths(app: AppHandle, paths: Vec<String>) -> Result<Vec<MusicTrack>, String> {
    let mut tracks: Vec<MusicTrack> = Vec::new();
    for p in paths {
        let pb = PathBuf::from(&p);
        if pb.is_dir() {
            allow_root(&app, &p);
            let entries: Vec<_> = jwalk::WalkDir::new(&pb)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .map(|e| e.path())
                .collect();
            let mut found: Vec<MusicTrack> = entries
                .into_par_iter()
                .filter(|path| is_audio_file(path))
                .filter_map(|path| parse_metadata(&path))
                .collect();
            tracks.append(&mut found);
        } else if pb.is_file() && is_audio_file(&pb) {
            if let Some(parent) = pb.parent() {
                let parent_str = parent.to_string_lossy().to_string();
                allow_root(&app, &parent_str);
            }
            if let Some(t) = parse_metadata(&pb) {
                tracks.push(t);
            }
        }
    }
    Ok(tracks)
}

// Return the subset of `paths` that still exist on disk (and are within an
// allowed music folder). Used to prune deleted files during a library refresh.
#[tauri::command]
fn filter_existing(app: AppHandle, paths: Vec<String>) -> Vec<String> {
    paths
        .into_iter()
        .filter(|p| {
            let pb = Path::new(p);
            is_allowed_audio(&app, pb) && pb.exists()
        })
        .collect()
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

// Extract up to three vibrant, visually-distinct colors from a decoded image for
// the Apple-Music-style animated gradient backdrop. This is a direct port of the
// frontend canvas heuristic (colorExtract.js) so the palette is identical whether
// it is computed here or (as a fallback) in the webview: downscale to a tiny
// grid, drop near-black/near-white/grey pixels, rank the rest by saturation and
// pick three that are far enough apart in RGB space.
fn extract_palette_from_image(img: &image::DynamicImage) -> Vec<String> {
    struct Px {
        r: i32,
        g: i32,
        b: i32,
        sat: i32,
    }

    let small = img.thumbnail(12, 12).to_rgba8();

    let mut pxs: Vec<Px> = Vec::new();
    for p in small.pixels() {
        let [r, g, b, a] = p.0;
        if a < 150 {
            continue;
        }
        let (r, g, b) = (r as i32, g as i32, b as i32);
        let sat = r.max(g).max(b) - r.min(g).min(b);
        let bright = (r + g + b) / 3;
        // Ignore extreme blacks/whites/greys for vibrancy.
        if bright > 240 && sat < 20 {
            continue;
        }
        if bright < 15 && sat < 10 {
            continue;
        }
        pxs.push(Px { r, g, b, sat });
    }

    // Fallback: an all-grey/mono cover leaves nothing after filtering, so keep
    // every pixel rather than returning the default palette.
    if pxs.is_empty() {
        for p in small.pixels() {
            let [r, g, b, _a] = p.0;
            let (r, g, b) = (r as i32, g as i32, b as i32);
            let sat = r.max(g).max(b) - r.min(g).min(b);
            pxs.push(Px { r, g, b, sat });
        }
    }

    pxs.sort_by(|a, b| b.sat.cmp(&a.sat));

    let mut chosen: Vec<usize> = Vec::new();
    for (i, p) in pxs.iter().enumerate() {
        let similar = chosen.iter().any(|&ci| {
            let c = &pxs[ci];
            let (dr, dg, db) = (c.r - p.r, c.g - p.g, c.b - p.b);
            ((dr * dr + dg * dg + db * db) as f64).sqrt() < 65.0
        });
        if !similar {
            chosen.push(i);
            if chosen.len() >= 3 {
                break;
            }
        }
    }
    // Not enough distinct colors: top up with the next-most-saturated pixels.
    if chosen.len() < 3 {
        for i in 0..pxs.len() {
            if !chosen.contains(&i) {
                chosen.push(i);
                if chosen.len() >= 3 {
                    break;
                }
            }
        }
    }

    let mut out: Vec<String> = chosen
        .iter()
        .map(|&i| format!("rgb({}, {}, {})", pxs[i].r, pxs[i].g, pxs[i].b))
        .collect();
    while out.len() < 3 {
        out.push("rgb(60, 60, 60)".to_string());
    }
    out
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

// Ensure the on-disk cover thumbnail exists and return its filesystem path.
//
// This is the fast path used by the UI: instead of base64-encoding the image
// and shipping tens of KB over IPC on every render (which also doubles the
// image's memory — once as a JS data URL, once as the decoded bitmap), the
// frontend loads the returned path directly through the asset protocol
// (convertFileSrc). The webview then caches the decoded image itself, so
// re-renders across navigation cost nothing.
//
// Returns None when the file has no embeddable/decodable cover art.
#[tauri::command]
async fn get_track_cover_path(app: AppHandle, path: String) -> Result<Option<String>, String> {
    let path_buf = PathBuf::from(&path);
    if !is_allowed_audio(&app, &path_buf) {
        return Err("Path is not within an allowed music folder".to_string());
    }

    let cache = cover_cache_dir(&app);

    // Same CPU-bound decode/downscale as get_track_cover, on the blocking pool,
    // but the result lives on disk (as {key}.jpg) rather than crossing the IPC
    // boundary. The key embeds mtime+size, so it self-invalidates on file change.
    let result = tauri::async_runtime::spawn_blocking(move || -> Option<String> {
        let dir = cache?;
        let key = cover_cache_key(&path_buf)?;
        let file = dir.join(format!("{key}.jpg"));

        // Fast path: thumbnail already cached on disk.
        if file.exists() {
            return Some(file.to_string_lossy().into_owned());
        }

        // Decode → downscale → cache a JPEG thumbnail, then hand back its path.
        let (raw, _mime) = extract_cover(&path_buf)?;
        let thumb = make_thumbnail(&raw)?;
        fs::write(&file, &thumb).ok()?;
        Some(file.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| format!("Cover task failed: {e}"))?;

    Ok(result)
}

// Return the 3-color gradient palette for a track's cover art, computed natively
// (see extract_palette_from_image) instead of decoding the cover a second time in
// the webview canvas. The result is cached on disk next to the thumbnail as a
// `{key}.pal` sidecar; the key embeds mtime+size, so it self-invalidates exactly
// like the thumbnail. Returns None only when the file has no decodable cover.
#[tauri::command]
async fn get_track_palette(app: AppHandle, path: String) -> Result<Option<Vec<String>>, String> {
    let path_buf = PathBuf::from(&path);
    if !is_allowed_audio(&app, &path_buf) {
        return Err("Path is not within an allowed music folder".to_string());
    }

    let cache = cover_cache_dir(&app);

    let result = tauri::async_runtime::spawn_blocking(move || -> Option<Vec<String>> {
        let key = cover_cache_key(&path_buf);

        // Fast path: a previously computed palette cached on disk.
        if let (Some(dir), Some(k)) = (&cache, &key) {
            if let Ok(text) = fs::read_to_string(dir.join(format!("{k}.pal"))) {
                if let Ok(colors) = serde_json::from_str::<Vec<String>>(&text) {
                    if colors.len() == 3 {
                        return Some(colors);
                    }
                }
            }
        }

        // Decode from the cached thumbnail when present (cheap: 300px JPEG),
        // otherwise extract + downscale the embedded cover once.
        let img = match (&cache, &key) {
            (Some(dir), Some(k)) if dir.join(format!("{k}.jpg")).exists() => {
                let bytes = fs::read(dir.join(format!("{k}.jpg"))).ok()?;
                image::load_from_memory(&bytes).ok()?
            }
            _ => {
                let (raw, _mime) = extract_cover(&path_buf)?;
                image::load_from_memory(&raw).ok()?
            }
        };

        let palette = extract_palette_from_image(&img);
        if let (Some(dir), Some(k)) = (&cache, &key) {
            if let Ok(text) = serde_json::to_string(&palette) {
                let _ = fs::write(dir.join(format!("{k}.pal")), text);
            }
        }
        Some(palette)
    })
    .await
    .map_err(|e| format!("Palette task failed: {e}"))?;

    Ok(result)
}

// Directory where precomputed waveforms are cached on disk.
fn waveform_cache_dir(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_cache_dir().ok()?.join("waveforms");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

// Decode a track and reduce it to WAVEFORM_BUCKETS peak amplitudes (max abs per
// window), normalized to the track's loudest peak and quantized to 0..255. Runs
// a full decode, so it is only ever called on the blocking pool and cached.
fn compute_waveform(path: &Path) -> Option<Vec<u8>> {
    let (decoder, dur_secs) = build_decoder(path).ok()?;
    let channels = decoder.channels().max(1) as f64;
    let sample_rate = decoder.sample_rate().max(1) as f64;
    // Estimate the total sample count so we can stream into a fixed bar count in
    // O(BUCKETS) memory instead of buffering the whole (possibly huge) file.
    let est_total = (dur_secs.max(0.0) * sample_rate * channels) as u64;

    let mut peaks: Vec<f32> = Vec::with_capacity(WAVEFORM_BUCKETS);

    if est_total > 0 {
        let bucket_size = (est_total / WAVEFORM_BUCKETS as u64).max(1);
        let mut cur_peak = 0.0f32;
        let mut count: u64 = 0;
        for s in decoder {
            let a = s.abs();
            if a > cur_peak {
                cur_peak = a;
            }
            count += 1;
            if count >= bucket_size {
                peaks.push(cur_peak);
                cur_peak = 0.0;
                count = 0;
                if peaks.len() >= WAVEFORM_BUCKETS {
                    break;
                }
            }
        }
        if peaks.len() < WAVEFORM_BUCKETS && count > 0 {
            peaks.push(cur_peak);
        }
    } else {
        // Unknown duration: buffer the samples, then bucket them.
        let all: Vec<f32> = decoder.map(|s| s.abs()).collect();
        if all.is_empty() {
            return None;
        }
        let bs = (all.len() / WAVEFORM_BUCKETS).max(1);
        let mut i = 0;
        while i < all.len() && peaks.len() < WAVEFORM_BUCKETS {
            let end = (i + bs).min(all.len());
            let p = all[i..end].iter().copied().fold(0.0f32, f32::max);
            peaks.push(p);
            i = end;
        }
    }

    if peaks.is_empty() {
        return None;
    }

    // Normalize to the loudest bar so quiet tracks still fill the bar height, then
    // pad to a fixed length and quantize to one byte each.
    let max = peaks.iter().copied().fold(0.0f32, f32::max);
    let scale = if max > 1e-6 { 1.0 / max } else { 0.0 };
    while peaks.len() < WAVEFORM_BUCKETS {
        peaks.push(0.0);
    }
    Some(
        peaks
            .iter()
            .map(|&p| ((p * scale).clamp(0.0, 1.0) * 255.0) as u8)
            .collect(),
    )
}

// Return a precomputed waveform (one byte per bar, 0..255) for a track, using a
// disk cache keyed by path+mtime+size so it self-invalidates on file change and
// is computed at most once per track. None when the file can't be decoded.
#[tauri::command]
async fn get_waveform(app: AppHandle, path: String) -> Result<Option<Vec<u8>>, String> {
    let path_buf = PathBuf::from(&path);
    if !is_allowed_audio(&app, &path_buf) {
        return Err("Path is not within an allowed music folder".to_string());
    }

    let cache = waveform_cache_dir(&app);

    let result = tauri::async_runtime::spawn_blocking(move || -> Option<Vec<u8>> {
        let key = cover_cache_key(&path_buf);

        // Fast path: cached waveform.
        if let (Some(dir), Some(k)) = (&cache, &key) {
            if let Ok(text) = fs::read_to_string(dir.join(format!("{k}.wf"))) {
                if let Ok(v) = serde_json::from_str::<Vec<u8>>(&text) {
                    if !v.is_empty() {
                        return Some(v);
                    }
                }
            }
        }

        let peaks = compute_waveform(&path_buf)?;
        if let (Some(dir), Some(k)) = (&cache, &key) {
            if let Ok(text) = serde_json::to_string(&peaks) {
                let _ = fs::write(dir.join(format!("{k}.wf")), text);
            }
        }
        Some(peaks)
    })
    .await
    .map_err(|e| format!("Waveform task failed: {e}"))?;

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

// Command sent to the dedicated audio thread (which owns the !Send OutputStream).
enum AudioCommand {
    // Open the output device with the given name, or the system default (None).
    // The reply channel is signalled once the device is open.
    OpenDevice(Option<String>, mpsc::Sender<()>),
    CreateSink(mpsc::Sender<Result<Arc<Sink>, String>>),
    CloseStream(mpsc::Sender<()>),
}

#[allow(dead_code)]
struct ActiveTrack {
    path: String,
    sink: Arc<Sink>,
    duration: f64,
    start_time: Instant,
}

#[allow(dead_code)]
struct FadingTrack {
    sink: Arc<Sink>,
    fade_end: Instant,
    fade_duration: Duration,
    initial_volume: f32,
}

struct PreparedTrack {
    path: String,
    decoder: Decoder<Cursor<Vec<u8>>>,
    duration: f64,
    gain_db: Option<f32>,
    peak: Option<f32>,
}

struct AudioPlayer {
    // The active default sink handle from device stream builder
    sink: Arc<Mutex<Option<Arc<Sink>>>>,
    // Duration of the currently loaded track, in seconds.
    duration: Arc<Mutex<f64>>,
    // True once a track has been loaded, so an empty sink means "finished"
    // rather than "nothing has played yet".
    active: Arc<AtomicBool>,
    // Bumped on every load. A decode that finishes after a newer load started
    // checks this and discards its (now stale) result instead of clobbering the
    // track the user actually wants.
    generation: Arc<AtomicU64>,
    // Latest frequency-band levels for the UI visualizer, fed by SpectrumSource.
    spectrum: Arc<SpectrumShared>,
    // 10-band graphic EQ settings, read live by each EqualizerSource in the chain.
    equalizer: Arc<EqualizerShared>,
    // Volume-normalization factor (linear) applied on top of the user volume.
    // 1.0 = no normalization.
    norm_factor: Arc<Mutex<f32>>,
    // Last user-requested volume (0..1), so the normalization factor can be
    // re-applied to the live sink without the frontend re-sending the volume.
    last_volume: Arc<Mutex<f32>>,
    // Channel to the audio thread for device-management commands.
    cmd_tx: mpsc::Sender<AudioCommand>,
    // Pre-decoded next track details
    prepared: Arc<Mutex<Option<PreparedTrack>>>,
    // Target path currently being prepared (prevents race conditions)
    prepared_target_path: Arc<Mutex<Option<String>>>,
    // Transition mode and crossfade settings
    transition_mode: Arc<Mutex<String>>,
    crossfade_secs: Arc<Mutex<f64>>,
    // Normalization configuration
    normalization_enabled: Arc<Mutex<bool>>,
    normalization_preamp_db: Arc<Mutex<f64>>,
    // Current primary track and fading tracks
    current_track: Arc<Mutex<Option<ActiveTrack>>>,
    fading_tracks: Arc<Mutex<Vec<FadingTrack>>>,
    // When true (Windows only), playback is routed to the WASAPI exclusive engine
    // instead of the rodio shared-mode sink.
    exclusive_enabled: Arc<AtomicBool>,
}

impl Clone for AudioPlayer {
    fn clone(&self) -> Self {
        AudioPlayer {
            sink: self.sink.clone(),
            current_track: self.current_track.clone(),
            fading_tracks: self.fading_tracks.clone(),
            duration: self.duration.clone(),
            active: self.active.clone(),
            generation: self.generation.clone(),
            spectrum: self.spectrum.clone(),
            equalizer: self.equalizer.clone(),
            norm_factor: self.norm_factor.clone(),
            last_volume: self.last_volume.clone(),
            cmd_tx: self.cmd_tx.clone(),
            prepared: self.prepared.clone(),
            prepared_target_path: self.prepared_target_path.clone(),
            transition_mode: self.transition_mode.clone(),
            crossfade_secs: self.crossfade_secs.clone(),
            normalization_enabled: self.normalization_enabled.clone(),
            normalization_preamp_db: self.normalization_preamp_db.clone(),
            exclusive_enabled: self.exclusive_enabled.clone(),
        }
    }
}

impl AudioPlayer {
    // Clone out the current sink handle (if any) without holding the lock.
    fn sink(&self) -> Option<Arc<Sink>> {
        self.current_track.lock().as_ref().map(|t| t.sink.clone())
    }
    // Effective sink volume = user volume * normalization factor. Capped above
    // 1.0 (≈ +12 dB) so normalization can boost quiet tracks; rodio amplifies
    // values > 1.0 and the peak limiter in player_set_normalization guards
    // against clipping.
    fn effective_volume(&self) -> f32 {
        let vol = *self.last_volume.lock();
        let factor = *self.norm_factor.lock();
        (vol * factor).clamp(0.0, 4.0)
    }
}

#[derive(Serialize)]
pub(crate) struct PlayerStatus {
    pub(crate) position: f64,
    pub(crate) duration: f64,
    pub(crate) playing: bool,
    pub(crate) finished: bool,
    pub(crate) path: Option<String>,
}

// Read a file into memory and build a *seekable* decoder. Decoding stays lazy
// (samples are produced on demand during playback), so playback starts almost
// immediately instead of waiting for the whole track. Reading into a Cursor
// keeps the audio callback off the disk, and `[profile.dev.package."*"]
// opt-level = 3` keeps the codec fast enough to never starve the callback —
// together that fixes both the slow start and the "bz bz bz" under load.
pub(crate) fn build_decoder(path: &Path) -> Result<(Decoder<Cursor<Vec<u8>>>, f64), String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let byte_len = bytes.len() as u64;
    let decoder = Decoder::builder()
        .with_data(Cursor::new(bytes))
        .with_byte_len(byte_len)
        .with_seekable(true)
        .build()
        .map_err(|e| e.to_string())?;
    // Cheap: read from the codec params populated at init (no full-file scan).
    // May be None for headerless MP3 — the caller falls back to a metadata hint.
    let duration = decoder
        .total_duration()
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    Ok((decoder, duration))
}

// ---------------------------------------------------------------------------
// Real-time spectrum analysis for the UI visualizer.
//
// `SpectrumSource` is a thin pass-through `Source` inserted between the decoder
// and the sink. It taps the samples on their way to the audio device, mixes
// them to mono, and accumulates FFT-sized windows; each completed window is
// reduced to six smoothed frequency-band levels (0..1) that the frontend reads
// via `player_spectrum`. The heavy work (one 1024-point FFT) happens at most
// ~43×/s on the audio thread and is gated by an `enabled` flag, so it costs
// essentially nothing when the visualizer is switched off in Settings.
// ---------------------------------------------------------------------------

const FFT_SIZE: usize = 1024;
const SPECTRUM_BANDS: usize = 6;
// Loudness window (in ~dBFS) mapped onto the 0..1 bar range. Magnitudes quieter
// than DB_MIN read as an empty bar, DB_MAX (and above) as a full one. Widen or
// shift these if bars sit too low/high across your library.
const SPECTRUM_ATTACK: f32 = 0.75; // how quickly a bar rises toward a louder value
const SPECTRUM_DECAY: f32 = 0.18; // how slowly it falls back (springy feel)

// Lock-free hand-off of the latest band levels from the audio thread to the
// `player_spectrum` command: each band is an f32 kept as its bit pattern.
pub(crate) struct SpectrumShared {
    enabled: AtomicBool,
    bands: [AtomicU32; SPECTRUM_BANDS],
}

impl SpectrumShared {
    fn new() -> Self {
        SpectrumShared {
            enabled: AtomicBool::new(true),
            bands: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }
    fn store(&self, vals: &[f32; SPECTRUM_BANDS]) {
        for (slot, v) in self.bands.iter().zip(vals) {
            slot.store(v.to_bits(), Ordering::Relaxed);
        }
    }
    fn load(&self) -> [f32; SPECTRUM_BANDS] {
        std::array::from_fn(|i| f32::from_bits(self.bands[i].load(Ordering::Relaxed)))
    }
    pub(crate) fn reset(&self) {
        for slot in &self.bands {
            slot.store(0, Ordering::Relaxed);
        }
    }
}

// In-place iterative radix-2 Cooley–Tukey FFT (`len` must be a power of two).
fn fft_in_place(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    // Butterflies.
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wcos, wsin) = (ang.cos(), ang.sin());
        let half = len / 2;
        let mut i = 0;
        while i < n {
            let mut wr = 1.0f32;
            let mut wi = 0.0f32;
            for k in 0..half {
                let a = i + k;
                let b = a + half;
                let vr = re[b] * wr - im[b] * wi;
                let vi = re[b] * wi + im[b] * wr;
                re[b] = re[a] - vr;
                im[b] = im[a] - vi;
                re[a] += vr;
                im[a] += vi;
                let nwr = wr * wcos - wi * wsin;
                wi = wr * wsin + wi * wcos;
                wr = nwr;
            }
            i += len;
        }
        len <<= 1;
    }
}

pub(crate) struct SpectrumSource<S> {
    inner: S,
    shared: Arc<SpectrumShared>,
    player_gen: Arc<AtomicU64>,
    my_gen: u64,
    channels: u16,
    frame_sum: f32,
    frame_ch: u16,
    buf: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    hann: Vec<f32>,
    band_bins: [(usize, usize); SPECTRUM_BANDS],
    smoothed: [f32; SPECTRUM_BANDS],
}

impl<S: Source> SpectrumSource<S> {
    pub(crate) fn new(inner: S, shared: Arc<SpectrumShared>, player_gen: Arc<AtomicU64>, my_gen: u64) -> Self {
        let channels = inner.channels().max(1);
        let sr = inner.sample_rate().max(1) as f32;
        // Six log-spaced bands spanning sub-bass → presence.
        let edges = [40.0f32, 160.0, 400.0, 1000.0, 2600.0, 6000.0, 14000.0];
        let bin_of =
            |hz: f32| ((hz * FFT_SIZE as f32 / sr).round() as usize).clamp(1, FFT_SIZE / 2 - 1);
        let band_bins = std::array::from_fn(|b| {
            let lo = bin_of(edges[b]);
            let hi = bin_of(edges[b + 1]).max(lo);
            (lo, hi)
        });
        let hann = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE as f32 - 1.0)).cos())
            .collect();
        SpectrumSource {
            inner,
            shared,
            player_gen,
            my_gen,
            channels,
            frame_sum: 0.0,
            frame_ch: 0,
            buf: Vec::with_capacity(FFT_SIZE),
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            hann,
            band_bins,
            smoothed: [0.0; SPECTRUM_BANDS],
        }
    }

    // Reduce one full window to six smoothed band levels and publish them.
    fn analyze(&mut self) {
        let n = FFT_SIZE;
        let mut energy = 0.0f32;
        for i in 0..n {
            let x = self.buf[i];
            energy += x * x;
            self.re[i] = x * self.hann[i];
            self.im[i] = 0.0;
        }
        let rms = (energy / n as f32).sqrt();
        fft_in_place(&mut self.re, &mut self.im);

        // Equalization gains to compensate for high-frequency roll-off (pink noise nature)
        // and make all bands react with satisfying height.
        let gains = [1.6f32, 2.2, 2.8, 3.4, 4.0, 4.6];

        let mut targets = [0.0f32; SPECTRUM_BANDS];
        if rms > 1e-4 {
            for (b, &(lo, hi)) in self.band_bins.iter().enumerate() {
                let mut peak = 0.0f32;
                for bin in lo..=hi {
                    let mag = (self.re[bin] * self.re[bin] + self.im[bin] * self.im[bin]).sqrt();
                    if mag > peak {
                        peak = mag;
                    }
                }
                let norm = peak / (n as f32 * 0.5);
                targets[b] = (norm * gains[b]).clamp(0.0, 1.0).sqrt();
            }
        }

        // Fast attack, slow decay for a lively-but-stable look.
        for b in 0..SPECTRUM_BANDS {
            let coeff = if targets[b] > self.smoothed[b] {
                SPECTRUM_ATTACK
            } else {
                SPECTRUM_DECAY
            };
            self.smoothed[b] += (targets[b] - self.smoothed[b]) * coeff;
        }
        self.shared.store(&self.smoothed);
    }
}

impl<S: Source> Iterator for SpectrumSource<S> {
    type Item = f32;
    #[inline]
    fn next(&mut self) -> Option<f32> {
        let s = self.inner.next()?;
        if self.shared.enabled.load(Ordering::Relaxed) && self.my_gen == self.player_gen.load(Ordering::Relaxed) {
            self.frame_sum += s;
            self.frame_ch += 1;
            if self.frame_ch >= self.channels {
                self.buf.push(self.frame_sum / self.channels as f32);
                self.frame_sum = 0.0;
                self.frame_ch = 0;
                if self.buf.len() >= FFT_SIZE {
                    self.analyze();
                    self.buf.clear();
                }
            }
        }
        Some(s)
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S: Source> Source for SpectrumSource<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        // Drop the half-filled window so the bars don't glitch across a seek.
        self.buf.clear();
        self.frame_sum = 0.0;
        self.frame_ch = 0;
        self.inner.try_seek(pos)
    }
}

// ---------------------------------------------------------------------------
// 10-band graphic equalizer.
//
// `EqualizerSource`, like `SpectrumSource`, is a pass-through `Source` inserted
// between the decoder and the sink — here it actually *modifies* the samples.
// Each of the ten ISO octave bands (31 Hz … 16 kHz) is a peaking biquad (RBJ
// cookbook); the bands are cascaded in series and run per channel. Gains live in
// a lock-free shared struct the UI writes via `player_set_equalizer`; a `version`
// counter lets the audio thread notice changes and recompute coefficients
// without recomputing per sample. When disabled (or flat + off) it is a near-free
// pass-through: one atomic load and a channel counter per sample.
// ---------------------------------------------------------------------------

const EQ_BANDS: usize = 10;
// ISO 1/1-octave center frequencies. The top band sits below the 22.05 kHz
// Nyquist of 44.1 kHz audio; `Biquad::peaking` falls back to a flat response for
// any band at/above Nyquist (e.g. unusual low sample rates).
const EQ_FREQS: [f32; EQ_BANDS] =
    [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
// Q ≈ √2 gives each peaking filter ~one octave of bandwidth, so adjacent bands
// overlap smoothly the way a graphic EQ should.
const EQ_Q: f32 = 1.41;

// Lock-free EQ settings shared from the UI thread to the audio thread. Gains and
// preamp are f32 stored as their bit patterns; `version` is bumped on any change
// so live `EqualizerSource`s know to recompute their coefficients.
pub(crate) struct EqualizerShared {
    enabled: AtomicBool,
    gains_db: [AtomicU32; EQ_BANDS],
    preamp_db: AtomicU32,
    version: AtomicU64,
}

impl EqualizerShared {
    fn new() -> Self {
        EqualizerShared {
            enabled: AtomicBool::new(false),
            gains_db: std::array::from_fn(|_| AtomicU32::new(0)),
            preamp_db: AtomicU32::new(0),
            version: AtomicU64::new(0),
        }
    }

    // Publish a full settings snapshot. Values are written first, then the
    // version is bumped with Release so a source that observes the new version
    // (with Acquire) is guaranteed to read these values.
    fn set(&self, enabled: bool, gains: &[f32; EQ_BANDS], preamp_db: f32) {
        self.enabled.store(enabled, Ordering::Relaxed);
        for (slot, g) in self.gains_db.iter().zip(gains) {
            slot.store(g.to_bits(), Ordering::Relaxed);
        }
        self.preamp_db.store(preamp_db.to_bits(), Ordering::Relaxed);
        self.version.fetch_add(1, Ordering::Release);
    }

    fn snapshot(&self) -> (bool, [f32; EQ_BANDS], f32) {
        let enabled = self.enabled.load(Ordering::Relaxed);
        let gains = std::array::from_fn(|i| f32::from_bits(self.gains_db[i].load(Ordering::Relaxed)));
        let preamp = f32::from_bits(self.preamp_db.load(Ordering::Relaxed));
        (enabled, gains, preamp)
    }
}

// Normalized biquad coefficients (a0 folded in) and its Direct-Form-I state.
#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Biquad {
    fn identity() -> Self {
        Biquad { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 }
    }

    // RBJ "cookbook" peaking EQ. `gain_db` > 0 boosts the band, < 0 cuts it.
    fn peaking(freq: f32, sample_rate: f32, q: f32, gain_db: f32) -> Self {
        // A band centered at/above Nyquist can't be realized — leave it flat.
        if freq * 2.0 >= sample_rate || sample_rate <= 0.0 {
            return Biquad::identity();
        }
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha / a;
        Biquad {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos_w0) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct BiquadState {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl BiquadState {
    #[inline]
    fn process(&mut self, x: f32, c: &Biquad) -> f32 {
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

pub(crate) struct EqualizerSource<S> {
    inner: S,
    shared: Arc<EqualizerShared>,
    sample_rate: f32,
    channels: u16,
    ch: u16,
    coeffs: [Biquad; EQ_BANDS],
    states: Vec<[BiquadState; EQ_BANDS]>,
    preamp: f32,
    enabled: bool,
    last_version: u64,
}

impl<S: Source> EqualizerSource<S> {
    pub(crate) fn new(inner: S, shared: Arc<EqualizerShared>) -> Self {
        let channels = inner.channels().max(1);
        let sample_rate = inner.sample_rate().max(1) as f32;
        let mut me = EqualizerSource {
            inner,
            shared,
            sample_rate,
            channels,
            ch: 0,
            coeffs: [Biquad::identity(); EQ_BANDS],
            states: vec![[BiquadState::default(); EQ_BANDS]; channels as usize],
            preamp: 1.0,
            enabled: false,
            last_version: 0,
        };
        me.last_version = me.shared.version.load(Ordering::Acquire);
        me.recompute();
        me
    }

    // Rebuild the per-band coefficients from the current shared settings. Cheap
    // and rare: only runs at construction and when the UI changes a value.
    fn recompute(&mut self) {
        let (enabled, gains, preamp_db) = self.shared.snapshot();
        self.enabled = enabled;
        self.preamp = 10f32.powf(preamp_db / 20.0);
        for b in 0..EQ_BANDS {
            self.coeffs[b] = if gains[b].abs() < 1e-3 {
                Biquad::identity()
            } else {
                Biquad::peaking(EQ_FREQS[b], self.sample_rate, EQ_Q, gains[b])
            };
        }
    }
}

impl<S: Source> Iterator for EqualizerSource<S> {
    type Item = f32;
    #[inline]
    fn next(&mut self) -> Option<f32> {
        let s = self.inner.next()?;
        // Pick up live UI changes (recompute coefficients at most once per edit).
        let v = self.shared.version.load(Ordering::Acquire);
        if v != self.last_version {
            self.recompute();
            self.last_version = v;
        }
        let ch = self.ch as usize;
        self.ch += 1;
        if self.ch >= self.channels {
            self.ch = 0;
        }
        if !self.enabled {
            return Some(s);
        }
        let mut x = s * self.preamp;
        if let Some(state) = self.states.get_mut(ch) {
            for b in 0..EQ_BANDS {
                x = state[b].process(x, &self.coeffs[b]);
            }
        }
        Some(x)
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S: Source> Source for EqualizerSource<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        // Clear filter memory so a jump doesn't ring out the old signal.
        for st in self.states.iter_mut() {
            *st = [BiquadState::default(); EQ_BANDS];
        }
        self.inner.try_seek(pos)
    }
}

// Load a track and start playing it, replacing whatever was playing. Returns
// the track duration in seconds. The file read + decoder setup run on the
// blocking pool so the UI/IPC thread never stalls; the previously playing track
// is stopped immediately so it doesn't bleed over the (brief) load gap.
#[derive(Serialize)]
pub(crate) struct PlaybackInfo {
    pub(crate) duration: f64,
    pub(crate) sample_rate: Option<u32>,
    pub(crate) bit_depth: Option<u8>,
}

#[tauri::command]
async fn player_load(
    app: AppHandle,
    player: State<'_, AudioPlayer>,
    path: String,
    volume: f64,
    start_at: Option<f64>,
    autoplay: bool,
    duration_hint: f64,
    fade_in_secs: Option<f64>,
) -> Result<PlaybackInfo, String> {
    let path_buf = PathBuf::from(&path);
    if !is_allowed_audio(&app, &path_buf) {
        return Err("Path is not within an allowed music folder".to_string());
    }

    // WASAPI exclusive path (Windows). Falls back to shared mode on any failure.
    #[cfg(target_os = "windows")]
    if player.exclusive_enabled.load(Ordering::SeqCst) {
        if let Some(ex) = app.try_state::<Arc<exclusive::ExclusivePlayer>>() {
            let ex = ex.inner().clone();
            let p = path.clone();
            let res = tauri::async_runtime::spawn_blocking(move || {
                ex.load(p, volume, start_at, autoplay, duration_hint)
            })
            .await
            .map_err(|e| format!("Exclusive load task failed: {e}"))?;
            match res {
                Ok(info) => return Ok(info),
                Err(e) => {
                    eprintln!("WASAPI exclusive load failed ({e}); using shared mode.");
                    let _ = app.emit("wasapi-exclusive-error", e);
                    // Disable exclusive so play/pause/status/seek route to
                    // the shared-mode path until the user re-enables it.
                    player.exclusive_enabled.store(false, Ordering::SeqCst);
                    
                    // Re-open the shared stream with a timeout so we never hang.
                    let (reply_tx, reply_rx) = mpsc::channel();
                    if player.cmd_tx.send(AudioCommand::OpenDevice(None, reply_tx)).is_ok() {
                        let _ = reply_rx.recv_timeout(Duration::from_secs(5));
                    }
                    // fall through to the shared-mode rodio path below
                }
            }
        }
    }

    // Stop all active sinks and clear fading list
    {
        let mut fading_guard = player.fading_tracks.lock();
        for t in fading_guard.drain(..) {
            t.sink.stop();
        }
        let mut current_guard = player.current_track.lock();
        if let Some(t) = current_guard.take() {
            if let Some(secs) = fade_in_secs {
                if secs > 0.0 {
                    fading_guard.push(FadingTrack {
                        sink: t.sink.clone(),
                        fade_end: Instant::now() + Duration::from_secs_f64(secs),
                        fade_duration: Duration::from_secs_f64(secs),
                        initial_volume: player.effective_volume(),
                    });
                } else {
                    t.sink.stop();
                }
            } else {
                t.sink.stop();
            }
        }
    }

    // Create a new sink via the audio thread
    let (reply_tx, reply_rx) = mpsc::channel();
    player
        .cmd_tx
        .send(AudioCommand::CreateSink(reply_tx))
        .map_err(|e| format!("Audio thread unavailable: {e}"))?;
    
    // Wait for a sink. Allow enough time for the audio thread to re-open the
    // device (with retries) if the shared stream needs to be recovered first.
    let sink = reply_rx
        .recv_timeout(Duration::from_secs(4))
        .map_err(|e| format!("Failed to create sink: {e}"))??;

    // Clone the shared handles out so we never hold the State guard across .await.
    let generation = player.generation.clone();
    let duration_slot = player.duration.clone();
    let active = player.active.clone();
    let spectrum = player.spectrum.clone();
    let norm_factor = player.norm_factor.clone();
    let last_volume = player.last_volume.clone();
    let prepared = player.prepared.clone();

    // Claim a generation and stop the current track right away. Marking the
    // player inactive during the load gap prevents the now-empty sink from
    // being misread as "track finished" (which would auto-skip to the next one).
    let my_gen = generation.fetch_add(1, Ordering::SeqCst) + 1;
    active.store(false, Ordering::SeqCst);
    spectrum.reset(); // clear the visualizer during the load gap
    // Apply the requested volume together with the active normalization factor.
    let user_vol = volume.clamp(0.0, 1.0) as f32;
    *last_volume.lock() = user_vol;
    let factor = *norm_factor.lock();
    sink.set_volume(user_vol * factor);

    // Reuse a pre-decoded next track when it matches (near-gapless); otherwise
    // read + decode on the blocking pool.
    let prepared_decoder = {
        let mut g = prepared.lock();
        let mut target_g = player.prepared_target_path.lock();
        if g.as_ref().map(|p| p.path == path).unwrap_or(false) {
            *target_g = None;
            g.take()
        } else {
            *g = None;
            *target_g = None;
            None
        }
    };
    let (decoder, decoded_duration) = match prepared_decoder {
        Some(prep) => {
            (prep.decoder, prep.duration)
        }
        None => {
            let path_buf_clone = path_buf.clone();
            let (dec, raw_dur) = tauri::async_runtime::spawn_blocking(move || build_decoder(&path_buf_clone))
                .await
                .map_err(|e| format!("Decode task failed: {e}"))??;
            let dur = if raw_dur > 0.0 {
                raw_dur
            } else {
                duration_hint.max(0.0)
            };
            (dec, dur)
        }
    };

    // A newer load was requested while we were reading — drop this stale one.
    if generation.load(Ordering::SeqCst) != my_gen {
        return Ok(PlaybackInfo {
            duration: 0.0,
            sample_rate: None,
            bit_depth: None,
        });
    }

    // Prefer the decoder's duration; fall back to the metadata hint (e.g. for
    // headerless MP3 where the decoder can't report one).
    let duration = if decoded_duration > 0.0 {
        decoded_duration
    } else {
        duration_hint.max(0.0)
    };
    *duration_slot.lock() = duration;
    // Equalize, then tap the decoded samples for the visualizer on their way to
    // the sink (so the bars reflect what you actually hear). In crossfade mode
    // each track is eased in with a fade.
    let equalized = EqualizerSource::new(decoder, player.equalizer.clone());
    let tapped = SpectrumSource::new(equalized, spectrum, player.generation.clone(), my_gen);
    match fade_in_secs {
        Some(secs) if secs > 0.0 => {
            sink.append(tapped.fade_in(Duration::from_secs_f64(secs.min(12.0))));
        }
        _ => sink.append(tapped),
    }

    // Resume support: jump to a saved position before (optionally) playing.
    if let Some(pos) = start_at {
        let target = if duration > 0.1 {
            pos.clamp(0.0, duration - 0.1)
        } else {
            pos.max(0.0)
        };
        if target > 0.0 {
            let _ = sink.try_seek(Duration::from_secs_f64(target));
        }
    }

    if autoplay {
        sink.play();
    } else {
        sink.pause();
    }
    active.store(true, Ordering::SeqCst);

    // Save as current track
    {
        let mut current_guard = player.current_track.lock();
        *current_guard = Some(ActiveTrack {
            path: path.clone(),
            sink: sink.clone(),
            duration,
            start_time: Instant::now(),
        });
    }

    // Read properties via lofty
    let mut sample_rate = None;
    let mut bit_depth = None;
    if let Ok(tagged_file) = lofty::probe::Probe::open(&path_buf).and_then(|p| p.read()) {
        let props = tagged_file.properties();
        sample_rate = props.sample_rate();
        bit_depth = props.bit_depth();
    }

    Ok(PlaybackInfo {
        duration,
        sample_rate,
        bit_depth,
    })
}

// Pre-decode the next track for near-gapless playback. The ready decoder is
// stored and consumed by the upcoming player_load (when paths match), skipping
// its disk read + decoder setup so the transition has no audible decode gap.
#[tauri::command]
async fn player_prepare_next(
    app: AppHandle,
    player: State<'_, AudioPlayer>,
    path: String,
    duration_hint: Option<f64>,
) -> Result<(), String> {
    let path_buf = PathBuf::from(&path);
    if !is_allowed_audio(&app, &path_buf) {
        return Err("Path is not within an allowed music folder".to_string());
    }

    // The exclusive engine doesn't use rodio pre-decode; skip entirely.
    #[cfg(target_os = "windows")]
    if player.exclusive_enabled.load(Ordering::SeqCst) {
        return Ok(());
    }

    // Set target path and clear any stale prepared track
    {
        let mut target_g = player.prepared_target_path.lock();
        if target_g.as_ref() == Some(&path) {
            return Ok(());
        }
        *target_g = Some(path.clone());
        *player.prepared.lock() = None;
    }

    let prepared = player.prepared.clone();
    let prepared_target_path = player.prepared_target_path.clone();
    let pb = path_buf.clone();
    let path_clone = path.clone();
    if let Ok(Ok((dec, raw_dur))) =
        tauri::async_runtime::spawn_blocking(move || build_decoder(&pb)).await
    {
        // Check if this path is still the target before continuing with RG tags and saving
        {
            let target_g = prepared_target_path.lock();
            if target_g.as_ref() != Some(&path_clone) {
                return Ok(());
            }
        }

        // Parse ReplayGain tags of the prepared track
        let mut gain_db = None;
        let mut peak = None;
        if let Ok(tagged_file) = Probe::open(&path_buf).and_then(|p| p.read()) {
            let tag = tagged_file.primary_tag().or_else(|| tagged_file.first_tag());
            gain_db = tag
                .and_then(|t| t.get_string(&ItemKey::ReplayGainTrackGain))
                .and_then(parse_rg_db);
            peak = tag
                .and_then(|t| t.get_string(&ItemKey::ReplayGainTrackPeak))
                .and_then(|s| s.trim().parse::<f32>().ok());
        }

        let dur = if raw_dur > 0.0 {
            raw_dur
        } else {
            duration_hint.unwrap_or(0.0)
        };

        // Check again after metadata read
        {
            let target_g = prepared_target_path.lock();
            if target_g.as_ref() != Some(&path_clone) {
                return Ok(());
            }
        }

        *prepared.lock() = Some(PreparedTrack {
            path: path_clone,
            decoder: dec,
            duration: dur,
            gain_db,
            peak,
        });
    }
    Ok(())
}

#[tauri::command]
fn player_pause(app: AppHandle, player: State<AudioPlayer>) {
    #[cfg(target_os = "windows")]
    if player.exclusive_enabled.load(Ordering::SeqCst) {
        if let Some(ex) = app.try_state::<Arc<exclusive::ExclusivePlayer>>() {
            if ex.is_active() {
                ex.set_playing(false);
                return;
            }
        }
    }
    if let Some(sink) = player.sink() {
        sink.pause();
    }
    // Also pause all fading tracks!
    let fading = player.fading_tracks.lock();
    for track in fading.iter() {
        track.sink.pause();
    }
}

#[tauri::command]
fn player_resume(app: AppHandle, player: State<AudioPlayer>) {
    #[cfg(target_os = "windows")]
    if player.exclusive_enabled.load(Ordering::SeqCst) {
        if let Some(ex) = app.try_state::<Arc<exclusive::ExclusivePlayer>>() {
            if ex.is_active() {
                ex.set_playing(true);
                return;
            }
        }
    }
    if let Some(sink) = player.sink() {
        sink.play();
    }
    // Also resume all fading tracks!
    let fading = player.fading_tracks.lock();
    for track in fading.iter() {
        track.sink.play();
    }
}

#[tauri::command]
fn player_set_volume(player: State<AudioPlayer>, volume: f64) {
    let user_vol = volume.clamp(0.0, 1.0) as f32;
    *player.last_volume.lock() = user_vol;
    let eff_vol = player.effective_volume();
    if let Some(sink) = player.sink() {
        sink.set_volume(eff_vol);
    }
    let mut fading = player.fading_tracks.lock();
    for track in fading.iter_mut() {
        track.initial_volume = eff_vol;
    }
}

#[tauri::command]
fn player_seek(app: AppHandle, player: State<AudioPlayer>, position: f64) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    if player.exclusive_enabled.load(Ordering::SeqCst) {
        if let Some(ex) = app.try_state::<Arc<exclusive::ExclusivePlayer>>() {
            if ex.is_active() {
                ex.seek(position);
                return Ok(());
            }
        }
    }
    // Stop all fading tracks on manual seek
    {
        let mut fading_guard = player.fading_tracks.lock();
        for t in fading_guard.drain(..) {
            t.sink.stop();
        }
    }
    if let Some(sink) = player.sink() {
        let duration = *player.duration.lock();
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
fn player_stop(app: AppHandle, player: State<AudioPlayer>) {
    #[cfg(target_os = "windows")]
    if let Some(ex) = app.try_state::<Arc<exclusive::ExclusivePlayer>>() {
        if ex.is_active() {
            ex.stop();
        }
    }
    {
        let mut fading_guard = player.fading_tracks.lock();
        for t in fading_guard.drain(..) {
            t.sink.stop();
        }
        let mut current_guard = player.current_track.lock();
        if let Some(t) = current_guard.take() {
            t.sink.stop();
        }
    }
    player.active.store(false, Ordering::SeqCst);
}

#[tauri::command]
fn player_status(app: AppHandle, player: State<AudioPlayer>) -> PlayerStatus {
    #[cfg(target_os = "windows")]
    if player.exclusive_enabled.load(Ordering::SeqCst) {
        if let Some(ex) = app.try_state::<Arc<exclusive::ExclusivePlayer>>() {
            if ex.is_active() {
                return ex.status();
            }
        }
    }
    match player.sink() {
        Some(sink) => {
            let empty = sink.empty();
            let path = player.current_track.lock().as_ref().map(|t| t.path.clone());
            PlayerStatus {
                position: sink.get_pos().as_secs_f64(),
                duration: *player.duration.lock(),
                playing: !sink.is_paused() && !empty,
                finished: player.active.load(Ordering::SeqCst) && empty,
                path,
            }
        }
        None => PlayerStatus {
            position: 0.0,
            duration: 0.0,
            playing: false,
            finished: false,
            path: None,
        },
    }
}

#[tauri::command]
fn player_set_transition(player: State<AudioPlayer>, mode: String, crossfade_secs: f64) {
    *player.transition_mode.lock() = mode;
    *player.crossfade_secs.lock() = crossfade_secs;
}

#[tauri::command]
fn player_set_normalization_settings(
    player: State<AudioPlayer>,
    enabled: bool,
    preamp_db: f64,
) {
    *player.normalization_enabled.lock() = enabled;
    *player.normalization_preamp_db.lock() = preamp_db;
}

// Latest six frequency-band levels (0..1), low → high. Returns all-zero when no
// track is playing or the visualizer is disabled. Polled by the UI at ~30fps.
#[tauri::command]
fn player_spectrum(player: State<AudioPlayer>) -> [f32; SPECTRUM_BANDS] {
    player.spectrum.load()
}

// Toggle the (cheap but non-zero) audio analysis on/off, mirroring the Settings
// switch so it truly costs nothing when the visualizer is hidden.
#[tauri::command]
fn player_set_spectrum_enabled(player: State<AudioPlayer>, enabled: bool) {
    player.spectrum.enabled.store(enabled, Ordering::SeqCst);
    if !enabled {
        player.spectrum.reset();
    }
}

// Apply the 10-band equalizer. `gains` is ten per-band gains in dB (low → high),
// `preamp_db` a master gain applied before the bands. The change is picked up by
// the live audio chain on its next sample, so it takes effect mid-track.
#[tauri::command]
fn player_set_equalizer(
    player: State<AudioPlayer>,
    enabled: bool,
    gains: Vec<f64>,
    preamp_db: f64,
) {
    let mut g = [0f32; EQ_BANDS];
    for (slot, v) in g.iter_mut().zip(gains.iter()) {
        *slot = (*v as f32).clamp(-12.0, 12.0);
    }
    player
        .equalizer
        .set(enabled, &g, (preamp_db as f32).clamp(-12.0, 12.0));
}

// Open an OutputStream for the named device (or the system default when None),
// build a fresh Sink on its mixer and publish it into `slot`. Returns the
// stream so the caller (the audio thread) can keep it alive; leaves `slot` as
// None on failure. The new sink inherits the current effective volume so a
// device switch doesn't reset levels.
fn open_device_stream(
    name: Option<&str>,
    slot: &Arc<Mutex<Option<Arc<Sink>>>>,
    last_volume: &Arc<Mutex<f32>>,
    norm_factor: &Arc<Mutex<f32>>,
) -> Option<OutputStream> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};

    // Opening can transiently fail right after an exclusive-mode stream releases
    // the device (USB DACs / IEMs re-enumerate slowly), so retry a few times
    // before giving up.
    let mut stream = None;
    for attempt in 0..4 {
        stream = match name {
            Some(want) => {
                let host = rodio::cpal::default_host();
                let device = host.output_devices().ok().and_then(|mut devs| {
                    devs.find(|d| d.name().map(|n| n == want).unwrap_or(false))
                });
                match device {
                    Some(dev) => OutputStreamBuilder::from_device(dev)
                        .and_then(|b| b.open_stream())
                        .ok(),
                    // Requested device vanished — fall back to default.
                    None => OutputStreamBuilder::open_default_stream().ok(),
                }
            }
            None => OutputStreamBuilder::open_default_stream().ok(),
        };
        if stream.is_some() {
            break;
        }
        if attempt < 3 {
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    match stream {
        Some(stream) => {
            let sink = Sink::connect_new(stream.mixer());
            let vol =
                (*last_volume.lock() * *norm_factor.lock()).clamp(0.0, 4.0);
            sink.set_volume(vol);
            *slot.lock() = Some(Arc::new(sink));
            Some(stream)
        }
        None => {
            *slot.lock() = None;
            None
        }
    }
}

// Build the audio player. The OutputStream is `!Send`, so it lives on a
// dedicated thread that owns it for the app's lifetime; the thread blocks on a
// command channel (keeping the stream alive) and rebuilds the stream/sink when
// the output device changes. Only the (Send) Sink handle is shared back.
fn init_audio_player() -> AudioPlayer {
    let (cmd_tx, cmd_rx) = mpsc::channel::<AudioCommand>();
    let (ready_tx, ready_rx) = mpsc::channel::<()>();

    let sink_slot: Arc<Mutex<Option<Arc<Sink>>>> = Arc::new(Mutex::new(None));
    let norm_factor = Arc::new(Mutex::new(1.0f32));
    let last_volume = Arc::new(Mutex::new(1.0f32));

    let slot_t = sink_slot.clone();
    let nf_t = norm_factor.clone();
    let lv_t = last_volume.clone();
    std::thread::spawn(move || {
        // Open the default device on startup, then signal readiness.
        let mut _stream = open_device_stream(None, &slot_t, &lv_t, &nf_t);
        let _ = ready_tx.send(());

        // Process device-change commands. recv() blocks (keeping `_stream`
        // alive) until the sender is dropped at shutdown.
        while let Ok(cmd) = cmd_rx.recv() {
            match cmd {
                AudioCommand::OpenDevice(name, reply) => {
                    // Stop and drop the old sink, then drop the old stream by
                    // overwriting it with the freshly opened one.
                    {
                        let mut guard = slot_t.lock();
                        if let Some(s) = guard.take() {
                            s.stop();
                        }
                    }
                    _stream = open_device_stream(name.as_deref(), &slot_t, &lv_t, &nf_t);
                    let _ = reply.send(());
                }
                AudioCommand::CreateSink(reply) => {
                    // Self-heal: the shared stream may be absent (just back from
                    // exclusive mode, or a transient open failure). Re-open the
                    // default device before giving up so playback recovers instead
                    // of leaving the UI stuck "loading".
                    if _stream.is_none() {
                        _stream = open_device_stream(None, &slot_t, &lv_t, &nf_t);
                    }
                    let res = if let Some(ref stream) = _stream {
                        Ok(Arc::new(Sink::connect_new(stream.mixer())))
                    } else {
                        Err("No active audio stream".to_string())
                    };
                    let _ = reply.send(res);
                }
                AudioCommand::CloseStream(reply) => {
                    {
                        let mut guard = slot_t.lock();
                        if let Some(s) = guard.take() {
                            s.stop();
                        }
                    }
                    _stream = None;
                    let _ = reply.send(());
                }
            }
        }
    });

    // Wait for the initial device open so the first player_load sees a sink
    // (or a definitive None when no output device exists).
    let _ = ready_rx.recv();

    AudioPlayer {
        sink: sink_slot,
        duration: Arc::new(Mutex::new(0.0)),
        active: Arc::new(AtomicBool::new(false)),
        generation: Arc::new(AtomicU64::new(0)),
        spectrum: Arc::new(SpectrumShared::new()),
        equalizer: Arc::new(EqualizerShared::new()),
        norm_factor,
        last_volume,
        cmd_tx,
        prepared: Arc::new(Mutex::new(None)),
        prepared_target_path: Arc::new(Mutex::new(None)),
        transition_mode: Arc::new(Mutex::new("off".to_string())),
        crossfade_secs: Arc::new(Mutex::new(6.0)),
        normalization_enabled: Arc::new(Mutex::new(false)),
        normalization_preamp_db: Arc::new(Mutex::new(0.0)),
        current_track: Arc::new(Mutex::new(None)),
        fading_tracks: Arc::new(Mutex::new(Vec::new())),
        exclusive_enabled: Arc::new(AtomicBool::new(false)),
    }
}

// True-gapless lead: how early (seconds before the current track ends) the next
// track is appended onto the SAME sink. rodio plays queued sources back-to-back
// with zero gap, so this lead only needs to clear the ~20ms ticker tick and the
// audio output buffer. Kept short so a last-moment queue edit has the smallest
// possible window to race the already-queued track.
const GAPLESS_LEAD_SECS: f64 = 0.3;

// A next track appended onto the *currently playing* sink's queue for true
// gapless playback, waiting for the active source to finish. The ticker promotes
// it to the current track the instant rodio advances the queue to it.
struct GaplessQueued {
    path: String,
    duration: f64,
    gain_db: Option<f32>,
    peak: Option<f32>,
    sink: Arc<Sink>,
}

fn spawn_player_ticker(app: AppHandle, player: AudioPlayer) {
    std::thread::spawn(move || {
        let mut transition_triggered_for_gen = 0;
        // Next track queued onto the live sink for true-gapless playback (None
        // when nothing is waiting ahead).
        let mut gapless_queued: Option<GaplessQueued> = None;
        loop {
            std::thread::sleep(Duration::from_millis(20));

            // 1. Process fading tracks (MUST run even if active is false, so fading tracks fade out and stop!)
            {
                let mut fading = player.fading_tracks.lock();
                let now = Instant::now();
                fading.retain(|track| {
                    if now >= track.fade_end {
                        track.sink.stop();
                        false
                    } else {
                        let total_secs = track.fade_duration.as_secs_f64();
                        let remaining_ratio = if total_secs > 0.0 {
                            (track.fade_end - now).as_secs_f64() / total_secs
                        } else {
                            0.0
                        };
                        let fade_vol = track.initial_volume * (remaining_ratio as f32).clamp(0.0, 1.0);
                        track.sink.set_volume(fade_vol);
                        true
                    }
                });
            }

            // 2. Process automatic transitions
            let active = player.active.load(Ordering::SeqCst);
            if !active {
                continue;
            }

            let current_gen = player.generation.load(Ordering::SeqCst);

            let (current_sink, position, duration, empty) = {
                let current_guard = player.current_track.lock();
                if let Some(ref track) = *current_guard {
                    let pos = track.sink.get_pos().as_secs_f64();
                    let dur = track.duration;
                    let empty = track.sink.empty();
                    (track.sink.clone(), pos, dur, empty)
                } else {
                    continue;
                }
            };

            let mode = player.transition_mode.lock().clone();
            let crossfade_secs = *player.crossfade_secs.lock();

            // --- True-gapless boundary: promote a queued next track ----------
            // When a next track was appended ahead onto the shared sink, watch
            // its queue: once the previous source drains (only the queued track
            // remains) rodio is already playing it seamlessly, so promote it to
            // the current track, fix up duration/normalization, and announce it.
            if let Some(q) = gapless_queued.take() {
                if !Arc::ptr_eq(&q.sink, &current_sink) {
                    // A manual load replaced the sink (and stopped the queued
                    // track with it) — drop the now-stale entry.
                } else if q.sink.len() <= 1 {
                    let user_vol = *player.last_volume.lock();
                    let norm_enabled = *player.normalization_enabled.lock();
                    let preamp_db = *player.normalization_preamp_db.lock();
                    let factor = if norm_enabled {
                        let total_db = q.gain_db.map(|g| g as f64).unwrap_or(0.0) + preamp_db;
                        let mut f = 10f64.powf(total_db / 20.0);
                        if let Some(pk) = q.peak {
                            if pk > 0.0 {
                                f = f.min(1.0 / pk as f64);
                            }
                        }
                        (f as f32).clamp(0.0, 4.0)
                    } else {
                        1.0
                    };
                    *player.norm_factor.lock() = factor;
                    q.sink.set_volume(user_vol * factor);
                    *player.duration.lock() = q.duration;
                    *player.current_track.lock() = Some(ActiveTrack {
                        path: q.path.clone(),
                        sink: q.sink.clone(),
                        duration: q.duration,
                        start_time: Instant::now(),
                    });
                    let _ = app.emit("track-changed", serde_json::json!({ "path": q.path }));
                    // Re-read fresh state next tick rather than acting on the
                    // outgoing track's stale position/duration this iteration.
                    continue;
                } else {
                    // Current source still playing — keep waiting.
                    gapless_queued = Some(q);
                }
            }

            if mode == "crossfade" && (duration > crossfade_secs && position >= (duration - crossfade_secs) || empty) {
                if transition_triggered_for_gen != current_gen {
                    let has_prep = player.prepared.lock().is_some();

                    if has_prep {
                        let prepared_opt = player.prepared.lock().take();
                        // Consuming the prepared track invalidates the "currently
                        // preparing" marker; clear it so the next prepare_next call
                        // (for the track after this one) is never mistaken for a no-op.
                        *player.prepared_target_path.lock() = None;

                        if let Some(prep) = prepared_opt {
                            let (reply_tx, reply_rx) = mpsc::channel();
                            if player.cmd_tx.send(AudioCommand::CreateSink(reply_tx)).is_ok() {
                                if let Ok(Ok(new_sink)) = reply_rx.recv_timeout(Duration::from_secs(2)) {
                                    let mut current_guard = player.current_track.lock();
                                    if let Some(ref old_track) = *current_guard {
                                        let mut fading_guard = player.fading_tracks.lock();
                                        fading_guard.push(FadingTrack {
                                            sink: old_track.sink.clone(),
                                            fade_end: Instant::now() + Duration::from_secs_f64(crossfade_secs),
                                            fade_duration: Duration::from_secs_f64(crossfade_secs),
                                            initial_volume: player.effective_volume(),
                                        });
                                    }

                                    let new_gen = player.generation.fetch_add(1, Ordering::SeqCst) + 1;
                                    // Mark the *outgoing* track's generation as handled — NOT the
                                    // new one. The new track starts via the `track-changed` event,
                                    // which the frontend handles with `skipNextLoad` and so never
                                    // calls `player_load` to bump the generation. Tagging `new_gen`
                                    // here left the guard equal to the new track's live generation
                                    // for its entire playback, blocking *its* transition — so
                                    // crossfade only fired on every other track. Keying on
                                    // `current_gen` lets each track transition exactly once.
                                    transition_triggered_for_gen = current_gen;

                                    let next_dur = prep.duration;
                                    *player.duration.lock() = next_dur;

                                    // Apply normalization to the new sink
                                    let norm_enabled = *player.normalization_enabled.lock();
                                    let preamp_db = *player.normalization_preamp_db.lock();
                                    let factor = if norm_enabled {
                                        let total_db = prep.gain_db.map(|g| g as f64).unwrap_or(0.0) + preamp_db;
                                        let mut f = 10f64.powf(total_db / 20.0);
                                        if let Some(pk) = prep.peak {
                                            if pk > 0.0 {
                                                f = f.min(1.0 / pk as f64);
                                            }
                                        }
                                        (f as f32).clamp(0.0, 4.0)
                                    } else {
                                        1.0
                                    };
                                    *player.norm_factor.lock() = factor;

                                    let equalized = EqualizerSource::new(prep.decoder, player.equalizer.clone());
                                    let tapped = SpectrumSource::new(equalized, player.spectrum.clone(), player.generation.clone(), new_gen);
                                    new_sink.append(tapped.fade_in(Duration::from_secs_f64(crossfade_secs)));

                                    let user_vol = *player.last_volume.lock();
                                    new_sink.set_volume(user_vol * factor);
                                    new_sink.play();

                                    *current_guard = Some(ActiveTrack {
                                        path: prep.path.clone(),
                                        sink: new_sink,
                                        duration: next_dur,
                                        start_time: Instant::now(),
                                    });

                                    let _ = app.emit("track-changed", serde_json::json!({ "path": prep.path }));
                                }
                            }
                        }
                    }
                }
            } else if mode == "gapless"
                && gapless_queued.is_none()
                && transition_triggered_for_gen != current_gen
                && ((duration > 0.0 && position >= (duration - GAPLESS_LEAD_SECS)) || empty)
            {
                // True gapless: append the pre-decoded next track straight onto
                // the live sink. rodio plays queued sources back-to-back, so it
                // starts the instant the current source ends — one continuous
                // sink, no fade, no gap. The boundary handler above promotes it
                // to the current track once it actually begins.
                let prepared_opt = player.prepared.lock().take();
                if let Some(prep) = prepared_opt {
                    // Consuming the prepared track invalidates the "currently
                    // preparing" marker; clear it so the next prepare_next call
                    // (for the track after this one) is never mistaken for a no-op.
                    *player.prepared_target_path.lock() = None;
                    // Reserve the generation now so this track's spectrum tap is
                    // live the moment it becomes the active source. (The outgoing
                    // track's visualizer goes quiet for the short lead window —
                    // imperceptible at a track's tail.)
                    let new_gen = player.generation.fetch_add(1, Ordering::SeqCst) + 1;
                    // Mark the *outgoing* track's generation as handled so we
                    // append exactly once per track (see the crossfade branch for
                    // why this keys on current_gen, not new_gen).
                    transition_triggered_for_gen = current_gen;
                    let equalized = EqualizerSource::new(prep.decoder, player.equalizer.clone());
                    let tapped = SpectrumSource::new(
                        equalized,
                        player.spectrum.clone(),
                        player.generation.clone(),
                        new_gen,
                    );
                    current_sink.append(tapped);
                    gapless_queued = Some(GaplessQueued {
                        path: prep.path.clone(),
                        duration: prep.duration,
                        gain_db: prep.gain_db,
                        peak: prep.peak,
                        sink: current_sink.clone(),
                    });
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Output device selection
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct OutputDeviceInfo {
    name: String,
    is_default: bool,
}

// Enumerate the available audio output devices.
#[tauri::command]
fn list_output_devices() -> Vec<OutputDeviceInfo> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    let host = rodio::cpal::default_host();
    let default_name = host.default_output_device().and_then(|d| d.name().ok());
    let mut out = Vec::new();
    if let Ok(devices) = host.output_devices() {
        for d in devices {
            if let Ok(name) = d.name() {
                let is_default = default_name.as_deref() == Some(name.as_str());
                out.push(OutputDeviceInfo { name, is_default });
            }
        }
    }
    out
}

// Switch the audio output device (None = system default). Blocks until the new
// device is open so the frontend can immediately reload the current track.
#[tauri::command]
fn set_output_device(player: State<AudioPlayer>, name: Option<String>) -> Result<(), String> {
    let (reply_tx, reply_rx) = mpsc::channel();
    player
        .cmd_tx
        .send(AudioCommand::OpenDevice(name, reply_tx))
        .map_err(|e| format!("Audio thread unavailable: {e}"))?;
    let _ = reply_rx.recv_timeout(Duration::from_secs(5));
    Ok(())
}

// ---------------------------------------------------------------------------
// Volume normalization (Sound Check) — ReplayGain tags + lazy EBU R128 compute
// ---------------------------------------------------------------------------

// Reference loudness for normalization (ReplayGain 2.0 standard).
const NORM_TARGET_LUFS: f64 = -18.0;

// Set the normalization factor from a track's gain (dB), an optional peak (to
// prevent clipping when boosting) and the user pre-amp. Re-applies immediately
// to the live sink. `enabled = false` resets the factor to 1.0 (no change).
#[tauri::command]
fn player_set_normalization(
    player: State<AudioPlayer>,
    gain_db: Option<f64>,
    preamp_db: f64,
    peak: Option<f64>,
    enabled: bool,
) {
    let factor = if enabled {
        let total_db = gain_db.unwrap_or(0.0) + preamp_db;
        let mut f = 10f64.powf(total_db / 20.0);
        if let Some(pk) = peak {
            if pk > 0.0 {
                f = f.min(1.0 / pk); // never amplify past the track's peak headroom
            }
        }
        (f as f32).clamp(0.0, 4.0)
    } else {
        1.0
    };
    *player.norm_factor.lock() = factor;
    if let Some(sink) = player.sink() {
        sink.set_volume(player.effective_volume());
    }
}

// Process-wide guard around the on-disk loudness cache file.
static LOUDNESS_LOCK: Mutex<()> = Mutex::new(());

fn loudness_cache_file(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_cache_dir().ok()?;
    fs::create_dir_all(&dir).ok()?;
    Some(dir.join("loudness.json"))
}

fn read_loudness(app: &AppHandle, key: &str) -> Option<f32> {
    let file = loudness_cache_file(app)?;
    let _g = LOUDNESS_LOCK.lock();
    let data = fs::read_to_string(&file).ok()?;
    let map: HashMap<String, f32> = serde_json::from_str(&data).ok()?;
    map.get(key).copied()
}

fn write_loudness(app: &AppHandle, key: &str, gain: f32) {
    if let Some(file) = loudness_cache_file(app) {
        let _g = LOUDNESS_LOCK.lock();
        let mut map: HashMap<String, f32> = fs::read_to_string(&file)
            .ok()
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default();
        map.insert(key.to_string(), gain);
        if let Ok(s) = serde_json::to_string(&map) {
            let _ = fs::write(&file, s);
        }
    }
}

// Decode the whole track and measure its integrated loudness (EBU R128), then
// return the gain (dB) needed to reach the reference target. Heavy — runs on
// the blocking pool via compute_track_gain.
fn compute_gain_blocking(path: &Path) -> Result<f32, String> {
    use ebur128::{EbuR128, Mode};
    let (decoder, _dur) = build_decoder(path)?;
    let channels = decoder.channels().max(1) as u32;
    let sample_rate = decoder.sample_rate().max(1);
    let mut ebu = EbuR128::new(channels, sample_rate, Mode::I).map_err(|e| e.to_string())?;
    let mut buf: Vec<f32> = Vec::with_capacity(65536);
    for s in decoder {
        buf.push(s);
        if buf.len() >= 65536 {
            ebu.add_frames_f32(&buf).map_err(|e| e.to_string())?;
            buf.clear();
        }
    }
    if !buf.is_empty() {
        ebu.add_frames_f32(&buf).map_err(|e| e.to_string())?;
    }
    let loudness = ebu.loudness_global().map_err(|e| e.to_string())?;
    if !loudness.is_finite() {
        return Ok(0.0); // silent / unmeasurable track
    }
    let gain = (NORM_TARGET_LUFS - loudness) as f32;
    Ok(gain.clamp(-15.0, 15.0))
}

// Return the normalization gain (dB) for a track without ReplayGain tags,
// computing and caching it on first request.
#[tauri::command]
async fn compute_track_gain(app: AppHandle, path: String) -> Result<f32, String> {
    let path_buf = PathBuf::from(&path);
    if !is_allowed_audio(&app, &path_buf) {
        return Err("Path is not within an allowed music folder".to_string());
    }
    let key = cover_cache_key(&path_buf).unwrap_or_else(|| path.clone());
    if let Some(g) = read_loudness(&app, &key) {
        return Ok(g);
    }
    let app2 = app.clone();
    let gain = tauri::async_runtime::spawn_blocking(move || compute_gain_blocking(&path_buf))
        .await
        .map_err(|e| format!("Loudness task failed: {e}"))??;
    write_loudness(&app2, &key, gain);
    Ok(gain)
}

// ---------------------------------------------------------------------------
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
    let path_buf = PathBuf::from(&path);
    if !is_allowed_audio(&app, &path_buf) {
        return None;
    }

    if lyrics_source == "none" {
        return None;
    }

    // Disk cache keyed by path+mtime+size+provider. "null" = previously not found.
    // `_v2` schema tag: bumped when LyricLine gained word-level timing + romaji,
    // so stale line-only cache entries are re-fetched instead of reused.
    let cache_file = cover_cache_key(&path_buf)
        .and_then(|k| lyrics_cache_dir(&app).map(|d| d.join(format!("{k}_{lyrics_source}_v2.json"))));
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
                            loaded_lyrics.lines.retain(|line| !lyrics::is_netease_metadata(line));
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
// A single RecommendedWatcher covers all scanned roots. Raw events are funnelled
// into a coalescing thread that debounces bursts (e.g. a bulk copy) into a
// single `library-changed` event, which the frontend reacts to with an
// incremental refresh.
// ---------------------------------------------------------------------------

struct FileWatcher {
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    // Notifies the coalescing thread that a relevant change occurred.
    evt_tx: mpsc::Sender<()>,
}

// Drain a burst of filesystem events and emit one `library-changed` per quiet
// window, so a folder copy doesn't trigger dozens of rescans.
fn spawn_fs_coalescer(app: AppHandle, rx: mpsc::Receiver<()>) {
    std::thread::spawn(move || loop {
        // Block until the first event of a burst.
        if rx.recv().is_err() {
            break; // all senders dropped → app shutting down
        }
        // Swallow further events until things go quiet for the debounce window.
        loop {
            match rx.recv_timeout(Duration::from_millis(800)) {
                Ok(()) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        let _ = app.emit("library-changed", ());
    });
}

// (Re)configure the watcher to cover exactly the given roots. Replacing the
// watcher drops the previous one, unwatching the old set.
#[tauri::command]
fn watch_roots(state: State<FileWatcher>, roots: Vec<String>) -> Result<(), String> {
    use notify::event::{EventKind, ModifyKind};
    use notify::{RecursiveMode, Watcher};

    let tx = state.evt_tx.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            // Only structural changes (add / remove / rename) warrant a rescan;
            // ignore pure content/attribute writes to avoid needless churn.
            let relevant = matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
            );
            if relevant {
                let _ = tx.send(());
            }
        }
    })
    .map_err(|e| e.to_string())?;

    for root in &roots {
        // Best-effort: a missing/again-removed folder shouldn't abort the rest.
        let _ = watcher.watch(Path::new(root), RecursiveMode::Recursive);
    }

    *state.watcher.lock() = Some(watcher);
    Ok(())
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
    let path_buf = Path::new(&path);
    if !is_allowed_audio(&app, path_buf) {
        return Err("Path is not within an allowed music folder".to_string());
    }
    if !path_buf.exists() {
        return Err("File does not exist".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let mut path_win = path.replace('/', "\\");
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
            .arg(&path)
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
fn player_delete_file(app: AppHandle, path: String) -> Result<(), String> {
    let path_buf = Path::new(&path);
    if !is_allowed_audio(&app, path_buf) {
        return Err("Path is not within an allowed music folder".to_string());
    }
    if path_buf.exists() {
        fs::remove_file(path_buf).map_err(|e| e.to_string())?;
        Ok(())
    } else {
        Err("File not found".to_string())
    }
}

// Toggle WASAPI exclusive output. Enabling frees the rodio shared sink and closes
// the shared stream so the exclusive engine can claim the device; disabling
// stops the exclusive engine and re-opens the shared-mode stream.
// The frontend reloads the current track afterwards so playback continues on the
// newly-selected engine. (Windows only; a no-op stub elsewhere.)
#[cfg(target_os = "windows")]
#[tauri::command]
async fn set_wasapi_exclusive(app: tauri::AppHandle, player: tauri::State<'_, AudioPlayer>, enabled: bool) -> Result<(), String> {
    player.exclusive_enabled.store(enabled, Ordering::SeqCst);
    if enabled {
        let mut fading_guard = player.fading_tracks.lock();
        for t in fading_guard.drain(..) {
            t.sink.stop();
        }
        let mut current_guard = player.current_track.lock();
        if let Some(t) = current_guard.take() {
            t.sink.stop();
        }
        player.active.store(false, Ordering::SeqCst);

        // Close the shared stream so WASAPI exclusive can claim the device
        let (reply_tx, reply_rx) = mpsc::channel();
        player
            .cmd_tx
            .send(AudioCommand::CloseStream(reply_tx))
            .map_err(|e| format!("Audio thread unavailable: {e}"))?;
        reply_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|e| format!("Failed to close shared stream: {e}"))?;
    } else {
        if let Some(ex) = app.try_state::<Arc<exclusive::ExclusivePlayer>>() {
            let ex = ex.inner().clone();
            tauri::async_runtime::spawn_blocking(move || {
                ex.stop();
            })
            .await
            .map_err(|e| format!("Exclusive stop task failed: {e}"))?;
        }
        // Re-open the shared stream so standard playback works again
        let (reply_tx, reply_rx) = mpsc::channel();
        player
            .cmd_tx
            .send(AudioCommand::OpenDevice(None, reply_tx))
            .map_err(|e| format!("Audio thread unavailable: {e}"))?;
        reply_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| format!("Failed to re-open shared stream: {e}"))?;
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
async fn set_wasapi_exclusive(_enabled: bool) -> Result<(), String> {
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Channel feeding the filesystem-watch coalescer (spawned in setup).
    let (fs_tx, fs_rx) = mpsc::channel::<()>();
    let fs_rx = Mutex::new(Some(fs_rx));

    let audio = init_audio_player();
    // The exclusive engine reuses the main player's EQ / spectrum / volume /
    // normalization state so those features behave identically in exclusive mode.
    #[cfg(target_os = "windows")]
    let exclusive_player = Arc::new(exclusive::ExclusivePlayer::new(
        audio.equalizer.clone(),
        audio.spectrum.clone(),
        audio.last_volume.clone(),
        audio.norm_factor.clone(),
    ));

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(audio)
        .manage(discord::DiscordState::new())
        .manage(FileWatcher {
            watcher: Mutex::new(None),
            evt_tx: fs_tx,
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
            
            // Spawn the audio player tick thread for crossfade/gapless transitions
            let player = _app.state::<AudioPlayer>().inner().clone();
            spawn_player_ticker(_app.handle().clone(), player);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scan_music_folder,
            scan_paths,
            filter_existing,
            get_track_cover,
            get_track_cover_path,
            get_track_palette,
            get_waveform,
            restore_roots,
            player_load,
            player_prepare_next,
            player_pause,
            player_resume,
            player_set_volume,
            player_seek,
            player_stop,
            player_status,
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
            player_set_transition,
            player_set_normalization_settings,
            set_wasapi_exclusive,
            discord::discord_set_enabled,
            discord::discord_update,
            discord::discord_clear,
            discord::discord_cover_art,
            db::db_import,
            db::db_upsert_tracks,
            db::db_remove_paths,
            db::db_remove_under_root,
            db::db_prune_missing,
            db::db_count,
            db::db_reset,
            db::db_roots,
            db::db_set_roots,
            db::db_tracks_page,
            db::db_search,
            db::db_tracks_by_paths,
            db::db_track,
            db::db_random_track,
            db::db_albums,
            db::db_album_tracks,
            db::db_artists,
            db::db_artist_tracks,
            db::db_station_tracks,
            db::db_has_genre,
            db::db_smart_tracks,
            db::db_record_play_start,
            db::db_record_play,
            db::db_record_skip,
            db::db_stat,
            db::db_stats_summary,
            db::db_recently_played,
            db::db_most_played,
            db::db_on_repeat,
            db::db_recently_added,
            db::db_rediscover,
            db::db_top_artists,
            db::db_top_genres,
            db::db_favorite_paths,
            db::db_favorites,
            db::db_toggle_favorite,
            db::db_move_favorite,
            db::db_playlists,
            db::db_playlist_tracks,
            db::db_upsert_playlist,
            db::db_delete_playlist,
            db::db_move_playlist_order,
            db::db_playlist_add,
            db::db_playlist_remove,
            db::db_playlist_move_item,
            db::db_recents,
            db::db_record_recent,
            db::db_kv_get,
            db::db_kv_set
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
