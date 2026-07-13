use std::fs;
use std::io::Cursor;
use std::path::Path;

use rodio::{Decoder, Source};

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
