<script setup>
import { ref, onMounted, onUnmounted } from 'vue';
import { store } from '../store';
import { invoke } from '@tauri-apps/api/core';

// Real-time 7-bar spectrum visualizer. The backend analyzes audio into 6 bands;
// we map these onto a beautiful 7-bar Apple-style equalizer.
// When paused/stopped, the bars smoothly settle to a premium static wave icon
// instead of flat lines.
const BAR_COUNT = 7;
const PLAYING_ENVELOPE = [0.7, 0.85, 1.0, 0.9, 0.8, 0.65, 0.5]; // preserves the curved shape at peak levels
const POLL_MS = 33; // ~30 Hz backend polling; rendering still runs at full rAF rate

const heights = ref(Array(BAR_COUNT).fill(0.0)); // start flat (all the way down)
const targets = new Array(BAR_COUNT).fill(0.0);  // start targets flat

let rafId = null;
let inflight = false;
let lastPoll = 0;

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

const tick = (now) => {
  rafId = requestAnimationFrame(tick);

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

  // Ease each bar toward its target with a low-pass easing (0.18) for a liquid, non-jittery flow.
  const h = heights.value;
  for (let i = 0; i < BAR_COUNT; i++) {
    const t = targets[i];
    h[i] += (t - h[i]) * 0.18;
  }
};

onMounted(() => {
  rafId = requestAnimationFrame(tick);
});

onUnmounted(() => {
  if (rafId) cancelAnimationFrame(rafId);
});
</script>

<template>
  <div
    class="flex items-end gap-[3px] h-[30px] shrink-0 mr-3 translate-y-[-3px]"
    :title="store.isPlaying ? 'Now playing' : 'Audio visualizer'"
    aria-hidden="true"
  >
    <div
      v-for="(h, i) in heights"
      :key="i"
      class="w-[2px] rounded-full bg-white origin-bottom"
      :style="{ height: Math.max(15, Math.min(100, h * 100)) + '%' }"
    ></div>
  </div>
</template>
