<script setup>
import { ref, watch, onMounted, onUnmounted, computed } from 'vue';
import { store } from '../store';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import CoverImage from './CoverImage.vue';
import Visualizer from './Visualizer.vue';

// Playback is handled natively in Rust (rodio + symphonia). This component just
// issues commands and polls the backend for the current position/duration.
const seekValue = ref(0);
const playbackError = ref(null);

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

const progressPercentage = computed(() => {
  const max = store.duration || 100;
  const val = Number(seekValue.value) || 0;
  return Math.min(Math.max((val / max) * 100, 0), 100);
});

const volumePercentage = computed(() => {
  return (store.isMuted ? 0 : store.volume) * 100;
});

let pollTimer = null;
let stateTimer = null;
let lastSeekAt = 0; // suppress poll overwriting the slider right after a seek
let endedHandledFor = null; // latch so a finished track only advances once
let loadToken = 0; // guards against a stale load winning after a rapid skip

// Load (and usually play) whenever the selected song changes.
watch(
  () => store.currentSong,
  async (song) => {
    if (!song) return;
    losslessPopupOpen.value = false;
    playbackError.value = null;
    endedHandledFor = null;

    // Consume the one-shot load hints (set by resume-on-launch / normal plays).
    const startAt = store.pendingSeek;
    const autoplay = store.pendingAutoplay;
    store.pendingSeek = null;
    store.pendingAutoplay = true;

    const token = ++loadToken;
    store.isBuffering = true;
    const startPos = startAt || 0;
    store.currentTime = startPos;
    seekValue.value = startPos;

    try {
      const info = await invoke('player_load', {
        path: song.path,
        volume: store.isMuted ? 0 : store.volume,
        startAt,
        autoplay,
        durationHint: song.duration_secs || 0,
      });
      if (token !== loadToken) return; // a newer track was selected meanwhile
      store.duration = info.duration || 0;
      store.currentSampleRate = info.sample_rate;
      store.currentBitDepth = info.bit_depth;
      store.currentTime = startPos;
      seekValue.value = startPos;
      store.isPlaying = autoplay;
      pushMediaMetadata(song);
      pushMediaPlayback();
    } catch (err) {
      if (token !== loadToken) return;
      playbackError.value = String(err);
      store.isPlaying = false;
    } finally {
      if (token === loadToken) store.isBuffering = false;
    }
  },
  { immediate: true }
);

watch(
  () => store.isPlaying,
  async (playing) => {
    pushMediaPlayback();
    try {
      await invoke(playing ? 'player_resume' : 'player_pause');
    } catch {
      // ignore — status poll keeps UI in sync
    }
  }
);

// ---- System Media Transport Controls (Windows media overlay + media keys) ---

const pushMediaMetadata = (song) => {
  if (!song) return;
  invoke('smtc_set_metadata', {
    title: song.title || '',
    artist: song.artist || '',
    album: song.album || '',
    duration: store.duration || 0,
    path: song.path,
  }).catch(() => {});
};

const pushMediaPlayback = () => {
  invoke('smtc_set_playback', {
    playing: store.isPlaying,
    position: store.currentTime || 0,
  }).catch(() => {});
};

let unlistenMedia = null;

const handleMediaControl = (payload) => {
  const action = payload && payload.action;
  switch (action) {
    case 'play':
      if (!store.isPlaying) store.togglePlay();
      break;
    case 'pause':
      if (store.isPlaying) store.togglePlay();
      break;
    case 'toggle':
      store.togglePlay();
      break;
    case 'next':
      store.nextSong(true);
      break;
    case 'previous':
      store.prevSong();
      break;
    case 'stop':
      store.isPlaying = false;
      break;
    case 'seek':
      if (typeof payload.position === 'number') {
        seekValue.value = payload.position;
        onSeekCommit();
      }
      break;
    case 'seek_forward': {
      const t = Math.min((store.currentTime || 0) + 10, store.duration || 0);
      seekValue.value = t;
      onSeekCommit();
      break;
    }
    case 'seek_backward': {
      const t = Math.max((store.currentTime || 0) - 10, 0);
      seekValue.value = t;
      onSeekCommit();
      break;
    }
  }
};

watch(
  () => [store.volume, store.isMuted],
  async ([vol, muted]) => {
    try {
      await invoke('player_set_volume', { volume: muted ? 0 : vol });
    } catch {
      // ignore
    }
  }
);

