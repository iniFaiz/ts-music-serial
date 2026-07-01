<script setup>
import { ref, computed, watch, nextTick, onMounted, onUnmounted } from 'vue';
import { store } from '../store';
import { loadCover, getCachedCover, hasCachedCover } from '../coverCache';
import { loadLyrics, activeLineIndex } from '../lyricsCache';
import { useRouter } from 'vue-router';
import LyricContent from './LyricContent.vue';
import { extractColorsForPath, defaultPalette } from '../colorExtract';

const router = useRouter();
const coverUrl = ref(null);
// undefined = loading, null = not found, object = resolved lyrics
const lyrics = ref(undefined);
const linesEl = ref(null);
const lyricsLoading = ref(false);
const showLyricsOption = ref(true);

const losslessPopupOpen = ref(false);

const isLossless = computed(() => {
  if (!store.currentSong || !store.currentSong.path) return false;
  const ext = store.currentSong.path.split('.').pop().toLowerCase();
  return ['flac', 'wav', 'alac', 'm4a'].includes(ext);
});

const formatLosslessSpecs = () => {
  if (!store.currentSong || !store.currentSong.path) return '24-bit 48kHz ALAC';
  const ext = store.currentSong.path.split('.').pop().toLowerCase();
  const bits = store.currentBitDepth || store.currentSong.bit_depth;
  const hz = store.currentSampleRate || store.currentSong.sample_rate;

  if (bits && hz) {
    const bitStr = `${bits}-bit`;
    const rateStr = hz >= 1000 ? `${(hz / 1000).toFixed(1).replace('.0', '')}kHz` : `${hz}Hz`;
    const codecStr = ext === 'm4a' ? 'ALAC' : ext.toUpperCase();
    return `${bitStr} ${rateStr} ${codecStr}`;
  }

  if (ext === 'flac') return '24-bit 48kHz FLAC';
  if (ext === 'wav') return '16-bit 44.1kHz WAV';
  return '24-bit 48kHz ALAC';
};

const closeLosslessPopup = () => {
  losslessPopupOpen.value = false;
};

const song = computed(() => store.currentSong);

const showLyricsColumn = computed(() => {
  return showLyricsOption.value;
});

async function resolveCover(path) {
  if (!path) {
    coverUrl.value = null;
    return;
  }
  if (hasCachedCover(path)) {
    coverUrl.value = getCachedCover(path);
    return;
  }
  const result = await loadCover(path);
  if (song.value && song.value.path === path) coverUrl.value = result;
}

async function fetchLyrics(force = false) {
  const current = song.value;
  if (!current) {
    lyrics.value = null;
    return;
  }
  lyricsLoading.value = true;
  lyrics.value = undefined;
  const res = await loadLyrics(current, { force });
  // Guard against the track changing while we awaited.
  if (song.value && song.value.path === current.path) {
    lyrics.value = res;
  }
  lyricsLoading.value = false;
}

// Load cover + lyrics when the overlay opens and whenever the track changes
// while it's open.
watch(
  () => store.fullscreenOpen,
  (open) => {
    losslessPopupOpen.value = false;
    if (open && song.value) {
      resolveCover(song.value.path);
      fetchLyrics();
    }
  }
);
watch(
  () => song.value && song.value.path,
  (path) => {
    losslessPopupOpen.value = false;
    if (store.fullscreenOpen && path) {
      resolveCover(path);
      fetchLyrics();
    }
  }
);
watch(
  () => store.lyricsSource,
  () => {
    lyrics.value = undefined;
    if (store.fullscreenOpen) {
      fetchLyrics(true);
    }
  }
);

const synced = computed(() => !!(lyrics.value && lyrics.value.synced));
// Whether the current lyrics carry a romanization (enables the romaji toggle).
const hasRomaji = computed(() => !!(lyrics.value && lyrics.value.has_romaji));

const lines = computed(() => {
  const rawLines = (lyrics.value && lyrics.value.lines) || [];
  if (!synced.value || rawLines.length === 0) {
    return rawLines;
  }

  const result = [];
  
  // 1. Check if there's an intro gap before the first line
  if (rawLines[0] && rawLines[0].time_ms > 6000) {
    result.push({
      isGap: true,
      time_ms: 2000,
      endTimeMs: rawLines[0].time_ms - 1000,
      text: '• • •'
    });
  }

  for (let i = 0; i < rawLines.length; i++) {
    const currentLine = rawLines[i];
    const textTrimmed = currentLine.text.trim();
    const isEmptyOrNote = textTrimmed === '' || textTrimmed === '♪' || textTrimmed === '🎵';

    if (isEmptyOrNote) {
      const nextLine = rawLines[i + 1];
      // Skip gap at end of song (no next line)
      if (!nextLine) continue;
      const gapStart = currentLine.time_ms;
      const gapEnd = nextLine.time_ms - 1000;
      if (gapEnd > gapStart) {
        result.push({
          isGap: true,
          time_ms: gapStart,
          endTimeMs: gapEnd,
          text: '• • •'
        });
      }
    } else {
      result.push(currentLine);
    }
  }

  return result;
});

