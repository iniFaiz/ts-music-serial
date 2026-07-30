//! Native playback engine, DSP, device routing, and loudness analysis.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use parking_lot::Mutex;
use rodio::{OutputStream, OutputStreamBuilder, Sink, Source};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::cache_manager::{self, CacheKind};
use crate::{cover_cache_key, library_db, parse_rg_db, resolve_allowed_audio};

#[cfg(target_os = "windows")]
mod exclusive;

mod session;
pub(crate) use session::{
    PlaybackEffect, PlaybackIntent, PlaybackSessionSnapshot, PlaybackSessionUpdate, PreparedToken,
    TransitionMode,
};

// ---------------------------------------------------------------------------
// Native audio playback
//
// Decoding/playback runs entirely in Rust (rodio + symphonia) rather than the
// webview's <audio> element. This guarantees consistent format support across
// platforms and gives precise, reliable seeking.
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

// Command sent to the dedicated audio thread (which owns the !Send OutputStream).
enum AudioCommand {
    // Open the output device with the given name, or the system default (None).
    // The reply channel is signalled once the device is open.
    OpenDevice(Option<String>, mpsc::Sender<()>),
    CreateSink(mpsc::Sender<Result<Arc<Sink>, String>>),
    #[cfg(target_os = "windows")]
    CloseStream(mpsc::Sender<()>),
}

#[allow(dead_code)]
struct ActiveTrack {
    path: String,
    sink: Arc<Sink>,
    duration: f64,
    start_time: Instant,
}

#[allow(dead_code)]
struct FadingTrack {
    sink: Arc<Sink>,
    fade_end: Instant,
    fade_duration: Duration,
    initial_volume: f32,
}

struct PreparedTrack {
    path: String,
    token: PreparedToken,
    decoder: AudioDecoder,
    duration: f64,
    gain_db: Option<f32>,
    peak: Option<f32>,
}

pub(crate) struct AudioPlayer {
    // The active default sink handle from device stream builder
    sink: Arc<Mutex<Option<Arc<Sink>>>>,
    // Duration of the currently loaded track, in seconds.
    duration: Arc<Mutex<f64>>,
    // True once a track has been loaded, so an empty sink means "finished"
    // rather than "nothing has played yet".
    active: Arc<AtomicBool>,
    // Bumped on every load. A decode that finishes after a newer load started
    // checks this and discards its (now stale) result instead of clobbering the
    // track the user actually wants.
    generation: Arc<AtomicU64>,
    // Latest frequency-band levels for the UI visualizer, fed by SpectrumSource.
    spectrum: Arc<SpectrumShared>,
    // 10-band graphic EQ settings, read live by each EqualizerSource in the chain.
    equalizer: Arc<EqualizerShared>,
    // Volume-normalization factor (linear) applied on top of the user volume.
    // 1.0 = no normalization.
    norm_factor: Arc<Mutex<f32>>,
    // Last user-requested volume (0..1), so the normalization factor can be
    // re-applied to the live sink without the frontend re-sending the volume.
    last_volume: Arc<Mutex<f32>>,
    // Channel to the audio thread for device-management commands.
    cmd_tx: mpsc::Sender<AudioCommand>,
    // Pre-decoded next track details
    prepared: Arc<Mutex<Option<PreparedTrack>>>,
    // Target token currently being decoded. Unlike a path-only marker this
    // distinguishes duplicate occurrences of the same file in the queue.
    preparing: Arc<Mutex<Option<PreparedToken>>>,
    // Authoritative queue, playback, transition, autoplay and sleep policy.
    session: session::PlaybackSession,
    // Normalization configuration
    normalization_enabled: Arc<Mutex<bool>>,
    normalization_preamp_db: Arc<Mutex<f64>>,
    // Current primary track and fading tracks
    current_track: Arc<Mutex<Option<ActiveTrack>>>,
    fading_tracks: Arc<Mutex<Vec<FadingTrack>>>,
    // When true (Windows only), playback is routed to the WASAPI exclusive engine
    // instead of the rodio shared-mode sink.
    exclusive_enabled: Arc<AtomicBool>,
}

impl Clone for AudioPlayer {
    fn clone(&self) -> Self {
        AudioPlayer {
            sink: self.sink.clone(),
            current_track: self.current_track.clone(),
            fading_tracks: self.fading_tracks.clone(),
            duration: self.duration.clone(),
            active: self.active.clone(),
            generation: self.generation.clone(),
            spectrum: self.spectrum.clone(),
            equalizer: self.equalizer.clone(),
            norm_factor: self.norm_factor.clone(),
            last_volume: self.last_volume.clone(),
            cmd_tx: self.cmd_tx.clone(),
            prepared: self.prepared.clone(),
            preparing: self.preparing.clone(),
            session: self.session.clone(),
            normalization_enabled: self.normalization_enabled.clone(),
            normalization_preamp_db: self.normalization_preamp_db.clone(),
            exclusive_enabled: self.exclusive_enabled.clone(),
        }
    }
}

impl AudioPlayer {
    // Clone out the current sink handle (if any) without holding the lock.
    fn sink(&self) -> Option<Arc<Sink>> {
        self.current_track.lock().as_ref().map(|t| t.sink.clone())
    }
    // Effective sink volume = user volume * normalization factor. Capped above
    // 1.0 (≈ +12 dB) so normalization can boost quiet tracks; rodio amplifies
    // values > 1.0 and the peak limiter in player_set_normalization guards
    // against clipping.
    fn effective_volume(&self) -> f32 {
        let vol = *self.last_volume.lock();
        let factor = *self.norm_factor.lock();
        (vol * factor).clamp(0.0, 4.0)
    }
}

#[derive(Clone, Serialize)]
pub(crate) struct PlayerStatus {
    pub(crate) position: f64,
    pub(crate) duration: f64,
    pub(crate) playing: bool,
    pub(crate) finished: bool,
    pub(crate) path: Option<String>,
}

fn player_status_snapshot(app: &AppHandle, player: &AudioPlayer) -> PlayerStatus {
    #[cfg(target_os = "windows")]
    if player.exclusive_enabled.load(Ordering::SeqCst) {
        if let Some(ex) = app.try_state::<Arc<exclusive::ExclusivePlayer>>() {
            if ex.is_active() {
                let mut status = ex.status();
                let update = player.session.observe_progress(
                    status.position,
                    status.duration,
                    status.finished,
                );
                if matches!(update.effect.as_ref(), Some(PlaybackEffect::Stop { .. })) {
                    status.playing = false;
                    status.finished = false;
                }
                apply_immediate_session_effect(app, player, update.effect.as_ref());
                emit_session_update(app, &update);
                return status;
            }
        }
    }

    let mut status = match player.sink() {
        Some(sink) => {
            let empty = sink.empty();
            let path = player.current_track.lock().as_ref().map(|t| t.path.clone());
            PlayerStatus {
                position: sink.get_pos().as_secs_f64(),
                duration: *player.duration.lock(),
                playing: !sink.is_paused() && !empty,
                finished: player.active.load(Ordering::SeqCst) && empty,
                path,
            }
        }
        None => PlayerStatus {
            position: 0.0,
            duration: 0.0,
            playing: false,
            finished: false,
            path: None,
        },
    };
    let update = player
        .session
        .observe_progress(status.position, status.duration, status.finished);
    if matches!(update.effect.as_ref(), Some(PlaybackEffect::Stop { .. })) {
        status.playing = false;
        status.finished = false;
    }
    apply_immediate_session_effect(app, player, update.effect.as_ref());
    emit_session_update(app, &update);
    status
}

