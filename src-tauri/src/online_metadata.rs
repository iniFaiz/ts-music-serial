//! Opt-in MusicBrainz / AcoustID metadata completion.
//!
//! The importer deliberately has "fill blanks only" semantics. It reads the
//! real tags immediately before writing and never replaces title, artist,
//! album, genre, year, track number, or artwork that the user already has.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use base64::{engine::general_purpose, Engine as _};
use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::Tag;
use reqwest::Client;
use rodio::Source;
use rusty_chromaprint::{Configuration, FingerprintCompressor, Fingerprinter};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use crate::{build_decoder, compute_fingerprint, db, is_allowed_audio, parse_metadata, MusicTrack};

static RUN_ID: AtomicU64 = AtomicU64::new(0);
const USER_AGENT: &str = "ts-music/0.1.0 (https://github.com/iniFaiz/ts-music-serial)";
// AcoustID application client key for ts-music. This identifies the application
// and is not a user's submission key or account password.
const ACOUSTID_CLIENT_KEY: &str = "MOoO4hWSvE";

#[derive(Clone, Default)]
struct MissingFields {
    title: bool,
    artist: bool,
    album: bool,
    genre: bool,
    year: bool,
    track_number: bool,
    cover: bool,
}

impl MissingFields {
    fn any(&self) -> bool {
        self.title
            || self.artist
            || self.album
            || self.genre
            || self.year
            || self.track_number
            || self.cover
    }

    fn needs_identity(&self) -> bool {
        self.title || self.artist
    }
}

#[derive(Clone, Default)]
struct FileInfo {
    missing: MissingFields,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    duration_secs: u64,
    filename_title: String,
    filename_artist: Option<String>,
    filename_album: Option<String>,
}

#[derive(Clone, Default, Debug)]
struct Match {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    genre: Option<String>,
    year: Option<u32>,
    track_number: Option<u32>,
    release_id: Option<String>,
    release_group_id: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
    processed: usize,
    total: usize,
    updated: usize,
    not_found: usize,
    failed: usize,
    current_path: Option<String>,
    done: bool,
    cancelled: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    scanned: usize,
    matched: usize,
    updated: usize,
    not_found: usize,
    failed: usize,
    cancelled: bool,
    tracks: Vec<MusicTrack>,
}

fn nonblank(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

// Turn common "01 - Artist - Title" filenames into useful search hints. These
// hints are never written directly; MusicBrainz still has to produce a strong
// duration-aware match.
fn filename_hints(path: &Path) -> (Option<String>, String) {
    let raw = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .trim()
        .to_string();
    let mut cleaned = raw.as_str();
    let prefix_len = cleaned
        .char_indices()
        .take_while(|(_, c)| {
            c.is_ascii_digit() || c.is_whitespace() || matches!(c, '.' | '_' | '-')
        })
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    if cleaned[..prefix_len].chars().any(|c| c.is_ascii_digit()) {
        cleaned = cleaned[prefix_len..].trim();
    }
    for separator in [" - ", " – ", " — "] {
        if let Some((artist, title)) = cleaned.split_once(separator) {
            if !artist.trim().is_empty() && !title.trim().is_empty() {
                return (Some(artist.trim().to_string()), title.trim().to_string());
            }
        }
    }
    (None, cleaned.replace('_', " ").trim().to_string())
}

fn useful_folder_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().trim().to_string();
    let generic = [
        "music",
        "audio",
        "downloads",
        "download",
        "songs",
        "unknown album",
    ];
    (!name.is_empty() && !generic.contains(&name.to_lowercase().as_str())).then_some(name)
}

