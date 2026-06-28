// Shared constants + presets for the 10-band graphic equalizer. The actual DSP
// runs in Rust (an EqualizerSource filter in the decoder→sink chain); this file
// only describes the band layout and preset gains the UI/store work with.

// ISO 1/1-octave center frequencies, matching EQ_FREQS in src-tauri/src/lib.rs.
export const EQ_FREQS = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

// Short labels for the slider column.
export const EQ_FREQ_LABELS = ['31', '62', '125', '250', '500', '1K', '2K', '4K', '8K', '16K'];

export const EQ_BAND_COUNT = EQ_FREQS.length;
export const EQ_MIN_DB = -12;
export const EQ_MAX_DB = 12;

// Preset gain curves (dB per band, low → high). `flat` doubles as the reset
// target. Order here is the order the preset pills are rendered in.
export const EQ_PRESETS = {
  flat: { label: 'Flat', gains: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
  rock: { label: 'Rock', gains: [5, 4, 3, 1, -1, -1, 1, 3, 4, 5] },
  pop: { label: 'Pop', gains: [-1, 1, 3, 4, 3, 1, 0, -1, -1, -1] },
  jazz: { label: 'Jazz', gains: [4, 3, 1, 2, -1, -1, 0, 1, 2, 3] },
  classical: { label: 'Classical', gains: [4, 3, 2, 1, -1, -1, 0, 2, 3, 4] },
  dance: { label: 'Dance', gains: [6, 5, 2, 0, -1, -2, 1, 3, 4, 4] },
  bass: { label: 'Bass Boost', gains: [7, 6, 5, 3, 1, 0, 0, 0, 0, 0] },
  treble: { label: 'Treble Boost', gains: [0, 0, 0, 0, 0, 1, 3, 5, 6, 7] },
  vocal: { label: 'Vocal', gains: [-2, -1, 0, 2, 4, 4, 3, 1, 0, -1] },
  acoustic: { label: 'Acoustic', gains: [4, 3, 2, 1, 2, 2, 3, 3, 2, 1] },
  loudness: { label: 'Loudness', gains: [6, 4, 0, 0, -2, 0, 0, -2, 4, 6] },
};

// List form for v-for in the panel.
export const EQ_PRESET_LIST = Object.entries(EQ_PRESETS).map(([id, p]) => ({
  id,
  label: p.label,
}));

// Find the preset id whose curve matches `gains` exactly, or 'custom'.
export function matchPreset(gains) {
  for (const [id, p] of Object.entries(EQ_PRESETS)) {
    if (p.gains.every((g, i) => g === gains[i])) return id;
  }
  return 'custom';
}
