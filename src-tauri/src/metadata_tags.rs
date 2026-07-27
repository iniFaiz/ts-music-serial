//! Safe audio tag editing and cover-art import commands.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose, Engine as _};
use lofty::picture::MimeType;
use lofty::prelude::*;
use lofty::probe::Probe;
use serde::Deserialize;
use tauri::Runtime;
use tauri::{AppHandle, State};

use crate::cover_cache::make_thumbnail;
use crate::{
    compute_fingerprint, db, is_allowed_path, limits, parse_metadata, resolve_allowed_audio,
    security, MusicTrack,
};

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
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > limits::MAX_COVER_BYTES as u64 {
        return Err(format!(
            "Cover image is too large (max {} MB)",
            limits::MAX_COVER_BYTES / 1024 / 1024
        ));
    }
    let data = fs::read(path).map_err(|e| e.to_string())?;
    let format = image::guess_format(&data).map_err(|_| "Not a valid image file".to_string())?;
    // Validate that the bytes really decode before embedding them.
    let img = limits::decode_image_limited(&data)?;
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

fn authorize_tag_target<R: Runtime>(app: &AppHandle<R>, path: &Path) -> Result<PathBuf, String> {
    resolve_allowed_audio(app, path)
}

fn authorize_cover_path<R: Runtime>(app: &AppHandle<R>, path: &Path) -> Result<(), String> {
    if is_allowed_path(app, path) {
        Ok(())
    } else {
        Err("Cover path was not authorized by the file picker".to_string())
    }
}

#[tauri::command]
pub(crate) async fn write_track_tags(
    app: AppHandle,
    db: State<'_, db::Db>,
    consent: State<'_, security::DestructiveConsentState>,
    path: String,
    edits: TagEdits,
    cover_path: Option<String>,
    remove_cover: bool,
    consent_token: String,
) -> Result<MusicTrack, String> {
    use lofty::config::WriteOptions;
    use lofty::picture::{Picture, PictureType};
    use lofty::tag::Tag;

    let path_buf = authorize_tag_target(&app, Path::new(&path))?;
    let canonical = path_buf.to_string_lossy().to_string();
    if db::tracks::db_track(db.clone(), canonical)?.is_none() {
        return Err("File is not an indexed library track".to_string());
    }
    limits::validate_text(&edits.title, "Title", 1_024)?;
    limits::validate_text(&edits.artist, "Artist", 1_024)?;
    limits::validate_text(&edits.album, "Album", 1_024)?;
    limits::validate_text(&edits.genre, "Genre", 256)?;
    if edits.year.is_some_and(|year| year > 9_999) {
        return Err("Year must be at most 9999".to_string());
    }
    if edits
        .track_number
        .is_some_and(|track_number| track_number == 0 || track_number > 9_999)
    {
        return Err("Track number must be between 1 and 9999".to_string());
    }
    if let Some(path) = cover_path.as_deref() {
        authorize_cover_path(&app, Path::new(path))?;
    }

    let new_cover = match cover_path.as_deref() {
        Some(path) => {
            let path = PathBuf::from(path);
            Some(
                tauri::async_runtime::spawn_blocking(move || load_cover_art(&path))
                    .await
                    .map_err(|error| format!("Cover validation task failed: {error}"))??,
            )
        }
        None => None,
    };
    consent.consume(
        &consent_token,
        security::ConsentAction::WriteTrackTags,
        Some(path_buf.to_string_lossy().as_ref()),
    )?;

    let (track, fingerprint) = tauri::async_runtime::spawn_blocking(
        move || -> Result<(MusicTrack, Option<String>), String> {
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
pub(crate) async fn preview_image(app: AppHandle, path: String) -> Result<String, String> {
    authorize_cover_path(&app, Path::new(&path))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
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
                .join(format!("ts-music-tag-auth-{}-{nonce}", std::process::id()));
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
    fn tag_and_cover_targets_require_their_own_scope_grants() {
        let app = tauri::test::mock_app();
        app.manage(crate::library_scan::LibraryAccessState::new());
        let dir = TestDir::new();
        let song = dir.join("song.flac");
        let cover = dir.join("cover.png");
        let sibling = dir.join("sibling.png");
        std::fs::write(&song, b"audio").expect("write song");
        std::fs::write(&cover, b"cover").expect("write cover");
        std::fs::write(&sibling, b"sibling").expect("write sibling");

        assert!(authorize_tag_target(app.handle(), &song).is_err());
        assert!(authorize_cover_path(app.handle(), &cover).is_err());

        app.asset_protocol_scope()
            .allow_file(&song)
            .expect("allow song");
        app.asset_protocol_scope()
            .allow_file(&cover)
            .expect("allow cover");

        // Dialog/asset scope alone is not write authority for audio files.
        assert!(authorize_tag_target(app.handle(), &song).is_err());
        crate::library_scan::grant_session_audio(app.handle(), &song)
            .expect("grant exact session audio");
        assert!(authorize_tag_target(app.handle(), &song).is_ok());
        assert!(authorize_cover_path(app.handle(), &cover).is_ok());
        assert!(authorize_cover_path(app.handle(), &sibling).is_err());
    }

    #[test]
    fn cover_loader_accepts_images_and_rejects_non_images() {
        let dir = TestDir::new();
        let image_path = dir.join("cover.png");
        let invalid_path = dir.join("not-an-image.png");
        image::RgbImage::from_pixel(2, 2, image::Rgb([10, 20, 30]))
            .save(&image_path)
            .expect("save test image");
        std::fs::write(&invalid_path, b"not an image").expect("write invalid image");

        let (bytes, mime) = load_cover_art(&image_path).expect("load valid cover");
        assert!(!bytes.is_empty());
        assert_eq!(mime, MimeType::Png);
        assert_eq!(
            load_cover_art(&invalid_path).expect_err("reject invalid image"),
            "Not a valid image file"
        );
    }

    #[test]
    fn cover_loader_rejects_files_over_native_limit_before_reading() {
        let dir = TestDir::new();
        let path = dir.join("huge.png");
        let file = std::fs::File::create(&path).expect("create sparse file");
        file.set_len(limits::MAX_COVER_BYTES as u64 + 1)
            .expect("resize sparse file");

        assert_eq!(
            load_cover_art(&path).expect_err("reject oversized cover"),
            format!(
                "Cover image is too large (max {} MB)",
                limits::MAX_COVER_BYTES / 1024 / 1024
            )
        );
    }
}
