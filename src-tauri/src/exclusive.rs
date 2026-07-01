// WASAPI exclusive-mode playback engine (Windows only).
//
// rodio/cpal only expose shared-mode output, so when the user turns on
// "WASAPI Exclusive" we bypass rodio entirely and render straight to the device
// in exclusive mode through the `wasapi` crate. The device is then owned solely
// by this app (no OS mixing/resampling) for as-direct-as-possible output.
//
// The decode + EQ + spectrum pipeline is reused (the same `EqualizerSource` /
// `SpectrumSource` adapters and the shared EQ/spectrum/volume state), so the
// equalizer and visualizer keep working in exclusive mode. Only the *output*
// (the rodio Sink) is replaced by a dedicated render thread that owns the
// (!Send) WASAPI COM objects for the lifetime of a track.
//
// Transport (play/pause/seek/volume/position/finished) is driven through a small
// set of atomics in `ExclusiveShared`; the audio commands in lib.rs route to
// this engine whenever it is the active output. If exclusive init fails for any
// reason (device busy, format unsupported, exclusive disallowed), `load` returns
// an error and the caller transparently falls back to the shared-mode path.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;

// Shares Arc<Mutex<f32>> (last_volume/norm_factor) with lib.rs's PlayerState, so
// this must use the same parking_lot::Mutex type.
use parking_lot::Mutex;
use std::time::Duration;

use lofty::prelude::*;
use lofty::probe::Probe;
use rodio::source::UniformSourceIterator;
use rodio::Source;
use wasapi::{
    calculate_period_100ns, AudioClient, DeviceEnumerator, Direction, SampleType, StreamMode,
    WaveFormat,
};

use crate::{
    build_decoder, EqualizerShared, EqualizerSource, PlaybackInfo, PlayerStatus, SpectrumShared,
    SpectrumSource,
};

// Lock-free transport/state shared with the render thread.
struct ExclusiveShared {
    active: AtomicBool,          // true while a render thread owns the device
    playing: AtomicBool,        // play / pause
    finished: AtomicBool,       // the track has drained
    position_frames: AtomicU64, // frames played so far (÷ sample_rate = seconds)
    sample_rate: AtomicU32,
    duration_ms: AtomicU64,
    seek_ms: AtomicI64, // -1 = no pending seek
    stop: AtomicBool,   // ask the current render thread to exit
}

impl ExclusiveShared {
    fn new() -> Self {
        ExclusiveShared {
            active: AtomicBool::new(false),
            playing: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            position_frames: AtomicU64::new(0),
            sample_rate: AtomicU32::new(0),
            duration_ms: AtomicU64::new(0),
            seek_ms: AtomicI64::new(-1),
            stop: AtomicBool::new(false),
        }
    }
}

pub struct ExclusivePlayer {
    shared: Arc<ExclusiveShared>,
    // Bumped on every load; the render loop exits when it no longer matches.
    generation: Arc<AtomicU64>,
    // Reused from the main AudioPlayer so EQ / visualizer / volume / normalization
    // behave identically to shared mode.
    equalizer: Arc<EqualizerShared>,
    spectrum: Arc<SpectrumShared>,
    last_volume: Arc<Mutex<f32>>,
    norm_factor: Arc<Mutex<f32>>,
    current_path: Arc<Mutex<Option<String>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl ExclusivePlayer {
    pub fn new(
        equalizer: Arc<EqualizerShared>,
        spectrum: Arc<SpectrumShared>,
        last_volume: Arc<Mutex<f32>>,
        norm_factor: Arc<Mutex<f32>>,
    ) -> Self {
        ExclusivePlayer {
            shared: Arc::new(ExclusiveShared::new()),
            generation: Arc::new(AtomicU64::new(0)),
            equalizer,
            spectrum,
            last_volume,
            norm_factor,
            current_path: Arc::new(Mutex::new(None)),
            thread: Mutex::new(None),
        }
    }

    pub fn is_active(&self) -> bool {
        self.shared.active.load(Ordering::Acquire)
    }

    // Signal the current render thread to exit and wait for it (so the WASAPI
    // client is dropped and the device released before anything else grabs it).
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(handle) = self.thread.lock().take() {
            let _ = handle.join();
        }
        self.shared.active.store(false, Ordering::SeqCst);
        self.shared.playing.store(false, Ordering::SeqCst);
        self.shared.finished.store(false, Ordering::SeqCst);
        self.spectrum.reset();
    }

