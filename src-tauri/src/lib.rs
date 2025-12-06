use rayon::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};
use std::fs;
use walkdir::WalkDir;
use lofty::probe::Probe;
use lofty::prelude::*; 
use lofty::picture::MimeType;
use base64::{Engine as _, engine::general_purpose};

// Data to frontend
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

// Filter supported audio files
fn is_audio_file(path: &Path) -> bool {
    match path.extension() {
        Some(ext) => {
            let ext_str = ext.to_string_lossy().to_lowercase();
            matches!(ext_str.as_str(), "mp3" | "flac" | "wav" | "m4a" | "ogg" | "aac")
        }
        None => false,
    }
}

// Extract metadata
fn parse_metadata(path: &Path) -> Option<MusicTrack> {
    let path_str = path.to_string_lossy().to_string();
    
    // Get date created/added
    let date_added = fs::metadata(path)
        .and_then(|m| m.created().or(m.modified()))
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // extract tags
    let probe = Probe::open(path).ok()?;
    let tagged_file = probe.read().ok()?;

    let tag = tagged_file.primary_tag().or_else(|| tagged_file.first_tag());
    let properties = tagged_file.properties();

    // Extract fields
    let title = tag.and_then(|t| t.title().map(|s| s.to_string())).unwrap_or_else(|| 
        path.file_stem().unwrap_or_default().to_string_lossy().to_string()
    );
    let artist = tag.and_then(|t| t.artist().map(|s| s.to_string())).unwrap_or("Unknown Artist".to_string());
    let album = tag.and_then(|t| t.album().map(|s| s.to_string())).unwrap_or("Unknown Album".to_string());
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

// Scan directory
#[tauri::command]
async fn scan_music_folder(path: String, use_parallelism: bool) -> Result<Vec<MusicTrack>, String> {
    println!("Starting scan for: {} (Parallel: {})", path, use_parallelism);
    let start_time = Instant::now();

    let tracks: Vec<MusicTrack> = if use_parallelism {
        // Use jwalk for parallel scanning
        let entries: Vec<_> = jwalk::WalkDir::new(&path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path())
            .collect();

        println!("Found {} files (jwalk). Processing metadata...", entries.len());

        // parallel processing
        entries
            .into_par_iter()
            .filter(|path| is_audio_file(path))
            .filter_map(|path| parse_metadata(&path))
            .collect()
    } else {
        // Use walkdir for sequential scanning        
        let entries: Vec<_> = WalkDir::new(&path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path().to_owned())
            .collect();

        println!("Found {} files (walkdir). Processing metadata...", entries.len());

        // sequential processing
        entries
            .into_iter()
            .filter(|path| is_audio_file(path))
            .filter_map(|path| parse_metadata(&path))
            .collect()
    };

    println!("Scanned {} tracks in {:?}", tracks.len(), start_time.elapsed());
    
    Ok(tracks)
}

// Get cover art
#[tauri::command]
async fn get_track_cover(path: String) -> Result<Option<String>, String> {
    let path_buf = PathBuf::from(path);

    // Run in separate thread to avoid blocking
    let result = std::thread::spawn(move || {
        // open and read file
        let tagged_file = Probe::open(&path_buf).ok()?.read().ok()?;
        // get tag and picture
        let picture = tagged_file.primary_tag().or_else(|| tagged_file.first_tag()).and_then(|tag| tag.pictures().first())?;

        let b64 = general_purpose::STANDARD.encode(picture.data());
        let mime_str = match picture.mime_type() {
            Some(MimeType::Png) => "image/png",
            Some(MimeType::Jpeg) => "image/jpeg",
            Some(MimeType::Tiff) => "image/tiff",
            Some(MimeType::Bmp) => "image/bmp",
            Some(MimeType::Gif) => "image/gif",
            _ => "image/jpeg"
        };
        Some(format!("data:{};base64,{}", mime_str, b64))
    }).join().map_err(|_| "Thread panicked".to_string())?;

    Ok(result)
}
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![scan_music_folder, get_track_cover])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}