fn invalidate_prepared_decoder(player: &AudioPlayer) {
    *player.prepared.lock() = None;
    *player.preparing.lock() = None;
}

fn emit_session_update(app: &AppHandle, update: &PlaybackSessionUpdate) {
    if let Some(database) = app.try_state::<library_db::Db>() {
        for event in &update.events {
            if let session::PlaybackSessionEvent::Accounting { kind, path, .. } = event {
                let result = match kind {
                    session::AccountingKind::PlayStarted => {
                        library_db::stats::record_play_start(database.inner(), path)
                    }
                    session::AccountingKind::PlayCounted => {
                        library_db::stats::record_play(database.inner(), path)
                    }
                    session::AccountingKind::SkipCounted => {
                        library_db::stats::record_skip(database.inner(), path)
                    }
                };
                if let Err(error) = result {
                    eprintln!("Failed to persist native playback accounting: {error}");
                }
            }
        }
    }
    // Progress is observed on every status poll; only publish meaningful
    // changes so this typed event does not become a 4 Hz snapshot broadcast.
    if !update.events.is_empty() || update.effect.is_some() {
        let _ = app.emit("playback-session-event", update.clone());
    }
}

fn apply_immediate_session_effect(
    _app: &AppHandle,
    player: &AudioPlayer,
    effect: Option<&PlaybackEffect>,
) {
    match effect {
        Some(PlaybackEffect::SetPlaying { playing }) => {
            #[cfg(target_os = "windows")]
            if player.exclusive_enabled.load(Ordering::SeqCst) {
                if let Some(ex) = _app.try_state::<Arc<exclusive::ExclusivePlayer>>() {
                    if ex.is_active() {
                        ex.set_playing(*playing);
                        return;
                    }
                }
            }
            if let Some(sink) = player.sink() {
                if *playing {
                    sink.play();
                } else {
                    sink.pause();
                }
            }
            for fading in player.fading_tracks.lock().iter() {
                if *playing {
                    fading.sink.play();
                } else {
                    fading.sink.pause();
                }
            }
        }
        Some(PlaybackEffect::Stop { .. }) => {
            #[cfg(target_os = "windows")]
            if let Some(ex) = _app.try_state::<Arc<exclusive::ExclusivePlayer>>() {
                if ex.is_active() {
                    ex.stop();
                }
            }
            for fading in player.fading_tracks.lock().drain(..) {
                fading.sink.stop();
            }
            if let Some(track) = player.current_track.lock().take() {
                track.sink.stop();
            }
            player.active.store(false, Ordering::SeqCst);
            invalidate_prepared_decoder(player);
        }
        _ => {}
    }
}

mod decoder;
pub(crate) use decoder::{build_decoder, AudioDecoder};

mod spectrum;
use spectrum::SPECTRUM_BANDS;
pub(crate) use spectrum::{SpectrumShared, SpectrumSource};

mod equalizer;
use equalizer::EQ_BANDS;
pub(crate) use equalizer::{EqualizerShared, EqualizerSource};

// Load a track and start playing it, replacing whatever was playing. Returns
// the track duration in seconds. The file read + decoder setup run on the
// blocking pool so the UI/IPC thread never stalls; the previously playing track
// is stopped immediately so it doesn't bleed over the (brief) load gap.
#[derive(Serialize)]
pub(crate) struct PlaybackInfo {
    pub(crate) duration: f64,
    pub(crate) sample_rate: Option<u32>,
    pub(crate) bit_depth: Option<u8>,
}

