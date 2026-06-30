<script setup>
// Apple-Music-style compact mini player. Rendered as a full-window overlay while
// store.miniPlayerOpen is true (the window itself is shrunk by the store). All
// playback state is shared with the main window — PlayerControls stays mounted
// underneath and keeps polling/handling media keys — so this is purely
// presentational + calls the same store actions.
//
// Three sub-views, each with its own window size (the store resizes to match):
//   • lyrics  — tall, synced karaoke lyrics
//   • compact — short horizontal bar (when lyrics are off / unavailable)
//   • artwork — full-bleed album art; chrome auto-hides and returns on hover
// In lyrics + artwork the chrome (top/bottom bars) auto-hides after the mouse
// goes idle and returns on movement. The queue lives in the middle region so the
// top window controls and bottom transport stay usable.
import { ref, computed, watch, nextTick, onMounted, onUnmounted } from 'vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useRouter } from 'vue-router';
import { store } from '../store';
import { loadCover, getCachedCover, hasCachedCover } from '../coverCache';
import { loadLyrics, activeLineIndex } from '../lyricsCache';
import { extractColorsFromImage, defaultPalette } from '../colorExtract';
import LyricContent from './LyricContent.vue';
import LosslessBadge from './LosslessBadge.vue';
import CoverImage from './CoverImage.vue';

const router = useRouter();

const coverUrl = ref(null);
// undefined = loading, null = not found, object = resolved lyrics
const lyrics = ref(undefined);
const lyricsLoading = ref(false);
const linesEl = ref(null);

// View state. `lyricsEnabled` is the lyrics toggle; `artworkMode` (clicking the
// cover) overrides into the full-art view; `queueOpen` shows the queue.
const lyricsEnabled = ref(true);
const artworkMode = ref(false);
const queueOpen = ref(false);
// True while the queue panel is in the DOM, including its slide-out animation, so
// the window stays tall until the close finishes (avoids a height snap on close).
const queuePresent = ref(false);

// Bottom-bar popovers.
const volumeOpen = ref(false);
const moreOpen = ref(false);

// Animated gradient backdrop palette (same system as the fullscreen player).
const colors = ref(defaultPalette());

const song = computed(() => store.currentSong);

// ---- Cover + lyrics loading (only while the mini player is open) ----------
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
  if (song.value && song.value.path === current.path) lyrics.value = res;
  lyricsLoading.value = false;
}

// Re-derive the backdrop colors whenever the cover changes; the blobs ease
// between palettes (CSS transition) for a smooth per-track background change.
watch(coverUrl, async (url) => {
  colors.value = url ? await extractColorsFromImage(url) : defaultPalette();
}, { immediate: true });

watch(
  () => store.miniPlayerOpen,
  (open) => {
    if (open) {
      // Open in a consistent state each time.
      artworkMode.value = false;
      queueOpen.value = false;
      queuePresent.value = false;
      lyricsEnabled.value = true;
      if (song.value) {
        resolveCover(song.value.path);
        fetchLyrics();
      }
    }
  },
  { immediate: true }
);
watch(
  () => song.value && song.value.path,
  (path) => {
    if (store.miniPlayerOpen && path) {
      resolveCover(path);
      fetchLyrics();
    }
  }
);
watch(
  () => store.lyricsSource,
  () => {
    if (store.miniPlayerOpen) {
      lyrics.value = undefined;
      fetchLyrics(true);
    }
  }
);

// ---- Effective view + window sizing ---------------------------------------
// The lyrics toggle is only meaningful (and only shown) when lyrics exist.
const lyricsButtonVisible = computed(() => {
  if (lyrics.value === undefined) return true; // still loading
  return !!(lyrics.value && lyrics.value.lines && lyrics.value.lines.length > 0);
});

const view = computed(() => {
  if (artworkMode.value) return 'artwork';
  if (!lyricsEnabled.value) return 'compact';
  if (lyrics.value === null) return 'compact'; // resolved, none found
  return 'lyrics'; // loading (undefined) or has lines
});
const isArtwork = computed(() => view.value === 'artwork');

// ---- Window sizing --------------------------------------------------------
// Lyrics/artwork use fixed sizes; the compact bar is measured so it fits its
// content exactly (no stray gap or clipping regardless of fonts/scale). The
// queue forces the tall lyrics size.
const MINI_WIDTH = 360;
const topChromeEl = ref(null);
const bottomChromeEl = ref(null);

function fitCompact() {
  if (!store.miniPlayerOpen) return;
  nextTick(() => {
    const top = topChromeEl.value ? topChromeEl.value.offsetHeight : 0;
    const bottom = bottomChromeEl.value ? bottomChromeEl.value.offsetHeight : 0;
    if (top && bottom) store.applyMiniSize(MINI_WIDTH, top + bottom);
  });
}

const windowView = computed(() => (queuePresent.value ? 'lyrics' : view.value));
watch(
  windowView,
  (v) => {
    if (!store.miniPlayerOpen) return;
    if (v === 'compact') fitCompact();
    else store.applyMiniViewSize(v);
  },
  { immediate: true }
);
// The compact bar's height depends on the lossless badge (per-track), so refit
// when the song changes while compact.
watch(
  () => song.value && song.value.path,
  () => {
    if (store.miniPlayerOpen && windowView.value === 'compact') fitCompact();
  }
);

// ---- Auto-hiding chrome ----------------------------------------------------
// Bottom controls auto-hide in lyrics + artwork. The TOP bar (album/title) is
// kept visible in the lyrics view — it only auto-hides in the artwork view.
const chromeActive = ref(true);
let hideTimer = null;
const popoverOpen = computed(() => queueOpen.value || volumeOpen.value || moreOpen.value);
const bottomAutohide = computed(() => (view.value === 'lyrics' || isArtwork.value) && !popoverOpen.value);
const topAutohide = computed(() => isArtwork.value && !popoverOpen.value);
const anyAutohide = computed(() => bottomAutohide.value || topAutohide.value);
const bottomChromeVisible = computed(() => !bottomAutohide.value || chromeActive.value);
const topChromeVisible = computed(() => !topAutohide.value || chromeActive.value);

