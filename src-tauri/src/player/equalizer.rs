use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rodio::source::SeekError;
use rodio::Source;

// ---------------------------------------------------------------------------
// 10-band graphic equalizer.
//
// `EqualizerSource`, like `SpectrumSource`, is a pass-through `Source` inserted
// between the decoder and the sink — here it actually *modifies* the samples.
// Each of the ten ISO octave bands (31 Hz … 16 kHz) is a peaking biquad (RBJ
// cookbook); the bands are cascaded in series and run per channel. Gains live in
// a lock-free shared struct the UI writes via `player_set_equalizer`; a `version`
// counter lets the audio thread notice changes and recompute coefficients
// without recomputing per sample. When disabled (or flat + off) it is a near-free
// pass-through: one atomic load and a channel counter per sample.
// ---------------------------------------------------------------------------

pub(super) const EQ_BANDS: usize = 10;
// ISO 1/1-octave center frequencies. The top band sits below the 22.05 kHz
// Nyquist of 44.1 kHz audio; `Biquad::peaking` falls back to a flat response for
// any band at/above Nyquist (e.g. unusual low sample rates).
const EQ_FREQS: [f32; EQ_BANDS] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
// Q ≈ √2 gives each peaking filter ~one octave of bandwidth, so adjacent bands
// overlap smoothly the way a graphic EQ should.
const EQ_Q: f32 = 1.41;

// Lock-free EQ settings shared from the UI thread to the audio thread. Gains and
// preamp are f32 stored as their bit patterns; `version` is bumped on any change
// so live `EqualizerSource`s know to recompute their coefficients.
pub(crate) struct EqualizerShared {
    enabled: AtomicBool,
    gains_db: [AtomicU32; EQ_BANDS],
    preamp_db: AtomicU32,
    version: AtomicU64,
}

impl EqualizerShared {
    pub(super) fn new() -> Self {
        EqualizerShared {
            enabled: AtomicBool::new(false),
            gains_db: std::array::from_fn(|_| AtomicU32::new(0)),
            preamp_db: AtomicU32::new(0),
            version: AtomicU64::new(0),
        }
    }

    // Publish a full settings snapshot. Values are written first, then the
    // version is bumped with Release so a source that observes the new version
    // (with Acquire) is guaranteed to read these values.
    pub(super) fn set(&self, enabled: bool, gains: &[f32; EQ_BANDS], preamp_db: f32) {
        self.enabled.store(enabled, Ordering::Relaxed);
        for (slot, g) in self.gains_db.iter().zip(gains) {
            slot.store(g.to_bits(), Ordering::Relaxed);
        }
        self.preamp_db.store(preamp_db.to_bits(), Ordering::Relaxed);
        self.version.fetch_add(1, Ordering::Release);
    }

    fn snapshot(&self) -> (bool, [f32; EQ_BANDS], f32) {
        let enabled = self.enabled.load(Ordering::Relaxed);
        let gains =
            std::array::from_fn(|i| f32::from_bits(self.gains_db[i].load(Ordering::Relaxed)));
        let preamp = f32::from_bits(self.preamp_db.load(Ordering::Relaxed));
        (enabled, gains, preamp)
    }
}

// Normalized biquad coefficients (a0 folded in) and its Direct-Form-I state.
#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Biquad {
    fn identity() -> Self {
        Biquad {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }

    // RBJ "cookbook" peaking EQ. `gain_db` > 0 boosts the band, < 0 cuts it.
    fn peaking(freq: f32, sample_rate: f32, q: f32, gain_db: f32) -> Self {
        // A band centered at/above Nyquist can't be realized — leave it flat.
        if freq * 2.0 >= sample_rate || sample_rate <= 0.0 {
            return Biquad::identity();
        }
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha / a;
        Biquad {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos_w0) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct BiquadState {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl BiquadState {
    #[inline]
    fn process(&mut self, x: f32, c: &Biquad) -> f32 {
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

pub(crate) struct EqualizerSource<S> {
    inner: S,
    shared: Arc<EqualizerShared>,
    sample_rate: f32,
    channels: u16,
    ch: u16,
    coeffs: [Biquad; EQ_BANDS],
    states: Vec<[BiquadState; EQ_BANDS]>,
    preamp: f32,
    enabled: bool,
    last_version: u64,
}

impl<S: Source> EqualizerSource<S> {
    pub(crate) fn new(inner: S, shared: Arc<EqualizerShared>) -> Self {
        let channels = inner.channels().max(1);
        let sample_rate = inner.sample_rate().max(1) as f32;
        let mut me = EqualizerSource {
            inner,
            shared,
            sample_rate,
            channels,
            ch: 0,
            coeffs: [Biquad::identity(); EQ_BANDS],
            states: vec![[BiquadState::default(); EQ_BANDS]; channels as usize],
            preamp: 1.0,
            enabled: false,
            last_version: 0,
        };
        me.last_version = me.shared.version.load(Ordering::Acquire);
        me.recompute();
        me
    }

    // Rebuild the per-band coefficients from the current shared settings. Cheap
    // and rare: only runs at construction and when the UI changes a value.
    fn recompute(&mut self) {
        let (enabled, gains, preamp_db) = self.shared.snapshot();
        self.enabled = enabled;
        self.preamp = 10f32.powf(preamp_db / 20.0);
        for b in 0..EQ_BANDS {
            self.coeffs[b] = if gains[b].abs() < 1e-3 {
                Biquad::identity()
            } else {
                Biquad::peaking(EQ_FREQS[b], self.sample_rate, EQ_Q, gains[b])
            };
        }
    }
}

impl<S: Source> Iterator for EqualizerSource<S> {
    type Item = f32;
    #[inline]
    fn next(&mut self) -> Option<f32> {
        let s = self.inner.next()?;
        // Pick up live UI changes (recompute coefficients at most once per edit).
        let v = self.shared.version.load(Ordering::Acquire);
        if v != self.last_version {
            self.recompute();
            self.last_version = v;
        }
        let ch = self.ch as usize;
        self.ch += 1;
        if self.ch >= self.channels {
            self.ch = 0;
        }
        if !self.enabled {
            return Some(s);
        }
        let mut x = s * self.preamp;
        if let Some(state) = self.states.get_mut(ch) {
            for b in 0..EQ_BANDS {
                x = state[b].process(x, &self.coeffs[b]);
            }
        }
        Some(x)
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S: Source> Source for EqualizerSource<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        // Clear filter memory so a jump doesn't ring out the old signal.
        for st in self.states.iter_mut() {
            *st = [BiquadState::default(); EQ_BANDS];
        }
        self.inner.try_seek(pos)
    }
}
