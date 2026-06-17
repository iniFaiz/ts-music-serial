<script setup>
import { ref, watch, onMounted, onUnmounted } from 'vue';
import { store } from '../store';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';

// Real-time 7-bar spectrum visualizer. The backend analyzes audio into 6 bands;
// we map these onto a beautiful 7-bar Apple-style equalizer.
// When paused/stopped, the bars smoothly settle to a premium static wave icon
// instead of flat lines.
const BAR_COUNT = 7;
const PLAYING_ENVELOPE = [0.7, 0.85, 1.0, 0.9, 0.8, 0.65, 0.5]; // preserves the curved shape at peak levels
const POLL_MS = 33; // ~30 Hz backend polling; rendering still runs at full rAF rate

const canvasRef = ref(null);

// Plain JS arrays — no Vue reactivity overhead here!
const heights = new Array(BAR_COUNT).fill(0.0);
const targets = new Array(BAR_COUNT).fill(0.0);

let rafId = null;
let inflight = false;
let lastPoll = 0;
let isVisible = null; // Unset initially to force the first updateVisibilityState to run
let pollTimer = null;
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
  invoke('player_set_spectrum_enabled', { enabled: visible && store.visualizerEnabled }).catch(() => {});

  if (visible) {
    // Only resume animation loop if the player is playing, or if we need to settle to 0
    if (!rafId && (store.isPlaying || heights.some(h => h > 0))) {
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

// Periodic check for window minimized state
const checkWindowStatus = async () => {
  try {
    const minimized = await appWindow.isMinimized();
    const docHidden = document.visibilityState === 'hidden';
    // The window is visible only if it is NOT minimized AND document is NOT hidden
    const visible = !minimized && !docHidden;
    updateVisibilityState(visible);
  } catch (e) {
    // ignore
  }
};

const draw = () => {
  const canvas = canvasRef.value;
  if (!canvas) return;
  const ctx = canvas.getContext('2d');
  if (!ctx) return;

  const width = 33;
  const height = 40;

  // Clear context
  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = '#ffffff';

  const barWidth = 2.5;
  const gap = 2.5;
  const minHeight = 2.5; // Matches barWidth so that idle state is a perfect circle

  for (let i = 0; i < BAR_COUNT; i++) {
    const hFactor = heights[i];
    const barHeight = Math.max(minHeight, hFactor * height);
    const x = i * (barWidth + gap);
    const y = height - barHeight;

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

  if (store.isPlaying) {
    // Throttle the IPC poll; a single request is kept in flight at a time.
    if (!inflight && now - lastPoll >= POLL_MS) {
      lastPoll = now;
      inflight = true;
      invoke('player_spectrum')
        .then((vals) => {
          if (Array.isArray(vals)) {
            const mapped = mapBandsTo7(vals);
            for (let i = 0; i < BAR_COUNT; i++) {
              targets[i] = (mapped[i] ?? 0) * PLAYING_ENVELOPE[i];
            }
          }
        })
        .catch(() => {})
        .finally(() => {
          inflight = false;
        });
    }
  } else {
    // Drop all bars down to the minimum flat level when paused/stopped.
    for (let i = 0; i < BAR_COUNT; i++) {
      targets[i] = 0.0;
    }
  }

  // Ease each bar toward its target.
  const easeFactor = store.isPlaying ? 0.22 : 0.08;
  let hasChanged = false;

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
  draw();

  // Optimize: stop the requestAnimationFrame loop entirely once the bars have fully settled to 0
  // to avoid consuming any CPU while the player is paused/stopped.
  if (store.isPlaying || hasChanged) {
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

onMounted(async () => {
  setupCanvas();

  // Listen to visibilitychange event
  document.addEventListener('visibilitychange', checkWindowStatus);

  // Listen to Tauri blur/focus events for prompt responsiveness
  let unlistenBlur, unlistenFocus;
  try {
    unlistenBlur = await appWindow.listen('tauri://blur', checkWindowStatus);
    unlistenFocus = await appWindow.listen('tauri://focus', checkWindowStatus);
  } catch (e) {
    // ignore
  }

  // Check state every 300ms as a fallback for OS-level minimization
  pollTimer = setInterval(checkWindowStatus, 300);

  // Initial check
  await checkWindowStatus();

  // If initial check starts in active playback, make sure loop runs
  if (store.isPlaying && isVisible && !rafId) {
    rafId = requestAnimationFrame(tick);
  }

  onUnmounted(() => {
    document.removeEventListener('visibilitychange', checkWindowStatus);
    if (pollTimer) clearInterval(pollTimer);
    if (unlistenBlur) unlistenBlur();
    if (unlistenFocus) unlistenFocus();
    if (rafId) {
      cancelAnimationFrame(rafId);
      rafId = null;
    }
  });
});
</script>

<template>
  <canvas
    ref="canvasRef"
    class="mr-3 shrink-0 translate-y-[-16px]"
    style="width: 33px; height: 55px;"
    :title="store.isPlaying ? 'Now playing' : 'Audio visualizer'"
    aria-hidden="true"
  ></canvas>
</template>
