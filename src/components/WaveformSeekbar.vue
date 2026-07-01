<script setup>
import { ref, watch, onMounted, onBeforeUnmount } from 'vue';

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
});
const emit = defineEmits(['input', 'commit']);

const PLAYED = '#f3b641'; // gold — already listened
const UNPLAYED = '#4a90e2'; // blue — upcoming

const canvas = ref(null);
let dragging = false;
let ro = null;
let raf = null;
let growth = 1; // bar-height multiplier for the rise animation

function draw() {
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
    ctx.fillStyle = x + drawW / 2 <= playedX ? PLAYED : UNPLAYED;
    ctx.fillRect(x, h - bh, drawW, bh); // bottom-anchored
  }
}

// Grow the bars up from the baseline (used when a track's peaks first load, and
// when the waveform is toggled on with peaks already cached).
function animateGrowth() {
  if (raf) cancelAnimationFrame(raf);
  growth = 0;
  const dur = 460;
  const t0 = performance.now();
  const step = (t) => {
    const p = Math.min(1, (t - t0) / dur);
    growth = 1 - Math.pow(1 - p, 3); // easeOutCubic
    draw();
    if (p < 1) raf = requestAnimationFrame(step);
    else raf = null;
  };
  raf = requestAnimationFrame(step);
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

watch(() => [props.current, props.duration], draw);
watch(
  () => props.peaks,
  (next) => {
    if (next && next.length) animateGrowth();
    else draw();
  }
);

onMounted(() => {
  ro = new ResizeObserver(draw);
  if (canvas.value) ro.observe(canvas.value);
  if (props.peaks && props.peaks.length) animateGrowth();
  else draw();
});

onBeforeUnmount(() => {
  if (raf) cancelAnimationFrame(raf);
  if (ro) ro.disconnect();
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