// +50ms lookahead: compensates for the ~50ms average lag from the 100ms poll interval
const currentMs = computed(() => (store.currentTime || 0) * 1000 + 50 + store.lyricsOffsetMs);
const songDurationMs = computed(() => (store.duration || 0) * 1000);
const activeIdx = computed(() =>
  synced.value ? activeLineIndex(lines.value, currentMs.value, songDurationMs.value) : -1
);

function getDotColor(line, dotIdx) {
  if (!line.isGap) return 'rgba(255, 255, 255, 0.2)';
  const duration = line.endTimeMs - line.time_ms;
  const now = currentMs.value;
  const elapsed = Math.max(0, Math.min(duration, now - line.time_ms));
  const p = elapsed / duration;
  
  const startRange = dotIdx * 0.33;
  const dotProgress = Math.max(0, Math.min(1, (p - startRange) / 0.33));
  const opacity = 0.2 + (0.95 - 0.2) * dotProgress;
  
  return `rgba(255, 255, 255, ${opacity.toFixed(3)})`;
}



// ---- Smooth scroll (custom RAF, same logic as LyricsPanel) ---------------
let npRafId = null;
let npIsAutoScrolling = false;
let npUserPausedUntil = 0;
let npUserScrollTimer = null;
let npLastScrolledIdx = -1;

function easeInOutQuart(t) {
  return t < 0.5 ? 8 * t * t * t * t : 1 - Math.pow(-2 * t + 2, 4) / 2;
}

function npSmoothScrollTo(container, target, duration = 650) {
  if (npRafId) cancelAnimationFrame(npRafId);
  const start = container.scrollTop;
  const delta = target - start;
  if (Math.abs(delta) < 2) return;
  const t0 = performance.now();
  npIsAutoScrolling = true;
  function step(now) {
    const p = Math.min((now - t0) / duration, 1);
    container.scrollTop = start + delta * easeInOutQuart(p);
    if (p < 1) { npRafId = requestAnimationFrame(step); }
    else { npRafId = null; setTimeout(() => { npIsAutoScrolling = false; }, 80); }
  }
  npRafId = requestAnimationFrame(step);
}

function npScrollToLine(idx) {
  const container = linesEl.value;
  if (!container) return;
  
  const el = container.querySelector(`[data-line="${idx}"]`);
  if (!el) return;
  const h = container.clientHeight;
  const target = Math.max(0, el.offsetTop - h / 2 + el.offsetHeight / 2);
  npSmoothScrollTo(container, target, 650);
}

watch(activeIdx, (idx, oldIdx) => {
  if (idx < 0 || idx === npLastScrolledIdx) return;
  if (Date.now() < npUserPausedUntil) return;
  npLastScrolledIdx = idx;

  const currentLine = lines.value[idx];

  // If current line is a gap, scroll to it immediately
  if (currentLine && currentLine.isGap) {
    nextTick(() => npScrollToLine(idx));
    return;
  }

  // If previous line was a gap, wait for its collapse transition to finish
  // so the layout is stable before we calculate scroll position
  const prevLine = (oldIdx >= 0 && oldIdx < lines.value.length) ? lines.value[oldIdx] : null;
  if (prevLine && prevLine.isGap) {
    nextTick(() => {
      const container = linesEl.value;
      if (!container) return;
      const gapEl = container.querySelector(`[data-line="${oldIdx}"]`);
      if (gapEl) {
        let done = false;
        const doScroll = () => {
          if (done) return;
          done = true;
          gapEl.removeEventListener('transitionend', onEnd);
          npScrollToLine(idx);
        };
        const onEnd = (e) => { if (e.propertyName === 'height') doScroll(); };
        gapEl.addEventListener('transitionend', onEnd);
        // Safety fallback if transitionend doesn't fire
        setTimeout(doScroll, 500);
      } else {
        npScrollToLine(idx);
      }
    });
    return;
  }

  // Normal scroll
  nextTick(() => npScrollToLine(idx));
});

watch(() => [song.value?.path, lines.value], () => {
  npLastScrolledIdx = -1;
  npUserPausedUntil = 0;
});

onMounted(() => {
  document.addEventListener('click', closeLosslessPopup);
});

onUnmounted(() => {
  if (npRafId) cancelAnimationFrame(npRafId);
  if (npUserScrollTimer) clearTimeout(npUserScrollTimer);
  document.removeEventListener('click', closeLosslessPopup);
});

