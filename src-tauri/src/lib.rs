use rayon::prelude::*;
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose, Engine as _};
use lofty::picture::MimeType;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use rodio::source::SeekError;
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use tauri::{AppHandle, Emitter, Manager, State};
use walkdir::WalkDir;

mod lyrics;

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
    has_cover: bool,
    sample_rate: Option<u32>,
    bit_depth: Option<u8>,
    // ReplayGain track gain/peak read from tags (if present), used by the volume
    // normalization feature. `None` when the file carries no ReplayGain tags.
    track_gain_db: Option<f32>,
    track_peak: Option<f32>,
}

// Parse a ReplayGain gain string like "-6.54 dB" / "+3.2" into decibels.
fn parse_rg_db(s: &str) -> Option<f32> {
    let cleaned = s.trim().trim_end_matches(|c: char| c.is_alphabetic() || c.is_whitespace());
    cleaned.trim().parse::<f32>().ok()
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

// Command sent to the dedicated audio thread (which owns the !Send OutputStream).
enum AudioCommand {
    // Open the output device with the given name, or the system default (None).
    // The reply channel is signalled once the device is open.
    OpenDevice(Option<String>, mpsc::Sender<()>),
}

struct AudioPlayer {
    // The active sink. Wrapped in a Mutex<Option<..>> so the audio thread can
    // swap it when the output device changes; None when no device is available.
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
    // Volume-normalization factor (linear) applied on top of the user volume.
    // 1.0 = no normalization.
    norm_factor: Arc<Mutex<f32>>,
    // Last user-requested volume (0..1), so the normalization factor can be
    // re-applied to the live sink without the frontend re-sending the volume.
    last_volume: Arc<Mutex<f32>>,
    // Channel to the audio thread for device-management commands.
    cmd_tx: mpsc::Sender<AudioCommand>,
    // Pre-decoded next track (path + ready decoder) for near-gapless playback.
    // player_load consumes this when the loaded path matches, skipping the disk
    // read + decoder setup so the transition has no audible decode gap.
    prepared: Arc<Mutex<Option<(String, Decoder<Cursor<Vec<u8>>>)>>>,
}

impl AudioPlayer {
    // Clone out the current sink handle (if any) without holding the lock.
    fn sink(&self) -> Option<Arc<Sink>> {
        self.sink.lock().unwrap().clone()
    }
    // Effective sink volume = user volume * normalization factor. Capped above
    // 1.0 (≈ +12 dB) so normalization can boost quiet tracks; rodio amplifies
    // values > 1.0 and the peak limiter in player_set_normalization guards
    // against clipping.
    fn effective_volume(&self) -> f32 {
        let vol = *self.last_volume.lock().unwrap();
        let factor = *self.norm_factor.lock().unwrap();
        (vol * factor).clamp(0.0, 4.0)
    }
}

#[derive(Serialize)]
struct PlayerStatus {
    position: f64,
    duration: f64,
    playing: bool,
    finished: bool,
}

// Read a file into memory and build a *seekable* decoder. Decoding stays lazy
// (samples are produced on demand during playback), so playback starts almost
// immediately instead of waiting for the whole track. Reading into a Cursor
// keeps the audio callback off the disk, and `[profile.dev.package."*"]
// opt-level = 3` keeps the codec fast enough to never starve the callback —
// together that fixes both the slow start and the "bz bz bz" under load.
fn build_decoder(path: &Path) -> Result<(Decoder<Cursor<Vec<u8>>>, f64), String> {
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
struct SpectrumShared {
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
    fn reset(&self) {
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

struct SpectrumSource<S> {
    inner: S,
    shared: Arc<SpectrumShared>,
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
    fn new(inner: S, shared: Arc<SpectrumShared>) -> Self {
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
        if self.shared.enabled.load(Ordering::Relaxed) {
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

// Load a track and start playing it, replacing whatever was playing. Returns
// the track duration in seconds. The file read + decoder setup run on the
// blocking pool so the UI/IPC thread never stalls; the previously playing track
// is stopped immediately so it doesn't bleed over the (brief) load gap.
#[derive(Serialize)]
struct PlaybackInfo {
    duration: f64,
    sample_rate: Option<u32>,
    bit_depth: Option<u8>,
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

    let sink = match player.sink() {
        Some(s) => s,
        None => return Err("No audio output device".to_string()),
    };
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
    sink.clear();
    active.store(false, Ordering::SeqCst);
    spectrum.reset(); // clear the visualizer during the load gap
    // Apply the requested volume together with the active normalization factor.
    let user_vol = volume.clamp(0.0, 1.0) as f32;
    *last_volume.lock().unwrap() = user_vol;
    let factor = *norm_factor.lock().unwrap();
    sink.set_volume((user_vol * factor).clamp(0.0, 4.0));

    // Reuse a pre-decoded next track when it matches (near-gapless); otherwise
    // read + decode on the blocking pool.
    let prepared_decoder = {
        let mut g = prepared.lock().unwrap();
        if g.as_ref().map(|(p, _)| p == &path).unwrap_or(false) {
            g.take().map(|(_, d)| d)
        } else {
            None
        }
    };
    let (decoder, decoded_duration) = match prepared_decoder {
        Some(dec) => {
            let dur = dec.total_duration().map(|d| d.as_secs_f64()).unwrap_or(0.0);
            (dec, dur)
        }
        None => {
            let path_buf_clone = path_buf.clone();
            tauri::async_runtime::spawn_blocking(move || build_decoder(&path_buf_clone))
                .await
                .map_err(|e| format!("Decode task failed: {e}"))??
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
    *duration_slot.lock().unwrap() = duration;
    // Tap the decoded samples for the visualizer on their way to the sink.
    // In crossfade mode each track is eased in with a fade.
    let tapped = SpectrumSource::new(decoder, spectrum);
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
// its disk read + decode pass so the transition has no audible decode gap.
#[tauri::command]
async fn player_prepare_next(
    app: AppHandle,
    player: State<'_, AudioPlayer>,
    path: String,
) -> Result<(), String> {
    let path_buf = PathBuf::from(&path);
    if !is_allowed_audio(&app, &path_buf) {
        return Err("Path is not within an allowed music folder".to_string());
    }
    // Already prepared for this path — nothing to do.
    if player
        .prepared
        .lock()
        .unwrap()
        .as_ref()
        .map(|(p, _)| p == &path)
        .unwrap_or(false)
    {
        return Ok(());
    }
    let prepared = player.prepared.clone();
    let pb = path_buf.clone();
    if let Ok(Ok((dec, _dur))) =
        tauri::async_runtime::spawn_blocking(move || build_decoder(&pb)).await
    {
        *prepared.lock().unwrap() = Some((path, dec));
    }
    Ok(())
}

#[tauri::command]
fn player_pause(player: State<AudioPlayer>) {
    if let Some(sink) = player.sink() {
        sink.pause();
    }
}

#[tauri::command]
fn player_resume(player: State<AudioPlayer>) {
    if let Some(sink) = player.sink() {
        sink.play();
    }
}

#[tauri::command]
fn player_set_volume(player: State<AudioPlayer>, volume: f64) {
    let user_vol = volume.clamp(0.0, 1.0) as f32;
    *player.last_volume.lock().unwrap() = user_vol;
    if let Some(sink) = player.sink() {
        sink.set_volume(player.effective_volume());
    }
}

#[tauri::command]
fn player_seek(player: State<AudioPlayer>, position: f64) -> Result<(), String> {
    if let Some(sink) = player.sink() {
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
    if let Some(sink) = player.sink() {
        sink.clear();
    }
    player.active.store(false, Ordering::SeqCst);
}

#[tauri::command]
fn player_status(player: State<AudioPlayer>) -> PlayerStatus {
    match player.sink() {
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

    let stream = match name {
        Some(want) => {
            let host = rodio::cpal::default_host();
            let device = host
                .output_devices()
                .ok()
                .and_then(|mut devs| devs.find(|d| d.name().map(|n| n == want).unwrap_or(false)));
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

    match stream {
        Some(stream) => {
            let sink = Sink::connect_new(stream.mixer());
            let vol =
                (*last_volume.lock().unwrap() * *norm_factor.lock().unwrap()).clamp(0.0, 4.0);
            sink.set_volume(vol);
            *slot.lock().unwrap() = Some(Arc::new(sink));
            Some(stream)
        }
        None => {
            *slot.lock().unwrap() = None;
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
                        let mut guard = slot_t.lock().unwrap();
                        if let Some(s) = guard.take() {
                            s.stop();
                        }
                    }
                    _stream = open_device_stream(name.as_deref(), &slot_t, &lv_t, &nf_t);
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
        norm_factor,
        last_volume,
        cmd_tx,
        prepared: Arc::new(Mutex::new(None)),
    }
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
    *player.norm_factor.lock().unwrap() = factor;
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
    let _g = LOUDNESS_LOCK.lock().ok()?;
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
// Lyrics — local tag / sidecar .lrc, then LRCLIB → NetEase → Musixmatch
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
    musixmatch_token: Option<String>,
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
    let cache_file = cover_cache_key(&path_buf)
        .and_then(|k| lyrics_cache_dir(&app).map(|d| d.join(format!("{k}_{lyrics_source}.json"))));
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
                        return Some(l);
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
        if let Ok(client) = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .user_agent("ts-music/0.1 (https://github.com/)")
            .build()
        {
            if lyrics_source == "lrclib" {
                result = lyrics::from_lrclib(&client, &title, &artist, &album, duration_secs).await;
            } else if lyrics_source == "netease" {
                result = lyrics::from_netease(&client, &title, &artist).await;
            } else if lyrics_source == "musixmatch" {
                if let Some(token) = musixmatch_token
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                {
                    result = lyrics::from_musixmatch(
                        &client,
                        &title,
                        &artist,
                        &album,
                        duration_secs,
                        token,
                    )
                    .await;
                }
            }
        }
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

    *state.watcher.lock().unwrap() = Some(watcher);
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
        *controller.0.lock().unwrap() = Some(controls);
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
        if let Ok(mut guard) = arc.0.lock() {
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
        if let Ok(mut guard) = arc.0.lock() {
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
        }
    });
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Channel feeding the filesystem-watch coalescer (spawned in setup).
    let (fs_tx, fs_rx) = mpsc::channel::<()>();
    let fs_rx = Mutex::new(Some(fs_rx));

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(init_audio_player())
        .manage(FileWatcher {
            watcher: Mutex::new(None),
            evt_tx: fs_tx,
        });

    #[cfg(target_os = "windows")]
    {
        builder = builder.manage(Arc::new(MediaController(Mutex::new(None))));
    }

    builder
        .setup(move |_app| {
            #[cfg(target_os = "windows")]
            init_media_controls(_app.handle());
            // Start the debounced filesystem-change → library-changed pump.
            if let Some(rx) = fs_rx.lock().unwrap().take() {
                spawn_fs_coalescer(_app.handle().clone(), rx);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scan_music_folder,
            scan_paths,
            filter_existing,
            get_track_cover,
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
            list_output_devices,
            set_output_device,
            player_set_normalization,
            compute_track_gain,
            get_lyrics,
            watch_roots,
            smtc_set_metadata,
            smtc_set_playback,
            player_show_in_folder,
            player_delete_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
