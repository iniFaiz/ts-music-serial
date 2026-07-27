//! Resource and payload limits enforced at native trust boundaries.
//!
//! The webview and remote services are treated as untrusted inputs. Keeping the
//! limits in one module prevents individual commands/providers from silently
//! drifting to unbounded reads or decodes.

use std::io::Cursor;

use image::{DynamicImage, ImageReader, Limits};
use serde::de::DeserializeOwned;
use serde_json::Value;

pub(crate) const MAX_COVER_BYTES: usize = 12 * 1024 * 1024;
pub(crate) const MAX_NETWORK_JSON_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_M3U_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const MAX_M3U_LINES: usize = 20_000;
pub(crate) const MAX_PATH_BYTES: usize = 32_768;
pub(crate) const MAX_QUEUE_ENTRIES: usize = 10_000;
pub(crate) const MAX_BATCH_PATHS: usize = 20_000;
pub(crate) const MAX_KV_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_RULES_BYTES: usize = 64 * 1024;
pub(crate) const MAX_JSON_DEPTH: usize = 16;

const MAX_IMAGE_DIMENSION: u32 = 8_192;
const MAX_IMAGE_PIXELS: u64 = 40_000_000;
const MAX_IMAGE_ALLOC_BYTES: u64 = 192 * 1024 * 1024;

pub(crate) fn validate_text(value: &str, field: &str, max_bytes: usize) -> Result<(), String> {
    if value.len() > max_bytes {
        return Err(format!("{field} is too long (max {max_bytes} bytes)"));
    }
    Ok(())
}

pub(crate) fn validate_paths(paths: &[String], max_items: usize) -> Result<(), String> {
    if paths.len() > max_items {
        return Err(format!("Too many paths (max {max_items})"));
    }
    for path in paths {
        validate_text(path, "Path", MAX_PATH_BYTES)?;
    }
    Ok(())
}

fn json_depth(value: &Value) -> usize {
    match value {
        Value::Array(values) => 1 + values.iter().map(json_depth).max().unwrap_or(0),
        Value::Object(values) => 1 + values.values().map(json_depth).max().unwrap_or(0),
        _ => 1,
    }
}

pub(crate) fn validate_json(
    value: &Value,
    field: &str,
    max_bytes: usize,
    max_depth: usize,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|error| format!("Invalid {field}: {error}"))?;
    if bytes.len() > max_bytes {
        return Err(format!("{field} is too large (max {max_bytes} bytes)"));
    }
    if json_depth(value) > max_depth {
        return Err(format!(
            "{field} is nested too deeply (max depth {max_depth})"
        ));
    }
    Ok(())
}

pub(crate) fn decode_image_limited(data: &[u8]) -> Result<DynamicImage, String> {
    if data.len() > MAX_COVER_BYTES {
        return Err(format!(
            "Image is too large (max {} MB)",
            MAX_COVER_BYTES / 1024 / 1024
        ));
    }

    let mut reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|_| "Not a valid image file".to_string())?;
    let mut decode_limits = Limits::default();
    decode_limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    decode_limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    decode_limits.max_alloc = Some(MAX_IMAGE_ALLOC_BYTES);
    reader.limits(decode_limits);
    let image = reader
        .decode()
        .map_err(|_| "Image is invalid or exceeds safe decode limits".to_string())?;
    let pixels = u64::from(image.width())
        .checked_mul(u64::from(image.height()))
        .ok_or_else(|| "Image dimensions overflow".to_string())?;
    if pixels > MAX_IMAGE_PIXELS {
        return Err(format!(
            "Image contains too many pixels (max {MAX_IMAGE_PIXELS})"
        ));
    }
    Ok(image)
}

pub(crate) async fn response_bytes_limited(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut response = response
        .error_for_status()
        .map_err(|error| error.to_string())?;
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(format!("Remote response exceeds {max_bytes} bytes"));
    }

    let capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(0)
        .min(max_bytes);
    let mut body = Vec::with_capacity(capacity);
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        let new_len = body
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| "Remote response size overflow".to_string())?;
        if new_len > max_bytes {
            return Err(format!("Remote response exceeds {max_bytes} bytes"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(crate) async fn response_json_limited<T: DeserializeOwned>(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<T, String> {
    let body = response_bytes_limited(response, max_bytes).await?;
    serde_json::from_slice(&body).map_err(|error| format!("Invalid remote JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_and_deep_json() {
        let oversized = Value::String("x".repeat(128));
        assert!(validate_json(&oversized, "payload", 16, 16).is_err());

        let deep = serde_json::json!({"a":{"b":{"c":{"d":1}}}});
        assert!(validate_json(&deep, "payload", 1024, 3).is_err());
        assert!(validate_json(&deep, "payload", 1024, 5).is_ok());
    }

    #[test]
    fn validates_path_count_and_length() {
        assert!(validate_paths(&["C:/Music/song.flac".to_string()], 1).is_ok());
        assert!(validate_paths(&["a".to_string(), "b".to_string()], 1).is_err());
        assert!(validate_paths(&["x".repeat(MAX_PATH_BYTES + 1)], 1).is_err());
    }
}