// ---- transport ----
const onSeekInput = (e) => {
  store.lastSeekAt = Date.now();
  store.currentTime = Number(e.target.value);
};
const onSeekCommit = (e) => store.seek(Number(e.target.value));
const seekToLine = (line) => {
  if (line.time_ms != null) store.seek(line.time_ms / 1000);
};

const progressPercentage = computed(() => {
  const max = store.duration || 100;
  return Math.min(Math.max(((store.currentTime || 0) / max) * 100, 0), 100);
});
const volumePercentage = computed(() => (store.isMuted ? 0 : store.volume) * 100);

const formatTime = (seconds) => {
  if (!seconds || isNaN(seconds)) return '0:00';
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, '0')}`;
};
const remaining = computed(() =>
  formatTime(Math.max(0, (store.duration || 0) - (store.currentTime || 0)))
);

// ---- Color Extraction for Apple Music Animated Gradient Background ----
const colors = ref(defaultPalette());

watch(coverUrl, async (newUrl) => {
  const path = store.currentSong?.path;
  if (path || newUrl) {
    colors.value = await extractColorsForPath(path, newUrl);
  } else {
    colors.value = defaultPalette();
  }
}, { immediate: true });

const close = () => {
  store.exitFullscreenWithTransition();
};

const goToArtist = (artistName) => {
  if (!artistName || artistName === 'Unknown Artist') return;
  close();
  router.push({ name: 'ArtistDetail', params: { name: artistName } });
};

const goToAlbum = (albumName) => {
  if (!albumName || albumName === 'Unknown Album') return;
  close();
  router.push({ name: 'AlbumDetail', params: { name: albumName } });
};
</script>

<template>
  <Transition name="np">
    <div
      v-if="store.fullscreenOpen && song"
      class="fixed inset-0 z-[200] overflow-hidden text-white select-none bg-[#060606]"
    >
      <!-- Animated gradient backdrop (not see-through) -->
      <div class="absolute inset-0 bg-[#0a0a0a] overflow-hidden">
        <div class="absolute inset-0 opacity-90 filter blur-[80px] transform scale-[2.2] origin-center pointer-events-none">
          <div class="blob blob-1" :style="{ backgroundColor: colors[0] }"></div>
          <div class="blob blob-2" :style="{ backgroundColor: colors[1] }"></div>
          <div class="blob blob-3" :style="{ backgroundColor: colors[2] }"></div>
          <div class="blob blob-4" :style="{ backgroundColor: colors[0] }"></div>
        </div>
        <!-- Dark overlay to ensure text contrast (removed expensive backdrop-blur) -->
        <div class="absolute inset-0 bg-[#0a0a0a]/38"></div>
        <div class="absolute inset-0 bg-gradient-to-t from-[#0a0a0a] via-[#0a0a0a]/20 to-[#0a0a0a]/10"></div>
      </div>

      <!-- Draggable top strip + close button -->
      <div
        data-tauri-drag-region
        class="absolute top-0 left-0 right-0 z-10 flex items-center px-4 h-14"
      >
        <button
          @click="close"
          class="text-white/70 hover:text-white transition rounded-full p-1.5 hover:bg-white/10"
          title="Close (Esc)"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="24"
            height="24"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <line x1="18" y1="6" x2="6" y2="18"></line>
            <line x1="6" y1="6" x2="18" y2="18"></line>
          </svg>
        </button>

        <!-- Romaji show/hide toggle (top-right), only when romanization exists -->
        <button
          v-if="hasRomaji && showLyricsColumn"
          @click="store.toggleRomaji()"
          class="flex items-center justify-center ml-auto text-white transition-all rounded-full w-9 h-9 active:scale-95"
          :class="store.showRomaji ? 'bg-white/25' : 'bg-white/10 hover:bg-white/20'"
          :title="store.showRomaji ? 'Hide romaji' : 'Show romaji'"
        >
          <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m5 8 6 6" /><path d="m4 14 6-6 2-3" /><path d="M2 5h12" /><path d="M7 2h1" /><path d="m22 22-5-10-5 10" /><path d="M14 18h6" />
          </svg>
        </button>
      </div>

      <!-- Content -->
      <div
        class="relative flex flex-col items-center justify-center h-full px-6 pb-8 lg:flex-row sm:px-12 lg:px-20 pt-14"
      >
        <!-- Left: cover + controls -->
        <div class="flex flex-col items-stretch w-full max-w-[420px] shrink-0 transition-all duration-500 ease-[cubic-bezier(0.25,1,0.5,1)]">
          <div
            class="np-cover aspect-square w-full rounded-xl overflow-hidden shadow-2xl bg-[#222] border border-white/10"
          >
            <img
              v-if="coverUrl"
              :src="coverUrl"
              class="object-cover w-full h-full"
              alt=""
              draggable="false"
            />
            <div
              v-else
              class="flex items-center justify-center w-full h-full bg-gradient-to-br from-gray-700 to-gray-900"
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                class="w-1/4 text-gray-500 opacity-50 h-1/4"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="1.5"
              >
                <path d="M9 18V5l12-2v13"></path>
                <circle cx="6" cy="18" r="3"></circle>
                <circle cx="18" cy="16" r="3"></circle>
              </svg>
            </div>
          </div>

          <!-- Title row -->
          <div class="flex items-center justify-between gap-3 mt-5">
            <div class="min-w-0">
              <div class="text-xl font-bold truncate">{{ song.title }}</div>
              <div class="text-sm truncate text-white/60">
                <span
                  @click="goToArtist(song.artist)"
                  class="hover:text-[var(--accent-color)] hover:underline cursor-pointer transition-colors"
                >
                  {{ song.artist }}
                </span>
                <span v-if="song.album">
                  — 
                  <span
                    @click="goToAlbum(song.album)"
                    class="hover:text-[var(--accent-color)] hover:underline cursor-pointer transition-colors"
                  >
                    {{ song.album }}
                  </span>
                </span>
              </div>
            </div>
            <button
              @click="store.toggleFavorite(song.path)"
              class="transition shrink-0 hover:scale-110"
              :class="store.isFavorite(song.path) ? 'text-[var(--accent-color)]' : 'text-white/60 hover:text-white'"
              title="Like"
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="22"
                height="22"
                viewBox="0 0 24 24"
                :fill="store.isFavorite(song.path) ? 'currentColor' : 'none'"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path
                  d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"
                ></path>
              </svg>
            </button>
          </div>

          <!-- Progress -->
          <div class="mt-4">
            <input
              type="range"
              min="0"
              :max="store.duration || 100"
              :value="store.currentTime"
              @input="onSeekInput"
              @change="onSeekCommit"
              class="w-full h-1 rounded-lg appearance-none cursor-pointer accent-[var(--accent-color)]"
              :style="{
                background: `linear-gradient(to right, #fff ${progressPercentage}%, rgba(255,255,255,0.25) ${progressPercentage}%)`,
              }"
            />
            <div class="flex justify-between text-[11px] text-white/50 mt-1 tabular-nums">
              <span>{{ formatTime(store.currentTime) }}</span>
              <span>-{{ remaining }}</span>
            </div>

            <!-- Lossless Badge Container (Centered under seek bar & timestamps) -->
            <div v-if="isLossless" class="relative flex justify-center mt-0.1">
              <button
                @click.stop="losslessPopupOpen = !losslessPopupOpen"
                class="flex items-center gap-1 px-1.5 py-0.5 rounded bg-white/10 hover:bg-white/15 active:scale-98 transition border border-white/10 text-white/70 hover:text-white text-[9px] font-bold uppercase tracking-wider select-none focus:outline-none"
                title="Lossless Audio"
              >
                <!-- SVG logo same as playercontrol -->
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  viewBox="0 0 15 9"
                  class="h-2 w-[13px] fill-current"
                >
                  <path
                    d="M8.184,0.35C9.944,0.35 10.703,3.296 11.338,5.238C11.673,3.842 11.497,3.542 11.857,3.542C11.99,3.542 12.126,3.633 12.126,3.798C12.126,3.809 12.123,3.839 12.117,3.883L12.091,4.058C12.02,4.522 11.845,5.494 11.654,6.144C13.198,10.191 14.345,4.861 14.474,3.772C14.493,3.615 14.612,3.542 14.731,3.542C14.891,3.542 15.022,3.662 14.997,3.843C14.72,5.605 14.295,8.35 12.547,8.35C11.582,8.35 11.04,7.595 10.611,6.73C9.54,4.626 9.047,1.093 7.997,1.093C7.66,1.093 7.411,1.444 7.394,1.444C7.362,1.444 7.337,1.301 7.023,0.909C7.322,0.567 7.734,0.35 8.184,0.35ZM2.458,0.354C5.211,0.354 5.456,7.618 7.014,7.618C7.197,7.618 7.394,7.507 7.61,7.256C7.729,7.458 7.851,7.638 7.978,7.796C7.667,8.151 7.28,8.35 6.795,8.35C5.054,8.349 4.306,5.434 3.663,3.466C3.511,4.097 3.432,4.669 3.402,4.925C3.382,5.088 3.263,5.163 3.143,5.163C3.009,5.163 2.874,5.071 2.874,4.908L2.874,4.908L2.877,4.87C2.966,4.223 3.146,3.243 3.347,2.56C3.079,1.858 2.745,1.091 2.252,1.091C1.257,1.091 0.687,3.591 0.527,4.925C0.508,5.088 0.388,5.163 0.268,5.163C0.135,5.163 0,5.071 0,4.908C0,4.896 0.001,4.883 0.002,4.87C0.283,2.836 0.808,0.354 2.458,0.354ZM5.315,0.35C5.809,0.35 6.339,0.608 6.797,1.211C6.822,1.241 7.078,1.639 7.159,1.777C8.277,3.802 8.818,7.627 9.881,7.627C10.065,7.627 10.264,7.513 10.484,7.256C10.604,7.458 10.726,7.638 10.852,7.796C10.542,8.15 10.155,8.35 9.67,8.35C6.933,8.349 6.636,1.09 5.128,1.09C4.788,1.09 4.536,1.444 4.519,1.444C4.487,1.444 4.462,1.301 4.148,0.909C4.455,0.558 4.87,0.35 5.315,0.35Z"
                  />
                </svg>
                <span>lossless</span>
              </button>

              <!-- Popover (slightly larger) -->
              <div
                v-if="losslessPopupOpen"
                class="lossless-popover-content absolute bottom-full left-1/2 -translate-x-1/2 mb-3 z-[100] bg-[#1c1c1e] border border-[#323236] rounded-xl shadow-2xl p-4 w-[230px] text-center select-none animate-fade-in-up"
                @click.stop
              >
                <!-- Downward pointing arrow -->
                <div
                  class="absolute top-full left-1/2 -translate-x-1/2 -translate-y-1/2 w-2 h-2 bg-[#1c1c1e] border-r border-b border-[#323236] rotate-45"
                ></div>

                <!-- Lossless Logo (Small) -->
                <div class="flex justify-center mb-2">
                  <svg
                    xmlns="http://www.w3.org/2000/svg"
                    viewBox="0 0 15 9"
                    class="h-5 w-[35px] text-white fill-current"
                  >
                    <path
                      d="M8.184,0.35C9.944,0.35 10.703,3.296 11.338,5.238C11.673,3.842 11.497,3.542 11.857,3.542C11.99,3.542 12.126,3.633 12.126,3.798C12.126,3.809 12.123,3.839 12.117,3.883L12.091,4.058C12.02,4.522 11.845,5.494 11.654,6.144C13.198,10.191 14.345,4.861 14.474,3.772C14.493,3.615 14.612,3.542 14.731,3.542C14.891,3.542 15.022,3.662 14.997,3.843C14.72,5.605 14.295,8.35 12.547,8.35C11.582,8.35 11.04,7.595 10.611,6.73C9.54,4.626 9.047,1.093 7.997,1.093C7.66,1.093 7.411,1.444 7.394,1.444C7.362,1.444 7.337,1.301 7.023,0.909C7.322,0.567 7.734,0.35 8.184,0.35ZM2.458,0.354C5.211,0.354 5.456,7.618 7.014,7.618C7.197,7.618 7.394,7.507 7.61,7.256C7.729,7.458 7.851,7.638 7.978,7.796C7.667,8.151 7.28,8.35 6.795,8.35C5.054,8.349 4.306,5.434 3.663,3.466C3.511,4.097 3.432,4.669 3.402,4.925C3.382,5.088 3.263,5.163 3.143,5.163C3.009,5.163 2.874,5.071 2.874,4.908L2.874,4.908L2.877,4.87C2.966,4.223 3.146,3.243 3.347,2.56C3.079,1.858 2.745,1.091 2.252,1.091C1.257,1.091 0.687,3.591 0.527,4.925C0.508,5.088 0.388,5.163 0.268,5.163C0.135,5.163 0,5.071 0,4.908C0,4.896 0.001,4.883 0.002,4.87C0.283,2.836 0.808,0.354 2.458,0.354ZM5.315,0.35C5.809,0.35 6.339,0.608 6.797,1.211C6.822,1.241 7.078,1.639 7.159,1.777C8.277,3.802 8.818,7.627 9.881,7.627C10.065,7.627 10.264,7.513 10.484,7.256C10.604,7.458 10.726,7.638 10.852,7.796C10.542,8.15 10.155,8.35 9.67,8.35C6.933,8.349 6.636,1.09 5.128,1.09C4.788,1.09 4.536,1.444 4.519,1.444C4.487,1.444 4.462,1.301 4.148,0.909C4.455,0.558 4.87,0.35 5.315,0.35Z"
                    />
                  </svg>
                </div>

                <!-- Title -->
                <h4 class="text-sm font-bold text-white mb-0.5">Lossless</h4>
                <!-- Description -->
                <p class="mb-3 text-xs leading-normal text-gray-400">
                  This audio is playing with lossless compression.
                </p>

                <!-- Technical Specs -->
                <div
                  class="bg-[#2c2c2e]/60 rounded-lg py-1 px-3 text-xs font-semibold text-[var(--accent-color)] font-variant-numeric tracking-wide border border-white/5"
                >
                  {{ formatLosslessSpecs() }}
                </div>
              </div>
            </div>
          </div>

          <!-- Controls -->
          <div class="flex items-center justify-center gap-6 mt-6">
            <button
              @click="store.toggleShuffle()"
              :class="store.shuffleMode ? 'text-[var(--accent-color)]' : 'text-white/60 hover:text-white'"
              title="Shuffle"
            >
              <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M16 3h5v5M4 20L21 3M21 16v5h-5M15 15l6 6M4 4l5 5" />
              </svg>
            </button>
            <button @click="store.prevSong()" class="text-white/90 hover:text-white" title="Previous">
              <svg xmlns="http://www.w3.org/2000/svg" width="30" height="30" viewBox="0 0 24 24" fill="currentColor">
                <polygon points="19 20 9 12 19 4 19 20"></polygon>
                <rect x="4" y="4" width="2.5" height="16"></rect>
              </svg>
            </button>
            <button
              @click="store.togglePlay()"
              class="flex items-center justify-center w-16 h-16 text-black transition bg-white rounded-full hover:scale-105"
            >
              <svg v-if="store.isPlaying" xmlns="http://www.w3.org/2000/svg" width="30" height="30" viewBox="0 0 24 24" fill="currentColor">
                <rect x="6" y="4" width="4" height="16"></rect>
                <rect x="14" y="4" width="4" height="16"></rect>
              </svg>
              <svg v-else xmlns="http://www.w3.org/2000/svg" width="30" height="30" viewBox="0 0 24 24" fill="currentColor">
                <polygon points="6 3 20 12 6 21 6 3"></polygon>
              </svg>
            </button>
            <button @click="store.nextSong(true)" class="text-white/90 hover:text-white" title="Next">
              <svg xmlns="http://www.w3.org/2000/svg" width="30" height="30" viewBox="0 0 24 24" fill="currentColor">
                <polygon points="5 4 15 12 5 20 5 4"></polygon>
                <rect x="17.5" y="4" width="2.5" height="16"></rect>
              </svg>
            </button>
            <button
              @click="store.toggleLoop()"
              class="relative"
              :class="store.loopMode > 0 ? 'text-[var(--accent-color)]' : 'text-white/60 hover:text-white'"
              title="Loop"
            >
              <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M17 1l4 4-4 4"></path>
                <path d="M3 11V9a4 4 0 0 1 4-4h14"></path>
                <path d="M7 23l-4-4 4-4"></path>
                <path d="M21 13v2a4 4 0 0 1-4 4H3"></path>
              </svg>
              <span v-if="store.loopMode === 2" class="absolute -top-1 -right-2 text-[8px] font-bold">1</span>
            </button>
          </div>

          <!-- Volume -->
          <div class="flex items-center gap-2 mt-4 text-white/60">
            <button @click="store.toggleMute()" class="hover:text-white" :title="store.isMuted ? 'Unmute' : 'Mute'">
              <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"></polygon>
                <path v-if="!store.isMuted" d="M15.54 8.46a5 5 0 0 1 0 7.07"></path>
                <template v-else>
                  <line x1="23" y1="9" x2="17" y2="15"></line>
                  <line x1="17" y1="9" x2="23" y2="15"></line>
                </template>
              </svg>
            </button>
            <input
              type="range"
              min="0"
              max="1"
              step="0.01"
              :value="store.isMuted ? 0 : store.volume"
              @input="store.setVolume($event.target.value)"
              class="flex-1 h-1 rounded-lg appearance-none cursor-pointer accent-[var(--accent-color)]"
              :style="{
                background: `linear-gradient(to right, #fff ${volumePercentage}%, rgba(255,255,255,0.25) ${volumePercentage}%)`,
              }"
            />
          </div>
        </div>

        <!-- Right: lyrics (with dynamic transition classes instead of v-if) -->
        <div
          class="lyrics-container h-full min-w-0 flex flex-col transition-all duration-500 ease-[cubic-bezier(0.25,1,0.5,1)]"
          :class="showLyricsColumn ? 'opacity-100 flex-1 max-w-[650px] translate-x-0 pointer-events-auto ml-8 lg:ml-16' : 'opacity-0 max-w-0 translate-x-12 pointer-events-none overflow-hidden ml-0'"
        >
          <div
            ref="linesEl"
            class="flex-1 overflow-y-auto np-lyrics-scroll py-[35vh] w-full lg:w-[650px] lg:min-w-[650px]"
            @scroll.passive="() => { if (npIsAutoScrolling) return; npUserPausedUntil = Date.now() + 3000; if (npUserScrollTimer) clearTimeout(npUserScrollTimer); npUserScrollTimer = setTimeout(() => { npUserPausedUntil = 0; }, 3100); }"
          >
            <!-- Loading -->
            <div v-if="lyricsLoading" class="text-2xl font-bold text-white/40">Loading lyrics…</div>

            <!-- Found -->
            <template v-else-if="lines.length">
              <p
                v-for="(line, i) in lines"
                :key="i"
                :data-line="i"
                @click="seekToLine(line)"
                class="text-2xl font-semibold leading-relaxed tracking-tight np-line sm:text-3xl"
                :class="[
                  synced ? 'cursor-pointer' : '',
                  i === activeIdx ? 'np-line-active' : 'np-line-dim',
                  line.isGap ? 'np-line-gap' : 'mb-4',
                  (line.words && line.words.length) ? 'np-words' : '',
                ]"
              >
                <span
                  v-if="line.isGap"
                  class="np-gap-dots"
                  :class="{ 'np-gap-dots-active': i === activeIdx }"
                >
                  <span class="dots-wrapper">
                    <span :style="{ color: i === activeIdx ? getDotColor(line, 0) : 'rgba(255,255,255,0.2)' }">•</span>
                    <span :style="{ color: i === activeIdx ? getDotColor(line, 1) : 'rgba(255,255,255,0.2)' }">•</span>
                    <span :style="{ color: i === activeIdx ? getDotColor(line, 2) : 'rgba(255,255,255,0.2)' }">•</span>
                  </span>
                </span>
                <LyricContent
                  v-else
                  :line="line"
                  :active="i === activeIdx"
                  :is-past="i < activeIdx"
                  :current-ms="(i === activeIdx || i === activeIdx - 1) ? currentMs : 0"
                  :show-romaji="store.showRomaji"
                />
              </p>
            </template>

            <!-- Not found -->
            <div v-else class="text-white/50">
              <div class="mb-2 text-3xl font-bold">Lyrics not found</div>
              <p class="mb-4 text-sm text-white/40">
                No lyrics were found locally, on LRCLIB, or NetEase
                <span v-if="!store.musixmatchToken">(add a Musixmatch token in Settings for more sources)</span>.
              </p>
              <button
                @click="fetchLyrics(true)"
                class="text-sm px-3 py-1.5 rounded-md bg-white/10 hover:bg-white/20 transition"
              >
                Retry
              </button>
            </div>
          </div>
          <div
            v-if="lyrics && lyrics.source && lines.length"
            class="text-[11px] text-white/30 pt-1 shrink-0"
          >
            Source: {{ lyrics.source }}
          </div>
        </div>
      </div>

      <!-- Bottom-right lyrics toggle button -->
      <div
        class="absolute z-30 bottom-6 right-6"
      >
        <button
          @click="showLyricsOption = !showLyricsOption"
          class="flex items-center justify-center w-10 h-10 text-white transition-all rounded-full bg-white/10 hover:bg-white/20 active:scale-95"
          :class="{ 'text-[var(--accent-color)] bg-white/20': showLyricsOption }"
          :title="showLyricsOption ? 'Hide Lyrics' : 'Show Lyrics'"
        >
          <!-- Speech bubble icon with lines -->
          <svg
            v-if="showLyricsOption"
            xmlns="http://www.w3.org/2000/svg"
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"></path>
            <line x1="8" y1="10" x2="16" y2="10"></line>
            <line x1="8" y1="14" x2="12" y2="14"></line>
          </svg>
          <!-- Disabled Speech bubble icon -->
          <svg
            v-else
            xmlns="http://www.w3.org/2000/svg"
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"></path>
          </svg>
        </button>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