function onActivity() {
  chromeActive.value = true;
  if (hideTimer) clearTimeout(hideTimer);
  if (anyAutohide.value) {
    hideTimer = setTimeout(() => {
      chromeActive.value = false;
    }, 2200);
  }
}
function onLeave() {
  if (hideTimer) clearTimeout(hideTimer);
  if (anyAutohide.value) chromeActive.value = false;
}
watch(anyAutohide, (auto) => {
  if (auto) onActivity();
  else {
    chromeActive.value = true;
    if (hideTimer) clearTimeout(hideTimer);
  }
});

// ---- Synced lyric lines (+ intro/instrumental gap dots) -------------------
const synced = computed(() => !!(lyrics.value && lyrics.value.synced));
const hasRomaji = computed(() => !!(lyrics.value && lyrics.value.has_romaji));

const lines = computed(() => {
  const rawLines = (lyrics.value && lyrics.value.lines) || [];
  if (!synced.value || rawLines.length === 0) return rawLines;

  const result = [];
  if (rawLines[0] && rawLines[0].time_ms > 6000) {
    result.push({ isGap: true, time_ms: 2000, endTimeMs: rawLines[0].time_ms - 1000, text: '• • •' });
  }
  for (let i = 0; i < rawLines.length; i++) {
    const currentLine = rawLines[i];
    const t = currentLine.text.trim();
    const isEmptyOrNote = t === '' || t === '♪' || t === '🎵';
    if (isEmptyOrNote) {
      const nextLine = rawLines[i + 1];
      if (!nextLine) continue;
      const gapStart = currentLine.time_ms;
      const gapEnd = nextLine.time_ms - 1000;
      if (gapEnd > gapStart) {
        result.push({ isGap: true, time_ms: gapStart, endTimeMs: gapEnd, text: '• • •' });
      }
    } else {
      result.push(currentLine);
    }
  }
  return result;
});

const currentMs = computed(() => (store.currentTime || 0) * 1000 + 50);
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

// ---- Smooth auto-scroll (same approach as the fullscreen player) ----------
let mpRafId = null;
let mpIsAutoScrolling = false;
let mpUserPausedUntil = 0;
let mpUserScrollTimer = null;
let mpLastScrolledIdx = -1;

function easeInOutQuart(t) {
  return t < 0.5 ? 8 * t * t * t * t : 1 - Math.pow(-2 * t + 2, 4) / 2;
}

function mpSmoothScrollTo(container, target, duration = 600) {
  if (mpRafId) cancelAnimationFrame(mpRafId);
  const start = container.scrollTop;
  const delta = target - start;
  if (Math.abs(delta) < 2) return;
  const t0 = performance.now();
  mpIsAutoScrolling = true;
  function step(now) {
    const p = Math.min((now - t0) / duration, 1);
    container.scrollTop = start + delta * easeInOutQuart(p);
    if (p < 1) {
      mpRafId = requestAnimationFrame(step);
    } else {
      mpRafId = null;
      setTimeout(() => {
        mpIsAutoScrolling = false;
      }, 80);
    }
  }
  mpRafId = requestAnimationFrame(step);
}

function mpScrollToLine(idx) {
  const container = linesEl.value;
  if (!container) return;
  const el = container.querySelector(`[data-line="${idx}"]`);
  if (!el) return;
  const h = container.clientHeight;
  const target = Math.max(0, el.offsetTop - h / 2 + el.offsetHeight / 2);
  mpSmoothScrollTo(container, target, 600);
}

watch(activeIdx, (idx) => {
  if (idx < 0 || idx === mpLastScrolledIdx) return;
  if (Date.now() < mpUserPausedUntil) return;
  mpLastScrolledIdx = idx;
  nextTick(() => mpScrollToLine(idx));
});

watch(
  () => [song.value && song.value.path, lines.value, view.value],
  () => {
    mpLastScrolledIdx = -1;
    mpUserPausedUntil = 0;
    if (view.value === 'lyrics' && activeIdx.value >= 0) {
      nextTick(() => mpScrollToLine(activeIdx.value));
    }
  }
);

const onLyricsScroll = () => {
  if (mpIsAutoScrolling) return;
  mpUserPausedUntil = Date.now() + 3000;
  if (mpUserScrollTimer) clearTimeout(mpUserScrollTimer);
  mpUserScrollTimer = setTimeout(() => {
    mpUserPausedUntil = 0;
  }, 3100);
};

// ---- Transport / formatting ----------------------------------------------
const seekValue = ref(0);
let seekHeld = false;

// Keep the seek slider in sync with playback unless the user is dragging it.
watch(
  () => store.currentTime,
  (t) => {
    if (!seekHeld) seekValue.value = t || 0;
  },
  { immediate: true }
);

const onSeekInput = () => {
  seekHeld = true;
  store.lastSeekAt = Date.now();
  store.currentTime = Number(seekValue.value);
};
const onSeekCommit = () => {
  seekHeld = false;
  store.seek(Number(seekValue.value));
};
const seekToLine = (line) => {
  if (line.time_ms != null) store.seek(line.time_ms / 1000);
};