#[allow(clippy::too_many_arguments)]
async fn load_track(
    app: &AppHandle,
    player: &AudioPlayer,
    path: String,
    volume: f64,
    start_at: Option<f64>,
    autoplay: bool,
    duration_hint: f64,
    fade_in_secs: Option<f64>,
    queue_entry_id: Option<String>,
) -> Result<PlaybackInfo, String> {
    let path_buf = resolve_allowed_audio(app, Path::new(&path))?;
    let path = path_buf.to_string_lossy().to_string();

    // WASAPI exclusive path (Windows). Falls back to shared mode on any failure.
    #[cfg(target_os = "windows")]
    if player.exclusive_enabled.load(Ordering::SeqCst) {
        if let Some(ex) = app.try_state::<Arc<exclusive::ExclusivePlayer>>() {
            let ex = ex.inner().clone();
            let p = path.clone();
            let res = tauri::async_runtime::spawn_blocking(move || {
                ex.load(p, volume, start_at, autoplay, duration_hint)
            })
            .await
            .map_err(|e| format!("Exclusive load task failed: {e}"))?;
            match res {
                Ok(info) => {
                    invalidate_prepared_decoder(player);
                    return Ok(info);
                }
                Err(e) => {
                    eprintln!("WASAPI exclusive load failed ({e}); using shared mode.");
                    let _ = app.emit("wasapi-exclusive-error", e);
                    // Disable exclusive so play/pause/status/seek route to
                    // the shared-mode path until the user re-enables it.
                    player.exclusive_enabled.store(false, Ordering::SeqCst);

                    // Re-open the shared stream with a timeout so we never hang.
                    let (reply_tx, reply_rx) = mpsc::channel();
                    if player
                        .cmd_tx
                        .send(AudioCommand::OpenDevice(None, reply_tx))
                        .is_ok()
                    {
                        let _ = reply_rx.recv_timeout(Duration::from_secs(5));
                    }
                    // fall through to the shared-mode rodio path below
                }
            }
        }
    }

    // Stop all active sinks and clear fading list
    {
        let mut fading_guard = player.fading_tracks.lock();
        for t in fading_guard.drain(..) {
            t.sink.stop();
        }
        let mut current_guard = player.current_track.lock();
        if let Some(t) = current_guard.take() {
            if let Some(secs) = fade_in_secs {
                if secs > 0.0 {
                    fading_guard.push(FadingTrack {
                        sink: t.sink.clone(),
                        fade_end: Instant::now() + Duration::from_secs_f64(secs),
                        fade_duration: Duration::from_secs_f64(secs),
                        initial_volume: player.effective_volume(),
                    });
                } else {
                    t.sink.stop();
                }
            } else {
                t.sink.stop();
            }
        }
    }

    // Create a new sink via the audio thread
    let (reply_tx, reply_rx) = mpsc::channel();
    player
        .cmd_tx
        .send(AudioCommand::CreateSink(reply_tx))
        .map_err(|e| format!("Audio thread unavailable: {e}"))?;

    // Wait for a sink. Allow enough time for the audio thread to re-open the
    // device (with retries) if the shared stream needs to be recovered first.
    let sink = reply_rx
        .recv_timeout(Duration::from_secs(4))
        .map_err(|e| format!("Failed to create sink: {e}"))??;

    // Clone the shared handles out so we never hold the State guard across .await.
    let generation = player.generation.clone();
    let duration_slot = player.duration.clone();
    let active = player.active.clone();
    let spectrum = player.spectrum.clone();
    let norm_factor = player.norm_factor.clone();
    let last_volume = player.last_volume.clone();
    let prepared = player.prepared.clone();

    // Claim a generation and stop the current track right away. Marking the
    // player inactive during the load gap prevents the now-empty sink from
    // being misread as "track finished" (which would auto-skip to the next one).
    let my_gen = generation.fetch_add(1, Ordering::SeqCst) + 1;
    active.store(false, Ordering::SeqCst);
    spectrum.reset(); // clear the visualizer during the load gap
                      // Apply the requested volume together with the active normalization factor.
    let user_vol = volume.clamp(0.0, 1.0) as f32;
    *last_volume.lock() = user_vol;
    let factor = *norm_factor.lock();
    sink.set_volume(user_vol * factor);

    // Reuse a pre-decoded next track when it matches (near-gapless); otherwise
    // read + decode on the blocking pool.
    let prepared_decoder = {
        let mut g = prepared.lock();
        let reusable = g
            .as_ref()
            .map(|prepared| {
                prepared.path == path
                    && queue_entry_id
                        .as_deref()
                        .map(|id| id == prepared.token.entry_id)
                        .unwrap_or(true)
                    && player.session.is_prepare_current(&prepared.token)
            })
            .unwrap_or(false);
        *player.preparing.lock() = None;
        if reusable {
            g.take()
        } else {
            *g = None;
            None
        }
    };
    let (decoder, decoded_duration) = match prepared_decoder {
        Some(prep) => (prep.decoder, prep.duration),
        None => {
            let path_buf_clone = path_buf.clone();
            let (dec, raw_dur) =
                tauri::async_runtime::spawn_blocking(move || build_decoder(&path_buf_clone))
                    .await
                    .map_err(|e| format!("Decode task failed: {e}"))??;
            let dur = if raw_dur > 0.0 {
                raw_dur
            } else {
                duration_hint.max(0.0)
            };
            (dec, dur)
        }
    };

    // A newer load was requested while we were reading — drop this stale one.
    if generation.load(Ordering::SeqCst) != my_gen {
        return Ok(PlaybackInfo {
            duration: 0.0,
            sample_rate: None,
            bit_depth: None,
        });
    }

    // Prefer the decoder's duration; fall back to the metadata hint (e.g. for
    // headerless MP3 where the decoder can't report one).
    let duration = if decoded_duration > 0.0 {
        decoded_duration
    } else {
        duration_hint.max(0.0)
    };
    *duration_slot.lock() = duration;
    // Equalize, then tap the decoded samples for the visualizer on their way to
    // the sink (so the bars reflect what you actually hear). In crossfade mode
    // each track is eased in with a fade.
    let equalized = EqualizerSource::new(decoder, player.equalizer.clone());
    let tapped = SpectrumSource::new(equalized, spectrum, player.generation.clone(), my_gen);
    match fade_in_secs {
        Some(secs) if secs > 0.0 => {
            sink.append(tapped.fade_in(Duration::from_secs_f64(secs.min(12.0))));
        }
        _ => sink.append(tapped),
    }

    // Resume support: jump to a saved position before (optionally) playing.
    if let Some(pos) = start_at {
        let target = if duration > 0.1 {
            pos.clamp(0.0, duration - 0.1)
        } else {
            pos.max(0.0)
        };
        if target > 0.0 {
            let _ = sink.try_seek(Duration::from_secs_f64(target));
        }
    }

    if autoplay {
        sink.play();
    } else {
        sink.pause();
    }
    active.store(true, Ordering::SeqCst);

    // Save as current track
    {
        let mut current_guard = player.current_track.lock();
        *current_guard = Some(ActiveTrack {
            path: path.clone(),
            sink: sink.clone(),
            duration,
            start_time: Instant::now(),
        });
    }

    // Read properties via lofty
    let mut sample_rate = None;
    let mut bit_depth = None;
    if let Ok(tagged_file) = lofty::probe::Probe::open(&path_buf).and_then(|p| p.read()) {
        let props = tagged_file.properties();
        sample_rate = props.sample_rate();
        bit_depth = props.bit_depth();
    }

    Ok(PlaybackInfo {
        duration,
        sample_rate,
        bit_depth,
    })
}

