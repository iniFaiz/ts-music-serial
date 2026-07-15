use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rodio::source::SeekError;
use rodio::Source;

// ---------------------------------------------------------------------------
// Real-time spectrum analysis for the UI visualizer.
//
// `SpectrumSource` is a thin pass-through `Source` inserted between the decoder
// and the sink. It taps the samples on their way to the audio device, mixes
// them to mono, and accumulates FFT-sized windows; each completed window is
// reduced to six smoothed frequency-band levels (0..1) that the frontend reads
// via `player_spectrum`. The heavy work (one 1024-point FFT) happens at most
// ~43×/s on the audio thread and is gated by an `enabled` flag, so it costs
// essentially nothing when the visualizer is switched off in Settings.
// ---------------------------------------------------------------------------

const FFT_SIZE: usize = 1024;
pub(super) const SPECTRUM_BANDS: usize = 6;
// Loudness window (in ~dBFS) mapped onto the 0..1 bar range. Magnitudes quieter
// than DB_MIN read as an empty bar, DB_MAX (and above) as a full one. Widen or
// shift these if bars sit too low/high across your library.
const SPECTRUM_ATTACK: f32 = 0.75; // how quickly a bar rises toward a louder value
const SPECTRUM_DECAY: f32 = 0.18; // how slowly it falls back (springy feel)

// Lock-free hand-off of the latest band levels from the audio thread to the
// `player_spectrum` command: each band is an f32 kept as its bit pattern.
pub(crate) struct SpectrumShared {
    pub(super) enabled: AtomicBool,
    bands: [AtomicU32; SPECTRUM_BANDS],
}

impl SpectrumShared {
    pub(super) fn new() -> Self {
        SpectrumShared {
            enabled: AtomicBool::new(true),
            bands: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }
    fn store(&self, vals: &[f32; SPECTRUM_BANDS]) {
        for (slot, v) in self.bands.iter().zip(vals) {
            slot.store(v.to_bits(), Ordering::Relaxed);
        }
    }
    pub(super) fn load(&self) -> [f32; SPECTRUM_BANDS] {
        std::array::from_fn(|i| f32::from_bits(self.bands[i].load(Ordering::Relaxed)))
    }
    pub(crate) fn reset(&self) {
        for slot in &self.bands {
            slot.store(0, Ordering::Relaxed);
        }
    }
}

// In-place iterative radix-2 Cooley–Tukey FFT (`len` must be a power of two).
fn fft_in_place(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    // Butterflies.
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wcos, wsin) = (ang.cos(), ang.sin());
        let half = len / 2;
        let mut i = 0;
        while i < n {
            let mut wr = 1.0f32;
            let mut wi = 0.0f32;
            for k in 0..half {
                let a = i + k;
                let b = a + half;
                let vr = re[b] * wr - im[b] * wi;
                let vi = re[b] * wi + im[b] * wr;
                re[b] = re[a] - vr;
                im[b] = im[a] - vi;
                re[a] += vr;
                im[a] += vi;
                let nwr = wr * wcos - wi * wsin;
                wi = wr * wsin + wi * wcos;
                wr = nwr;
            }
            i += len;
        }
        len <<= 1;
    }
}

pub(crate) struct SpectrumSource<S> {
    inner: S,
    shared: Arc<SpectrumShared>,
    player_gen: Arc<AtomicU64>,
    my_gen: u64,
    channels: u16,
    frame_sum: f32,
    frame_ch: u16,
    buf: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    hann: Vec<f32>,
    band_bins: [(usize, usize); SPECTRUM_BANDS],
    smoothed: [f32; SPECTRUM_BANDS],
}

impl<S: Source> SpectrumSource<S> {
    pub(crate) fn new(
        inner: S,
        shared: Arc<SpectrumShared>,
        player_gen: Arc<AtomicU64>,
        my_gen: u64,
    ) -> Self {
        let channels = inner.channels().max(1);
        let sr = inner.sample_rate().max(1) as f32;
        // Six log-spaced bands spanning sub-bass → presence.
        let edges = [40.0f32, 160.0, 400.0, 1000.0, 2600.0, 6000.0, 14000.0];
        let bin_of =
            |hz: f32| ((hz * FFT_SIZE as f32 / sr).round() as usize).clamp(1, FFT_SIZE / 2 - 1);
        let band_bins = std::array::from_fn(|b| {
            let lo = bin_of(edges[b]);
            let hi = bin_of(edges[b + 1]).max(lo);
            (lo, hi)
        });
        let hann = (0..FFT_SIZE)
            .map(|i| {
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE as f32 - 1.0)).cos()
            })
            .collect();
        SpectrumSource {
            inner,
            shared,
            player_gen,
            my_gen,
            channels,
            frame_sum: 0.0,
            frame_ch: 0,
            buf: Vec::with_capacity(FFT_SIZE),
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            hann,
            band_bins,
            smoothed: [0.0; SPECTRUM_BANDS],
        }
    }

    // Reduce one full window to six smoothed band levels and publish them.
    fn analyze(&mut self) {
        let n = FFT_SIZE;
        let mut energy = 0.0f32;
        for i in 0..n {
            let x = self.buf[i];
            energy += x * x;
            self.re[i] = x * self.hann[i];
            self.im[i] = 0.0;
        }
        let rms = (energy / n as f32).sqrt();
        fft_in_place(&mut self.re, &mut self.im);

        // Equalization gains to compensate for high-frequency roll-off (pink noise nature)
        // and make all bands react with satisfying height.
        let gains = [1.6f32, 2.2, 2.8, 3.4, 4.0, 4.6];

        let mut targets = [0.0f32; SPECTRUM_BANDS];
        if rms > 1e-4 {
            for (b, &(lo, hi)) in self.band_bins.iter().enumerate() {
                let mut peak = 0.0f32;
                for bin in lo..=hi {
                    let mag = (self.re[bin] * self.re[bin] + self.im[bin] * self.im[bin]).sqrt();
                    if mag > peak {
                        peak = mag;
                    }
                }
                let norm = peak / (n as f32 * 0.5);
                targets[b] = (norm * gains[b]).clamp(0.0, 1.0).sqrt();
            }
        }

        // Fast attack, slow decay for a lively-but-stable look.
        for (target, smoothed) in targets.iter().zip(&mut self.smoothed) {
            let coeff = if target > smoothed {
                SPECTRUM_ATTACK
            } else {
                SPECTRUM_DECAY
            };
            *smoothed += (target - *smoothed) * coeff;
        }
        self.shared.store(&self.smoothed);
    }
}

impl<S: Source> Iterator for SpectrumSource<S> {
    type Item = f32;
    #[inline]
    fn next(&mut self) -> Option<f32> {
        let s = self.inner.next()?;
        if self.shared.enabled.load(Ordering::Relaxed)
            && self.my_gen == self.player_gen.load(Ordering::Relaxed)
        {
            self.frame_sum += s;
            self.frame_ch += 1;
            if self.frame_ch >= self.channels {
                self.buf.push(self.frame_sum / self.channels as f32);
                self.frame_sum = 0.0;
                self.frame_ch = 0;
                if self.buf.len() >= FFT_SIZE {
                    self.analyze();
                    self.buf.clear();
                }
            }
        }
        Some(s)
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S: Source> Source for SpectrumSource<S> {
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
        // Drop the half-filled window so the bars don't glitch across a seek.
        self.buf.clear();
        self.frame_sum = 0.0;
        self.frame_ch = 0;
        self.inner.try_seek(pos)
    }
}