fn inspect_file(path: &Path) -> Result<FileInfo, String> {
    let tagged = Probe::open(path)
        .map_err(|e| e.to_string())?
        .read()
        .map_err(|e| e.to_string())?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let title = tag.and_then(|t| nonblank(t.title().as_deref()));
    let artist = tag.and_then(|t| nonblank(t.artist().as_deref()));
    let album = tag.and_then(|t| nonblank(t.album().as_deref()));
    let genre = tag.and_then(|t| nonblank(t.genre().as_deref()));
    let (mut filename_artist, filename_title) = filename_hints(path);
    let filename_album = path.parent().and_then(useful_folder_name);
    if filename_artist.is_none() && filename_album.is_some() {
        filename_artist = path
            .parent()
            .and_then(Path::parent)
            .and_then(useful_folder_name);
    }
    let missing = MissingFields {
        title: title.is_none(),
        artist: artist.is_none(),
        album: album.is_none(),
        genre: genre.is_none(),
        year: tag.and_then(|t| t.year()).is_none(),
        track_number: tag.and_then(|t| t.track()).is_none(),
        cover: tag.map_or(true, |t| t.pictures().is_empty()),
    };
    Ok(FileInfo {
        missing,
        title,
        artist,
        album,
        duration_secs: tagged.properties().duration().as_secs(),
        filename_title,
        filename_artist,
        filename_album,
    })
}

fn json_string(value: Option<&Value>) -> Option<String> {
    nonblank(value.and_then(Value::as_str))
}

fn artist_name(recording: &Value) -> Option<String> {
    let credits = recording.get("artist-credit")?.as_array()?;
    let joined = credits
        .iter()
        .filter_map(|credit| {
            json_string(credit.get("name"))
                .or_else(|| json_string(credit.get("artist").and_then(|a| a.get("name"))))
        })
        .collect::<Vec<_>>()
        .join(", ");
    nonblank(Some(&joined))
}

fn first_genre(recording: &Value) -> Option<String> {
    recording
        .get("genres")
        .and_then(Value::as_array)
        .and_then(|v| v.first())
        .and_then(|v| json_string(v.get("name")))
        .or_else(|| {
            recording
                .get("tags")
                .and_then(Value::as_array)
                .and_then(|tags| {
                    tags.iter().max_by_key(|tag| {
                        tag.get("count").and_then(Value::as_i64).unwrap_or_default()
                    })
                })
                .and_then(|v| json_string(v.get("name")))
        })
}

fn year_from_date(date: Option<&str>) -> Option<u32> {
    date?.get(..4)?.parse::<u32>().ok()
}

fn normalized(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn choose_release<'a>(recording: &'a Value, album_hint: Option<&str>) -> Option<&'a Value> {
    let releases = recording.get("releases")?.as_array()?;
    if let Some(hint) = album_hint {
        let wanted = normalized(hint);
        if let Some(exact) = releases.iter().find(|release| {
            release
                .get("title")
                .and_then(Value::as_str)
                .map(normalized)
                .as_deref()
                == Some(wanted.as_str())
        }) {
            return Some(exact);
        }
    }
    releases.first()
}

fn track_position(release: &Value, recording_id: Option<&str>) -> Option<u32> {
    for media in release.get("media")?.as_array()? {
        for track in media.get("tracks")?.as_array()? {
            let same_recording = recording_id.is_none_or(|id| {
                track
                    .get("recording")
                    .and_then(|r| r.get("id"))
                    .and_then(Value::as_str)
                    == Some(id)
            });
            if same_recording {
                return track
                    .get("position")
                    .and_then(Value::as_u64)
                    .map(|v| v as u32)
                    .or_else(|| {
                        track
                            .get("number")
                            .and_then(Value::as_str)
                            .and_then(|v| v.parse().ok())
                    });
            }
        }
    }
    None
}

fn match_from_recording(recording: &Value, album_hint: Option<&str>) -> Match {
    let release = choose_release(recording, album_hint);
    let recording_id = json_string(recording.get("id"));
    Match {
        title: json_string(recording.get("title")),
        artist: artist_name(recording),
        album: release.and_then(|r| json_string(r.get("title"))),
        genre: first_genre(recording),
        year: release
            .and_then(|r| r.get("date"))
            .and_then(Value::as_str)
            .and_then(|d| year_from_date(Some(d)))
            .or_else(|| {
                recording
                    .get("first-release-date")
                    .and_then(Value::as_str)
                    .and_then(|d| year_from_date(Some(d)))
            }),
        track_number: release.and_then(|r| track_position(r, recording_id.as_deref())),
        release_id: release.and_then(|r| json_string(r.get("id"))),
        release_group_id: release.and_then(|r| {
            r.get("release-group")
                .and_then(|g| json_string(g.get("id")))
        }),
    }
}