input[type='range']::-webkit-slider-thumb {
  -webkit-appearance: none;
  height: 13px;
  width: 13px;
  border-radius: 50%;
  background: #fff;
  margin-top: -5px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4);
}



.np-lyrics-scroll {
  scrollbar-width: thin;
  scrollbar-color: transparent transparent;
  -webkit-mask-image: linear-gradient(
    to bottom,
    transparent 0,
    #000 12%,
    #000 88%,
    transparent 100%
  );
  mask-image: linear-gradient(to bottom, transparent 0, #000 12%, #000 88%, transparent 100%);
}
.np-lyrics-scroll:hover {
  scrollbar-color: rgba(255, 255, 255, 0.18) transparent;
}
.np-lyrics-scroll::-webkit-scrollbar {
  width: 4px;
}
.np-lyrics-scroll::-webkit-scrollbar-track {
  background: transparent;
}
.np-lyrics-scroll::-webkit-scrollbar-thumb {
  background: transparent;
  border-radius: 4px;
  transition: background 0.25s ease;
}
.np-lyrics-scroll:hover::-webkit-scrollbar-thumb {
  background: rgba(255, 255, 255, 0.18);
}

.np-line {
  transition:
    color     0.45s cubic-bezier(0.25, 1, 0.5, 1),
    opacity   0.45s cubic-bezier(0.25, 1, 0.5, 1),
    transform 0.5s  cubic-bezier(0.34, 1.56, 0.64, 1);
  transform-origin: left center;
  padding-right: 24px; /* Prevent text clipping on scale/translate */
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.2), 0 2px 6px rgba(0, 0, 0, 0.15);
}
/* Word-by-word (karaoke) lines drive their brightness with the gradient wipe, so
   entering the active state must NOT also ride the slow opacity ramp: transitioning
   opacity up from 0.28 while the dim (unsung, 34%) fill is shown would dip the text
   dark before brightening — the "dark flash". A CSS transition is governed by the
   *destination* state, so we strip opacity only on the active state (snap in, no
   dip). The base .np-line keeps opacity in its transition, so leaving active
   (→ .np-line-dim) still fades the finished line out. */
