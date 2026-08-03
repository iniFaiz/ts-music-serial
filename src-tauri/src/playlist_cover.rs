//! Native playlist-cover selection, validation, resize, and compression.

use std::fs;
use std::io::Cursor;

use base64::{engine::general_purpose, Engine as _};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{GenericImageView, ImageFormat};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::limits;

const PLAYLIST_COVER_EDGE: u32 = 400;
const PLAYLIST_COVER_QUALITY: u8 = 85;
const MAX_PLAYLIST_COVER_DATA_URL_BYTES: usize = 2 * 1024 * 1024;
const JPEG_DATA_URL_PREFIX: &str = "data:image/jpeg;base64,";

fn encode_playlist_cover(bytes: &[u8]) -> Result<String, String> {
    let image = limits::decode_image_limited(bytes)?;
    let (width, height) = image.dimensions();
    let resized = if width > PLAYLIST_COVER_EDGE || height > PLAYLIST_COVER_EDGE {
        image.resize(
            PLAYLIST_COVER_EDGE,
            PLAYLIST_COVER_EDGE,
            FilterType::Lanczos3,
        )
    } else {
        image
    };
    let rgb = resized.to_rgb8();
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(Cursor::new(&mut jpeg), PLAYLIST_COVER_QUALITY)
        .encode_image(&rgb)
        .map_err(|error| format!("Failed to compress playlist cover: {error}"))?;
    let data_url = format!(
        "{JPEG_DATA_URL_PREFIX}{}",
        general_purpose::STANDARD.encode(jpeg)
    );
    if data_url.len() > MAX_PLAYLIST_COVER_DATA_URL_BYTES {
        return Err("Compressed playlist cover is too large".to_string());
    }
    Ok(data_url)
}

/// Revalidate the persisted representation at the database boundary. The
/// webview can invoke commands directly, so a data-URL prefix alone is not a
/// sufficient image check.
pub(crate) fn validate_playlist_cover_data_url(value: &str) -> Result<(), String> {
    limits::validate_text(value, "Playlist cover", MAX_PLAYLIST_COVER_DATA_URL_BYTES)?;
    let encoded = value
        .strip_prefix(JPEG_DATA_URL_PREFIX)
        .ok_or_else(|| "Playlist cover must be a native-processed JPEG".to_string())?;
    let bytes = general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "Playlist cover contains invalid base64".to_string())?;
    if image::guess_format(&bytes).ok() != Some(ImageFormat::Jpeg) {
        return Err("Playlist cover payload must be JPEG".to_string());
    }
    let image = limits::decode_image_limited(&bytes)?;
    if image.width() > PLAYLIST_COVER_EDGE || image.height() > PLAYLIST_COVER_EDGE {
        return Err(format!(
            "Playlist cover exceeds {PLAYLIST_COVER_EDGE}px after processing"
        ));
    }
    Ok(())
}

/// Open the trusted native picker and return only a bounded, canonical JPEG.
/// Raw image bytes and arbitrary filesystem paths never cross the IPC boundary.
#[tauri::command]
pub(crate) async fn pick_playlist_cover(app: AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(selected) = app
            .dialog()
            .file()
            .set_title("Choose playlist cover")
            .add_filter("Images", &["jpg", "jpeg", "png", "webp", "gif", "bmp"])
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let path = selected
            .into_path()
            .map_err(|error| format!("Image picker did not return a local path: {error}"))?;
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        if metadata.len() > limits::MAX_COVER_BYTES as u64 {
            return Err(format!(
                "Cover image is too large (max {} MB)",
                limits::MAX_COVER_BYTES / 1024 / 1024
            ));
        }
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        encode_playlist_cover(&bytes).map(Some)
    })
    .await
    .map_err(|error| format!("Playlist-cover task failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};

    #[test]
    fn cover_pipeline_downscales_and_returns_valid_jpeg() {
        let source =
            DynamicImage::ImageRgba8(ImageBuffer::from_pixel(1_200, 600, Rgba([20, 40, 80, 200])));
        let mut png = Vec::new();
        source
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .expect("encode source png");

        let result = encode_playlist_cover(&png).expect("process cover");
        validate_playlist_cover_data_url(&result).expect("validate canonical cover");
        let jpeg = general_purpose::STANDARD
            .decode(result.strip_prefix(JPEG_DATA_URL_PREFIX).unwrap())
            .expect("decode result");
        let decoded = image::load_from_memory(&jpeg).expect("decode jpeg");
        assert_eq!(decoded.dimensions(), (400, 200));
    }

    #[test]
    fn persisted_cover_validation_rejects_forged_payloads() {
        assert!(validate_playlist_cover_data_url("data:image/jpeg;base64,bm90LWltYWdl").is_err());
        assert!(validate_playlist_cover_data_url("data:image/png;base64,bm90LWltYWdl").is_err());

        let source = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(1, 1, Rgba([0, 0, 0, 255])));
        let mut png = Vec::new();
        source
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .expect("encode forged png");
        let forged = format!(
            "{JPEG_DATA_URL_PREFIX}{}",
            general_purpose::STANDARD.encode(png)
        );
        assert!(validate_playlist_cover_data_url(&forged).is_err());
    }
}
