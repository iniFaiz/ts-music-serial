<script setup>
import { ref, watch, onMounted, onBeforeUnmount } from 'vue';
import { nyancatRainbowRgb } from '../nyancatTheme';

// Amplitude waveform that acts as the seek bar (see the reference: thin bottom-
// anchored bars, the already-played portion in warm gold and the upcoming part
// in blue). Fills its parent, so the parent controls the size/placement; here it
// lives inside a fixed-height seek track and cross-fades with the plain slider.
// Click or drag anywhere to seek.
//
// Emits `input` (seconds) live while scrubbing and `commit` (seconds) on release,
// so PlayerControls reuses its existing onSeekInput/onSeekCommit handlers.
//
// When a track's peaks arrive the bars rise up from the baseline (JS `growth`),
// which — together with the parent's opacity cross-fade — gives the toggle-on a
// lively "build up" feel.
const props = defineProps({
  peaks: { type: Object, default: null }, // Uint8Array (0..255) or null
  current: { type: Number, default: 0 }, // seconds
  duration: { type: Number, default: 0 }, // seconds
  disabled: { type: Boolean, default: false },
  nyancat: { type: Boolean, default: false },
});
const emit = defineEmits(['input', 'commit']);

const PLAYED = '#f3b641'; // gold — already listened
const UNPLAYED = '#4a90e2'; // blue — upcoming

const canvas = ref(null);
let dragging = false;
let ro = null;
let growthRaf = null;
let nyancatRaf = null;
let growth = 1; // bar-height multiplier for the rise animation
let nyancatMix = props.nyancat ? 1 : 0;
let reduceMotion = false;
let motionQuery = null;

function mixedColor(base, target, amount) {
  const channels = base.map((value, index) =>
    Math.round(value + (target[index] - value) * amount)
  );
  return `rgb(${channels[0]}, ${channels[1]}, ${channels[2]})`;
}

// Each bar receives a different point on the Nyan Cat rainbow. Moving that
// phase produces a continuous color trail across the entire waveform.
function nyancatColor(played, index, count, time) {
  const base = played ? [243, 182, 65] : [74, 144, 226];
  if (nyancatMix <= 0) return played ? PLAYED : UNPLAYED;
  const target = nyancatRainbowRgb(
    time,
    index,
    count,
    played ? 0.98 : 0.82,
    played ? 0.62 : 0.44
  );
  return mixedColor(base, target, nyancatMix);
}

function draw(time = performance.now()) {
  const cv = canvas.value;
  if (!cv) return;
  const ctx = cv.getContext('2d');
  if (!ctx) return;
  const dpr = window.devicePixelRatio || 1;
  const w = cv.clientWidth;
  const h = cv.clientHeight;
  if (w === 0 || h === 0) return;
  if (cv.width !== Math.round(w * dpr) || cv.height !== Math.round(h * dpr)) {
    cv.width = Math.round(w * dpr);
    cv.height = Math.round(h * dpr);
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);

  const peaks = props.peaks;
  if (!peaks || peaks.length === 0) {
    // Loading placeholder: a dim baseline until the peaks arrive.
    ctx.fillStyle = 'rgba(255,255,255,0.06)';
    ctx.fillRect(0, h - 2, w, 2);
    return;
  }

  // Downsample to roughly one bar per ~3px so bars stay thin with clear gaps
  // regardless of the player width.
  const n = Math.max(48, Math.min(peaks.length, Math.floor(w / 3)));
  const slot = w / n;
  const drawW = Math.max(1.5, slot * 0.6);
  const frac = props.duration > 0 ? Math.min(1, Math.max(0, props.current / props.duration)) : 0;
  const playedX = frac * w;

  for (let i = 0; i < n; i++) {
    const s = Math.floor((i * peaks.length) / n);
    const e = Math.max(s + 1, Math.floor(((i + 1) * peaks.length) / n));
    let p = 0;
    for (let j = s; j < e; j++) if (peaks[j] > p) p = peaks[j];
    const bh = Math.max(2, (p / 255) * (h - 1) * growth);
    const x = i * slot + (slot - drawW) / 2;
    const played = x + drawW / 2 <= playedX;
    ctx.fillStyle = nyancatColor(played, i, n, time);
    ctx.fillRect(x, h - bh, drawW, bh); // bottom-anchored
  }
}