.np-line.np-words.np-line-active {
  transition:
    color     0.45s cubic-bezier(0.25, 1, 0.5, 1),
    transform 0.5s  cubic-bezier(0.34, 1.56, 0.64, 1);
}

.np-line-active {
  color: rgba(255, 255, 255, 0.97);
  opacity: 1;
  transform: translateX(6px) scale(1.015);
}
.np-line-dim {
  color: rgba(255, 255, 255, 0.97);
  opacity: 0.28;
  transform: translateX(0) scale(1);
}
.np-line-dim:hover {
  opacity: 0.55;
}

/* Open / close transition: fade the backdrop, lift + scale the panel. */
.np-enter-active,
.np-leave-active {
  transition: opacity 0.32s cubic-bezier(0.32, 0.72, 0, 1);
}
.np-enter-from,
.np-leave-to {
  opacity: 0;
}
.np-enter-active .np-cover,
.np-leave-active .np-cover {
  transition: transform 0.42s cubic-bezier(0.32, 0.72, 0, 1);
}
.np-enter-from .np-cover {
  transform: translateY(24px) scale(0.92);
}
.np-leave-to .np-cover {
  transform: translateY(24px) scale(0.92);
}

/* Animated blobs for Apple Music gradient */
.blob {
  position: absolute;
  border-radius: 50%;
  mix-blend-mode: screen;
  transition: background-color 1.8s ease-in-out;
  opacity: 0.95;
  will-change: transform;
}

