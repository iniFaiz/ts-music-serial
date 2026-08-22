<script setup>
import { ref, watch, onMounted, onUnmounted } from 'vue';
import { store } from '../store';
import { invokeCommand as invoke } from '../generated/ipc';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import { nyancatRainbowRgb } from '../nyancatTheme';

// Real-time 7-bar spectrum visualizer. The backend analyzes audio into 6 bands;
// we map these onto a beautiful 7-bar Apple-style equalizer.
// When paused/stopped, the bars smoothly settle to a premium static wave icon
// instead of flat lines.
const BAR_COUNT = 7;
const PLAYING_ENVELOPE = [0.7, 0.85, 1.0, 0.9, 0.8, 0.65, 0.5]; // preserves the curved shape at peak levels
const canvasRef = ref(null);

// Plain JS arrays — no Vue reactivity overhead here!
const heights = new Array(BAR_COUNT).fill(0.0);
const targets = new Array(BAR_COUNT).fill(0.0);

let rafId = null;
let isVisible = null; // Unset initially to force the first updateVisibilityState to run
let unlistenSpectrum = null;
let unlistenBlur = null;
let unlistenFocus = null;
// Set when the component unmounts. Lifecycle hooks must be registered
// synchronously during setup — calling onUnmounted after an await inside
// onMounted silently no-ops (no active instance), leaking every listener.
// This flag also detaches anything that resolves after an early unmount.
let disposed = false;
let nyancatColorMix = store.nyancatMode ? 1 : 0;
let reduceMotion = false;
let motionQuery = null;
const appWindow = getCurrentWindow();

// Linear interpolation to map 6 backend bands to 7 visualizer bars
const mapBandsTo7 = (vals) => {
  const mapped = new Array(7);
  for (let i = 0; i < 7; i++) {
    const frac = (i / 6) * 5; // maps 0..6 (7 items) to 0..5 (6 items)
    const idx = Math.floor(frac);
    const nextIdx = Math.min(idx + 1, 5);
    const weight = frac - idx;
    mapped[i] = (vals[idx] ?? 0) * (1 - weight) + (vals[nextIdx] ?? 0) * weight;
  }
  return mapped;
};

// Update active state based on visibility/minimize
const updateVisibilityState = (visible) => {
  if (visible === isVisible) return;
  isVisible = visible;

  // Tell Rust backend to enable/disable FFT computation
  invoke('player_set_spectrum_enabled', { enabled: visible && store.visualizerEnabled }).catch(
    () => {}
  );

  if (visible) {
    // Only resume animation loop if the player is playing, or if we need to settle to 0
    if (
      !rafId &&
      (store.isPlaying || (store.nyancatMode && !reduceMotion) || heights.some((h) => h > 0))
    ) {
      rafId = requestAnimationFrame(tick);
    }
  } else {
    // Stop animation loop completely to save CPU
    if (rafId) {
      cancelAnimationFrame(rafId);
      rafId = null;
    }
    // Instantly collapse heights to 0 and redraw
    for (let i = 0; i < BAR_COUNT; i++) {
      heights[i] = 0.0;
      targets[i] = 0.0;
    }
    draw();
  }
};

// Window-state events update the frontend immediately; Rust also verifies the
// native window state before emitting telemetry.
const checkWindowStatus = async () => {
  try {
    const minimized = await appWindow.isMinimized();
    const docHidden = document.visibilityState === 'hidden';
    // The window is visible only if it is NOT minimized AND document is NOT hidden
    const visible = !minimized && !docHidden;
    updateVisibilityState(visible);
  } catch {
    // ignore
  }
};

const draw = (now = performance.now()) => {
  const canvas = canvasRef.value;
  if (!canvas) return;
  const ctx = canvas.getContext('2d');
  if (!ctx) return;

  const width = 33;
  const height = 40;

  // Clear context
  ctx.clearRect(0, 0, width, height);
  const barWidth = 2.5;
  const gap = 2.5;
  const minHeight = 2.5; // Matches barWidth so that idle state is a perfect circle

  for (let i = 0; i < BAR_COUNT; i++) {
    const hFactor = heights[i];
    const barHeight = Math.max(minHeight, hFactor * height);
    const x = i * (barWidth + gap);
    const y = height - barHeight;

    if (nyancatColorMix > 0) {
      const rainbow = nyancatRainbowRgb(reduceMotion ? 0 : now, i, BAR_COUNT, 0.98, 0.62);
      const mixed = rainbow.map((channel) => Math.round(255 + (channel - 255) * nyancatColorMix));
      ctx.fillStyle = `rgb(${mixed[0]}, ${mixed[1]}, ${mixed[2]})`;
    } else {
      ctx.fillStyle = '#ffffff';
    }

    ctx.beginPath();
    ctx.roundRect(x, y, barWidth, barHeight, barWidth / 2);
    ctx.fill();
  }
};