function loopNyancat(time) {
  draw(time);
  if (props.nyancat && !reduceMotion) nyancatRaf = requestAnimationFrame(loopNyancat);
  else nyancatRaf = null;
}

function animateNyancat(next) {
  if (nyancatRaf) cancelAnimationFrame(nyancatRaf);
  nyancatRaf = null;

  const from = nyancatMix;
  const to = next ? 1 : 0;
  if (reduceMotion || from === to) {
    nyancatMix = to;
    draw();
    if (next && !reduceMotion) nyancatRaf = requestAnimationFrame(loopNyancat);
    return;
  }

  const startedAt = performance.now();
  const duration = 900;
  const step = (time) => {
    const progress = Math.min(1, (time - startedAt) / duration);
    const eased = 1 - Math.pow(1 - progress, 3);
    nyancatMix = from + (to - from) * eased;
    draw(time);
    if (progress < 1) {
      nyancatRaf = requestAnimationFrame(step);
    } else if (next && !reduceMotion) {
      nyancatRaf = requestAnimationFrame(loopNyancat);
    } else {
      nyancatRaf = null;
    }
  };
  nyancatRaf = requestAnimationFrame(step);
}

// Grow the bars up from the baseline (used when a track's peaks first load, and
// when the waveform is toggled on with peaks already cached).
function animateGrowth() {
  if (growthRaf) cancelAnimationFrame(growthRaf);
  growth = 0;
  const dur = 460;
  const t0 = performance.now();
  const step = (t) => {
    const p = Math.min(1, (t - t0) / dur);
    growth = 1 - Math.pow(1 - p, 3); // easeOutCubic
    draw(t);
    if (p < 1) growthRaf = requestAnimationFrame(step);
    else growthRaf = null;
  };
  growthRaf = requestAnimationFrame(step);
}

function fracFromEvent(e) {
  const cv = canvas.value;
  if (!cv) return 0;
  const rect = cv.getBoundingClientRect();
  if (rect.width === 0) return 0;
  return Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
}
const toSeconds = (frac) => frac * (props.duration || 0);

function onDown(e) {
  if (props.disabled || !props.peaks) return;
  dragging = true;
  try {
    canvas.value.setPointerCapture(e.pointerId);
  } catch {
    /* ignore */
  }
  emit('input', toSeconds(fracFromEvent(e)));
}
function onMove(e) {
  if (!dragging) return;
  emit('input', toSeconds(fracFromEvent(e)));
}
function onUp(e) {
  if (!dragging) return;
  dragging = false;
  emit('commit', toSeconds(fracFromEvent(e)));
}

watch(
  () => [props.current, props.duration],
  () => draw()
);
watch(
  () => props.nyancat,
  (next) => animateNyancat(next)
);
watch(
  () => props.peaks,
  (next) => {
    if (next && next.length) animateGrowth();
    else draw();
  }
);

const onMotionPreferenceChange = (event) => {
  reduceMotion = event.matches;
  animateNyancat(props.nyancat);
};

onMounted(() => {
  motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
  reduceMotion = motionQuery.matches;
  motionQuery.addEventListener('change', onMotionPreferenceChange);
  ro = new ResizeObserver(() => draw());
  if (canvas.value) ro.observe(canvas.value);
  if (props.peaks && props.peaks.length) animateGrowth();
  else draw();
  if (props.nyancat) animateNyancat(true);
});

onBeforeUnmount(() => {
  if (growthRaf) cancelAnimationFrame(growthRaf);
  if (nyancatRaf) cancelAnimationFrame(nyancatRaf);
  if (ro) ro.disconnect();
  if (motionQuery) motionQuery.removeEventListener('change', onMotionPreferenceChange);
});
</script>

<template>
  <canvas
    ref="canvas"
    class="w-full h-full cursor-pointer touch-none select-none"
    :class="{ 'opacity-30 pointer-events-none': disabled }"
    @pointerdown="onDown"
    @pointermove="onMove"
    @pointerup="onUp"
    @pointercancel="onUp"
  />
</template>