struct MusicBrainzLimiter {
    last_request: Option<Instant>,
}

impl MusicBrainzLimiter {
    async fn wait(&mut self) {
        if let Some(last) = self.last_request {
            let elapsed = last.elapsed();
            let minimum = Duration::from_millis(1100);
            if elapsed < minimum {
                tokio_sleep(minimum - elapsed).await;
            }
        }
        self.last_request = Some(Instant::now());
    }
}

async fn tokio_sleep(duration: Duration) {
    // Tauri re-exports the active Tokio runtime without requiring a direct Tokio
    // dependency in this crate.
    tauri::async_runtime::spawn_blocking(move || std::thread::sleep(duration))
        .await
        .ok();
}

fn safe_query_value(value: &str) -> String {
    value.replace(['"', '\\'], " ").trim().to_string()
}

async fn musicbrainz_search(
    client: &Client,
    limiter: &mut MusicBrainzLimiter,
    info: &FileInfo,
) -> Result<Option<Match>, String> {
    let title = info.title.as_deref().unwrap_or(&info.filename_title);
    if title.trim().is_empty() {
        return Ok(None);
    }
    let artist = info.artist.as_deref().or(info.filename_artist.as_deref());
    let album = info.album.as_deref().or(info.filename_album.as_deref());
    let mut parts = vec![format!("recording:\"{}\"", safe_query_value(title))];
    if let Some(a) = artist.filter(|a| !a.trim().is_empty()) {
        parts.push(format!("artist:\"{}\"", safe_query_value(a)));
    }
    if let Some(a) = album.filter(|a| !a.trim().is_empty()) {
        parts.push(format!("release:\"{}\"", safe_query_value(a)));
    }

    limiter.wait().await;
    let response = client
        .get("https://musicbrainz.org/ws/2/recording")
        .query(&[
            ("query", parts.join(" AND ")),
            ("fmt", "json".to_string()),
            ("limit", "15".to_string()),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let value: Value = response.json().await.map_err(|e| e.to_string())?;
    let recordings = value
        .get("recordings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let using_filename_only = info.title.is_none() && info.artist.is_none();
    let minimum_score = if using_filename_only { 95 } else { 90 };
    let wanted_title = normalized(title);
    let local_ms = info.duration_secs * 1000;
    let best = recordings
        .into_iter()
        .filter_map(|recording| {
            let score = recording.get("score").and_then(Value::as_i64).unwrap_or(0);
            if score < minimum_score {
                return None;
            }
            let found_title = recording
                .get("title")
                .and_then(Value::as_str)
                .map(normalized)
                .unwrap_or_default();
            if found_title != wanted_title {
                return None;
            }
            let remote_ms = recording.get("length").and_then(Value::as_u64);
            let delta = match (local_ms, remote_ms) {
                (0, _) => 0,
                (_, Some(ms)) => (ms as i64 - local_ms as i64).unsigned_abs(),
                (_, None) => return None,
            };
            (delta <= 8_000).then_some((delta, recording))
        })
        .min_by_key(|(delta, _)| *delta)
        .map(|(_, recording)| recording);
    Ok(best.map(|r| match_from_recording(&r, album)))
}

fn acoustid_fingerprint(path: &Path) -> Result<(String, u64), String> {
    let (decoder, duration) = build_decoder(path)?;
    let sample_rate = decoder.sample_rate();
    let channels = decoder.channels() as u32;
    let max_samples = sample_rate as usize * channels as usize * 120;
    let config = Configuration::default();
    let mut fingerprinter = Fingerprinter::new(&config);
    fingerprinter
        .start(sample_rate, channels)
        .map_err(|e| e.to_string())?;
    let mut buffer = Vec::with_capacity(16_384);
    for sample in decoder.take(max_samples) {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        buffer.push(value);
        if buffer.len() >= 16_384 {
            fingerprinter.consume(&buffer);
            buffer.clear();
        }
    }
    if !buffer.is_empty() {
        fingerprinter.consume(&buffer);
    }
    fingerprinter.finish();
    if fingerprinter.fingerprint().is_empty() {
        return Err("Audio was too short to fingerprint".to_string());
    }
    let compressed = FingerprintCompressor::from(&config).compress(fingerprinter.fingerprint());
    Ok((
        general_purpose::URL_SAFE_NO_PAD.encode(compressed),
        duration.round() as u64,
    ))
}

async fn acoustid_lookup(
    client: &Client,
    path: PathBuf,
    key: &str,
) -> Result<Option<String>, String> {
    let (fingerprint, duration) =
        tauri::async_runtime::spawn_blocking(move || acoustid_fingerprint(&path))
            .await
            .map_err(|e| e.to_string())??;
    let response = client
        .get("https://api.acoustid.org/v2/lookup")
        .query(&[
            ("client", key),
            ("duration", &duration.to_string()),
            ("fingerprint", &fingerprint),
            ("meta", "recordings"),
            ("format", "json"),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let value: Value = response.json().await.map_err(|e| e.to_string())?;
    if value.get("status").and_then(Value::as_str) != Some("ok") {
        return Err(value
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("AcoustID rejected the lookup")
            .to_string());
    }
    let result = value
        .get("results")
        .and_then(Value::as_array)
        .and_then(|results| {
            results
                .iter()
                .find(|r| r.get("score").and_then(Value::as_f64).unwrap_or(0.0) >= 0.75)
        });
    Ok(result
        .and_then(|r| r.get("recordings"))
        .and_then(Value::as_array)
        .and_then(|v| v.first())
        .and_then(|r| json_string(r.get("id"))))
}

async fn musicbrainz_recording(
    client: &Client,
    limiter: &mut MusicBrainzLimiter,
    recording_id: &str,
    album_hint: Option<&str>,
) -> Result<Option<Match>, String> {
    limiter.wait().await;
    let response = client
        .get(format!(
            "https://musicbrainz.org/ws/2/recording/{recording_id}"
        ))
        .query(&[("inc", "artists+releases+genres"), ("fmt", "json")])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let value: Value = response.json().await.map_err(|e| e.to_string())?;
    Ok(Some(match_from_recording(&value, album_hint)))
}

async fn download_cover(client: &Client, matched: &Match) -> Option<Vec<u8>> {
    let url = if let Some(id) = matched.release_id.as_deref() {
        format!("https://coverartarchive.org/release/{id}/front-500")
    } else if let Some(id) = matched.release_group_id.as_deref() {
        format!("https://coverartarchive.org/release-group/{id}/front-500")
    } else {
        return None;
    };
    let response = client.get(url).send().await.ok()?.error_for_status().ok()?;
    if response
        .content_length()
        .is_some_and(|n| n > 12 * 1024 * 1024)
    {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    (bytes.len() <= 12 * 1024 * 1024).then(|| bytes.to_vec())
}

fn cover_picture(bytes: Vec<u8>) -> Result<Picture, String> {
    let format = image::guess_format(&bytes).map_err(|_| "Invalid cover image".to_string())?;
    let image = image::load_from_memory(&bytes).map_err(|_| "Invalid cover image".to_string())?;
    let (data, mime) = match format {
        image::ImageFormat::Jpeg => (bytes, MimeType::Jpeg),
        image::ImageFormat::Png => (bytes, MimeType::Png),
        _ => {
            let mut cursor = std::io::Cursor::new(Vec::new());
            image
                .to_rgb8()
                .write_to(&mut cursor, image::ImageFormat::Jpeg)
                .map_err(|e| e.to_string())?;
            (cursor.into_inner(), MimeType::Jpeg)
        }
    };
    Ok(Picture::new_unchecked(
        PictureType::CoverFront,
        Some(mime),
        None,
        data,
    ))
}

fn fill_missing_tags(
    path: &Path,
    matched: &Match,
    cover: Option<Vec<u8>>,
) -> Result<Option<(MusicTrack, Option<String>)>, String> {
    let mut tagged = Probe::open(path)
        .map_err(|e| e.to_string())?
        .read()
        .map_err(|e| e.to_string())?;
    if tagged.primary_tag_mut().is_none() {
        // Some MP3s only carry an ID3v1 tag. Artwork requires the primary
        // (ID3v2) tag, so seed the new tag with every standard value/picture we
        // can see first. Otherwise merely adding a cover would make the old
        // metadata disappear from primary-tag readers even though we never
        // intended to replace it.
        let seed = tagged.first_tag().map(|tag| {
            (
                nonblank(tag.title().as_deref()),
                nonblank(tag.artist().as_deref()),
                nonblank(tag.album().as_deref()),
                nonblank(tag.genre().as_deref()),
                tag.year(),
                tag.track(),
                tag.pictures().to_vec(),
            )
        });
        let mut primary = Tag::new(tagged.primary_tag_type());
        if let Some((title, artist, album, genre, year, track, pictures)) = seed {
            if let Some(value) = title {
                primary.set_title(value);
            }
            if let Some(value) = artist {
                primary.set_artist(value);
            }
            if let Some(value) = album {
                primary.set_album(value);
            }
            if let Some(value) = genre {
                primary.set_genre(value);
            }
            if let Some(value) = year {
                primary.set_year(value);
            }
            if let Some(value) = track {
                primary.set_track(value);
            }
            for picture in pictures {
                primary.push_picture(picture);
            }
        }
        tagged.insert_tag(primary);
    }
    let tag = tagged
        .primary_tag_mut()
        .ok_or("File format does not support writable tags")?;
    let mut changed = false;

    if nonblank(tag.title().as_deref()).is_none() {
        if let Some(value) = matched.title.clone().filter(|v| !v.trim().is_empty()) {
            tag.set_title(value);
            changed = true;
        }
    }
    if nonblank(tag.artist().as_deref()).is_none() {
        if let Some(value) = matched.artist.clone().filter(|v| !v.trim().is_empty()) {
            tag.set_artist(value);
            changed = true;
        }
    }
    if nonblank(tag.album().as_deref()).is_none() {
        if let Some(value) = matched.album.clone().filter(|v| !v.trim().is_empty()) {
            tag.set_album(value);
            changed = true;
        }
    }
    if nonblank(tag.genre().as_deref()).is_none() {
        if let Some(value) = matched.genre.clone().filter(|v| !v.trim().is_empty()) {
            tag.set_genre(value);
            changed = true;
        }
    }
    if tag.year().is_none() {
        if let Some(value) = matched.year {
            tag.set_year(value);
            changed = true;
        }
    }
    if tag.track().is_none() {
        if let Some(value) = matched.track_number {
            tag.set_track(value);
            changed = true;
        }
    }
    if tag.pictures().is_empty() {
        if let Some(bytes) = cover {
            tag.push_picture(cover_picture(bytes)?);
            changed = true;
        }
    }
    if !changed {
        return Ok(None);
    }

    tagged
        .save_to_path(path, WriteOptions::default())
        .map_err(|e| format!("Failed to write online metadata: {e}"))?;
    let track = parse_metadata(path).ok_or("Failed to re-read updated tags")?;
    Ok(Some((track, compute_fingerprint(path))))
}

fn emit_progress(app: &AppHandle, progress: &Progress) {
    let _ = app.emit("online-metadata-progress", progress);
}

#[tauri::command]
pub fn cancel_online_metadata() {
    RUN_ID.fetch_add(1, Ordering::SeqCst);
}

#[tauri::command]
pub async fn import_online_metadata(
    app: AppHandle,
    db: State<'_, db::Db>,
    paths: Option<Vec<String>>,
) -> Result<ImportSummary, String> {
    let run_id = RUN_ID.fetch_add(1, Ordering::SeqCst) + 1;
    let requested = paths.unwrap_or(db::all_track_paths(&db)?);
    let mut inspect_paths = Vec::new();
    for value in requested {
        let path = PathBuf::from(&value);
        if !is_allowed_audio(&app, &path) || !path.exists() {
            continue;
        }
        inspect_paths.push(path);
    }
    let candidates = tauri::async_runtime::spawn_blocking(move || {
        inspect_paths
            .into_iter()
            .filter_map(|path| {
                let info = inspect_file(&path).ok()?;
                info.missing.any().then_some((path, info))
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| format!("Metadata inspection failed: {e}"))?;

    let total = candidates.len();
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let mut limiter = MusicBrainzLimiter { last_request: None };
    let mut summary = ImportSummary {
        scanned: total,
        matched: 0,
        updated: 0,
        not_found: 0,
        failed: 0,
        cancelled: false,
        tracks: Vec::new(),
    };
    let mut progress = Progress {
        processed: 0,
        total,
        updated: 0,
        not_found: 0,
        failed: 0,
        current_path: None,
        done: false,
        cancelled: false,
    };
    emit_progress(&app, &progress);

    for (path, info) in candidates {
        if RUN_ID.load(Ordering::SeqCst) != run_id {
            summary.cancelled = true;
            break;
        }
        progress.current_path = Some(path.to_string_lossy().to_string());
        emit_progress(&app, &progress);

        let mut matched = None;
        if info.missing.needs_identity() {
            match acoustid_lookup(&client, path.clone(), ACOUSTID_CLIENT_KEY).await {
                Ok(Some(recording_id)) => {
                    matched = musicbrainz_recording(
                        &client,
                        &mut limiter,
                        &recording_id,
                        info.album.as_deref(),
                    )
                    .await
                    .ok()
                    .flatten();
                }
                Ok(None) => {}
                Err(error) => eprintln!("AcoustID lookup failed for {}: {error}", path.display()),
            }
        }
        if matched.is_none() {
            matched = match musicbrainz_search(&client, &mut limiter, &info).await {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("MusicBrainz lookup failed for {}: {error}", path.display());
                    summary.failed += 1;
                    progress.failed = summary.failed;
                    progress.processed += 1;
                    emit_progress(&app, &progress);
                    continue;
                }
            };
        }

        // Off/cancel may have been clicked while a fingerprint or HTTP request
        // was in flight. Do not write anything after that explicit opt-out.
        if RUN_ID.load(Ordering::SeqCst) != run_id {
            summary.cancelled = true;
            break;
        }

        if let Some(matched) = matched {
            summary.matched += 1;
            let cover = if info.missing.cover {
                download_cover(&client, &matched).await
            } else {
                None
            };
            if RUN_ID.load(Ordering::SeqCst) != run_id {
                summary.cancelled = true;
                break;
            }
            let path_for_write = path.clone();
            let matched_for_write = matched.clone();
            match tauri::async_runtime::spawn_blocking(move || {
                fill_missing_tags(&path_for_write, &matched_for_write, cover)
            })
            .await
            {
                Ok(Ok(Some((track, fingerprint)))) => {
                    db::reindex_track(&db, &track, fingerprint.as_deref())?;
                    summary.updated += 1;
                    summary.tracks.push(track);
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    eprintln!("Online tag write failed for {}: {error}", path.display());
                    summary.failed += 1;
                }
                Err(error) => {
                    eprintln!("Online tag task failed for {}: {error}", path.display());
                    summary.failed += 1;
                }
            }
        } else {
            summary.not_found += 1;
        }

        progress.processed += 1;
        progress.updated = summary.updated;
        progress.not_found = summary.not_found;
        progress.failed = summary.failed;
        emit_progress(&app, &progress);
    }

    progress.done = true;
    progress.cancelled = summary.cancelled;
    progress.current_path = None;
    progress.updated = summary.updated;
    progress.not_found = summary.not_found;
    progress.failed = summary.failed;
    emit_progress(&app, &progress);
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_hints_strip_track_number_and_split_artist() {
        let (artist, title) = filename_hints(Path::new("01 - Radiohead - Creep.flac"));
        assert_eq!(artist.as_deref(), Some("Radiohead"));
        assert_eq!(title, "Creep");
    }

    #[test]
    fn year_parser_accepts_partial_musicbrainz_dates() {
        assert_eq!(year_from_date(Some("1997-05-21")), Some(1997));
        assert_eq!(year_from_date(Some("2020")), Some(2020));
        assert_eq!(year_from_date(None), None);
    }
}
