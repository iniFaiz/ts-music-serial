<script setup>
import { ref, computed, watch, nextTick, onUnmounted } from 'vue';
import { store } from '../store';
import { loadCover, getCachedCover, hasCachedCover } from '../coverCache';
import { loadLyrics, activeLineIndex } from '../lyricsCache';
import { getCurrentWindow } from '@tauri-apps/api/window';

const coverUrl = ref(null);
// undefined = loading, null = not found, object = resolved lyrics
const lyrics = ref(undefined);
const linesEl = ref(null);
const lyricsLoading = ref(false);
const appWindow = getCurrentWindow();
const showLyricsOption = ref(true);

const song = computed(() => store.currentSong);

const hasLyrics = computed(() => lines.value && lines.value.length > 0);
const showLyricsColumn = computed(() => {
  return showLyricsOption.value && (lyricsLoading.value || hasLyrics.value);
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
    if (open && song.value) {
      resolveCover(song.value.path);
      fetchLyrics();
    }
  }
);
watch(
  () => song.value && song.value.path,
  (path) => {
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

const lines = computed(() => (lyrics.value && lyrics.value.lines) || []);
const synced = computed(() => !!(lyrics.value && lyrics.value.synced));
// +50ms lookahead: compensates for the ~50ms average lag from the 100ms poll interval
const currentMs = computed(() => (store.currentTime || 0) * 1000 + 50);
const activeIdx = computed(() =>
  synced.value ? activeLineIndex(lines.value, currentMs.value) : -1
);



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

watch(activeIdx, (idx) => {
  if (idx < 0 || idx === npLastScrolledIdx) return;
  if (Date.now() < npUserPausedUntil) return;
  npLastScrolledIdx = idx;
  nextTick(() => npScrollToLine(idx));
});

watch(() => [song.value?.path, lines.value], () => {
  npLastScrolledIdx = -1;
  npUserPausedUntil = 0;
});

onUnmounted(() => {
  if (npRafId) cancelAnimationFrame(npRafId);
  if (npUserScrollTimer) clearTimeout(npUserScrollTimer);
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
const colors = ref(['#ff2d55', '#5856d6', '#007aff']);

function extractColorsFromImage(url) {
  if (!url) {
    return Promise.resolve(['#ff2d55', '#5856d6', '#007aff']);
  }
  return new Promise((resolve) => {
    const img = new Image();
    img.crossOrigin = 'anonymous';
    img.onload = () => {
      try {
        const canvas = document.createElement('canvas');
        canvas.width = 12;
        canvas.height = 12;
        const ctx = canvas.getContext('2d');
        if (!ctx) {
          resolve(['#ff2d55', '#5856d6', '#007aff']);
          return;
        }
        ctx.drawImage(img, 0, 0, 12, 12);
        const imgData = ctx.getImageData(0, 0, 12, 12).data;

        const pxs = [];
        for (let i = 0; i < imgData.length; i += 4) {
          const r = imgData[i];
          const g = imgData[i + 1];
          const b = imgData[i + 2];
          const a = imgData[i + 3];
          if (a < 150) continue;

          const max = Math.max(r, g, b);
          const min = Math.min(r, g, b);
          const saturation = max - min;
          const brightness = (r + g + b) / 3;

          // Ignore extreme blacks/whites/greys for vibrancy
          if (brightness > 240 && saturation < 20) continue;
          if (brightness < 15 && saturation < 10) continue;

          pxs.push({ r, g, b, saturation, brightness });
        }

        if (pxs.length === 0) {
          for (let i = 0; i < imgData.length; i += 4) {
            const r = imgData[i];
            const g = imgData[i + 1];
            const b = imgData[i + 2];
            pxs.push({ r, g, b, saturation: Math.max(r,g,b) - Math.min(r,g,b), brightness: (r+g+b)/3 });
          }
        }

        pxs.sort((a, b) => b.saturation - a.saturation);

        const chosen = [];
        for (const p of pxs) {
          const isSimilar = chosen.some(c => {
            const dr = c.r - p.r;
            const dg = c.g - p.g;
            const db = c.b - p.b;
            return Math.sqrt(dr * dr + dg * dg + db * db) < 65;
          });
          if (!isSimilar) {
            chosen.push(p);
            if (chosen.length >= 3) break;
          }
        }

        if (chosen.length < 3) {
          for (const p of pxs) {
            if (!chosen.includes(p)) {
              chosen.push(p);
              if (chosen.length >= 3) break;
            }
          }
        }

        while (chosen.length < 3) {
          chosen.push({ r: 60, g: 60, b: 60, saturation: 0, brightness: 60 });
        }

        resolve(chosen.map(c => `rgb(${c.r}, ${c.g}, ${c.b})`));
      } catch (e) {
        console.error("Color extraction failed", e);
        resolve(['#ff2d55', '#5856d6', '#007aff']);
      }
    };
    img.onerror = () => {
      resolve(['#ff2d55', '#5856d6', '#007aff']);
    };
    img.src = url;
  });
}

watch(coverUrl, async (newUrl) => {
  if (newUrl) {
    colors.value = await extractColorsFromImage(newUrl);
  } else {
    colors.value = ['#ff2d55', '#5856d6', '#007aff'];
  }
}, { immediate: true });

const close = () => {
  store.closeFullscreen();
  setTimeout(() => {
    try {
      appWindow.setFullscreen(false);
    } catch (e) {
      console.warn("Tauri fullscreen restore error:", e);
    }
  }, 50);
};
</script>

<template>
  <Transition name="np">
    <div
      v-if="store.fullscreenOpen && song"
      class="fixed inset-0 z-[200] overflow-hidden text-white select-none bg-[#060606]"
    >
      <!-- Animated gradient backdrop (not see-through) -->
      <div class="absolute inset-0 bg-[#060606] overflow-hidden">
        <div class="absolute inset-0 opacity-75 filter blur-[30px] transform scale-[4.0] origin-center pointer-events-none">
          <div class="blob blob-1" :style="{ backgroundColor: colors[0] }"></div>
          <div class="blob blob-2" :style="{ backgroundColor: colors[1] }"></div>
          <div class="blob blob-3" :style="{ backgroundColor: colors[2] }"></div>
          <div class="blob blob-4" :style="{ backgroundColor: colors[0] }"></div>
        </div>
        <!-- Dark overlay to ensure text contrast (removed expensive backdrop-blur) -->
        <div class="absolute inset-0 bg-[#060606]/55"></div>
        <div class="absolute inset-0 bg-gradient-to-t from-[#060606] via-transparent to-black/20"></div>
      </div>

      <!-- Draggable top strip + close button -->
      <div
        data-tauri-drag-region
        class="absolute top-0 left-0 right-0 h-14 flex items-center px-4 z-10"
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
      </div>

      <!-- Content -->
      <div
        class="relative h-full flex flex-col lg:flex-row items-center justify-center px-6 sm:px-12 lg:px-20 pt-14 pb-8"
      >
        <!-- Left: cover + controls -->
        <div class="flex flex-col items-stretch w-full max-w-[420px] shrink-0 transition-all duration-500 ease-[cubic-bezier(0.25,1,0.5,1)]">
          <div
            class="np-cover aspect-square w-full rounded-xl overflow-hidden shadow-2xl bg-[#222] border border-white/10"
          >
            <img
              v-if="coverUrl"
              :src="coverUrl"
              class="w-full h-full object-cover"
              alt=""
              draggable="false"
            />
            <div
              v-else
              class="w-full h-full bg-gradient-to-br from-gray-700 to-gray-900 flex items-center justify-center"
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                class="w-1/4 h-1/4 text-gray-500 opacity-50"
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
          <div class="flex items-center justify-between mt-5 gap-3">
            <div class="min-w-0">
              <div class="text-xl font-bold truncate">{{ song.title }}</div>
              <div class="text-sm text-white/60 truncate">
                {{ song.artist }}<span v-if="song.album"> — {{ song.album }}</span>
              </div>
            </div>
            <button
              @click="store.toggleFavorite(song.path)"
              class="shrink-0 transition hover:scale-110"
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
          </div>

          <!-- Controls -->
          <div class="flex items-center justify-center gap-6 mt-3">
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
              class="bg-white text-black rounded-full w-16 h-16 flex items-center justify-center hover:scale-105 transition"
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
            <div v-if="lyricsLoading" class="text-white/40 text-2xl font-bold">Loading lyrics…</div>

            <!-- Found -->
            <template v-else-if="lines.length">
              <p
                v-for="(line, i) in lines"
                :key="i"
                :data-line="i"
                @click="seekToLine(line)"
                class="np-line text-2xl sm:text-3xl font-semibold leading-relaxed tracking-tight mb-4"
                :class="[
                  synced ? 'cursor-pointer' : '',
                  i === activeIdx ? 'np-line-active' : 'np-line-dim',
                ]"
              >
                <span>{{ line.text || (synced ? '♪' : '') }}</span>
              </p>
            </template>

            <!-- Not found -->
            <div v-else class="text-white/50">
              <div class="text-3xl font-bold mb-2">Lyrics not found</div>
              <p class="text-sm text-white/40 mb-4">
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
        v-if="lines.length > 0"
        class="absolute bottom-6 right-6 z-30"
      >
        <button
          @click="showLyricsOption = !showLyricsOption"
          class="flex items-center justify-center w-10 h-10 rounded-full transition-all bg-white/10 text-white hover:bg-white/20 active:scale-95"
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
}
.np-line-active {
  color: rgba(255, 255, 255, 0.97);
  opacity: 1;
  transform: translateX(6px) scale(1.015);
}
.np-line-dim {
  color: rgba(255, 255, 255, 0.28);
  opacity: 1;
  transform: translateX(0) scale(1);
}
.np-line-dim:hover {
  color: rgba(255, 255, 255, 0.55);
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
  transition: background-color 1.5s ease-in-out;
  opacity: 0.6;
  will-change: transform;
}

.blob-1 {
  width: 30%;
  height: 30%;
  left: 5%;
  top: 5%;
  animation: blob-move-1 28s infinite alternate ease-in-out;
}

.blob-2 {
  width: 25%;
  height: 25%;
  right: 8%;
  bottom: 8%;
  animation: blob-move-2 33s infinite alternate ease-in-out;
}

.blob-3 {
  width: 20%;
  height: 20%;
  left: 35%;
  top: 35%;
  animation: blob-move-3 30s infinite alternate ease-in-out;
}

.blob-4 {
  width: 22%;
  height: 22%;
  right: 15%;
  top: 5%;
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
</style>