    pub fn set_playing(&self, playing: bool) {
        self.shared.playing.store(playing, Ordering::SeqCst);
    }

    pub fn seek(&self, position_secs: f64) {
        self.shared
            .seek_ms
            .store((position_secs.max(0.0) * 1000.0) as i64, Ordering::SeqCst);
    }

    pub fn status(&self) -> PlayerStatus {
        let sr = self.shared.sample_rate.load(Ordering::Relaxed).max(1) as f64;
        let position = self.shared.position_frames.load(Ordering::Relaxed) as f64 / sr;
        let duration = self.shared.duration_ms.load(Ordering::Relaxed) as f64 / 1000.0;
        let finished = self.shared.finished.load(Ordering::Relaxed);
        PlayerStatus {
            position,
            duration,
            playing: self.shared.playing.load(Ordering::Relaxed) && !finished,
            finished: self.shared.active.load(Ordering::Relaxed) && finished,
            path: self.current_path.lock().clone(),
        }
    }

    // Load a track and start exclusive playback. Blocks until the device is open
    // (or fails). Run this on a blocking thread, not an async worker.
    pub fn load(
        &self,
        path: String,
        volume: f64,
        start_at: Option<f64>,
        autoplay: bool,
        duration_hint: f64,
    ) -> Result<PlaybackInfo, String> {
        // Release any previous track's device first.
        self.stop();

        let my_gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.shared.stop.store(false, Ordering::SeqCst);
        self.shared.finished.store(false, Ordering::SeqCst);
        self.shared.position_frames.store(0, Ordering::SeqCst);
        self.shared.seek_ms.store(-1, Ordering::SeqCst);
        *self.last_volume.lock() = volume.clamp(0.0, 1.0) as f32;
        *self.current_path.lock() = Some(path.clone());

        let (init_tx, init_rx) = mpsc::channel::<Result<PlaybackInfo, String>>();
        let shared = self.shared.clone();
        let generation = self.generation.clone();
        let equalizer = self.equalizer.clone();
        let spectrum = self.spectrum.clone();
        let last_volume = self.last_volume.clone();
        let norm_factor = self.norm_factor.clone();

        let handle = std::thread::spawn(move || {
            render_loop(
                path,
                start_at,
                autoplay,
                duration_hint,
                my_gen,
                shared,
                generation,
                equalizer,
                spectrum,
                last_volume,
                norm_factor,
                init_tx,
            );
        });
        *self.thread.lock() = Some(handle);

        // Covers file decode + device negotiation + start; generous so large
        // hi-res files on a slow disk don't spuriously fall back to shared mode.
        match init_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(info)) => {
                self.shared.active.store(true, Ordering::SeqCst);
                Ok(info)
            }
            Ok(Err(e)) => {
                self.shared.active.store(false, Ordering::SeqCst);
                Err(e)
            }
            Err(e) => {
                // Init hung — tear the thread down and fall back.
                self.stop();
                Err(format!("WASAPI exclusive init timed out: {e}"))
            }
        }
    }
}

