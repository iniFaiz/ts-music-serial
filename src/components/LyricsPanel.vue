<script setup>
import { ref, computed, watch, nextTick, onUnmounted } from 'vue';
import { store } from '../store';
import { loadLyrics, activeLineIndex } from '../lyricsCache';

const lyrics = ref(null);
const lyricsState = ref('idle');

async function fetchLyrics(force = false) {
  const song = store.currentSong;
  if (!song) { lyrics.value = null; lyricsState.value = 'idle'; return; }
  lyricsState.value = 'loading';
  const result = await loadLyrics(song, { force });
  lyrics.value = result;
  lyricsState.value = 'done';
}

watch(() => store.lyricsPanelOpen, (open) => {
  if (open && lyricsState.value === 'idle') fetchLyrics();
});

watch(() => store.currentSong, () => {
  lyrics.value = null;
  lyricsState.value = 'idle';
  if (store.lyricsPanelOpen) fetchLyrics();
});

watch(() => store.lyricsSource, () => {
  lyrics.value = null;
  lyricsState.value = 'idle';
  if (store.lyricsPanelOpen) fetchLyrics(true);
});

// +50ms lookahead: compensates for the ~50ms average lag from the 100ms poll interval
const currentTimeMs = computed(() => Math.floor(store.currentTime * 1000) + 50);

const activeIdx = computed(() => {
  if (!lyrics.value || !lyrics.value.synced) return -1;
  return activeLineIndex(lyrics.value.lines, currentTimeMs.value);
});

// ---- Smooth scroll --------------------------------------------------------

const scrollRef = ref(null);
let rafId = null;
let isAutoScrolling = false;   // true while our RAF animation is running
let userPausedUntil = 0;       // epoch ms — ignore auto-scroll until this time
let userScrollTimer = null;
let lastScrolledIdx = -1;

// easeInOutQuart — slow start, fast middle, slow end
function easeInOutQuart(t) {
  return t < 0.5 ? 8 * t * t * t * t : 1 - Math.pow(-2 * t + 2, 4) / 2;
}

function smoothScrollTo(container, targetTop, duration = 550) {
  if (rafId) cancelAnimationFrame(rafId);

  const start = container.scrollTop;
  const delta = targetTop - start;
  if (Math.abs(delta) < 2) return;

  const t0 = performance.now();
  isAutoScrolling = true;

  function step(now) {
    const elapsed = now - t0;
    const progress = Math.min(elapsed / duration, 1);
    container.scrollTop = start + delta * easeInOutQuart(progress);
    if (progress < 1) {
      rafId = requestAnimationFrame(step);
    } else {
      rafId = null;
      // Short grace period so the scroll-end event doesn't flip the flag yet
      setTimeout(() => { isAutoScrolling = false; }, 80);
    }
  }

  rafId = requestAnimationFrame(step);
}

function scrollToLine(idx) {
  const container = scrollRef.value;
  if (!container) return;
  const el = container.querySelector(`[data-line="${idx}"]`);
  if (!el) return;

  const containerH = container.clientHeight;
  const elTop = el.offsetTop;
  const elH = el.offsetHeight;
  // Center the active line vertically inside the scroll area
  const target = Math.max(0, elTop - containerH / 2 + elH / 2);
  smoothScrollTo(container, target, 600);
}

// Pause auto-scroll for 3 s when the user manually scrolls
function onScroll() {
  if (isAutoScrolling) return;
  userPausedUntil = Date.now() + 3000;
  if (userScrollTimer) clearTimeout(userScrollTimer);
  userScrollTimer = setTimeout(() => { userPausedUntil = 0; }, 3100);
}

// Trigger scroll whenever the active line changes
watch(activeIdx, async (idx) => {
  if (idx < 0 || idx === lastScrolledIdx) return;
  if (Date.now() < userPausedUntil) return; // user is in control
  lastScrolledIdx = idx;
  await nextTick();
  scrollToLine(idx);
});

// Reset scroll state on song/lyrics change
watch(() => [store.currentSong?.path, lyrics.value], () => {
  lastScrolledIdx = -1;
  userPausedUntil = 0;
});

onUnmounted(() => {
  if (rafId) cancelAnimationFrame(rafId);
  if (userScrollTimer) clearTimeout(userScrollTimer);
});

// ---- Seek on click -------------------------------------------------------

function seekToLine(line) {
  if (line.time_ms != null) store.seek(line.time_ms / 1000);
}
</script>

