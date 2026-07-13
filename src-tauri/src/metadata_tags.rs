//! Safe audio tag editing and cover-art import commands.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose, Engine as _};
use lofty::picture::MimeType;
use lofty::prelude::*;
use lofty::probe::Probe;
use serde::Deserialize;
use tauri::{AppHandle, State};

use crate::cover_cache::make_thumbnail;
use crate::{compute_fingerprint, db, is_allowed_audio, parse_metadata, MusicTrack};

// ---------------------------------------------------------------------------
// Tag editor: write metadata (and cover art) back into the audio file with
// lofty, then re-index the row in SQLite. Playback never holds the file open
// (build_decoder reads the whole file into memory), so editing the currently
// playing track is safe.

// Full form state from the edit modal. Empty string / None = remove that tag.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagEdits {
    title: String,
    artist: String,
    album: String,
    genre: String,
    year: Option<u32>,
    track_number: Option<u32>,
}

// Load new cover-art bytes: must decode as a real image (this is also what
// keeps the command from being abused as an arbitrary-file-embed primitive),
// and formats without broad tag support (webp/bmp/…) are re-encoded to JPEG.
fn load_cover_art(path: &Path) -> Result<(Vec<u8>, MimeType), String> {
    const MAX_COVER_BYTES: u64 = 20 * 1024 * 1024;
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_COVER_BYTES {
        return Err("Cover image is too large (max 20 MB)".to_string());
    }
    let data = fs::read(path).map_err(|e| e.to_string())?;
    let format = image::guess_format(&data).map_err(|_| "Not a valid image file".to_string())?;
    // Validate that the bytes really decode before embedding them.
    let img = image::load_from_memory(&data).map_err(|_| "Not a valid image file".to_string())?;
    match format {
        image::ImageFormat::Jpeg => Ok((data, MimeType::Jpeg)),
        image::ImageFormat::Png => Ok((data, MimeType::Png)),
        _ => {
            let mut buf = Vec::new();
            image::DynamicImage::ImageRgb8(img.to_rgb8())
                .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Jpeg)
                .map_err(|e| format!("Failed to convert cover: {e}"))?;
            Ok((buf, MimeType::Jpeg))
        }
    }
}

#[tauri::command]
pub(crate) async fn write_track_tags(
    app: AppHandle,
    db: State<'_, db::Db>,
    path: String,
    edits: TagEdits,
    cover_path: Option<String>,
    remove_cover: bool,
) -> Result<MusicTrack, String> {
    use lofty::config::WriteOptions;
    use lofty::picture::{Picture, PictureType};
    use lofty::tag::Tag;

    let path_buf = PathBuf::from(&path);
    if !is_allowed_audio(&app, &path_buf) {
        return Err("Path is not within an allowed music folder".to_string());
    }

    let (track, fingerprint) = tauri::async_runtime::spawn_blocking(
        move || -> Result<(MusicTrack, Option<String>), String> {
            let new_cover = match cover_path.as_deref() {
                Some(p) => Some(load_cover_art(Path::new(p))?),
                None => None,
            };

            let mut tagged_file = Probe::open(&path_buf)
                .map_err(|e| e.to_string())?
                .read()
                .map_err(|e| e.to_string())?;
            if tagged_file.primary_tag_mut().is_none() {
                let tag_type = tagged_file.primary_tag_type();
                tagged_file.insert_tag(Tag::new(tag_type));
            }
            let tag = tagged_file
                .primary_tag_mut()
                .ok_or("File format does not support tags")?;

            let title = edits.title.trim();
            if title.is_empty() {
                tag.remove_title();
            } else {
                tag.set_title(title.to_string());
            }
            let artist = edits.artist.trim();
            if artist.is_empty() {
                tag.remove_artist();
            } else {
                tag.set_artist(artist.to_string());
            }
            let album = edits.album.trim();
            if album.is_empty() {
                tag.remove_album();
            } else {
                tag.set_album(album.to_string());
            }
            let genre = edits.genre.trim();
            if genre.is_empty() {
                tag.remove_genre();
            } else {
                tag.set_genre(genre.to_string());
            }
            match edits.year {
                Some(y) => tag.set_year(y),
                None => tag.remove_year(),
            }
            match edits.track_number {
                Some(n) => tag.set_track(n),
                None => tag.remove_track(),
            }

            if remove_cover || new_cover.is_some() {
                while !tag.pictures().is_empty() {
                    tag.remove_picture(0);
                }
            }
            if let Some((data, mime)) = new_cover {
                tag.push_picture(Picture::new_unchecked(
                    PictureType::CoverFront,
                    Some(mime),
                    None,
                    data,
                ));
            }

            tagged_file
                .save_to_path(&path_buf, WriteOptions::default())
                .map_err(|e| format!("Failed to write tags: {e}"))?;

            let track =
                parse_metadata(&path_buf).ok_or("Failed to re-read file after writing tags")?;
            // Content changed (and so did size/mtime) — refresh the fingerprint
            // so moved-file detection keeps recognizing this file.
            let fp = compute_fingerprint(&path_buf);
            Ok((track, fp))
        },
    )
    .await
    .map_err(|e| format!("Tag write task failed: {e}"))??;

    db::reindex_track(&db, &track, fingerprint.as_deref())?;
    Ok(track)
}

// Small thumbnail preview (base64 data URL) of an image the user picked as new
// cover art, so the edit modal can show it before saving. Same validation as
// load_cover_art: the bytes must decode as an image.
#[tauri::command]
pub(crate) async fn preview_image(path: String) -> Result<String, String> {
    let thumb = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let (data, _) = load_cover_art(Path::new(&path))?;
        make_thumbnail(&data).ok_or_else(|| "Failed to decode image".to_string())
    })
    .await
    .map_err(|e| format!("Preview task failed: {e}"))??;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        general_purpose::STANDARD.encode(thumb)
    ))
}