// Pick the first exclusive-capable format for the given rate/channels, matching
// the source's lossless bit depth as closely as the device allows. Returns the
// negotiated format plus the sample type and storage bits so the writer knows how
// to encode.
fn negotiate_format(
    client: &AudioClient,
    sample_rate: u32,
    channels: u16,
    target_bits: Option<u8>,
) -> Result<(WaveFormat, SampleType, u16), String> {
    // (store_bits, valid_bits, sample_type). The order is chosen to honour the
    // source depth while staying on *unambiguous* layouts:
    //   - 16-bit          → (16,16)
    //   - 24-bit          → (32,24)  i.e. 24-in-a-32-bit-container
    //   - 32-bit / hi-res → (32,32)
    // **Packed 24-bit (store=valid=24, blockalign 6) is deliberately near-last**:
    // many Realtek/Intel-HDA drivers accept it via IsFormatSupported but actually
    // render it as 24-in-32, so the packed 3-byte samples land misaligned and come
    // out as loud buzzing. The integer encoder writes a full-scale 32-bit value for
    // any 32-bit container, which is left-justified and therefore correct for both
    // 24-in-32 and full-32 (the device ignores bits below `valid_bits`).
    let bit16 = (16, 16, SampleType::Int);
    let bit24 = (32, 24, SampleType::Int); // 24-in-32, NOT packed
    let bit32 = (32, 32, SampleType::Int);
    let packed24 = (24, 24, SampleType::Int); // last resort
    let float32 = (32, 32, SampleType::Float);

    let mut candidates: Vec<(usize, usize, SampleType)> = Vec::with_capacity(6);
    match target_bits {
        Some(b) if b <= 16 => candidates.extend([bit16, bit24, bit32]),
        Some(24) => candidates.extend([bit24, bit32, bit16]),
        Some(b) if b >= 25 => candidates.extend([bit32, bit24, bit16]),
        // Unknown depth — default to the always-safe 16-bit first.
        _ => candidates.extend([bit16, bit24, bit32]),
    }
    for c in [packed24, float32] {
        if !candidates.contains(&c) {
            candidates.push(c);
        }
    }
    for (store_bits, valid_bits, sample_type) in candidates {
        let fmt = WaveFormat::new(
            store_bits,
            valid_bits,
            &sample_type,
            sample_rate as usize,
            channels as usize,
            None,
        );
        if let Ok(supported) = client.is_supported_exclusive_with_quirks(&fmt) {
            eprintln!(
                "WASAPI exclusive: negotiated {sample_type:?} store={store_bits} valid={valid_bits} blockalign={}",
                supported.get_blockalign()
            );
            return Ok((supported, sample_type, store_bits as u16));
        }
    }
    Err(format!(
        "device has no exclusive format for {sample_rate} Hz / {channels} ch"
    ))
}

// Encode one f32 sample (already volume-scaled) into `out` for the device format.
#[inline]
fn write_sample(out: &mut [u8], s: f32, sample_type: SampleType, store_bits: u16) {
    let s = s.clamp(-1.0, 1.0);
    match (sample_type, store_bits) {
        (SampleType::Float, 32) => out[..4].copy_from_slice(&s.to_le_bytes()),
        (SampleType::Int, 16) => {
            let v = (s * 32767.0) as i16;
            out[..2].copy_from_slice(&v.to_le_bytes());
        }
        (SampleType::Int, 24) => {
            let v = (s * 8_388_607.0) as i32;
            let b = v.to_le_bytes();
            out[0] = b[0];
            out[1] = b[1];
            out[2] = b[2];
        }
        (SampleType::Int, 32) => {
            let v = (s * 2_147_483_647.0) as i32;
            out[..4].copy_from_slice(&v.to_le_bytes());
        }
        _ => {}
    }
}

// The source's declared bit depth (16 / 24 / 32) so exclusive output can match
// the lossless original instead of always defaulting to 16-bit.
fn source_bit_depth(path: &Path) -> Option<u8> {
    Probe::open(path).ok()?.read().ok()?.properties().bit_depth()
}