const progressPercentage = computed(() => {
  const max = store.duration || 100;
  return Math.min(Math.max((Number(seekValue.value) / max) * 100, 0), 100);
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

// ---- Queue (mirrors the main QueuePanel: drag-to-reorder, remove, etc.) ----
const queueListEl = ref(null);
const dragIndex = ref(-1);
const overIndex = ref(-1);

const isCurrent = (s) => store.currentSong && store.currentSong.path === s.path;

const keyMap = new WeakMap();
let keySeq = 0;
const keyFor = (item) => {
  let k = keyMap.get(item);
  if (k === undefined) {
    k = ++keySeq;
    keyMap.set(item, k);
  }
  return k;
};

const getRowIndexFromY = (clientY) => {
  if (!queueListEl.value) return -1;
  const rows = queueListEl.value.querySelectorAll('[data-queue-idx]');
  for (const row of rows) {
    const rect = row.getBoundingClientRect();
    if (clientY >= rect.top && clientY <= rect.bottom) {
      return parseInt(row.dataset.queueIdx, 10);
    }
  }
  if (rows.length > 0) {
    const firstRect = rows[0].getBoundingClientRect();
    if (clientY < firstRect.top) return 0;
    const lastRect = rows[rows.length - 1].getBoundingClientRect();
    if (clientY > lastRect.bottom) return rows.length - 1;
  }
  return -1;
};

const onQueueMouseMove = (e) => {
  if (dragIndex.value === -1) return;
  e.preventDefault();
  const idx = getRowIndexFromY(e.clientY);
  if (idx !== -1) overIndex.value = idx;
};

const onQueueMouseUp = () => {
  if (dragIndex.value !== -1 && overIndex.value !== -1 && dragIndex.value !== overIndex.value) {
    store.moveInQueue(dragIndex.value, overIndex.value);
  }
  dragIndex.value = -1;
  overIndex.value = -1;
  document.removeEventListener('mousemove', onQueueMouseMove);
  document.removeEventListener('mouseup', onQueueMouseUp);
  document.body.style.userSelect = '';
  document.body.style.cursor = '';
};

const onGripMouseDown = (index, e) => {
  e.preventDefault();
  e.stopPropagation();
  dragIndex.value = index;
  overIndex.value = index;
  document.body.style.userSelect = 'none';
  document.body.style.cursor = 'grabbing';
  document.addEventListener('mousemove', onQueueMouseMove);
  document.addEventListener('mouseup', onQueueMouseUp);
};

// ---- Navigation / window / view actions -----------------------------------
const goToArtist = (artistName) => {
  if (!artistName || artistName === 'Unknown Artist') return;
  store.exitMiniPlayer().finally(() => {
    router.push({ name: 'ArtistDetail', params: { name: artistName } });
  });
};

// Expand to the fullscreen Now-Playing view (restore the window first).
const expandToFull = () => {
  store.exitMiniPlayer().finally(() => store.openFullscreen());
};

const closeWindow = () => {
  store.exitMiniPlayer().finally(() => {
    getCurrentWindow().close().catch(() => {});
  });
};

// Remember whether the queue was opened from the album-art view, so closing it
// returns there instead of dropping back to the lyrics/compact view. The restore
// is *deferred* to onQueueAfterLeave so the player chrome doesn't change (e.g. the
// album info row popping in) while the queue is still sliding out.
let queuePrevArtwork = false;
let restoreArtworkOnLeave = false;

const toggleLyrics = () => {
  // From the lyrics view → hide (compact); from anywhere else → show lyrics.
  lyricsEnabled.value = view.value !== 'lyrics';
  artworkMode.value = false;
  restoreArtworkOnLeave = false; // explicit view change — don't restore on close
  queueOpen.value = false;
};
const openArtwork = () => {
  artworkMode.value = true;
  restoreArtworkOnLeave = false; // explicit view change — don't restore on close
  queueOpen.value = false;
};
const closeArtwork = () => {
  artworkMode.value = false;
};
const toggleQueue = () => {
  if (queueOpen.value) {
    // Close: keep the current (queue) chrome through the slide-out; the view we
    // came from is restored only once the animation finishes.
    restoreArtworkOnLeave = true;
    queueOpen.value = false;
  } else {
    queuePrevArtwork = artworkMode.value;
    restoreArtworkOnLeave = false;
    queueOpen.value = true;
    queuePresent.value = true; // keep the window tall through the slide-in
    artworkMode.value = false; // the queue always shows over the gradient, not art
  }
};
// Settle the final view only after the queue's slide-out has finished, so nothing
// flashes into the player controls mid-animation.
const onQueueAfterLeave = () => {
  if (restoreArtworkOnLeave) {
    artworkMode.value = queuePrevArtwork;
    restoreArtworkOnLeave = false;
  }
  queuePresent.value = false;
};

// Close popovers when clicking elsewhere.
const closePopovers = () => {
  volumeOpen.value = false;
  moreOpen.value = false;
};
onMounted(() => document.addEventListener('click', closePopovers));
onUnmounted(() => {
  document.removeEventListener('click', closePopovers);
  document.removeEventListener('mousemove', onQueueMouseMove);
  document.removeEventListener('mouseup', onQueueMouseUp);
  if (hideTimer) clearTimeout(hideTimer);
  if (mpRafId) cancelAnimationFrame(mpRafId);
  if (mpUserScrollTimer) clearTimeout(mpUserScrollTimer);
});
</script>

<template>
  <Transition name="mini">
    <div
      v-if="store.miniPlayerOpen"
      class="fixed inset-0 z-[300] flex flex-col overflow-hidden text-white select-none bg-[#0a0a0a]"
      @mousemove="onActivity"
      @mouseleave="onLeave"
    >
      <!-- Background: full art in artwork view, animated gradient elsewhere -->
      <div v-if="isArtwork" data-tauri-drag-region class="absolute inset-0 bg-[#0a0a0a]">
        <img
          v-if="coverUrl"
          :src="coverUrl"
          class="object-cover w-full h-full pointer-events-none"
          alt=""
          draggable="false"
        />
        <div v-else class="flex items-center justify-center w-full h-full text-white/20">
          <svg xmlns="http://www.w3.org/2000/svg" class="w-1/4 h-1/4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.2">
            <path d="M9 18V5l12-2v13"></path><circle cx="6" cy="18" r="3"></circle><circle cx="18" cy="16" r="3"></circle>
          </svg>
        </div>
      </div>
      <div v-else class="absolute inset-0 overflow-hidden pointer-events-none bg-[#0a0a0a]">
        <div class="absolute inset-0 opacity-90 blur-[60px] scale-[2.2] origin-center">
          <div class="mini-blob mini-blob-1" :style="{ backgroundColor: colors[0] }"></div>
          <div class="mini-blob mini-blob-2" :style="{ backgroundColor: colors[1] }"></div>
          <div class="mini-blob mini-blob-3" :style="{ backgroundColor: colors[2] }"></div>
          <div class="mini-blob mini-blob-4" :style="{ backgroundColor: colors[0] }"></div>
        </div>
        <div class="absolute inset-0 bg-[#0a0a0a]/40"></div>
        <div class="absolute inset-0 bg-gradient-to-t from-[#0a0a0a] via-[#0a0a0a]/20 to-[#0a0a0a]/10"></div>
      </div>

      <!-- ============================ TOP CHROME ======================== -->
      <div
        ref="topChromeEl"
        data-tauri-drag-region
        class="z-20 flex items-start gap-3 px-3 pt-3 pb-2 shrink-0 transition-opacity duration-300"
        :class="[
          isArtwork ? 'absolute top-0 left-0 right-0 bg-gradient-to-b from-black/60 to-transparent pb-6' : 'relative',
          topChromeVisible ? 'opacity-100' : 'opacity-0 pointer-events-none',
        ]"
      >
        <!-- Cover thumbnail (click → artwork view) — hidden in artwork view -->
        <button
          v-if="!isArtwork"
          @click="openArtwork"
          class="h-12 w-12 rounded-md overflow-hidden shrink-0 shadow-lg bg-[#333] border border-white/10 relative group focus:outline-none"
          title="Show album art"
        >
          <img v-if="coverUrl" :src="coverUrl" class="object-cover w-full h-full" alt="" draggable="false" />
          <div v-else class="flex items-center justify-center w-full h-full text-white/30">
            <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
              <path d="M9 18V5l12-2v13"></path><circle cx="6" cy="18" r="3"></circle><circle cx="18" cy="16" r="3"></circle>
            </svg>
          </div>
          <div class="absolute inset-0 flex items-center justify-center transition-opacity opacity-0 bg-black/40 group-hover:opacity-100">
            <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="15 3 21 3 21 9"></polyline><polyline points="9 21 3 21 3 15"></polyline><line x1="21" y1="3" x2="14" y2="10"></line><line x1="3" y1="21" x2="10" y2="14"></line>
            </svg>
          </div>
        </button>

        <!-- Title / artist / lossless (non-artwork) -->
        <div v-if="!isArtwork" class="flex-1 min-w-0 text-center pt-0.5">
          <div class="text-sm font-semibold leading-tight truncate pointer-events-none">
            {{ song ? song.title : 'Not Playing' }}
          </div>
          <div v-if="song" class="text-xs leading-tight truncate text-white/60 pointer-events-none">
            {{ song.artist }}<span v-if="song.album"> — {{ song.album }}</span>
          </div>
          <div v-if="song" class="flex justify-center mt-1">
            <LosslessBadge placement="down" />
          </div>
        </div>
        <div v-else class="flex-1"></div>

        <!-- Romaji toggle (only in the lyrics view when romanization exists) -->
        <button
          v-if="view === 'lyrics' && hasRomaji"
          @click="store.toggleRomaji()"
          class="flex items-center justify-center w-7 h-7 rounded-full text-white shrink-0 self-center transition-colors"
          :class="store.showRomaji ? 'bg-white/25' : 'bg-white/10 hover:bg-white/20'"
          :title="store.showRomaji ? 'Hide romaji' : 'Show romaji'"
        >
          <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m5 8 6 6" /><path d="m4 14 6-6 2-3" /><path d="M2 5h12" /><path d="M7 2h1" /><path d="m22 22-5-10-5 10" /><path d="M14 18h6" />
          </svg>
        </button>

        <!-- Window controls -->
        <div class="flex items-center gap-0.5 shrink-0">
          <button
            @click="expandToFull"
            class="p-1.5 rounded-md text-white/70 hover:text-white hover:bg-white/15 transition"
            title="Expand to full player"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="15 3 21 3 21 9"></polyline><polyline points="9 21 3 21 3 15"></polyline><line x1="21" y1="3" x2="14" y2="10"></line><line x1="3" y1="21" x2="10" y2="14"></line>
            </svg>
          </button>
          <button
            @click="closeWindow"
            class="p-1.5 rounded-md text-white/70 hover:text-white hover:bg-[#e81123] transition"
            title="Close window"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line>
            </svg>
          </button>
        </div>
      </div>

      <!-- ============================ MIDDLE ============================ -->
      <div
        class="relative z-10 min-h-0"
        :class="(view === 'lyrics' || isArtwork || queuePresent) ? 'flex-1' : ''"
        :data-tauri-drag-region="isArtwork && !queueOpen ? '' : null"
      >
        <!-- Queue (mirrors the main QueuePanel) — slides up as a frosted sheet over
             the lyrics/backdrop; positioned so it overlays whatever is behind it. -->
        <Transition name="mini-queue" @after-leave="onQueueAfterLeave">
          <div v-if="queueOpen" class="absolute inset-0 z-10 flex flex-col mini-queue-panel">
          <div class="flex items-center justify-between px-4 py-2.5 shrink-0">
            <h2 class="text-sm font-bold">Queue</h2>
            <div class="flex items-center gap-3">
              <button
                v-if="store.queue.length > 1"
                @click="store.clearQueue()"
                class="text-xs text-[var(--text-secondary)] hover:text-white transition"
                title="Clear queue"
              >
                Clear
              </button>
              <button
                @click="store.toggleAutoplay()"
                class="transition"
                :class="store.autoplayMode ? 'text-[var(--accent-color)]' : 'text-gray-400 hover:text-white'"
                :title="store.autoplayMode ? 'Autoplay on' : 'Autoplay off'"
              >
                <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M12 12c-2-2.67-4-4-6-4a4 4 0 1 0 0 8c2 0 4-1.33 6-4Zm0 0c2 2.67 4 4 6 4a4 4 0 0 0 0-8c-2 0-4 1.33-6 4Z" />
                </svg>
              </button>
            </div>
          </div>
          <div ref="queueListEl" class="relative flex-1 px-2 pt-1 pb-36 overflow-auto mini-scroll">
            <div v-if="store.queue.length === 0" class="p-8 text-sm text-center text-gray-600">
              The queue is empty.
            </div>
            <TransitionGroup v-else name="queue" tag="div" class="space-y-1">
              <div
                v-for="(qsong, index) in store.queue"
                :key="keyFor(qsong)"
                :data-queue-idx="index"
                @dblclick="store.playQueueIndex(index)"
                class="queue-row group flex items-center gap-2 p-1.5 rounded-md hover:bg-white/10 transition-colors"
                :class="{
                  'bg-white/10': isCurrent(qsong),
                  'opacity-30': index === dragIndex,
                  'drop-target-above': overIndex === index && dragIndex !== index && dragIndex > index,
                  'drop-target-below': overIndex === index && dragIndex !== index && dragIndex < index,
                }"
              >
                <div
                  class="shrink-0 cursor-grab active:cursor-grabbing text-gray-500 hover:text-gray-200 transition-colors"
                  @mousedown="onGripMouseDown(index, $event)"
                  title="Drag to reorder"
                >
                  <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="currentColor" stroke="none">
                    <circle cx="9" cy="5" r="1.5"></circle><circle cx="15" cy="5" r="1.5"></circle><circle cx="9" cy="12" r="1.5"></circle><circle cx="15" cy="12" r="1.5"></circle><circle cx="9" cy="19" r="1.5"></circle><circle cx="15" cy="19" r="1.5"></circle>
                  </svg>
                </div>
                <CoverImage :path="qsong.path" className="h-9 w-9 rounded shrink-0 bg-[#333]" />
                <div class="flex-1 min-w-0" @click="store.playQueueIndex(index)">
                  <div class="text-[12px] font-medium truncate leading-tight" :class="isCurrent(qsong) ? 'text-[var(--accent-color)]' : 'text-white'">
                    {{ qsong.title }}
                  </div>
                  <div
                    @click.stop="goToArtist(qsong.artist)"
                    class="text-[11px] text-[var(--text-secondary)] hover:text-[var(--accent-color)] hover:underline cursor-pointer truncate transition-colors"
                  >
                    {{ qsong.artist }}
                  </div>
                </div>
                <button
                  @click.stop="store.removeFromQueue(index)"
                  class="text-gray-400 transition opacity-0 group-hover:opacity-100 hover:text-white shrink-0"
                  title="Remove from queue"
                >
                  <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line>
                  </svg>
                </button>
              </div>
            </TransitionGroup>
          </div>
          </div>
        </Transition>

        <!-- Lyrics (stays mounted behind the queue; the frosted queue overlays it) -->
        <div
          v-if="view === 'lyrics'"
          ref="linesEl"
          class="h-full overflow-y-auto mini-lyrics-scroll px-5 py-[30%]"
          @scroll.passive="onLyricsScroll"
        >
          <div v-if="lyricsLoading" class="text-lg font-semibold text-white/40">Loading lyrics…</div>

          <template v-else-if="lines.length">
            <p
              v-for="(line, i) in lines"
              :key="i"
              :data-line="i"
              @click="seekToLine(line)"
              class="text-xl font-semibold leading-snug tracking-tight mini-line"
              :class="[
                synced ? 'cursor-pointer' : '',
                i === activeIdx ? 'mini-line-active' : 'mini-line-dim',
                line.isGap ? 'mini-line-gap' : 'mb-3',
                line.words && line.words.length ? 'mini-words' : '',
              ]"
            >
              <span v-if="line.isGap" class="mini-gap-dots" :class="{ 'mini-gap-dots-active': i === activeIdx }">
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
                :show-romaji="store.showRomaji && hasRomaji"
              />
            </p>
          </template>

          <div v-else class="flex flex-col items-center justify-center h-full text-center text-white/50">
            <div class="mb-1 text-base font-semibold">Lyrics not found</div>
            <button @click="fetchLyrics(true)" class="mt-2 text-xs px-3 py-1.5 rounded-md bg-white/10 hover:bg-white/20 transition">
              Retry
            </button>
          </div>
        </div>
      </div>

      <!-- ============================ BOTTOM CHROME ===================== -->
      <!-- Absolute (overlay) in lyrics + artwork so the lyrics fill the full
           height and run right to the bottom when these controls fade out. -->
      <div
        ref="bottomChromeEl"
        class="z-20 px-4 pt-1 pb-4 shrink-0 transition-opacity duration-300"
        :class="[
          (isArtwork || view === 'lyrics' || queuePresent) ? 'absolute bottom-0 left-0 right-0 bg-gradient-to-t from-black/85 via-black/55 to-transparent pt-12' : 'relative',
          bottomChromeVisible ? 'opacity-100' : 'opacity-0 pointer-events-none',
        ]"
      >
        <!-- Artwork-view info row (collapse + title/artist/lossless + love) -->
        <div v-if="isArtwork && song" class="flex items-end gap-2 mb-2">
          <button
            @click="closeArtwork"
            class="p-1.5 -ml-1 rounded-md text-white/80 hover:text-white hover:bg-white/15 transition shrink-0"
            title="Back to player"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <rect x="3" y="4" width="18" height="16" rx="2"></rect><rect x="6.5" y="13" width="7" height="4" rx="1" fill="currentColor" stroke="none"></rect>
            </svg>
          </button>
          <div class="flex-1 min-w-0">
            <div class="text-sm font-semibold leading-tight truncate">{{ song.title }}</div>
            <div class="text-xs leading-tight truncate text-white/60">
              {{ song.artist }}<span v-if="song.album"> — {{ song.album }}</span>
            </div>
            <div class="mt-1"><LosslessBadge placement="up" /></div>
          </div>
          <button
            @click="store.toggleFavorite(song.path)"
            class="transition shrink-0 hover:scale-110"
            :class="store.isFavorite(song.path) ? 'text-[var(--accent-color)]' : 'text-white/60 hover:text-white'"
            title="Love"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="17" height="17" viewBox="0 0 24 24" :fill="store.isFavorite(song.path) ? 'currentColor' : 'none'" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"></path>
            </svg>
          </button>
        </div>

        <!-- Progress (reactive thumb, like the main player) -->
        <input
          type="range"
          min="0"
          :max="Math.max(store.duration || 100, seekValue)"
          v-model.number="seekValue"
          @input="onSeekInput"
          @change="onSeekCommit"
          :disabled="!song"
          class="mini-seek w-full appearance-none cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
          :style="{ background: `linear-gradient(to right, #fff ${progressPercentage}%, rgba(255,255,255,0.28) ${progressPercentage}%)` }"
        />
        <div class="flex justify-between text-[10px] text-white/50 mt-1 mb-2 tabular-nums">
          <span>{{ formatTime(store.currentTime) }}</span>
          <span>-{{ remaining }}</span>
        </div>

        <!-- Controls -->
        <div class="flex items-center justify-between">
          <!-- Left: volume + more -->
          <div class="flex items-center gap-1">
            <div class="relative">
              <button
                @click.stop="volumeOpen = !volumeOpen; moreOpen = false"
                class="p-1.5 rounded-full text-white/70 hover:text-white hover:bg-white/10 transition"
                :title="store.isMuted ? 'Unmute' : 'Volume'"
              >
                <svg
                  xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none"
                  stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
                >
                  <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"></polygon>
                  <template v-if="store.isMuted">
                    <line x1="23" y1="9" x2="17" y2="15"></line><line x1="17" y1="9" x2="23" y2="15"></line>
                  </template>
                  <template v-else>
                    <path d="M15.54 8.46a5 5 0 0 1 0 7.07"></path>
                    <path v-if="store.volume > 0.5" d="M19.07 4.93a10 10 0 0 1 0 14.14"></path>
                  </template>
                </svg>
              </button>
              <!-- Volume popover (clean horizontal slider) -->
              <div
                v-if="volumeOpen"
                @click.stop
                class="absolute bottom-full left-0 mb-3 flex items-center gap-2.5 px-3 py-2.5 rounded-full bg-[#2c2c2e] border border-white/10 shadow-2xl w-[200px] animate-mini-pop"
              >
                <div class="absolute top-full left-4 -translate-y-1/2 w-2.5 h-2.5 bg-[#2c2c2e] border-r border-b border-white/10 rotate-45"></div>
                <button
                  @click="store.toggleMute()"
                  class="text-white/80 hover:text-white shrink-0 flex items-center justify-center w-[15px] h-[15px]"
                  :title="store.isMuted ? 'Unmute' : 'Mute'"
                >
                  <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="block">
                    <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"></polygon>
                    <template v-if="store.isMuted">
                      <line x1="23" y1="9" x2="17" y2="15"></line><line x1="17" y1="9" x2="23" y2="15"></line>
                    </template>
                    <path v-else d="M15.54 8.46a5 5 0 0 1 0 7.07"></path>
                  </svg>
                </button>
                <input
                  type="range" min="0" max="1" step="0.01"
                  :value="store.isMuted ? 0 : store.volume"
                  @input="store.setVolume($event.target.value)"
                  class="mini-vol flex-1 appearance-none cursor-pointer"
                  :style="{ background: `linear-gradient(to right, var(--accent-color) ${volumePercentage}%, rgba(255,255,255,0.22) ${volumePercentage}%)` }"
                />
                <span class="shrink-0 flex items-center justify-center w-[15px] h-[15px] text-white/80">
                  <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="block">
                    <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"></polygon><path d="M19.07 4.93a10 10 0 0 1 0 14.14"></path><path d="M15.54 8.46a5 5 0 0 1 0 7.07"></path>
                  </svg>
                </span>
              </div>
            </div>

            <div class="relative">
              <button
                @click.stop="moreOpen = !moreOpen; volumeOpen = false"
                class="p-1.5 rounded-full transition"
                :class="(store.shuffleMode || store.loopMode > 0) ? 'text-[var(--accent-color)]' : 'text-white/70 hover:text-white hover:bg-white/10'"
                title="More"
              >
                <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
                  <circle cx="5" cy="12" r="1.6"></circle><circle cx="12" cy="12" r="1.6"></circle><circle cx="19" cy="12" r="1.6"></circle>
                </svg>
              </button>
              <div
                v-if="moreOpen"
                @click.stop
                class="absolute bottom-full left-0 mb-2 py-1.5 rounded-xl bg-[#1c1c1e] border border-[#323236] shadow-2xl w-[160px] animate-mini-pop"
              >
                <button
                  @click="store.toggleShuffle()"
                  class="flex items-center justify-between w-full px-3 py-2 text-xs hover:bg-white/10 transition"
                  :class="store.shuffleMode ? 'text-[var(--accent-color)]' : 'text-white/80'"
                >
                  <span class="flex items-center gap-2">
                    <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M16 3h5v5M4 20L21 3M21 16v5h-5M15 15l6 6M4 4l5 5" />
                    </svg>
                    Shuffle
                  </span>
                  <span v-if="store.shuffleMode" class="text-[9px] font-bold uppercase">On</span>
                </button>
                <button
                  @click="store.toggleLoop()"
                  class="flex items-center justify-between w-full px-3 py-2 text-xs hover:bg-white/10 transition"
                  :class="store.loopMode > 0 ? 'text-[var(--accent-color)]' : 'text-white/80'"
                >
                  <span class="flex items-center gap-2">
                    <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M17 1l4 4-4 4"></path><path d="M3 11V9a4 4 0 0 1 4-4h14"></path><path d="M7 23l-4-4 4-4"></path><path d="M21 13v2a4 4 0 0 1-4 4H3"></path>
                    </svg>
                    Repeat{{ store.loopMode === 2 ? ' One' : '' }}
                  </span>
                  <span v-if="store.loopMode > 0" class="text-[9px] font-bold uppercase">On</span>
                </button>
                <button
                  v-if="song"
                  @click="store.toggleFavorite(song.path)"
                  class="flex items-center gap-2 w-full px-3 py-2 text-xs hover:bg-white/10 transition"
                  :class="store.isFavorite(song.path) ? 'text-[var(--accent-color)]' : 'text-white/80'"
                >
                  <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" :fill="store.isFavorite(song.path) ? 'currentColor' : 'none'" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"></path>
                  </svg>
                  {{ store.isFavorite(song.path) ? 'Loved' : 'Love' }}
                </button>
              </div>
            </div>
          </div>

          <!-- Center: transport -->
          <div class="flex items-center gap-4">
            <button @click="store.prevSong()" :disabled="!song" class="text-white/90 hover:text-white transition disabled:opacity-30" title="Previous">
              <svg xmlns="http://www.w3.org/2000/svg" width="22" height="22" viewBox="0 0 24 24" fill="currentColor">
                <polygon points="19 20 9 12 19 4 19 20"></polygon><rect x="4" y="4" width="2.4" height="16"></rect>
              </svg>
            </button>
            <button @click="store.togglePlay()" :disabled="!song" class="text-white hover:scale-105 transition disabled:opacity-30" title="Play/Pause">
              <svg v-if="store.isPlaying" xmlns="http://www.w3.org/2000/svg" width="30" height="30" viewBox="0 0 24 24" fill="currentColor">
                <rect x="6" y="4" width="4" height="16"></rect><rect x="14" y="4" width="4" height="16"></rect>
              </svg>
              <svg v-else xmlns="http://www.w3.org/2000/svg" width="30" height="30" viewBox="0 0 24 24" fill="currentColor">
                <polygon points="6 3 20 12 6 21 6 3"></polygon>
              </svg>
            </button>
            <button @click="store.nextSong(true)" :disabled="!song" class="text-white/90 hover:text-white transition disabled:opacity-30" title="Next">
              <svg xmlns="http://www.w3.org/2000/svg" width="22" height="22" viewBox="0 0 24 24" fill="currentColor">
                <polygon points="5 4 15 12 5 20 5 4"></polygon><rect x="17.6" y="4" width="2.4" height="16"></rect>
              </svg>
            </button>
          </div>

          <!-- Right: lyrics (only when available) + queue -->
          <div class="flex items-center gap-1">
            <button
              v-if="lyricsButtonVisible"
              @click="toggleLyrics"
              class="p-1.5 rounded-md transition"
              :class="view === 'lyrics' ? 'text-white bg-white/20' : 'text-white/70 hover:text-white hover:bg-white/10'"
              :title="view === 'lyrics' ? 'Hide lyrics' : 'Show lyrics'"
            >
              <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"></path>
                <line x1="8" y1="10" x2="16" y2="10"></line><line x1="8" y1="14" x2="12" y2="14"></line>
              </svg>
            </button>
            <button
              @click="toggleQueue"
              class="p-1.5 rounded-md transition relative"
              :class="queueOpen ? 'text-white bg-white/20' : 'text-white/70 hover:text-white hover:bg-white/10'"
              title="Queue"
            >
              <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <line x1="3" y1="6" x2="16" y2="6"></line><line x1="3" y1="12" x2="13" y2="12"></line><line x1="3" y1="18" x2="13" y2="18"></line>
                <polygon points="18 14 22 16.5 18 19" fill="currentColor" stroke="none"></polygon><line x1="18" y1="9" x2="18" y2="13"></line>
              </svg>
              <span
                v-if="store.autoplayMode"
                class="absolute -top-1.5 -right-1.5 h-3.5 w-3.5 rounded-full bg-[var(--accent-color)] flex items-center justify-center ring-2 ring-[#0a0a0a] shadow"
                title="Autoplay on"
              >
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  width="9"
                  height="9"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="white"
                  stroke-width="2.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <path d="M12 12c-2-2.67-4-4-6-4a4 4 0 1 0 0 8c2 0 4-1.33 6-4Zm0 0c2 2.67 4 4 6 4a4 4 0 0 0 0-8c-2 0-4 1.33-6 4Z" />
                </svg>
              </span>
            </button>
          </div>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
