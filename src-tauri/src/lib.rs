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

// Data structure sent to Frontend
#[derive(Serialize, Clone, Debug)]
struct MusicTrack {
    path: String,
    title: String,
    artist: String,
    album: String,
    duration_secs: u64,
    date_added: u64, 
}

// Filter supported audio extensions
fn is_audio_file(path: &Path) -> bool {
    match path.extension() {
        Some(ext) => {
            let ext_str = ext.to_string_lossy().to_lowercase();
            matches!(ext_str.as_str(), "mp3" | "flac" | "wav" | "m4a" | "ogg" | "aac")
        }
        None => false,
    }
}

// Extract metadata from a single file
fn parse_metadata(path: &Path) -> Option<MusicTrack> {
    let path_str = path.to_string_lossy().to_string();
    
    // 1. Get File Creation Time (OS Metadata)
    let date_added = fs::metadata(path)
        .and_then(|m| m.created().or(m.modified()))
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // 2. Parse Audio Tags (Lofty)
    // Probe the file to handle different formats transparently
    let tagged_file = match Probe::open(path) {
        Ok(probe) => match probe.read() {
            Ok(tf) => tf,
            Err(_) => return None,
        },
        Err(_) => return None,
    };

    let tag = tagged_file.primary_tag().or_else(|| tagged_file.first_tag());
    let properties = tagged_file.properties();

    // Extract fields with fallbacks
    let title = tag.and_then(|t| t.title().map(|s| s.to_string())).unwrap_or_else(|| 
        path.file_stem().unwrap_or_default().to_string_lossy().to_string()
    );
    let artist = tag.and_then(|t| t.artist().map(|s| s.to_string())).unwrap_or("Unknown Artist".to_string());
    let album = tag.and_then(|t| t.album().map(|s| s.to_string())).unwrap_or("Unknown Album".to_string());
    let duration_secs = properties.duration().as_secs();

    Some(MusicTrack {
        path: path_str,
        title,
        artist,
        album,
        duration_secs,
        date_added,
    })
}

// COMMAND: Scan a directory recursively
#[tauri::command]
async fn scan_music_folder(path: String) -> Result<Vec<MusicTrack>, String> {
    println!("Starting scan for: {}", path);
    let start_time = Instant::now();

    // 1. Collect paths (Synchronous IO)
    let walker = WalkDir::new(&path).into_iter();
    
    let entries: Vec<_> = walker
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_owned())
        .collect();

    println!("Found {} files. Processing metadata...", entries.len());

    // 2. Parse metadata in parallel (CPU Bound)
    let tracks: Vec<MusicTrack> = entries
        .par_iter()
        .filter(|path| is_audio_file(path))
        .filter_map(|path| parse_metadata(path))
        .collect();

    println!("Scanned {} tracks in {:?}", tracks.len(), start_time.elapsed());
    
    Ok(tracks)
}

// COMMAND: Extract cover art on demand (Lazy Loading)
#[tauri::command]
async fn get_track_cover(path: String) -> Result<Option<String>, String> {
    let path_buf = PathBuf::from(path);

    // Offload to thread to prevent blocking async runtime
    let result = std::thread::spawn(move || {
        if let Ok(probe) = Probe::open(&path_buf) {
            if let Ok(tagged_file) = probe.read() {
                if let Some(tag) = tagged_file.primary_tag().or_else(|| tagged_file.first_tag()) {
                    if let Some(picture) = tag.pictures().first() {
                        // Convert to Base64 Data URI
                        let b64 = general_purpose::STANDARD.encode(picture.data());
                        let mime_str = match picture.mime_type() {
                            Some(MimeType::Png) => "image/png",
                            Some(MimeType::Jpeg) => "image/jpeg",
                            Some(MimeType::Tiff) => "image/tiff",
                            Some(MimeType::Bmp) => "image/bmp",
                            Some(MimeType::Gif) => "image/gif",
                            _ => "image/jpeg"
                        };
                        return Some(format!("data:{};base64,{}", mime_str, b64));
                    }
                }
            }
        }
        None
    }).join().map_err(|_| "Thread panicked".to_string())?;

    Ok(result)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![scan_music_folder, get_track_cover])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}