.blob-1 {
  width: 55%;
  height: 55%;
  left: -10%;
  top: -10%;
  animation: blob-move-1 28s infinite alternate ease-in-out;
}

.blob-2 {
  width: 50%;
  height: 50%;
  right: -5%;
  bottom: -5%;
  animation: blob-move-2 33s infinite alternate ease-in-out;
}

.blob-3 {
  width: 45%;
  height: 45%;
  left: 20%;
  top: 20%;
  animation: blob-move-3 30s infinite alternate ease-in-out;
}

.blob-4 {
  width: 48%;
  height: 48%;
  right: 5%;
  top: -5%;
  animation: blob-move-4 25s infinite alternate ease-in-out;
}

@keyframes blob-move-1 {
  0% {
    transform: translate(0px, 0px) scale(1) rotate(0deg);
  }
  50% {
    transform: translate(80px, 40px) scale(1.1) rotate(180deg);
  }
  100% {
    transform: translate(30px, 90px) scale(0.95) rotate(360deg);
  }
}

@keyframes blob-move-2 {
  0% {
    transform: translate(0px, 0px) scale(1) rotate(0deg);
  }
  50% {
    transform: translate(-70px, -90px) scale(0.9) rotate(-180deg);
  }
  100% {
    transform: translate(-30px, 30px) scale(1.05) rotate(-360deg);
  }
}