/* Reactive progress seeker — thumb appears on hover, bar thickens (main player) */
.mini-seek {
  height: 4px;
  border-radius: 9999px;
  transition: height 0.15s cubic-bezier(0.4, 0, 0.2, 1);
}
.mini-seek::-webkit-slider-thumb {
  -webkit-appearance: none;
  height: 12px;
  width: 12px;
  border-radius: 50%;
  background: #fff;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.45);
  transform: scale(0);
  transition: transform 0.15s cubic-bezier(0.4, 0, 0.2, 1), margin-top 0.15s cubic-bezier(0.4, 0, 0.2, 1);
}
.mini-seek::-moz-range-thumb {
  height: 12px;
  width: 12px;
  border: 0;
  border-radius: 50%;
  background: #fff;
  transform: scale(0);
  transition: transform 0.15s cubic-bezier(0.4, 0, 0.2, 1);
}
.mini-seek:hover,
.mini-seek:active {
  height: 6px;
}
.mini-seek:hover::-webkit-slider-thumb,
.mini-seek:active::-webkit-slider-thumb {
  transform: scale(1);
  margin-top: -3px;
}
.mini-seek:hover::-moz-range-thumb,
.mini-seek:active::-moz-range-thumb {
  transform: scale(1);
}

/* Volume slider — always-visible accent thumb */
.mini-vol {
  height: 4px;
  border-radius: 9999px;
}
.mini-vol::-webkit-slider-thumb {
  -webkit-appearance: none;
  height: 13px;
  width: 13px;
  border-radius: 50%;
  background: var(--accent-color);
  margin-top: -4.5px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.5);
}
.mini-vol::-moz-range-thumb {
  height: 13px;
  width: 13px;
  border: 0;
  border-radius: 50%;
  background: var(--accent-color);
}