// While dragging: update the visible time only, and keep the poll from snapping
// the thumb back to the old position.
const onSeekInput = () => {
  lastSeekAt = Date.now();
  store.currentTime = Number(seekValue.value);
};

// On release: issue a single seek command (range v-model yields a string, so
// coerce to a number for the f64 backend arg).
const onSeekCommit = async () => {
  const time = Number(seekValue.value);
  store.currentTime = time;
  lastSeekAt = Date.now();
  if (store.playbackFinished && store.currentSong) {
    store.pendingSeek = time;
    store.pendingAutoplay = true;
    store.playSong(store.currentSong);
  } else {
    try {
      await invoke('player_seek', { position: time });
    } catch (err) {
      playbackError.value = String(err);
    }
  }
};

const handleTrackEnded = async () => {
  const current = store.currentSong;
  if (!current || endedHandledFor === current.path) return;
  endedHandledFor = current.path;

  if (store.loopMode === 2) {
    // Loop one: reload the same track from the start.
    try {
      await invoke('player_load', {
        path: current.path,
        volume: store.isMuted ? 0 : store.volume,
        startAt: null,
        autoplay: true,
        durationHint: current.duration_secs || 0,
      });
      store.currentTime = 0;
      seekValue.value = 0;
      endedHandledFor = null;
    } catch {
      // ignore
    }
  } else {
    store.nextSong(false);
  }
};

let pollTick = 0;

const poll = async () => {
  if (!store.currentSong) return;
  try {
    const status = await invoke('player_status');
    if (status.duration > 0) store.duration = status.duration;
    if (Date.now() - lastSeekAt > 500) {
      if (status.finished) {
        store.currentTime = store.duration;
        seekValue.value = store.duration;
      } else {
        store.currentTime = status.position;
        seekValue.value = status.position;
      }
    }
    if (status.finished) {
      await handleTrackEnded();
    }
    // Keep the OS media overlay's timeline roughly in sync (~every 2s).
    if (++pollTick % 7 === 0 && !store.isBuffering) {
      pushMediaPlayback();
    }
  } catch {
    // ignore transient errors
  }
};

const closeLosslessPopup = () => {
  losslessPopupOpen.value = false;
};

onMounted(async () => {
  pollTimer = setInterval(poll, 300);
  // Checkpoint playback position periodically so resume-on-launch is accurate.
  stateTimer = setInterval(() => {
    if (store.currentSong) store.persistState();
  }, 5000);
  window.addEventListener('beforeunload', flushState);
  document.addEventListener('click', closeLosslessPopup);

  // Forward OS media-key / overlay button presses into the player.
  try {
    unlistenMedia = await listen('media-control', (e) => handleMediaControl(e.payload));
  } catch {
    // ignore — media controls are best-effort
  }
});

onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer);
  if (stateTimer) clearInterval(stateTimer);
  if (unlistenMedia) unlistenMedia();
  window.removeEventListener('beforeunload', flushState);
  document.removeEventListener('click', closeLosslessPopup);
});

const flushState = () => {
  if (store.currentSong) store.persistState();
};

