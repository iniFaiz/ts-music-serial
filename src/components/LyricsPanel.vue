<script setup>
import { ref, computed, watch } from 'vue';
import { store } from '../store';
import { activeLineIndex, processLyricLines } from '../lyricsCache';
import { useTrackLyrics } from '../useTrackLyrics';
import { useLyricAutoScroll } from '../useLyricAutoScroll';
import { gapDotColor } from '../lyricVisuals';
import LyricContent from './LyricContent.vue';

const { lyrics, lyricsLoading, fetchLyrics } = useTrackLyrics({
  song: () => store.currentSong,
  active: () => store.lyricsPanelOpen,
  source: () => store.lyricsSource,
});
const lyricsState = computed(() => {
  if (lyricsLoading.value) return 'loading';
  return lyrics.value === undefined ? 'idle' : 'done';
});

// Whether the current lyrics carry a romanization (enables the romaji toggle).
const hasRomaji = computed(() => !!(lyrics.value && lyrics.value.has_romaji));

const currentTimeMs = computed(
  () => (store.currentTime || 0) * 1000 + 50 + store.lyricsOffsetMs
);

const panelLines = computed(() => {
  const rawLines = (lyrics.value && lyrics.value.lines) || [];
  const isSynced = !!(lyrics.value && lyrics.value.synced);
  const durationMs = (store.duration || 0) * 1000;
  return processLyricLines(rawLines, isSynced, durationMs);
});

const hasLyrics = computed(() => {
  if (!lyrics.value) return false;
  if (lyrics.value.synced) {
    return panelLines.value.length > 0;
  } else {
    return lyrics.value.lines && lyrics.value.lines.length > 0;
  }
});

const songDurationMs = computed(() => (store.duration || 0) * 1000);

const activeIdx = computed(() => {
  if (!lyrics.value || !lyrics.value.synced) return -1;
  return activeLineIndex(panelLines.value, currentTimeMs.value, songDurationMs.value);
});

function getDotColor(line, dotIdx) {
  return gapDotColor(line, dotIdx, currentTimeMs.value);
}

// ---- Smooth scroll (shared engine) ----------------------------------------

const scrollRef = ref(null);

const { resetScrollState, onUserScroll } = useLyricAutoScroll({
  container: () => scrollRef.value,
  lines: panelLines,
  activeIdx,
  scrollDuration: 600,
  gapTargetRem: 2.2,
});

// Reset scroll state on song/lyrics change
watch(
  () => [store.currentSong?.path, lyrics.value],
  () => resetScrollState()
);

// ---- Seek on click -------------------------------------------------------

function seekToLine(line) {
  if (line.time_ms != null) store.seek(line.time_ms / 1000);
}
</script>