/* Animated gradient backdrop blobs (ported from the fullscreen player) */
.mini-blob {
  position: absolute;
  border-radius: 50%;
  mix-blend-mode: screen;
  transition: background-color 1.8s ease-in-out;
  opacity: 0.95;
  will-change: transform;
}
.mini-blob-1 {
  width: 55%;
  height: 55%;
  left: -10%;
  top: -10%;
  animation: mini-blob-move-1 28s infinite alternate ease-in-out;
}
.mini-blob-2 {
  width: 50%;
  height: 50%;
  right: -5%;
  bottom: -5%;
  animation: mini-blob-move-2 33s infinite alternate ease-in-out;
}
.mini-blob-3 {
  width: 45%;
  height: 45%;
  left: 20%;
  top: 20%;
  animation: mini-blob-move-3 30s infinite alternate ease-in-out;
}
.mini-blob-4 {
  width: 48%;
  height: 48%;
  right: 5%;
  top: -5%;
  animation: mini-blob-move-4 25s infinite alternate ease-in-out;
}
@keyframes mini-blob-move-1 {
  0% { transform: translate(0, 0) scale(1) rotate(0deg); }
  50% { transform: translate(40px, 25px) scale(1.1) rotate(180deg); }
  100% { transform: translate(15px, 55px) scale(0.95) rotate(360deg); }
}
@keyframes mini-blob-move-2 {
  0% { transform: translate(0, 0) scale(1) rotate(0deg); }
  50% { transform: translate(-40px, -50px) scale(0.9) rotate(-180deg); }
  100% { transform: translate(-15px, 20px) scale(1.05) rotate(-360deg); }
}
@keyframes mini-blob-move-3 {
  0% { transform: translate(0, 0) scale(1); }
  50% { transform: translate(30px, -30px) scale(1.08); }
  100% { transform: translate(-40px, 40px) scale(0.92); }
}
@keyframes mini-blob-move-4 {
  0% { transform: translate(0, 0) scale(1); }
  50% { transform: translate(-35px, 35px) scale(1.04); }
  100% { transform: translate(35px, -20px) scale(0.95); }
}