// Pre-decode the next track for near-gapless playback. The ready decoder is
// stored and consumed by the native session load/transition path.
#[tauri::command]
pub(crate) async fn player_prepare_next(
    app: AppHandle,
    player: State<'_, AudioPlayer>,
    path: String,
    duration_hint: Option<f64>,
    queue_entry_id: Option<String>,
) -> Result<(), String> {
    let path_buf = resolve_allowed_audio(&app, Path::new(&path))?;
    let path = path_buf.to_string_lossy().to_string();

    // The exclusive engine doesn't use rodio pre-decode; skip entirely.
    #[cfg(target_os = "windows")]
    if player.exclusive_enabled.load(Ordering::SeqCst) {
        return Ok(());
    }

    let Some(token) = player
        .session
        .begin_prepare(&path, queue_entry_id.as_deref())
    else {
        // Transition-off and a path which is no longer part of the queue both
        // mean the decoder must be released, not merely ignored by the ticker.
        *player.prepared.lock() = None;
        *player.preparing.lock() = None;
        return Ok(());
    };

    // Claim the exact entry+generation target and clear any stale decoder.
    {
        let mut target_g = player.preparing.lock();
        if target_g.as_ref() == Some(&token) {
            return Ok(());
        }
        *target_g = Some(token.clone());
        *player.prepared.lock() = None;
    }

    let prepared = player.prepared.clone();
    let preparing = player.preparing.clone();
    let session = player.session.clone();
    let pb = path_buf.clone();
    let token_for_decode = token.clone();
    if let Ok(Ok((dec, raw_dur))) =
        tauri::async_runtime::spawn_blocking(move || build_decoder(&pb)).await
    {
        // Queue edits and mode changes bump the policy generation.  Check both
        // policy and audio-layer target before doing more work.
        if preparing.lock().as_ref() != Some(&token_for_decode)
            || !session.is_prepare_current(&token_for_decode)
        {
            return Ok(());
        }

        // Parse ReplayGain tags of the prepared track
        let mut gain_db = None;
        let mut peak = None;
        if let Ok(tagged_file) = Probe::open(&path_buf).and_then(|p| p.read()) {
            let tag = tagged_file
                .primary_tag()
                .or_else(|| tagged_file.first_tag());
            gain_db = tag
                .and_then(|t| t.get_string(&ItemKey::ReplayGainTrackGain))
                .and_then(parse_rg_db);
            peak = tag
                .and_then(|t| t.get_string(&ItemKey::ReplayGainTrackPeak))
                .and_then(|s| s.trim().parse::<f32>().ok());
        }

        let dur = if raw_dur > 0.0 {
            raw_dur
        } else {
            duration_hint.unwrap_or(0.0)
        };

        // Check again after metadata I/O; this is the race window that used to
        // allow an old queue's decoder to overwrite the new target.
        if preparing.lock().as_ref() != Some(&token_for_decode)
            || !session.is_prepare_current(&token_for_decode)
        {
            return Ok(());
        }

        *prepared.lock() = Some(PreparedTrack {
            path,
            token: token_for_decode,
            decoder: dec,
            duration: dur,
            gain_db,
            peak,
        });
    } else {
        let mut target = preparing.lock();
        if target.as_ref() == Some(&token) {
            *target = None;
        }
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn player_pause(app: AppHandle, player: State<AudioPlayer>) {
    #[cfg(target_os = "windows")]
    if player.exclusive_enabled.load(Ordering::SeqCst) {
        if let Some(ex) = app.try_state::<Arc<exclusive::ExclusivePlayer>>() {
            if ex.is_active() {
                ex.set_playing(false);
                let update = player.session.set_playing(false);
                emit_session_update(&app, &update);
                return;
            }
        }
    }
    if let Some(sink) = player.sink() {
        sink.pause();
    }
    // Also pause all fading tracks!
    let fading = player.fading_tracks.lock();
    for track in fading.iter() {
        track.sink.pause();
    }
    let update = player.session.set_playing(false);
    emit_session_update(&app, &update);
}

#[tauri::command]
pub(crate) fn player_resume(app: AppHandle, player: State<AudioPlayer>) {
    #[cfg(target_os = "windows")]
    if player.exclusive_enabled.load(Ordering::SeqCst) {
        if let Some(ex) = app.try_state::<Arc<exclusive::ExclusivePlayer>>() {
            if ex.is_active() {
                ex.set_playing(true);
                let update = player.session.set_playing(true);
                emit_session_update(&app, &update);
                return;
            }
        }
    }
    if let Some(sink) = player.sink() {
        sink.play();
    }
    // Also resume all fading tracks!
    let fading = player.fading_tracks.lock();
    for track in fading.iter() {
        track.sink.play();
    }
    let update = player.session.set_playing(true);
    emit_session_update(&app, &update);
}

#[tauri::command]
pub(crate) fn player_set_volume(player: State<AudioPlayer>, volume: f64) {
    let user_vol = volume.clamp(0.0, 1.0) as f32;
    *player.last_volume.lock() = user_vol;
    let eff_vol = player.effective_volume();
    if let Some(sink) = player.sink() {
        sink.set_volume(eff_vol);
    }
    let mut fading = player.fading_tracks.lock();
    for track in fading.iter_mut() {
        track.initial_volume = eff_vol;
    }
}

#[tauri::command]
pub(crate) fn player_seek(
    app: AppHandle,
    player: State<AudioPlayer>,
    position: f64,
) -> Result<(), String> {
    #[cfg(not(target_os = "windows"))]
    let _ = app;

    #[cfg(target_os = "windows")]
    if player.exclusive_enabled.load(Ordering::SeqCst) {
        if let Some(ex) = app.try_state::<Arc<exclusive::ExclusivePlayer>>() {
            if ex.is_active() {
                ex.seek(position);
                return Ok(());
            }
        }
    }
    // Stop all fading tracks on manual seek
    {
        let mut fading_guard = player.fading_tracks.lock();
        for t in fading_guard.drain(..) {
            t.sink.stop();
        }
    }
    if let Some(sink) = player.sink() {
        let duration = *player.duration.lock();
        // Keep the target inside the track; seeking to/past the end can error.
        let target = if duration > 0.1 {
            position.clamp(0.0, duration - 0.1)
        } else {
            position.max(0.0)
        };
        sink.try_seek(Duration::from_secs_f64(target))
            .map_err(|e| format!("Seek failed: {e:?}"))?;
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn player_stop(app: AppHandle, player: State<AudioPlayer>) {
    #[cfg(target_os = "windows")]
    if let Some(ex) = app.try_state::<Arc<exclusive::ExclusivePlayer>>() {
        if ex.is_active() {
            ex.stop();
        }
    }
    {
        let mut fading_guard = player.fading_tracks.lock();
        for t in fading_guard.drain(..) {
            t.sink.stop();
        }
        let mut current_guard = player.current_track.lock();
        if let Some(t) = current_guard.take() {
            t.sink.stop();
        }
    }
    player.active.store(false, Ordering::SeqCst);
    invalidate_prepared_decoder(&player);
    let update = player.session.set_playing(false);
    emit_session_update(&app, &update);
}

#[tauri::command]
pub(crate) fn player_status(app: AppHandle, player: State<AudioPlayer>) -> PlayerStatus {
    player_status_snapshot(&app, &player)
}

#[tauri::command]
pub(crate) fn player_set_transition(
    app: AppHandle,
    player: State<AudioPlayer>,
    mode: String,
    crossfade_secs: f64,
) {
    let typed_mode = TransitionMode::from_legacy(&mode);
    let update = player.session.set_transition(typed_mode, crossfade_secs);
    if update.prepared_invalidated || typed_mode == TransitionMode::Off {
        invalidate_prepared_decoder(&player);
    }
    emit_session_update(&app, &update);
}

/// Read-only native playback state for window initialization and recovery.
#[tauri::command]
pub(crate) fn playback_session_snapshot(player: State<AudioPlayer>) -> PlaybackSessionSnapshot {
    player.session.snapshot()
}

/// Apply an intent and execute its audio effect before publishing the snapshot.
/// This ordering prevents a webview watcher from racing a prepared transition.
#[tauri::command]
pub(crate) async fn playback_session_intent(
    app: AppHandle,
    player: State<'_, AudioPlayer>,
    intent: PlaybackIntent,
) -> Result<PlaybackSessionUpdate, String> {
    // Canonicalize and authorize every new queue path before mutating session
    // state. A rejected path can no longer leave a partially replaced queue.
    let intent = intent.validate_and_authorize(&app)?;
    let update = player.session.apply(intent)?;
    if update.prepared_invalidated {
        invalidate_prepared_decoder(&player);
    }
    if let Some(PlaybackEffect::Load {
        entry,
        autoplay,
        start_at,
    }) = update.effect.as_ref()
    {
        let normalization_enabled = *player.normalization_enabled.lock();
        let normalization_preamp_db = *player.normalization_preamp_db.lock();
        set_normalization_factor(
            &player,
            entry.track_gain_db,
            normalization_preamp_db,
            entry.track_peak,
            normalization_enabled,
        );
        let volume = *player.last_volume.lock() as f64;
        if let Err(error) = load_track(
            &app,
            &player,
            entry.path.clone(),
            volume,
            *start_at,
            *autoplay,
            entry.duration_hint,
            Some(0.0),
            Some(entry.id.clone()),
        )
        .await
        {
            let failed = player.session.set_playing(false);
            apply_immediate_session_effect(&app, &player, failed.effect.as_ref());
            emit_session_update(&app, &failed);
            return Err(error);
        }
        if player.session.snapshot().revision != update.snapshot.revision {
            // Another window changed the session while decoder setup was in
            // flight. Audio generation already discarded the stale decoder;
            // never publish its older snapshot back into the webviews.
            return Ok(player.session.snapshot_update());
        }
    } else {
        apply_immediate_session_effect(&app, &player, update.effect.as_ref());
    }
    emit_session_update(&app, &update);
    Ok(update)
}

#[tauri::command]
pub(crate) fn player_set_normalization_settings(
    player: State<AudioPlayer>,
    enabled: bool,
    preamp_db: f64,
) {
    *player.normalization_enabled.lock() = enabled;
    *player.normalization_preamp_db.lock() = preamp_db;
}

// Latest six frequency-band levels (0..1), low → high. Returns all-zero when no
// track is playing or the visualizer is disabled. Polled by the UI at ~30fps.
#[tauri::command]
pub(crate) fn player_spectrum(player: State<AudioPlayer>) -> [f32; SPECTRUM_BANDS] {
    player.spectrum.load()
}

// Toggle the (cheap but non-zero) audio analysis on/off, mirroring the Settings
// switch so it truly costs nothing when the visualizer is hidden.
#[tauri::command]
pub(crate) fn player_set_spectrum_enabled(player: State<AudioPlayer>, enabled: bool) {
    player.spectrum.enabled.store(enabled, Ordering::SeqCst);
    if !enabled {
        player.spectrum.reset();
    }
}

// Apply the 10-band equalizer. `gains` is ten per-band gains in dB (low → high),
// `preamp_db` a master gain applied before the bands. The change is picked up by
// the live audio chain on its next sample, so it takes effect mid-track.
#[tauri::command]
pub(crate) fn player_set_equalizer(
    player: State<AudioPlayer>,
    enabled: bool,
    gains: Vec<f64>,
    preamp_db: f64,
) {
    let mut g = [0f32; EQ_BANDS];
    for (slot, v) in g.iter_mut().zip(gains.iter()) {
        *slot = (*v as f32).clamp(-12.0, 12.0);
    }
    player
        .equalizer
        .set(enabled, &g, (preamp_db as f32).clamp(-12.0, 12.0));
}

// Open an OutputStream for the named device (or the system default when None),
// build a fresh Sink on its mixer and publish it into `slot`. Returns the
// stream so the caller (the audio thread) can keep it alive; leaves `slot` as
// None on failure. The new sink inherits the current effective volume so a
// device switch doesn't reset levels.
fn open_device_stream(
    name: Option<&str>,
    slot: &Arc<Mutex<Option<Arc<Sink>>>>,
    last_volume: &Arc<Mutex<f32>>,
    norm_factor: &Arc<Mutex<f32>>,
) -> Option<OutputStream> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};

    // Opening can transiently fail right after an exclusive-mode stream releases
    // the device (USB DACs / IEMs re-enumerate slowly), so retry a few times
    // before giving up.
    let mut stream = None;
    for attempt in 0..4 {
        stream = match name {
            Some(want) => {
                let host = rodio::cpal::default_host();
                let device = host.output_devices().ok().and_then(|mut devs| {
                    devs.find(|d| d.name().map(|n| n == want).unwrap_or(false))
                });
                match device {
                    Some(dev) => OutputStreamBuilder::from_device(dev)
                        .and_then(|b| b.open_stream())
                        .ok(),
                    // Requested device vanished — fall back to default.
                    None => OutputStreamBuilder::open_default_stream().ok(),
                }
            }
            None => OutputStreamBuilder::open_default_stream().ok(),
        };
        if stream.is_some() {
            break;
        }
        if attempt < 3 {
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    match stream {
        Some(stream) => {
            let sink = Sink::connect_new(stream.mixer());
            let vol = (*last_volume.lock() * *norm_factor.lock()).clamp(0.0, 4.0);
            sink.set_volume(vol);
            *slot.lock() = Some(Arc::new(sink));
            Some(stream)
        }
        None => {
            *slot.lock() = None;
            None
        }
    }
}

// Build the audio player. The OutputStream is `!Send`, so it lives on a
// dedicated thread that owns it for the app's lifetime; the thread blocks on a
// command channel (keeping the stream alive) and rebuilds the stream/sink when
// the output device changes. Only the (Send) Sink handle is shared back.
pub(crate) fn init_audio_player() -> AudioPlayer {
    let (cmd_tx, cmd_rx) = mpsc::channel::<AudioCommand>();
    let (ready_tx, ready_rx) = mpsc::channel::<()>();

    let sink_slot: Arc<Mutex<Option<Arc<Sink>>>> = Arc::new(Mutex::new(None));
    let norm_factor = Arc::new(Mutex::new(1.0f32));
    let last_volume = Arc::new(Mutex::new(1.0f32));

    let slot_t = sink_slot.clone();
    let nf_t = norm_factor.clone();
    let lv_t = last_volume.clone();
    std::thread::spawn(move || {
        // Open the default device on startup, then signal readiness.
        let mut _stream = open_device_stream(None, &slot_t, &lv_t, &nf_t);
        let _ = ready_tx.send(());

        // Process device-change commands. recv() blocks (keeping `_stream`
        // alive) until the sender is dropped at shutdown.
        while let Ok(cmd) = cmd_rx.recv() {
            match cmd {
                AudioCommand::OpenDevice(name, reply) => {
                    // Stop and drop the old sink, then drop the old stream by
                    // overwriting it with the freshly opened one.
                    {
                        let mut guard = slot_t.lock();
                        if let Some(s) = guard.take() {
                            s.stop();
                        }
                    }
                    _stream = open_device_stream(name.as_deref(), &slot_t, &lv_t, &nf_t);
                    let _ = reply.send(());
                }
                AudioCommand::CreateSink(reply) => {
                    // Self-heal: the shared stream may be absent (just back from
                    // exclusive mode, or a transient open failure). Re-open the
                    // default device before giving up so playback recovers instead
                    // of leaving the UI stuck "loading".
                    if _stream.is_none() {
                        _stream = open_device_stream(None, &slot_t, &lv_t, &nf_t);
                    }
                    let res = if let Some(ref stream) = _stream {
                        Ok(Arc::new(Sink::connect_new(stream.mixer())))
                    } else {
                        Err("No active audio stream".to_string())
                    };
                    let _ = reply.send(res);
                }
                #[cfg(target_os = "windows")]
                AudioCommand::CloseStream(reply) => {
                    {
                        let mut guard = slot_t.lock();
                        if let Some(s) = guard.take() {
                            s.stop();
                        }
                    }
                    _stream = None;
                    let _ = reply.send(());
                }
            }
        }
    });

    // Wait for the initial device open so the first session load sees a sink
    // (or a definitive None when no output device exists).
    let _ = ready_rx.recv();

    AudioPlayer {
        sink: sink_slot,
        duration: Arc::new(Mutex::new(0.0)),
        active: Arc::new(AtomicBool::new(false)),
        generation: Arc::new(AtomicU64::new(0)),
        spectrum: Arc::new(SpectrumShared::new()),
        equalizer: Arc::new(EqualizerShared::new()),
        norm_factor,
        last_volume,
        cmd_tx,
        prepared: Arc::new(Mutex::new(None)),
        preparing: Arc::new(Mutex::new(None)),
        session: session::PlaybackSession::default(),
        normalization_enabled: Arc::new(Mutex::new(false)),
        normalization_preamp_db: Arc::new(Mutex::new(0.0)),
        current_track: Arc::new(Mutex::new(None)),
        fading_tracks: Arc::new(Mutex::new(Vec::new())),
        exclusive_enabled: Arc::new(AtomicBool::new(false)),
    }
}

// True-gapless lead: how early (seconds before the current track ends) the next
// track is appended onto the SAME sink. rodio plays queued sources back-to-back
// with zero gap, so this lead only needs to clear the ~20ms ticker tick and the
// audio output buffer. Kept short so a last-moment queue edit has the smallest
// possible window to race the already-queued track.
const GAPLESS_LEAD_SECS: f64 = 0.3;

// A next track appended onto the *currently playing* sink's queue for true
// gapless playback, waiting for the active source to finish. The ticker promotes
// it to the current track the instant rodio advances the queue to it.
struct GaplessQueued {
    path: String,
    token: PreparedToken,
    duration: f64,
    gain_db: Option<f32>,
    peak: Option<f32>,
    sink: Arc<Sink>,
}

pub(crate) fn spawn_player_ticker(app: AppHandle, player: AudioPlayer) {
    // Spawn the background audio device hotplug/change detector thread
    let app_clone = app.clone();
    std::thread::spawn(move || {
        use rodio::cpal::traits::{DeviceTrait, HostTrait};

        let get_default_device_name = || -> Option<String> {
            let host = rodio::cpal::default_host();
            host.default_output_device().and_then(|d| d.name().ok())
        };

        let get_all_device_names = || -> Vec<String> {
            let host = rodio::cpal::default_host();
            if let Ok(devices) = host.output_devices() {
                devices.filter_map(|d| d.name().ok()).collect()
            } else {
                Vec::new()
            }
        };

        let mut last_default = get_default_device_name();
        let mut last_devices = get_all_device_names();

        loop {
            std::thread::sleep(Duration::from_millis(1500));

            let current_default = get_default_device_name();
            let current_devices = get_all_device_names();

            let mut changed = false;

            if current_default != last_default {
                last_default = current_default.clone();
                changed = true;
            }

            let mut current_sorted = current_devices.clone();
            current_sorted.sort();
            let mut last_sorted = last_devices.clone();
            last_sorted.sort();

            if current_sorted != last_sorted {
                last_devices = current_devices;
                changed = true;
            }

            if changed {
                // Emit an event to notify the frontend to refresh the output device list
                // and potentially switch the default output stream.
                let _ = app_clone.emit("audio-devices-changed", ());
            }
        }
    });

    std::thread::spawn(move || {
        let mut transition_triggered_for_gen = 0;
        let mut telemetry_visible = true;
        let mut last_visibility_check = Instant::now() - Duration::from_secs(1);
        let mut last_status_emit = Instant::now() - Duration::from_secs(1);
        let mut last_spectrum_emit = Instant::now() - Duration::from_secs(1);
        // Next track queued onto the live sink for true-gapless playback (None
        // when nothing is waiting ahead).
        let mut gapless_queued: Option<GaplessQueued> = None;
        loop {
            std::thread::sleep(Duration::from_millis(20));
            let tick_now = Instant::now();

            // Push one shared telemetry stream instead of making every mounted
            // component poll IPC. Status runs at 8 Hz and spectrum at 25 Hz.
            // Events stop while the main window is hidden or minimized, while
            // the native status snapshot keeps session accounting up to date.
            if tick_now.duration_since(last_visibility_check) >= Duration::from_millis(250) {
                last_visibility_check = tick_now;
                telemetry_visible = app
                    .get_webview_window("main")
                    .map(|window| {
                        window.is_visible().unwrap_or(true)
                            && !window.is_minimized().unwrap_or(false)
                    })
                    .unwrap_or(true);
            }
            if tick_now.duration_since(last_status_emit) >= Duration::from_millis(125) {
                last_status_emit = tick_now;
                let status = player_status_snapshot(&app, &player);
                if telemetry_visible {
                    let _ = app.emit("player-telemetry", status);
                }
            }
            if telemetry_visible
                && player.spectrum.enabled.load(Ordering::Relaxed)
                && tick_now.duration_since(last_spectrum_emit) >= Duration::from_millis(40)
            {
                last_spectrum_emit = tick_now;
                let _ = app.emit("player-spectrum", player.spectrum.load());
            }

            // 1. Process fading tracks (MUST run even if active is false, so fading tracks fade out and stop!)
            {
                let mut fading = player.fading_tracks.lock();
                let now = Instant::now();
                fading.retain(|track| {
                    if now >= track.fade_end {
                        track.sink.stop();
                        false
                    } else {
                        let total_secs = track.fade_duration.as_secs_f64();
                        let remaining_ratio = if total_secs > 0.0 {
                            (track.fade_end - now).as_secs_f64() / total_secs
                        } else {
                            0.0
                        };
                        let fade_vol =
                            track.initial_volume * (remaining_ratio as f32).clamp(0.0, 1.0);
                        track.sink.set_volume(fade_vol);
                        true
                    }
                });
            }

            // 2. Process automatic transitions
            let active = player.active.load(Ordering::SeqCst);
            if !active {
                continue;
            }

            let current_gen = player.generation.load(Ordering::SeqCst);

            let (current_sink, position, duration, empty) = {
                let current_guard = player.current_track.lock();
                if let Some(ref track) = *current_guard {
                    let pos = track.sink.get_pos().as_secs_f64();
                    let dur = track.duration;
                    let empty = track.sink.empty();
                    (track.sink.clone(), pos, dur, empty)
                } else {
                    continue;
                }
            };

            let session_snapshot = player.session.snapshot();
            let mode = session_snapshot.transition;
            let crossfade_secs = session_snapshot.crossfade_secs;

            // --- True-gapless boundary: promote a queued next track ----------
            // When a next track was appended ahead onto the shared sink, watch
            // its queue: once the previous source drains (only the queued track
            // remains) rodio is already playing it seamlessly, so promote it to
            // the current track, fix up duration/normalization, and announce it.
            if let Some(q) = gapless_queued.take() {
                if !Arc::ptr_eq(&q.sink, &current_sink) {
                    // A manual load replaced the sink (and stopped the queued
                    // track with it) — drop the now-stale entry.
                } else if q.sink.len() <= 1 {
                    let user_vol = *player.last_volume.lock();
                    let norm_enabled = *player.normalization_enabled.lock();
                    let preamp_db = *player.normalization_preamp_db.lock();
                    let factor = if norm_enabled {
                        let total_db = q.gain_db.map(|g| g as f64).unwrap_or(0.0) + preamp_db;
                        let mut f = 10f64.powf(total_db / 20.0);
                        if let Some(pk) = q.peak {
                            if pk > 0.0 {
                                f = f.min(1.0 / pk as f64);
                            }
                        }
                        (f as f32).clamp(0.0, 4.0)
                    } else {
                        1.0
                    };
                    *player.norm_factor.lock() = factor;
                    q.sink.set_volume(user_vol * factor);
                    *player.duration.lock() = q.duration;
                    *player.current_track.lock() = Some(ActiveTrack {
                        path: q.path.clone(),
                        sink: q.sink.clone(),
                        duration: q.duration,
                        start_time: Instant::now(),
                    });
                    let Some(session_update) = player.session.promote_prepared(&q.token) else {
                        // The queue changed after this source was appended. Never
                        // rewrite native policy to match stale audio.
                        q.sink.stop();
                        player.active.store(false, Ordering::SeqCst);
                        continue;
                    };
                    emit_session_update(&app, &session_update);
                    let _ = app.emit(
                        "track-changed",
                        serde_json::json!({
                            "path": q.path,
                            "queueEntryId": session_update.snapshot.current_entry_id,
                            "generation": session_update.snapshot.generation,
                            "reason": "gapless"
                        }),
                    );
                    // Re-read fresh state next tick rather than acting on the
                    // outgoing track's stale position/duration this iteration.
                    continue;
                } else {
                    // Current source still playing — keep waiting.
                    gapless_queued = Some(q);
                }
            }

            if mode == TransitionMode::Crossfade
                && (duration > crossfade_secs && position >= (duration - crossfade_secs) || empty)
            {
                if transition_triggered_for_gen != current_gen {
                    let prepared_opt = player
                        .prepared
                        .lock()
                        .take()
                        .filter(|prepared| player.session.is_prepare_current(&prepared.token));
                    // Consuming or rejecting a decoder always releases its
                    // in-flight marker so the following entry can prepare.
                    *player.preparing.lock() = None;

                    if let Some(prep) = prepared_opt {
                        let (reply_tx, reply_rx) = mpsc::channel();
                        if player
                            .cmd_tx
                            .send(AudioCommand::CreateSink(reply_tx))
                            .is_ok()
                        {
                            if let Ok(Ok(new_sink)) = reply_rx.recv_timeout(Duration::from_secs(2))
                            {
                                let prepared_token = prep.token.clone();
                                let prepared_path = prep.path.clone();
                                let Some(session_update) =
                                    player.session.promote_prepared(&prepared_token)
                                else {
                                    new_sink.stop();
                                    continue;
                                };
                                let mut current_guard = player.current_track.lock();
                                if let Some(ref old_track) = *current_guard {
                                    let mut fading_guard = player.fading_tracks.lock();
                                    fading_guard.push(FadingTrack {
                                        sink: old_track.sink.clone(),
                                        fade_end: Instant::now()
                                            + Duration::from_secs_f64(crossfade_secs),
                                        fade_duration: Duration::from_secs_f64(crossfade_secs),
                                        initial_volume: player.effective_volume(),
                                    });
                                }

                                let new_gen = player.generation.fetch_add(1, Ordering::SeqCst) + 1;
                                // Mark the outgoing generation as handled. The native
                                // transition owns both audio promotion and session state;
                                // the new generation must remain eligible for its own boundary.
                                transition_triggered_for_gen = current_gen;

                                let next_dur = prep.duration;
                                *player.duration.lock() = next_dur;

                                // Apply normalization to the new sink
                                let norm_enabled = *player.normalization_enabled.lock();
                                let preamp_db = *player.normalization_preamp_db.lock();
                                let factor = if norm_enabled {
                                    let total_db =
                                        prep.gain_db.map(|g| g as f64).unwrap_or(0.0) + preamp_db;
                                    let mut f = 10f64.powf(total_db / 20.0);
                                    if let Some(pk) = prep.peak {
                                        if pk > 0.0 {
                                            f = f.min(1.0 / pk as f64);
                                        }
                                    }
                                    (f as f32).clamp(0.0, 4.0)
                                } else {
                                    1.0
                                };
                                *player.norm_factor.lock() = factor;

                                let equalized =
                                    EqualizerSource::new(prep.decoder, player.equalizer.clone());
                                let tapped = SpectrumSource::new(
                                    equalized,
                                    player.spectrum.clone(),
                                    player.generation.clone(),
                                    new_gen,
                                );
                                new_sink.append(
                                    tapped.fade_in(Duration::from_secs_f64(crossfade_secs)),
                                );

                                let user_vol = *player.last_volume.lock();
                                new_sink.set_volume(user_vol * factor);
                                new_sink.play();

                                *current_guard = Some(ActiveTrack {
                                    path: prep.path.clone(),
                                    sink: new_sink,
                                    duration: next_dur,
                                    start_time: Instant::now(),
                                });

                                emit_session_update(&app, &session_update);
                                let _ = app.emit(
                                    "track-changed",
                                    serde_json::json!({
                                        "path": prepared_path,
                                        "queueEntryId": session_update.snapshot.current_entry_id,
                                        "generation": session_update.snapshot.generation,
                                        "reason": "crossfade"
                                    }),
                                );
                            }
                        }
                    }
                }
            } else if mode == TransitionMode::Gapless
                && gapless_queued.is_none()
                && transition_triggered_for_gen != current_gen
                && ((duration > 0.0 && position >= (duration - GAPLESS_LEAD_SECS)) || empty)
            {
                // True gapless: append the pre-decoded next track straight onto
                // the live sink. rodio plays queued sources back-to-back, so it
                // starts the instant the current source ends — one continuous
                // sink, no fade, no gap. The boundary handler above promotes it
                // to the current track once it actually begins.
                let prepared_opt = player
                    .prepared
                    .lock()
                    .take()
                    .filter(|prepared| player.session.is_prepare_current(&prepared.token));
                if let Some(prep) = prepared_opt {
                    // Consuming the prepared track invalidates the "currently
                    // preparing" marker; clear it so the next prepare_next call
                    // (for the track after this one) is never mistaken for a no-op.
                    *player.preparing.lock() = None;
                    // Reserve the generation now so this track's spectrum tap is
                    // live the moment it becomes the active source. (The outgoing
                    // track's visualizer goes quiet for the short lead window —
                    // imperceptible at a track's tail.)
                    let new_gen = player.generation.fetch_add(1, Ordering::SeqCst) + 1;
                    // Mark the *outgoing* track's generation as handled so we
                    // append exactly once per track (see the crossfade branch for
                    // why this keys on current_gen, not new_gen).
                    transition_triggered_for_gen = current_gen;
                    let equalized = EqualizerSource::new(prep.decoder, player.equalizer.clone());
                    let tapped = SpectrumSource::new(
                        equalized,
                        player.spectrum.clone(),
                        player.generation.clone(),
                        new_gen,
                    );
                    current_sink.append(tapped);
                    gapless_queued = Some(GaplessQueued {
                        path: prep.path.clone(),
                        token: prep.token,
                        duration: prep.duration,
                        gain_db: prep.gain_db,
                        peak: prep.peak,
                        sink: current_sink.clone(),
                    });
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Output device selection
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub(crate) struct OutputDeviceInfo {
    name: String,
    is_default: bool,
}

// Enumerate the available audio output devices.
#[tauri::command]
pub(crate) fn list_output_devices() -> Vec<OutputDeviceInfo> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    let host = rodio::cpal::default_host();
    let default_name = host.default_output_device().and_then(|d| d.name().ok());
    let mut out = Vec::new();
    if let Ok(devices) = host.output_devices() {
        for d in devices {
            if let Ok(name) = d.name() {
                let is_default = default_name.as_deref() == Some(name.as_str());
                out.push(OutputDeviceInfo { name, is_default });
            }
        }
    }
    out
}

// Switch the audio output device (None = system default). Blocks until the new
// device is open so the frontend can immediately reload the current track.
#[tauri::command]
pub(crate) fn set_output_device(
    player: State<AudioPlayer>,
    name: Option<String>,
) -> Result<(), String> {
    let (reply_tx, reply_rx) = mpsc::channel();
    player
        .cmd_tx
        .send(AudioCommand::OpenDevice(name, reply_tx))
        .map_err(|e| format!("Audio thread unavailable: {e}"))?;
    let _ = reply_rx.recv_timeout(Duration::from_secs(5));
    Ok(())
}

// ---------------------------------------------------------------------------
// Volume normalization (Sound Check) — ReplayGain tags + lazy EBU R128 compute
// ---------------------------------------------------------------------------

// Reference loudness for normalization (ReplayGain 2.0 standard).
const NORM_TARGET_LUFS: f64 = -18.0;

// Set the normalization factor from a track's gain (dB), an optional peak (to
// prevent clipping when boosting) and the user pre-amp. Re-applies immediately
// to the live sink. `enabled = false` resets the factor to 1.0 (no change).
#[tauri::command]
pub(crate) fn player_set_normalization(
    player: State<AudioPlayer>,
    gain_db: Option<f64>,
    preamp_db: f64,
    peak: Option<f64>,
    enabled: bool,
) {
    set_normalization_factor(&player, gain_db, preamp_db, peak, enabled);
}

fn set_normalization_factor(
    player: &AudioPlayer,
    gain_db: Option<f64>,
    preamp_db: f64,
    peak: Option<f64>,
    enabled: bool,
) {
    let factor = if enabled {
        let total_db = gain_db.unwrap_or(0.0) + preamp_db;
        let mut f = 10f64.powf(total_db / 20.0);
        if let Some(pk) = peak {
            if pk > 0.0 {
                f = f.min(1.0 / pk); // never amplify past the track's peak headroom
            }
        }
        (f as f32).clamp(0.0, 4.0)
    } else {
        1.0
    };
    *player.norm_factor.lock() = factor;
    if let Some(sink) = player.sink() {
        sink.set_volume(player.effective_volume());
    }
}

fn read_loudness(app: &AppHandle, key: &str) -> Option<f32> {
    let cache = cache_manager::manager(app)?;
    let bytes = cache.read(CacheKind::Loudness, &format!("{key}.json"))?;
    serde_json::from_slice(&bytes).ok()
}

fn write_loudness(app: &AppHandle, key: &str, gain: f32, source_path: &Path) {
    if let (Some(cache), Ok(bytes)) = (cache_manager::manager(app), serde_json::to_vec(&gain)) {
        let _ = cache.write(
            CacheKind::Loudness,
            &format!("{key}.json"),
            &bytes,
            Some(source_path),
        );
    }
}

// Decode the whole track and measure its integrated loudness (EBU R128), then
// return the gain (dB) needed to reach the reference target. Heavy — runs on
// the blocking pool via compute_track_gain.
fn compute_gain_blocking(path: &Path) -> Result<f32, String> {
    use ebur128::{EbuR128, Mode};
    let (decoder, _dur) = build_decoder(path)?;
    let channels = decoder.channels().max(1) as u32;
    let sample_rate = decoder.sample_rate().max(1);
    let mut ebu = EbuR128::new(channels, sample_rate, Mode::I).map_err(|e| e.to_string())?;
    let mut buf: Vec<f32> = Vec::with_capacity(65536);
    for s in decoder {
        buf.push(s);
        if buf.len() >= 65536 {
            ebu.add_frames_f32(&buf).map_err(|e| e.to_string())?;
            buf.clear();
        }
    }
    if !buf.is_empty() {
        ebu.add_frames_f32(&buf).map_err(|e| e.to_string())?;
    }
    let loudness = ebu.loudness_global().map_err(|e| e.to_string())?;
    if !loudness.is_finite() {
        return Ok(0.0); // silent / unmeasurable track
    }
    let gain = (NORM_TARGET_LUFS - loudness) as f32;
    Ok(gain.clamp(-15.0, 15.0))
}

// Return the normalization gain (dB) for a track without ReplayGain tags,
// computing and caching it on first request.
#[tauri::command]
pub(crate) async fn compute_track_gain(app: AppHandle, path: String) -> Result<f32, String> {
    let path_buf = resolve_allowed_audio(&app, Path::new(&path))?;
    let key = cover_cache_key(&path_buf).ok_or_else(|| "Track metadata unavailable".to_string())?;
    if let Some(g) = read_loudness(&app, &key) {
        return Ok(g);
    }
    let app2 = app.clone();
    let source_path = path_buf.clone();
    let gain = tauri::async_runtime::spawn_blocking(move || compute_gain_blocking(&path_buf))
        .await
        .map_err(|e| format!("Loudness task failed: {e}"))??;
    write_loudness(&app2, &key, gain, &source_path);
    Ok(gain)
}

// Toggle WASAPI exclusive output. Enabling frees the rodio shared sink and closes
// the shared stream so the exclusive engine can claim the device; disabling
// stops the exclusive engine and re-opens the shared-mode stream.
// The frontend reloads the current track afterwards so playback continues on the
// newly-selected engine. (Windows only; a no-op stub elsewhere.)
#[cfg(target_os = "windows")]
#[tauri::command]
pub(crate) async fn set_wasapi_exclusive(
    app: tauri::AppHandle,
    player: tauri::State<'_, AudioPlayer>,
    enabled: bool,
) -> Result<(), String> {
    player.exclusive_enabled.store(enabled, Ordering::SeqCst);
    if enabled {
        let mut fading_guard = player.fading_tracks.lock();
        for t in fading_guard.drain(..) {
            t.sink.stop();
        }
        let mut current_guard = player.current_track.lock();
        if let Some(t) = current_guard.take() {
            t.sink.stop();
        }
        player.active.store(false, Ordering::SeqCst);

        // Close the shared stream so WASAPI exclusive can claim the device
        let (reply_tx, reply_rx) = mpsc::channel();
        player
            .cmd_tx
            .send(AudioCommand::CloseStream(reply_tx))
            .map_err(|e| format!("Audio thread unavailable: {e}"))?;
        reply_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|e| format!("Failed to close shared stream: {e}"))?;
    } else {
        if let Some(ex) = app.try_state::<Arc<exclusive::ExclusivePlayer>>() {
            let ex = ex.inner().clone();
            tauri::async_runtime::spawn_blocking(move || {
                ex.stop();
            })
            .await
            .map_err(|e| format!("Exclusive stop task failed: {e}"))?;
        }
        // Re-open the shared stream so standard playback works again
        let (reply_tx, reply_rx) = mpsc::channel();
        player
            .cmd_tx
            .send(AudioCommand::OpenDevice(None, reply_tx))
            .map_err(|e| format!("Audio thread unavailable: {e}"))?;
        reply_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| format!("Failed to re-open shared stream: {e}"))?;
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub(crate) async fn set_wasapi_exclusive(_enabled: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn init_exclusive_player(audio: &AudioPlayer) -> Arc<exclusive::ExclusivePlayer> {
    Arc::new(exclusive::ExclusivePlayer::new(
        audio.equalizer.clone(),
        audio.spectrum.clone(),
        audio.last_volume.clone(),
        audio.norm_factor.clone(),
    ))
}
