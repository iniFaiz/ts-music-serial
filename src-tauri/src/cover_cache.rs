//! Embedded cover extraction, thumbnails, palette generation, and disk cache.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use base64::{engine::general_purpose, Engine as _};
use lofty::picture::MimeType;
use lofty::prelude::*;
use lofty::probe::Probe;
use tauri::{AppHandle, Manager, State};

use crate::{db, is_allowed_audio, limits};

const THUMB_SIZE: u32 = 300;

// Directory where downscaled cover thumbnails are cached on disk.
pub(crate) fn cover_cache_dir(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_cache_dir().ok()?.join("covers");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

// Cache key derived from path + mtime + size, so the thumbnail is invalidated
// automatically if the underlying file changes.
pub(crate) fn cover_cache_key(path: &Path) -> Option<String> {
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
    if picture.data().len() > limits::MAX_COVER_BYTES {
        return None;
    }

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
pub(crate) fn make_thumbnail(data: &[u8]) -> Option<Vec<u8>> {
    let img = limits::decode_image_limited(data).ok()?;
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

    pxs.sort_by_key(|pixel| std::cmp::Reverse(pixel.sat));

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
pub(crate) async fn get_track_cover(
    app: AppHandle,
    db: State<'_, db::Db>,
    path: String,
) -> Result<Option<String>, String> {
    let path_buf = PathBuf::from(&path);
    if !is_allowed_audio(&app, &path_buf) {
        return Err("Path is not within an allowed music folder".to_string());
    }

    let cache = cover_cache_dir(&app);

    // 1. Try to get it from the database first
    if let Some((_, _, bytes)) = db::db_get_cover_art(&db, &path) {
        if let Some(thumb) = make_thumbnail(&bytes) {
            let b64 = general_purpose::STANDARD.encode(thumb);
            return Ok(Some(format!("data:image/jpeg;base64,{b64}")));
        }
    }

    // 2. If it is NOT in the database, and the file exists on disk:
    if path_buf.exists() {
        let p_buf = path_buf.clone();
        let result = tauri::async_runtime::spawn_blocking(move || -> Option<String> {
            let key = cover_cache_key(&p_buf);

            // Fast path: serve a previously cached thumbnail.
            if let (Some(dir), Some(k)) = (&cache, &key) {
                if let Ok(bytes) = fs::read(dir.join(format!("{k}.jpg"))) {
                    let b64 = general_purpose::STANDARD.encode(&bytes);
                    return Some(format!("data:image/jpeg;base64,{b64}"));
                }
            }

            let (raw, _) = extract_cover(&p_buf)?;
            let thumb = make_thumbnail(&raw)?;
            if let (Some(dir), Some(k)) = (&cache, &key) {
                let _ = fs::write(dir.join(format!("{k}.jpg")), &thumb);
            }
            let b64 = general_purpose::STANDARD.encode(&thumb);
            Some(format!("data:image/jpeg;base64,{b64}"))
        })
        .await
        .map_err(|e| format!("Cover task failed: {e}"))?;

        // 3. Since we generated the thumbnail/cover, save it to the DB if we can find its album/artist
        if let Some(ref b64_str) = result {
            if let Some(comma_idx) = b64_str.find(',') {
                if let Ok(bytes) = general_purpose::STANDARD.decode(&b64_str[comma_idx + 1..]) {
                    if let Ok(tagged_file) = lofty::read_from_path(&path_buf) {
                        if let Some(tag) = tagged_file
                            .primary_tag()
                            .or_else(|| tagged_file.first_tag())
                        {
                            let album = tag.album().map(|v| v.to_string()).unwrap_or_default();
                            let artist = tag.artist().map(|v| v.to_string()).unwrap_or_default();
                            if !album.is_empty() || !artist.is_empty() {
                                let _ = db::db_save_cover_art(&db, &album, &artist, &bytes);
                            }
                        }
                    }
                }
            }
        }

        return Ok(result);
    }

    Ok(None)
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
pub(crate) async fn get_track_cover_path(
    app: AppHandle,
    db: State<'_, db::Db>,
    path: String,
) -> Result<Option<String>, String> {
    let path_buf = PathBuf::from(&path);
    if !is_allowed_audio(&app, &path_buf) {
        return Err("Path is not within an allowed music folder".to_string());
    }

    let cache = cover_cache_dir(&app);

    // 1. Try to get it from the database first
    if let Some((album, artist, bytes)) = db::db_get_cover_art(&db, &path) {
        let result = tauri::async_runtime::spawn_blocking(move || -> Option<String> {
            let dir = cache?;
            let thumb = make_thumbnail(&bytes)?;
            let mut hasher = DefaultHasher::new();
            album.hash(&mut hasher);
            artist.hash(&mut hasher);
            let key = format!("db_{:016x}", hasher.finish());
            let file = dir.join(format!("{key}.jpg"));
            if !file.exists() {
                fs::write(&file, thumb).ok()?;
            }
            Some(file.to_string_lossy().into_owned())
        })
        .await
        .map_err(|e| format!("Cover task failed: {e}"))?;

        return Ok(result);
    }

    // 2. If it is NOT in the database, and the file exists on disk:
    if path_buf.exists() {
        let p_buf = path_buf.clone();
        let result = tauri::async_runtime::spawn_blocking(move || -> Option<String> {
            let dir = cache?;
            let key = cover_cache_key(&p_buf)?;
            let file = dir.join(format!("{key}.jpg"));

            // Fast path: thumbnail already cached on disk.
            if file.exists() {
                return Some(file.to_string_lossy().into_owned());
            }

            // Decode → downscale → cache a JPEG thumbnail, then hand back its path.
            let (raw, _mime) = extract_cover(&p_buf)?;
            let thumb = make_thumbnail(&raw)?;
            fs::write(&file, &thumb).ok()?;
            Some(file.to_string_lossy().into_owned())
        })
        .await
        .map_err(|e| format!("Cover task failed: {e}"))?;

        // 3. Since we generated the thumbnail/cover, save it to the DB if we can find its album/artist
        if let Some(ref thumb_path_str) = result {
            if let Ok(thumb_bytes) = fs::read(thumb_path_str) {
                if let Ok(tagged_file) = lofty::read_from_path(&path_buf) {
                    if let Some(tag) = tagged_file
                        .primary_tag()
                        .or_else(|| tagged_file.first_tag())
                    {
                        let album = tag.album().map(|v| v.to_string()).unwrap_or_default();
                        let artist = tag.artist().map(|v| v.to_string()).unwrap_or_default();
                        if !album.is_empty() || !artist.is_empty() {
                            let _ = db::db_save_cover_art(&db, &album, &artist, &thumb_bytes);
                        }
                    }
                }
            }
        }

        return Ok(result);
    }

    Ok(None)
}

// Return the 3-color gradient palette for a track's cover art, computed natively
// (see extract_palette_from_image) instead of decoding the cover a second time in
// the webview canvas. The result is cached on disk next to the thumbnail as a
// `{key}.pal` sidecar; the key embeds mtime+size, so it self-invalidates exactly
// like the thumbnail. Returns None only when the file has no decodable cover.
#[tauri::command]
pub(crate) async fn get_track_palette(
    app: AppHandle,
    path: String,
) -> Result<Option<Vec<String>>, String> {
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
                limits::decode_image_limited(&bytes).ok()?
            }
            _ => {
                let (raw, _mime) = extract_cover(&path_buf)?;
                limits::decode_image_limited(&raw).ok()?
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