/* Lyrics scroll area: fade top/bottom, hidden scrollbar */
.mini-lyrics-scroll {
  scrollbar-width: none;
  -webkit-mask-image: linear-gradient(to bottom, transparent 0, #000 14%, #000 86%, transparent 100%);
  mask-image: linear-gradient(to bottom, transparent 0, #000 14%, #000 86%, transparent 100%);
}
.mini-lyrics-scroll::-webkit-scrollbar {
  width: 0;
}
.mini-scroll {
  scrollbar-width: thin;
  scrollbar-color: rgba(255, 255, 255, 0.18) transparent;
}
.mini-scroll::-webkit-scrollbar {
  width: 5px;
}
.mini-scroll::-webkit-scrollbar-thumb {
  background: rgba(255, 255, 255, 0.18);
  border-radius: 4px;
}

.mini-line {
  transition:
    color 0.45s cubic-bezier(0.25, 1, 0.5, 1),
    opacity 0.45s cubic-bezier(0.25, 1, 0.5, 1),
    transform 0.5s cubic-bezier(0.34, 1.56, 0.64, 1);
  transform-origin: left center;
  padding-right: 12px;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.25);
}
.mini-line.mini-words.mini-line-active {
  transition:
    color 0.45s cubic-bezier(0.25, 1, 0.5, 1),
    transform 0.5s cubic-bezier(0.34, 1.56, 0.64, 1);
}
.mini-line-active {
  color: rgba(255, 255, 255, 0.98);
  opacity: 1;
  transform: translateX(4px) scale(1.01);
}
.mini-line-dim {
  color: rgba(255, 255, 255, 0.98);
  opacity: 0.3;
  transform: translateX(0) scale(1);
}
.mini-line-dim:hover {
  opacity: 0.55;
}

.mini-gap-dots {
  opacity: 0;
  transition: opacity 0.35s ease;
  pointer-events: none;
  display: inline-block;
}
.mini-gap-dots-active {
  opacity: 1;
  pointer-events: auto;
}
.dots-wrapper {
  display: inline-flex;
  gap: 0.45rem;
  font-size: 1.5rem;
  line-height: 1;
  vertical-align: middle;
  font-weight: 800;
}
.dots-wrapper span {
  transition: color 0.25s linear;
}
.mini-line-gap {
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
.mini-line-gap.mini-line-active {
  height: 2.5rem;
  margin-bottom: 0.75rem !important;
  opacity: 1;
}

/* Overlay open/close */
.mini-enter-active,
.mini-leave-active {
  transition: opacity 0.2s ease;
}
.mini-enter-from,
.mini-leave-to {
  opacity: 0;
}

/* Queue drag-to-reorder (mirrors the main QueuePanel) */
.drop-target-above {
  box-shadow: inset 0 2px 0 0 var(--accent-color);
}
.drop-target-below {
  box-shadow: inset 0 -2px 0 0 var(--accent-color);
}
.queue-move {
  transition: transform 0.3s cubic-bezier(0.22, 0.61, 0.36, 1);
}
.queue-leave-active {
  transition: opacity 0.2s ease;
  position: absolute;
  width: calc(100% - 1rem);
}
.queue-leave-to {
  opacity: 0;
}

/* Queue panel open/close: a frosted sheet that slides up over the lyrics/backdrop
   and cross-fades out on close. */
.mini-queue-panel {
  background-color: rgba(10, 10, 10, 0.72);
  backdrop-filter: blur(26px) saturate(135%);
  -webkit-backdrop-filter: blur(26px) saturate(135%);
}
/* The blur + background are animated alongside opacity so the frosted sheet truly
   dissolves. Chromium/WebView2 does NOT fade `backdrop-filter` with opacity alone,
   so without this the panel stays a solid blurred block and snaps out at the end. */
.mini-queue-enter-active {
  transition:
    opacity 0.32s ease,
    transform 0.4s cubic-bezier(0.16, 1, 0.3, 1),
    background-color 0.32s ease,
    backdrop-filter 0.32s ease,
    -webkit-backdrop-filter 0.32s ease;
}
.mini-queue-leave-active {
  transition:
    opacity 0.32s ease,
    transform 0.34s cubic-bezier(0.33, 0, 0.67, 1),
    background-color 0.32s ease,
    backdrop-filter 0.32s ease,
    -webkit-backdrop-filter 0.32s ease;
}
.mini-queue-enter-from,
.mini-queue-leave-to {
  opacity: 0;
  transform: translateY(20px);
  background-color: rgba(10, 10, 10, 0);
  backdrop-filter: blur(0px) saturate(100%);
  -webkit-backdrop-filter: blur(0px) saturate(100%);
}

.animate-mini-pop {
  animation: miniPop 0.14s cubic-bezier(0.16, 1, 0.3, 1) forwards;
  transform-origin: bottom left;
}
@keyframes miniPop {
  from {
    opacity: 0;
    transform: translateY(4px) scale(0.96);
  }
  to {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
}
</style>