@keyframes blob-move-3 {
  0% {
    transform: translate(0px, 0px) scale(1);
  }
  50% {
    transform: translate(50px, -50px) scale(1.08);
  }
  100% {
    transform: translate(-70px, 70px) scale(0.92);
  }
}

@keyframes blob-move-4 {
  0% {
    transform: translate(0px, 0px) scale(1);
  }
  50% {
    transform: translate(-60px, 60px) scale(1.04);
  }
  100% {
    transform: translate(60px, -30px) scale(0.95);
  }
}

.np-gap-dots {
  opacity: 0;
  transition: opacity 0.35s ease;
  pointer-events: none;
  display: inline-block;
}

.np-gap-dots-active {
  opacity: 1;
  pointer-events: auto;
}

.dots-wrapper {
  display: inline-flex;
  gap: 0.6rem;
  font-size: 2.25rem;
  line-height: 1;
  vertical-align: middle;
  font-weight: 800;
}

.dots-wrapper span {
  transition: color 0.25s linear;
}

.np-line-gap {
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

.np-line-gap.np-line-active {
  height: 3.5rem;
  margin-bottom: 1rem !important;
  opacity: 1;
}

.animate-fade-in-up {
  animation: fadeInUp 0.15s cubic-bezier(0.16, 1, 0.3, 1) forwards;
  transform-origin: bottom center;
}

@keyframes fadeInUp {
  from {
    opacity: 0;
    transform: translate(-50%, 4px) scale(0.95);
  }
  to {
    opacity: 1;
    transform: translate(-50%, 0) scale(1);
  }
}
</style>
