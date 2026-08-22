//! Native waveform analysis and its on-disk cache.

use std::path::{Path, PathBuf};

use rodio::Source;
use tauri::AppHandle;

use crate::cache_manager::{self, CacheKind};
use crate::cover_cache::cover_cache_key;
use crate::{build_decoder, is_allowed_audio};

const WAVEFORM_BUCKETS: usize = 400;

// Decode a track and reduce it to WAVEFORM_BUCKETS peak amplitudes (max abs per
// window), normalized to the track's loudest peak and quantized to 0..255. Runs
// a full decode, so it is only ever called on the blocking pool and cached.
fn compute_waveform(path: &Path) -> Option<Vec<u8>> {
    let (decoder, dur_secs) = build_decoder(path).ok()?;
    let channels = f64::from(decoder.channels().get());
    let sample_rate = f64::from(decoder.sample_rate().get());
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
        // Unknown duration: reduce peaks on the fly instead of buffering every
        // sample first — a malformed/huge file could otherwise balloon to
        // gigabytes of RAM. Buckets start small and are repeatedly pair-folded
        // as they fill up, so memory stays O(WAVEFORM_BUCKETS) while short and
        // long tracks still spread evenly across the final bars.
        const START_BUCKET_SAMPLES: u64 = 4096;
        let mut bucket_samples = START_BUCKET_SAMPLES;
        let mut cur_peak = 0.0f32;
        let mut count: u64 = 0;
        for s in decoder {
            let a = s.abs();
            if a > cur_peak {
                cur_peak = a;
            }
            count += 1;
            if count >= bucket_samples {
                peaks.push(cur_peak);
                cur_peak = 0.0;
                count = 0;
                if peaks.len() >= WAVEFORM_BUCKETS {
                    for i in 0..WAVEFORM_BUCKETS / 2 {
                        peaks[i] = peaks[2 * i].max(peaks[2 * i + 1]);
                    }
                    peaks.truncate(WAVEFORM_BUCKETS / 2);
                    bucket_samples *= 2;
                }
            }
        }
        if count > 0 {
            peaks.push(cur_peak);
        }
        // Stretch whatever we collected back out to WAVEFORM_BUCKETS bars
        // (each source bucket feeds 1-2 adjacent bars via max).
        let collected = peaks.len();
        if collected > 0 && collected < WAVEFORM_BUCKETS {
            let stretched: Vec<f32> = (0..WAVEFORM_BUCKETS)
                .map(|i| {
                    let start = i * collected / WAVEFORM_BUCKETS;
                    let end = (((i + 1) * collected) / WAVEFORM_BUCKETS).clamp(start + 1, collected);
                    peaks[start..end].iter().copied().fold(0.0f32, f32::max)
                })
                .collect();
            peaks = stretched;
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
pub(crate) async fn get_waveform(app: AppHandle, path: String) -> Result<Option<Vec<u8>>, String> {
    let path_buf = PathBuf::from(&path);
    if !is_allowed_audio(&app, &path_buf) {
        return Err("Path is not within an allowed music folder".to_string());
    }

    let cache = cache_manager::manager(&app);

    let result = tauri::async_runtime::spawn_blocking(move || -> Option<Vec<u8>> {
        let key = cover_cache_key(&path_buf);

        // Fast path: cached waveform.
        if let (Some(cache), Some(k)) = (&cache, &key) {
            if let Some(bytes) = cache.read(CacheKind::Waveforms, &format!("{k}.wf")) {
                let text = String::from_utf8(bytes).ok()?;
                if let Ok(v) = serde_json::from_str::<Vec<u8>>(&text) {
                    if !v.is_empty() {
                        return Some(v);
                    }
                }
            }
        }

        let peaks = compute_waveform(&path_buf)?;
        if let (Some(cache), Some(k)) = (&cache, &key) {
            if let Ok(text) = serde_json::to_string(&peaks) {
                let _ = cache.write(
                    CacheKind::Waveforms,
                    &format!("{k}.wf"),
                    text.as_bytes(),
                    Some(&path_buf),
                );
            }
        }
        Some(peaks)
    })
    .await
    .map_err(|e| format!("Waveform task failed: {e}"))?;

    Ok(result)
}