#[allow(clippy::too_many_arguments)]
fn render_loop(
    path: String,
    start_at: Option<f64>,
    autoplay: bool,
    duration_hint: f64,
    my_gen: u64,
    shared: Arc<ExclusiveShared>,
    generation: Arc<AtomicU64>,
    equalizer: Arc<EqualizerShared>,
    spectrum: Arc<SpectrumShared>,
    last_volume: Arc<Mutex<f32>>,
    norm_factor: Arc<Mutex<f32>>,
    init_tx: mpsc::Sender<Result<PlaybackInfo, String>>,
) {
    // COM for this thread (ignore "already initialized in another mode").
    let _ = wasapi::initialize_mta();

    // Decode (file read + lazy decoder) on this thread.
    let (decoder, decoded_dur) = match build_decoder(Path::new(&path)) {
        Ok(d) => d,
        Err(e) => {
            let _ = init_tx.send(Err(e));
            return;
        }
    };
    let sample_rate = decoder.sample_rate();
    let channels = decoder.channels();
    let source_bits = source_bit_depth(Path::new(&path));
    let duration = if decoded_dur > 0.0 { decoded_dur } else { duration_hint.max(0.0) };

    // Open the default render device in exclusive mode.
    //
    // For true bit-perfect / lossless output we first try to open the device at
    // the *source's own* sample rate + channels, matching its bit depth (16-bit
    // → 16-bit, 24-bit → 24-in-32, hi-res → 32-bit). If the codec can't do the
    // source rate in exclusive, we fall back to its native mix rate and resample
    // to it (like rodio's mixer does in shared mode). Polling mode + a
    // multi-period buffer keeps the endpoint from underrunning.
    let setup = (|| -> Result<_, String> {
        let enumerator = DeviceEnumerator::new().map_err(|e| e.to_string())?;
        let device = enumerator
            .get_default_device(&Direction::Render)
            .map_err(|e| e.to_string())?;

        // One client can serve all IsFormatSupported queries (those don't init).
        let query_client = device.get_iaudioclient().map_err(|e| e.to_string())?;

        let (dev_rate, dev_ch, format, sample_type, store_bits) =
            match negotiate_format(&query_client, sample_rate, channels, source_bits) {
                // Source rate supported in exclusive → bit-perfect, no resampling.
                Ok((fmt, st, sb)) => (sample_rate, channels, fmt, st, sb),
                // Otherwise drop to the device's native mix rate + resample.
                Err(_) => {
                    let mix = query_client.get_mixformat().map_err(|e| e.to_string())?;
                    let dr = mix.get_samplespersec();
                    let dc = mix.get_nchannels().max(1);
                    let (fmt, st, sb) = negotiate_format(&query_client, dr, dc, source_bits)?;
                    (dr, dc, fmt, st, sb)
                }
            };
        drop(query_client);

        let blockalign = format.get_blockalign() as usize;
        let rate_i64 = format.get_samplespersec() as i64;

        // Probe the device periods and a 128-byte-aligned period (Intel HDA and
        // many codecs require buffer sizes aligned to 128 bytes).
        let probe = device.get_iaudioclient().map_err(|e| e.to_string())?;
        let (def_period, min_period) = probe.get_device_period().map_err(|e| e.to_string())?;
        let aligned = probe
            .calculate_aligned_period_near(def_period, Some(128), &format)
            .unwrap_or(def_period);
        drop(probe);

        // Candidate periods, most-compatible first.
        let candidates = [aligned, def_period, min_period];

        let mut last_err = String::new();
        for &period in candidates.iter() {
            if period <= 0 {
                continue;
            }
            // A buffer 16 periods long gives plenty of refill headroom so the
            // device never underruns (the other classic source of buzzing).
            let mode = StreamMode::PollingExclusive {
                period_hns: period,
                buffer_duration_hns: 16 * period,
            };
            // A client that failed init cannot be reused – always get a fresh one.
            let mut audio_client = match device.get_iaudioclient() {
                Ok(c) => c,
                Err(e) => {
                    last_err = format!("get_iaudioclient: {e}");
                    continue;
                }
            };
            match audio_client.initialize_client(&format, &Direction::Render, &mode) {
                Ok(()) => {
                    let render_client =
                        audio_client.get_audiorenderclient().map_err(|e| e.to_string())?;
                    let buffer_frames =
                        audio_client.get_buffer_size().map_err(|e| e.to_string())?;
                    return Ok((
                        audio_client, render_client, sample_type, store_bits, blockalign,
                        buffer_frames, dev_rate, dev_ch,
                    ));
                }
                Err(e) => {
                    last_err = format!("period {period} hns: {e}");
                    // Documented recovery for AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED:
                    // ask the failed client for its next-highest aligned buffer
                    // size and retry once on a fresh client with that period.
                    if let Ok(frames) = audio_client.get_buffer_size() {
                        let p2 = calculate_period_100ns(frames as i64, rate_i64);
                        drop(audio_client);
                        if p2 > 0 {
                            if let Ok(mut c2) = device.get_iaudioclient() {
                                let mode2 = StreamMode::PollingExclusive {
                                    period_hns: p2,
                                    buffer_duration_hns: 16 * p2,
                                };
                                if c2
                                    .initialize_client(&format, &Direction::Render, &mode2)
                                    .is_ok()
                                {
                                    let render_client =
                                        c2.get_audiorenderclient().map_err(|e| e.to_string())?;
                                    let buffer_frames =
                                        c2.get_buffer_size().map_err(|e| e.to_string())?;
                                    return Ok((
                                        c2, render_client, sample_type, store_bits, blockalign,
                                        buffer_frames, dev_rate, dev_ch,
                                    ));
                                }
                                last_err = format!("aligned period {p2} hns also failed");
                            }
                        }
                    }
                    continue;
                }
            }
        }
        Err(format!(
            "all exclusive period strategies failed for {dev_rate}Hz/{dev_ch}ch ({last_err})"
        ))
    })();

    let (
        audio_client,
        render_client,
        sample_type,
        store_bits,
        blockalign,
        buffer_frames,
        dev_rate,
        dev_ch,
    ) = match setup {
        Ok(v) => v,
        Err(e) => {
            let _ = init_tx.send(Err(e));
            return;
        }
    };

    eprintln!(
        "WASAPI exclusive: device {dev_rate} Hz / {dev_ch} ch, format {sample_type:?}/{store_bits}-bit (source {sample_rate} Hz / {channels} ch / {source_bits:?}-bit)"
    );

    shared.sample_rate.store(dev_rate, Ordering::SeqCst);
    shared.duration_ms.store((duration * 1000.0) as u64, Ordering::SeqCst);

    // Decode → EQ → spectrum tap → resample to the device's rate/channels. The
    // EQ runs at the file's rate (its coefficients are computed for it) and the
    // resampler is last, exactly mirroring the shared-mode signal path.
    let equalized = EqualizerSource::new(decoder, equalizer);
    let tapped = SpectrumSource::new(equalized, spectrum.clone(), generation.clone(), my_gen);
    let mut src = UniformSourceIterator::new(tapped, dev_ch, dev_rate);

    if let Some(pos) = start_at {
        if pos > 0.0 {
            let _ = src.try_seek(Duration::from_secs_f64(pos));
            shared
                .position_frames
                .store((pos * dev_rate as f64) as u64, Ordering::SeqCst);
        }
    }

    let bytes_per_sample = (store_bits / 8) as usize;
    let ch = dev_ch as usize;
    let mut finished = false;
    let mut consecutive_write_errors: u32 = 0;
    const MAX_WRITE_ERRORS: u32 = 5;

    // Pre-fill the whole buffer with audio before starting, so the device never
    // plays uninitialised memory at startup (a classic source of an initial pop
    // or burst of noise). MSDN recommends priming an exclusive buffer like this.
    {
        let volume = (*last_volume.lock() * *norm_factor.lock()).clamp(0.0, 4.0);
        if let Ok(frames) = audio_client.get_available_space_in_frames() {
            let frames = frames as usize;
            if frames > 0 {
                let mut data = vec![0u8; frames * blockalign];
                if autoplay {
                    let mut produced: u64 = 0;
                    'prefill: for f in 0..frames {
                        for c in 0..ch {
                            match src.next() {
                                Some(sample) => {
                                    let off = f * blockalign + c * bytes_per_sample;
                                    write_sample(
                                        &mut data[off..],
                                        sample * volume,
                                        sample_type,
                                        store_bits,
                                    );
                                }
                                None => break 'prefill,
                            }
                        }
                        produced += 1;
                    }
                    shared.position_frames.fetch_add(produced, Ordering::SeqCst);
                }
                let _ = render_client.write_to_device(frames, &data, None);
            }
        }
    }

    shared.playing.store(autoplay, Ordering::SeqCst);
    if let Err(e) = audio_client.start_stream() {
        let _ = init_tx.send(Err(format!("start_stream failed: {e}")));
        shared.active.store(false, Ordering::SeqCst);
        return;
    }

    // Only report success once audio is actually flowing, so the caller's
    // fallback to shared mode triggers on any real failure above. Report the
    // *effective* depth = min(source, container): a 24-bit track through a 32-bit
    // container reads as 24-bit (bit-perfect), but if we had to fall back to a
    // 16-bit format the badge honestly shows 16-bit.
    let _ = init_tx.send(Ok(PlaybackInfo {
        duration,
        sample_rate: Some(sample_rate),
        bit_depth: source_bits.map(|sb| sb.min(store_bits as u8)),
    }));

    // Poll roughly every half-buffer; capped so stop/seek/pause stay responsive.
    let buffer_ms = (buffer_frames as u64 * 1000) / dev_rate.max(1) as u64;
    let sleep_dur = Duration::from_millis((buffer_ms / 2).clamp(10, 60));

    loop {
        if shared.stop.load(Ordering::SeqCst) || generation.load(Ordering::SeqCst) != my_gen {
            break;
        }

        // Apply a pending seek.
        let seek = shared.seek_ms.swap(-1, Ordering::SeqCst);
        if seek >= 0 {
            let _ = src.try_seek(Duration::from_millis(seek as u64));
            shared
                .position_frames
                .store((seek as u64) * dev_rate as u64 / 1000, Ordering::SeqCst);
            finished = false;
            shared.finished.store(false, Ordering::SeqCst);
        }

        let frames = match audio_client.get_available_space_in_frames() {
            Ok(f) => f as usize,
            // Device invalidated / unplugged – stop cleanly.
            Err(_) => break,
        };
        if frames == 0 {
            std::thread::sleep(sleep_dur);
            continue;
        }
        let mut data = vec![0u8; frames * blockalign];

        let playing = shared.playing.load(Ordering::SeqCst);
        if playing && !finished {
            let volume =
                (*last_volume.lock() * *norm_factor.lock()).clamp(0.0, 4.0);
            let mut produced: u64 = 0;
            'fill: for f in 0..frames {
                for c in 0..ch {
                    match src.next() {
                        Some(sample) => {
                            let off = f * blockalign + c * bytes_per_sample;
                            write_sample(&mut data[off..], sample * volume, sample_type, store_bits);
                        }
                        None => {
                            finished = true;
                            break 'fill;
                        }
                    }
                }
                produced += 1;
            }
            shared.position_frames.fetch_add(produced, Ordering::SeqCst);
            if finished {
                shared.finished.store(true, Ordering::SeqCst);
                shared.playing.store(false, Ordering::SeqCst);
            }
        }
        // When paused or finished, `data` stays zeroed → silence keeps the
        // exclusive endpoint fed without advancing the track.

        if let Err(e) = render_client.write_to_device(frames, &data, None) {
            consecutive_write_errors += 1;
            eprintln!(
                "WASAPI exclusive write error ({}/{}): {e}",
                consecutive_write_errors, MAX_WRITE_ERRORS
            );
            if consecutive_write_errors >= MAX_WRITE_ERRORS {
                break;
            }
        } else {
            consecutive_write_errors = 0;
        }

        std::thread::sleep(sleep_dur);
    }

    let _ = audio_client.stop_stream();
    shared.active.store(false, Ordering::SeqCst);
    spectrum.reset();
}