const tick = (now) => {
  if (isVisible) {
    // We conditionally reschedule the next frame at the end of the tick
  } else {
    rafId = null;
    return;
  }

  if (!store.isPlaying) {
    // Drop all bars down to the minimum flat level when paused/stopped.
    for (let i = 0; i < BAR_COUNT; i++) {
      targets[i] = 0.0;
    }
  }

  // Ease each bar toward its target.
  const easeFactor = store.isPlaying ? 0.22 : 0.08;
  let hasChanged = false;
  const colorTarget = store.nyancatMode ? 1 : 0;
  const colorDiff = colorTarget - nyancatColorMix;
  let colorChanging = false;

  if (Math.abs(colorDiff) > 0.002) {
    nyancatColorMix += colorDiff * 0.1;
    colorChanging = true;
  } else {
    nyancatColorMix = colorTarget;
  }

  for (let i = 0; i < BAR_COUNT; i++) {
    const t = targets[i];
    const diff = t - heights[i];
    if (Math.abs(diff) > 1e-4) {
      heights[i] += diff * easeFactor;
      hasChanged = true;
    } else {
      heights[i] = t;
    }
  }

  // Draw the updated heights on the canvas
  draw(now);

  // Optimize: stop the requestAnimationFrame loop entirely once the bars have fully settled to 0
  // to avoid consuming any CPU while the player is paused/stopped.
  if (store.isPlaying || hasChanged || colorChanging || (store.nyancatMode && !reduceMotion)) {
    rafId = requestAnimationFrame(tick);
  } else {
    rafId = null;
  }
};

// Watch play state to restart the tick loop when music starts
watch(
  () => store.isPlaying,
  (playing) => {
    if (playing && isVisible && !rafId) {
      rafId = requestAnimationFrame(tick);
    }
  }
);

// Rainbow colors keep flowing even while playback is paused. Toggling the
// easter egg off lets the loop run only until the bars fade back to white.
watch(
  () => store.nyancatMode,
  (enabled) => {
    if (reduceMotion) {
      nyancatColorMix = enabled ? 1 : 0;
      draw(0);
      return;
    }
    if (isVisible && !rafId) rafId = requestAnimationFrame(tick);
  }
);

const onMotionPreferenceChange = (event) => {
  reduceMotion = event.matches;
  if (reduceMotion) {
    nyancatColorMix = store.nyancatMode ? 1 : 0;
    draw(0);
  } else if (store.nyancatMode && isVisible && !rafId) {
    rafId = requestAnimationFrame(tick);
  }
};

const setupCanvas = () => {
  const canvas = canvasRef.value;
  if (!canvas) return;
  const dpr = window.devicePixelRatio || 1;
  canvas.width = 33 * dpr;
  canvas.height = 40 * dpr;
  const ctx = canvas.getContext('2d');
  if (ctx) {
    ctx.scale(dpr, dpr);
  }
  draw();
};

// Tear-down must be registered synchronously in setup, not inside the async
// onMounted callback below (after awaits there is no active instance and the
// hook would never attach).
onUnmounted(() => {
  disposed = true;
  document.removeEventListener('visibilitychange', checkWindowStatus);
  if (motionQuery) motionQuery.removeEventListener('change', onMotionPreferenceChange);
  if (unlistenSpectrum) unlistenSpectrum();
  if (unlistenBlur) unlistenBlur();
  if (unlistenFocus) unlistenFocus();
  if (rafId) {
    cancelAnimationFrame(rafId);
    rafId = null;
  }
});

onMounted(async () => {
  motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
  reduceMotion = motionQuery.matches;
  motionQuery.addEventListener('change', onMotionPreferenceChange);
  setupCanvas();

  try {
    const unlisten = await listen('player-spectrum', (event) => {
      const vals = event.payload;
      if (!isVisible || !Array.isArray(vals)) return;
      const mapped = mapBandsTo7(vals);
      for (let i = 0; i < BAR_COUNT; i++) {
        targets[i] = (mapped[i] ?? 0) * PLAYING_ENVELOPE[i];
      }
      if (!rafId && store.isPlaying) rafId = requestAnimationFrame(tick);
    });
    if (disposed) {
      unlisten();
      return;
    }
    unlistenSpectrum = unlisten;
  } catch {
    // Spectrum telemetry is best-effort during development upgrades.
  }

  // Listen to visibilitychange event
  document.addEventListener('visibilitychange', checkWindowStatus);

  // Listen to Tauri blur/focus events for prompt responsiveness
  try {
    const blur = await appWindow.listen('tauri://blur', checkWindowStatus);
    const focus = await appWindow.listen('tauri://focus', checkWindowStatus);
    if (disposed) {
      blur();
      focus();
      return;
    }
    unlistenBlur = blur;
    unlistenFocus = focus;
  } catch {
    // ignore
  }

  // Initial check
  await checkWindowStatus();
  if (disposed) return;

  // If initial check starts in active playback, make sure loop runs
  if ((store.isPlaying || (store.nyancatMode && !reduceMotion)) && isVisible && !rafId) {
    rafId = requestAnimationFrame(tick);
  }
});
</script>

<template>
  <canvas
    ref="canvasRef"
    class="mr-3 shrink-0 translate-y-[-16px] hidden md:block"
    :class="{ 'nyancat-visualizer-glow': store.nyancatMode }"
    style="width: 33px; height: 55px"
    :title="store.isPlaying ? 'Now playing' : 'Audio visualizer'"
    aria-hidden="true"
  ></canvas>
</template>

<style scoped>
canvas {
  filter: drop-shadow(0 0 0 rgba(50, 210, 255, 0));
  transition: filter 0.75s cubic-bezier(0.22, 1, 0.36, 1);
}

.nyancat-visualizer-glow {
  filter: drop-shadow(0 0 3px rgba(50, 220, 255, 0.82))
    drop-shadow(0 0 6px rgba(192, 70, 255, 0.52));
}

@media (prefers-reduced-motion: reduce) {
  canvas {
    transition-duration: 0.01ms;
  }
}
</style>