<template>
  <Transition name="slide">
    <aside
      v-if="store.lyricsPanelOpen"
      class="absolute top-0 right-0 h-full w-80 bg-[#161616] border-l border-[var(--border-color)] flex flex-col shadow-2xl z-40"
    >
      <div
        ref="scrollRef"
        class="flex-1 overflow-y-auto px-5 lyrics-scroll"
        @scroll.passive="onScroll"
      >
        <!-- Loading -->
        <div v-if="lyricsState === 'loading'" class="flex items-center justify-center h-full">
          <svg class="animate-spin text-gray-700" xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none">
            <circle class="opacity-20" cx="12" cy="12" r="9" stroke="currentColor" stroke-width="3"></circle>
            <path class="opacity-80" fill="currentColor" d="M12 3a9 9 0 0 1 9 9h-3a6 6 0 0 0-6-6V3z"></path>
          </svg>
        </div>

        <!-- Synced lyrics -->
        <div v-else-if="lyrics && lyrics.synced" class="py-[45%]">
          <div
            v-for="(line, i) in lyrics.lines"
            :key="i"
            :data-line="i"
            @click="seekToLine(line)"
            class="lp-line cursor-pointer"
            :class="i === activeIdx ? 'lp-active' : 'lp-dim'"
          >{{ line.text }}</div>
        </div>

        <!-- Plain lyrics -->
        <div v-else-if="lyrics && !lyrics.synced" class="py-[45%]">
          <div
            v-for="(line, i) in lyrics.lines"
            :key="i"
            class="lp-line lp-active"
            :class="line.text === '' ? 'mt-5' : ''"
          >{{ line.text || '\u00A0' }}</div>
        </div>

        <!-- Not found -->
        <div v-else-if="lyricsState === 'done' && !lyrics" class="flex flex-col items-center justify-center h-full gap-3 text-center px-4">
          <svg xmlns="http://www.w3.org/2000/svg" width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" class="text-gray-700">
            <path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z" />
          </svg>
          <p class="text-xs text-gray-600">Lyrics not found</p>
          <button @click="fetchLyrics(true)" class="text-[11px] text-gray-500 hover:text-white transition-colors">
            Try again
          </button>
        </div>
      </div>
    </aside>
  </Transition>
</template>

<style scoped>
.slide-enter-active,
.slide-leave-active {
  transition: transform 0.28s cubic-bezier(0.4, 0, 0.2, 1);
}
.slide-enter-from,
.slide-leave-to {
  transform: translateX(100%);
}

/* Soft fade at top/bottom so lines disappear gently into the edges */
.lyrics-scroll {
  scrollbar-width: thin;
  scrollbar-color: transparent transparent;
  mask-image: linear-gradient(
    to bottom,
    transparent 0%,
    black 12%,
    black 88%,
    transparent 100%
  );
}
.lyrics-scroll:hover {
  scrollbar-color: rgba(255, 255, 255, 0.14) transparent;
}
.lyrics-scroll::-webkit-scrollbar {
  width: 4px;
}
.lyrics-scroll::-webkit-scrollbar-track {
  background: transparent;
}
.lyrics-scroll::-webkit-scrollbar-thumb {
  background: transparent;
  border-radius: 4px;
  transition: background 0.25s ease;
}
.lyrics-scroll:hover::-webkit-scrollbar-thumb {
  background: rgba(255, 255, 255, 0.14);
}

/* Base line style */
.lp-line {
  font-size: 1.125rem;
  font-weight: 600;
  line-height: 1.65;
  letter-spacing: -0.01em;
  padding: 0.18rem 0;
  /* Animate color, opacity, and the subtle left nudge */
  transition:
    color      0.45s cubic-bezier(0.25, 1, 0.5, 1),
    opacity    0.45s cubic-bezier(0.25, 1, 0.5, 1),
    transform  0.5s  cubic-bezier(0.34, 1.56, 0.64, 1);
  transform-origin: left center;
}

/* Active line: full white, nudged slightly right, very subtle scale pop */
.lp-active {
  color: rgba(255, 255, 255, 0.96);
  opacity: 1;
  transform: translateX(4px) scale(1.015);
}

/* Dim lines */
.lp-dim {
  color: rgba(255, 255, 255, 0.22);
  opacity: 1;
  transform: translateX(0) scale(1);
}
.lp-dim:hover {
  color: rgba(255, 255, 255, 0.5);
}
</style>