const formatTime = (seconds) => {
  if (!seconds || isNaN(seconds)) return '0:00';
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, '0')}`;
};
</script>

<template>
  <div
    class="bg-[#181818] border-t border-[#282828] z-50 select-none flex flex-col"
    style="view-transition-name: player-bar"
  >
    <div v-if="playbackError" class="bg-red-900/50 text-[10px] text-red-200 p-1 px-4 text-center">
      {{ playbackError }}
    </div>

    <div class="h-24 flex items-center justify-between px-4">
      <!-- Controls -->
      <div class="flex items-center gap-2 sm:gap-4 md:gap-5 w-1/3 pl-1 sm:pl-4">
        <!-- Shuffle -->
        <button
          @click="store.toggleShuffle()"
          class="transition hidden sm:block"
          :class="
            store.shuffleMode ? 'text-[var(--accent-color)]' : 'text-gray-400 hover:text-white'
          "
          title="Shuffle"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M16 3h5v5M4 20L21 3M21 16v5h-5M15 15l6 6M4 4l5 5" />
          </svg>
        </button>

        <!-- Prev -->
        <button @click="store.prevSong()" class="text-gray-300 hover:text-white transition">
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="24"
            height="24"
            viewBox="0 0 24 24"
            fill="currentColor"
            stroke="none"
          >
            <polygon points="19 20 9 12 19 4 19 20"></polygon>
            <line x1="5" y1="19" x2="5" y2="5" stroke="currentColor" stroke-width="2"></line>
          </svg>
        </button>

        <!-- Play/Pause -->
        <button
          @click="store.togglePlay()"
          class="bg-white text-black rounded-full p-2 hover:scale-105 transition flex items-center justify-center"
        >
          <svg
            v-if="store.isBuffering"
            class="animate-spin"
            xmlns="http://www.w3.org/2000/svg"
            width="24"
            height="24"
            viewBox="0 0 24 24"
            fill="none"
          >
            <circle
              class="opacity-25"
              cx="12"
              cy="12"
              r="9"
              stroke="currentColor"
              stroke-width="3"
            ></circle>
            <path
              class="opacity-90"
              fill="currentColor"
              d="M12 3a9 9 0 0 1 9 9h-3a6 6 0 0 0-6-6V3z"
            ></path>
          </svg>
          <svg
            v-else-if="store.isPlaying"
            xmlns="http://www.w3.org/2000/svg"
            width="24"
            height="24"
            viewBox="0 0 24 24"
            fill="currentColor"
            stroke="none"
          >
            <rect x="6" y="4" width="4" height="16"></rect>
            <rect x="14" y="4" width="4" height="16"></rect>
          </svg>
          <svg
            v-else
            xmlns="http://www.w3.org/2000/svg"
            width="24"
            height="24"
            viewBox="0 0 24 24"
            fill="currentColor"
            stroke="none"
          >
            <polygon points="5 3 19 12 5 21 5 3"></polygon>
          </svg>
        </button>

        <!-- Next -->
        <button @click="store.nextSong(true)" class="text-gray-300 hover:text-white transition">
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="24"
            height="24"
            viewBox="0 0 24 24"
            fill="currentColor"
            stroke="none"
          >
            <polygon points="5 4 15 12 5 20 5 4"></polygon>
            <line x1="19" y1="5" x2="19" y2="19" stroke="currentColor" stroke-width="2"></line>
          </svg>
        </button>

        <!-- Loop -->
        <button
          @click="store.toggleLoop()"
          class="transition relative hidden sm:block"
          :class="
            store.loopMode > 0 ? 'text-[var(--accent-color)]' : 'text-gray-400 hover:text-white'
          "
          title="Loop"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M17 1l4 4-4 4"></path>
            <path d="M3 11V9a4 4 0 0 1 4-4h14"></path>
            <path d="M7 23l-4-4 4-4"></path>
            <path d="M21 13v2a4 4 0 0 1-4 4H3"></path>
          </svg>
          <span v-if="store.loopMode === 2" class="absolute -top-1 -right-2 text-[8px] font-bold"
            >1</span
          >
        </button>
      </div>

      <!-- Progress bar -->
      <div class="flex flex-col items-center w-1/3 px-1 sm:px-4">
        <div
          v-if="store.currentSong"
          class="flex items-center gap-2 md:gap-4 mb-1.5 md:mb-2 w-full justify-center"
        >
          <!-- Group container: CoverImage on left, Lossless Badge on right, both aligned to top -->
          <div class="hidden sm:flex items-start shrink-0 gap-1.5 relative">
            <CoverImage
              :path="store.currentSong.path"
              className="h-8 w-8 md:h-10 md:w-10 rounded shadow-sm bg-[#333]"
            />

            <!-- Lossless Badge Container -->
            <div v-if="isLossless" class="relative mt-0.5 shrink-0">
              <button
                @click.stop="losslessPopupOpen = !losslessPopupOpen"
                class="flex shrink-0 items-center justify-center text-gray-500 hover:text-gray-300 transition-colors focus:outline-none"
                title="Lossless Audio"
              >
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  viewBox="0 0 15 9"
                  class="h-2.5 w-[17px] fill-current"
                >
                  <path
                    d="M8.184,0.35C9.944,0.35 10.703,3.296 11.338,5.238C11.673,3.842 11.497,3.542 11.857,3.542C11.99,3.542 12.126,3.633 12.126,3.798C12.126,3.809 12.123,3.839 12.117,3.883L12.091,4.058C12.02,4.522 11.845,5.494 11.654,6.144C13.198,10.191 14.345,4.861 14.474,3.772C14.493,3.615 14.612,3.542 14.731,3.542C14.891,3.542 15.022,3.662 14.997,3.843C14.72,5.605 14.295,8.35 12.547,8.35C11.582,8.35 11.04,7.595 10.611,6.73C9.54,4.626 9.047,1.093 7.997,1.093C7.66,1.093 7.411,1.444 7.394,1.444C7.362,1.444 7.337,1.301 7.023,0.909C7.322,0.567 7.734,0.35 8.184,0.35ZM2.458,0.354C5.211,0.354 5.456,7.618 7.014,7.618C7.197,7.618 7.394,7.507 7.61,7.256C7.729,7.458 7.851,7.638 7.978,7.796C7.667,8.151 7.28,8.35 6.795,8.35C5.054,8.349 4.306,5.434 3.663,3.466C3.511,4.097 3.432,4.669 3.402,4.925C3.382,5.088 3.263,5.163 3.143,5.163C3.009,5.163 2.874,5.071 2.874,4.908L2.874,4.908L2.877,4.87C2.966,4.223 3.146,3.243 3.347,2.56C3.079,1.858 2.745,1.091 2.252,1.091C1.257,1.091 0.687,3.591 0.527,4.925C0.508,5.088 0.388,5.163 0.268,5.163C0.135,5.163 0,5.071 0,4.908C0,4.896 0.001,4.883 0.002,4.87C0.283,2.836 0.808,0.354 2.458,0.354ZM5.315,0.35C5.809,0.35 6.339,0.608 6.797,1.211C6.822,1.241 7.078,1.639 7.159,1.777C8.277,3.802 8.818,7.627 9.881,7.627C10.065,7.627 10.264,7.513 10.484,7.256C10.604,7.458 10.726,7.638 10.852,7.796C10.542,8.15 10.155,8.35 9.67,8.35C6.933,8.349 6.636,1.09 5.128,1.09C4.788,1.09 4.536,1.444 4.519,1.444C4.487,1.444 4.462,1.301 4.148,0.909C4.455,0.558 4.87,0.35 5.315,0.35Z"
                  />
                </svg>
              </button>

              <!-- Popover (slightly larger) -->
              <div
                v-if="losslessPopupOpen"
                class="lossless-popover-content absolute top-full left-1/2 -translate-x-1/2 mt-3 z-[100] bg-[#1c1c1e] border border-[#323236] rounded-xl shadow-2xl p-4 w-[230px] text-center select-none animate-fade-in"
                @click.stop
              >
                <!-- Upward pointing arrow -->
                <div
                  class="absolute bottom-full left-1/2 -translate-x-1/2 translate-y-1/2 w-2 h-2 bg-[#1c1c1e] border-l border-t border-[#323236] rotate-45"
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
                <p class="text-xs text-gray-400 mb-3 leading-normal">
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
          <div class="flex flex-col overflow-hidden text-center min-w-0 flex-1">
            <span
              class="text-xs md:text-sm font-medium text-white truncate max-w-[80px] sm:max-w-[180px] md:max-w-[260px]"
              >{{ store.currentSong.title }}</span
            >
            <span
              class="text-[10px] md:text-xs text-gray-400 truncate max-w-[80px] sm:max-w-[180px] md:max-w-[260px]"
              >{{ store.currentSong.artist }}</span
            >
          </div>
          <button
            @click="store.toggleFavorite(store.currentSong.path)"
            class="transition hover:scale-110 shrink-0"
            :class="
              store.isFavorite(store.currentSong.path)
                ? 'text-[var(--accent-color)]'
                : 'text-gray-400 hover:text-white'
            "
            :title="
              store.isFavorite(store.currentSong.path)
                ? 'Remove from Liked Songs'
                : 'Add to Liked Songs'
            "
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="18"
              height="18"
              viewBox="0 0 24 24"
              :fill="store.isFavorite(store.currentSong.path) ? 'currentColor' : 'none'"
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
        <div v-else class="h-10 mb-2 flex items-center text-gray-500 text-sm">Select a song</div>

        <div
          class="w-full flex items-center gap-1.5 sm:gap-3 text-[10px] sm:text-xs text-gray-400 font-variant-numeric tabular-nums"
        >
          <span>{{ formatTime(store.currentTime) }}</span>
          <input
            type="range"
            min="0"
            :max="store.duration || 100"
            v-model="seekValue"
            @input="onSeekInput"
            @change="onSeekCommit"
            class="flex-1 h-1 rounded-lg appearance-none cursor-pointer accent-[var(--accent-color)] hover:accent-white"
            :style="{
              background: `linear-gradient(to right, var(--accent-color) ${progressPercentage}%, #4b5563 ${progressPercentage}%)`,
            }"
          />
          <span>{{ formatTime(store.duration) }}</span>
        </div>
      </div>

      <!-- Volume -->
      <div class="flex items-center justify-end gap-1.5 sm:gap-3 w-1/3 pr-1 sm:pr-4">
        <!-- Real-time audio visualizer (reacts to the playing track) -->
        <Visualizer v-if="store.visualizerEnabled && store.currentSong" />

        <!-- Queue toggle (with an ∞ badge when unlimited autoplay is on) -->
        <button
          @click="store.queuePanelOpen = !store.queuePanelOpen"
          class="transition hover:text-white relative"
          :class="store.queuePanelOpen ? 'text-[var(--accent-color)]' : 'text-gray-400'"
          title="Queue"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <line x1="3" y1="6" x2="16" y2="6"></line>
            <line x1="3" y1="12" x2="13" y2="12"></line>
            <line x1="3" y1="18" x2="13" y2="18"></line>
            <polygon points="18 14 22 16.5 18 19" fill="currentColor" stroke="none"></polygon>
            <line x1="18" y1="9" x2="18" y2="13"></line>
          </svg>
          <span
            v-if="store.autoplayMode"
            class="absolute -top-2 -right-2 h-3.5 w-3.5 rounded-full bg-[var(--accent-color)] flex items-center justify-center ring-2 ring-[#181818] shadow"
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
              <path
                d="M12 12c-2-2.67-4-4-6-4a4 4 0 1 0 0 8c2 0 4-1.33 6-4Zm0 0c2 2.67 4 4 6 4a4 4 0 0 0 0-8c-2 0-4 1.33-6 4Z"
              />
            </svg>
          </span>
        </button>
        <button
          @click="store.toggleMute()"
          class="text-gray-400 hover:text-white transition cursor-pointer flex items-center justify-center shrink-0"
          :title="store.isMuted ? 'Unmute' : 'Mute'"
        >
          <!-- Mute Icon -->
          <svg
            v-if="store.isMuted"
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
            <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"></polygon>
            <line x1="23" y1="9" x2="17" y2="15"></line>
            <line x1="17" y1="9" x2="23" y2="15"></line>
          </svg>
          <!-- Low Volume Icon -->
          <svg
            v-else-if="store.volume <= 0.5"
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
            <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"></polygon>
            <path d="M15.54 8.46a5 5 0 0 1 0 7.07"></path>
          </svg>
          <!-- High Volume Icon -->
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
            <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"></polygon>
            <path d="M19.07 4.93a10 10 0 0 1 0 14.14"></path>
            <path d="M15.54 8.46a5 5 0 0 1 0 7.07"></path>
          </svg>
        </button>
        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          :value="store.isMuted ? 0 : store.volume"
          @input="store.setVolume($event.target.value)"
          class="hidden sm:block w-16 md:w-24 h-1 rounded-lg appearance-none cursor-pointer accent-[var(--accent-color)] hover:accent-white transition-opacity duration-200"
          :class="store.isMuted ? 'opacity-40' : 'opacity-100'"
          :style="{
            background: `linear-gradient(to right, var(--accent-color) ${volumePercentage}%, #4b5563 ${volumePercentage}%)`,
          }"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
input[type='range']::-webkit-slider-thumb {
  -webkit-appearance: none;
  height: 12px;
  width: 12px;
  border-radius: 50%;
  background: currentColor;
  margin-top: -4px;
}

.animate-fade-in {
  animation: fadeIn 0.15s cubic-bezier(0.16, 1, 0.3, 1) forwards;
  transform-origin: top center;
}

@keyframes fadeIn {
  from {
    opacity: 0;
    transform: translate(-50%, -4px) scale(0.95);
  }
  to {
    opacity: 1;
    transform: translate(-50%, 0) scale(1);
  }
}
</style>