<template>
  <Transition name="slide">
    <aside
      v-if="store.lyricsPanelOpen"
      class="group absolute top-0 right-0 h-full w-80 bg-[#161616] border-l border-[var(--border-color)] flex flex-col shadow-2xl z-40"
      style="view-transition-name: lyrics-panel"
    >
      <!-- Romaji toggle: revealed on hover, top-right (Apple Music style) -->
      <button
        v-if="hasRomaji"
        @click="store.toggleRomaji()"
        class="absolute top-3 right-3 z-50 flex items-center justify-center w-8 h-8 rounded-full text-white opacity-0 group-hover:opacity-100 transition-all duration-200 active:scale-95"
        :class="store.showRomaji ? 'bg-white/25' : 'bg-white/10 hover:bg-white/20'"
        :title="store.showRomaji ? 'Hide romaji' : 'Show romaji'"
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="15"
          height="15"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="m5 8 6 6" />
          <path d="m4 14 6-6 2-3" />
          <path d="M2 5h12" />
          <path d="M7 2h1" />
          <path d="m22 22-5-10-5 10" />
          <path d="M14 18h6" />
        </svg>
      </button>

      <div
        ref="scrollRef"
        class="flex-1 overflow-y-auto px-5 lyrics-scroll"
        @scroll.passive="onUserScroll"
      >
        <!-- Loading -->
        <div v-if="lyricsState === 'loading'" class="flex items-center justify-center h-full">
          <svg
            class="animate-spin text-gray-700"
            xmlns="http://www.w3.org/2000/svg"
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="none"
          >
            <circle
              class="opacity-20"
              cx="12"
              cy="12"
              r="9"
              stroke="currentColor"
              stroke-width="3"
            ></circle>
            <path
              class="opacity-80"
              fill="currentColor"
              d="M12 3a9 9 0 0 1 9 9h-3a6 6 0 0 0-6-6V3z"
            ></path>
          </svg>
        </div>

        <!-- Synced lyrics -->
        <div v-else-if="hasLyrics && lyrics && lyrics.synced" class="py-[45%]">
          <div
            v-for="(line, i) in panelLines"
            :key="i"
            :data-line="i"
            role="button"
            tabindex="0"
            @click="seekToLine(line)"
            @keydown.enter="seekToLine(line)"
            @keydown.space.prevent="seekToLine(line)"
            class="lp-line cursor-pointer focus:outline-none focus-visible:underline"
            :class="[
              i === activeIdx ? 'lp-active' : 'lp-dim',
              line.isGap ? 'lp-line-gap' : '',
              line.words && line.words.length ? 'lp-words' : '',
            ]"
          >
            <span
              v-if="line.isGap"
              class="lp-gap-dots"
              :class="{ 'lp-gap-dots-active': i === activeIdx }"
            >
              <span class="dots-wrapper">
                <span
                  :style="{
                    color: i === activeIdx ? getDotColor(line, 0) : 'rgba(255,255,255,0.2)',
                  }"
                  >•</span
                >
                <span
                  :style="{
                    color: i === activeIdx ? getDotColor(line, 1) : 'rgba(255,255,255,0.2)',
                  }"
                  >•</span
                >
                <span
                  :style="{
                    color: i === activeIdx ? getDotColor(line, 2) : 'rgba(255,255,255,0.2)',
                  }"
                  >•</span
                >
              </span>
            </span>
            <LyricContent
              v-else
              :line="line"
              :active="i === activeIdx"
              :is-past="i < activeIdx"
              :current-ms="i === activeIdx || i === activeIdx - 1 ? currentTimeMs : 0"
              :show-romaji="store.showRomaji"
            />
          </div>
        </div>

        <!-- Plain lyrics -->
        <div v-else-if="hasLyrics && lyrics && !lyrics.synced" class="py-[45%]">
          <div
            v-for="(line, i) in lyrics.lines"
            :key="i"
            class="lp-line lp-active"
            :class="line.text === '' ? 'mt-5' : ''"
          >
            {{ line.text || '\u00A0' }}
          </div>
        </div>

        <!-- Not found -->
        <div
          v-else-if="lyricsState === 'done' && (!lyrics || !hasLyrics)"
          class="flex flex-col items-center justify-center h-full gap-3 text-center px-4"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="26"
            height="26"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            class="text-gray-700"
          >
            <path
              d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"
            />
          </svg>
          <p class="text-xs text-gray-600">{{ $t('lyricsPanel.noLyrics') }}</p>
          <button
            @click="fetchLyrics(true)"
            class="text-[11px] text-gray-500 hover:text-white transition-colors"
          >
            {{ $t('common.retry') }}
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
  mask-image: linear-gradient(to bottom, transparent 0%, black 12%, black 88%, transparent 100%);
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
    color 0.45s cubic-bezier(0.25, 1, 0.5, 1),
    opacity 0.45s cubic-bezier(0.25, 1, 0.5, 1),
    transform 0.5s cubic-bezier(0.34, 1.56, 0.64, 1);
  transform-origin: left center;
}

/* Word-by-word (karaoke) lines drive their brightness with the gradient wipe, so
   entering the active state must NOT also ride the slow opacity ramp: transitioning
   opacity up from 0.22 while the dim (unsung, 34%) fill is shown would dip the text
   darker than the line it replaced before brightening — the "dark flash". A CSS
   transition is governed by the *destination* state, so we strip opacity only on
   the active state (snap in, no dip). The base .lp-line keeps opacity in its
   transition, so leaving active (→ .lp-dim) still fades the finished line out. */
.lp-line.lp-words.lp-active {
  transition:
    color 0.45s cubic-bezier(0.25, 1, 0.5, 1),
    transform 0.5s cubic-bezier(0.34, 1.56, 0.64, 1);
}

/* Active line: full white, nudged slightly right, very subtle scale pop */
.lp-active {
  color: rgba(255, 255, 255, 0.96);
  opacity: 1;
  transform: translateX(4px) scale(1.015);
}

/* Dim lines */
.lp-dim {
  color: rgba(255, 255, 255, 0.96);
  opacity: 0.22;
  transform: translateX(0) scale(1);
}
.lp-dim:hover {
  opacity: 0.5;
}

.lp-gap-dots {
  opacity: 0;
  transition: opacity 0.35s ease;
  pointer-events: none;
  display: inline-block;
}

.lp-gap-dots-active {
  opacity: 1;
  pointer-events: auto;
}

.dots-wrapper {
  display: inline-flex;
  gap: 0.35rem;
  font-size: 1.5rem;
  line-height: 1;
  vertical-align: middle;
  font-weight: 800;
}

.dots-wrapper span {
  transition: color 0.25s linear;
}

.lp-line-gap {
  height: 0;
  margin: 0 !important;
  padding: 0 !important;
  opacity: 0;
  overflow: hidden;
  transition:
    height 0.4s cubic-bezier(0.25, 1, 0.5, 1),
    margin 0.4s cubic-bezier(0.25, 1, 0.5, 1),
    opacity 0.4s cubic-bezier(0.25, 1, 0.5, 1);
}

.lp-line-gap.lp-active {
  height: 2.2rem;
  opacity: 1;
}
</style>
