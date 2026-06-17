<script setup>
import { ref, watch, onMounted, onUnmounted } from 'vue';
import { store } from '../store';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import CoverImage from './CoverImage.vue';

// Playback is handled natively in Rust (rodio + symphonia). This component just
// issues commands and polls the backend for the current position/duration.
const seekValue = ref(0);
const playbackError = ref(null);

let pollTimer = null;
let stateTimer = null;
let lastSeekAt = 0; // suppress poll overwriting the slider right after a seek
let endedHandledFor = null; // latch so a finished track only advances once
let loadToken = 0; // guards against a stale load winning after a rapid skip

// Load (and usually play) whenever the selected song changes.
watch(() => store.currentSong, async (song) => {
  if (!song) return;
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
    const duration = await invoke('player_load', {
      path: song.path,
      volume: store.volume,
      startAt,
      autoplay,
      durationHint: song.duration_secs || 0,
    });
    if (token !== loadToken) return; // a newer track was selected meanwhile
    store.duration = duration || 0;
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
}, { immediate: true });

watch(() => store.isPlaying, async (playing) => {
  pushMediaPlayback();
  try {
    await invoke(playing ? 'player_resume' : 'player_pause');
  } catch {
    // ignore — status poll keeps UI in sync
  }
});

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

watch(() => store.volume, async (vol) => {
  try {
    await invoke('player_set_volume', { volume: vol });
  } catch {
    // ignore
  }
});

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
  try {
    await invoke('player_seek', { position: time });
  } catch (err) {
    playbackError.value = String(err);
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
        volume: store.volume,
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
      store.currentTime = status.position;
      seekValue.value = status.position;
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

onMounted(async () => {
  pollTimer = setInterval(poll, 300);
  // Checkpoint playback position periodically so resume-on-launch is accurate.
  stateTimer = setInterval(() => {
    if (store.currentSong) store.persistState();
  }, 5000);
  window.addEventListener('beforeunload', flushState);

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
});

const flushState = () => {
  if (store.currentSong) store.persistState();
};

const formatTime = (seconds) => {
  if (!seconds || isNaN(seconds)) return "0:00";
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, '0')}`;
};
</script>

<template>
  <div class="bg-[#181818] border-t border-[#282828] z-50 select-none flex flex-col">
    
    <div v-if="playbackError" class="bg-red-900/50 text-[10px] text-red-200 p-1 px-4 text-center">
      {{ playbackError }}
    </div>

    <div class="h-24 flex items-center justify-between px-4">
      <!-- Controls -->
      <div class="flex items-center gap-5 w-1/3 pl-4">
        <!-- Shuffle -->
        <button 
          @click="store.toggleShuffle()" 
          class="transition"
          :class="store.shuffleMode ? 'text-[var(--accent-color)]' : 'text-gray-400 hover:text-white'"
          title="Shuffle"
        >
          <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 3h5v5M4 20L21 3M21 16v5h-5M15 15l6 6M4 4l5 5"/></svg>
        </button>

        <!-- Prev -->
        <button @click="store.prevSong()" class="text-gray-300 hover:text-white transition">
          <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor" stroke="none"><polygon points="19 20 9 12 19 4 19 20"></polygon><line x1="5" y1="19" x2="5" y2="5" stroke="currentColor" stroke-width="2"></line></svg>
        </button>

        <!-- Play/Pause -->
        <button
          @click="store.togglePlay()"
          class="bg-white text-black rounded-full p-2 hover:scale-105 transition flex items-center justify-center"
        >
          <svg v-if="store.isBuffering" class="animate-spin" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><circle class="opacity-25" cx="12" cy="12" r="9" stroke="currentColor" stroke-width="3"></circle><path class="opacity-90" fill="currentColor" d="M12 3a9 9 0 0 1 9 9h-3a6 6 0 0 0-6-6V3z"></path></svg>
          <svg v-else-if="store.isPlaying" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor" stroke="none"><rect x="6" y="4" width="4" height="16"></rect><rect x="14" y="4" width="4" height="16"></rect></svg>
          <svg v-else xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor" stroke="none"><polygon points="5 3 19 12 5 21 5 3"></polygon></svg>
        </button>

        <!-- Next -->
        <button @click="store.nextSong(true)" class="text-gray-300 hover:text-white transition">
          <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor" stroke="none"><polygon points="5 4 15 12 5 20 5 4"></polygon><line x1="19" y1="5" x2="19" y2="19" stroke="currentColor" stroke-width="2"></line></svg>
        </button>

        <!-- Loop -->
        <button 
          @click="store.toggleLoop()" 
          class="transition relative"
          :class="store.loopMode > 0 ? 'text-[var(--accent-color)]' : 'text-gray-400 hover:text-white'"
          title="Loop"
        >
          <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17 1l4 4-4 4"></path><path d="M3 11V9a4 4 0 0 1 4-4h14"></path><path d="M7 23l-4-4 4-4"></path><path d="M21 13v2a4 4 0 0 1-4 4H3"></path></svg>
          <span v-if="store.loopMode === 2" class="absolute -top-1 -right-2 text-[8px] font-bold">1</span>
        </button>
      </div>

      <!-- Progress bar -->
      <div class="flex flex-col items-center w-1/3 px-4">
        <div v-if="store.currentSong" class="flex items-center gap-4 mb-2 w-full justify-center">
          <CoverImage :path="store.currentSong.path" className="h-10 w-10 rounded shadow-sm bg-[#333]" />
          <div class="flex flex-col overflow-hidden text-center">
            <span class="text-sm font-medium text-white truncate max-w-[260px]">{{ store.currentSong.title }}</span>
            <span class="text-xs text-gray-400 truncate max-w-[260px]">{{ store.currentSong.artist }}</span>
          </div>
          <button
            @click="store.toggleFavorite(store.currentSong.path)"
            class="transition hover:scale-110 shrink-0"
            :class="store.isFavorite(store.currentSong.path) ? 'text-[var(--accent-color)]' : 'text-gray-400 hover:text-white'"
            :title="store.isFavorite(store.currentSong.path) ? 'Remove from Liked Songs' : 'Add to Liked Songs'"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" :fill="store.isFavorite(store.currentSong.path) ? 'currentColor' : 'none'" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"></path></svg>
          </button>
        </div>
        <div v-else class="h-10 mb-2 flex items-center text-gray-500 text-sm">
          Select a song
        </div>

        <div class="w-full flex items-center gap-3 text-xs text-gray-400 font-variant-numeric tabular-nums">
          <span>{{ formatTime(store.currentTime) }}</span>
          <input 
            type="range" 
            min="0" 
            :max="store.duration || 100" 
            v-model="seekValue"
            @input="onSeekInput"
            @change="onSeekCommit"
            class="w-full h-1 bg-gray-600 rounded-lg appearance-none cursor-pointer accent-[var(--accent-color)] hover:accent-white"
          >
          <span>{{ formatTime(store.duration) }}</span>
        </div>
      </div>

      <!-- Volume -->
      <div class="flex items-center justify-end gap-3 w-1/3 pr-4">
        <!-- Queue toggle -->
        <button
          @click="store.queuePanelOpen = !store.queuePanelOpen"
          class="transition hover:text-white relative"
          :class="store.queuePanelOpen ? 'text-[var(--accent-color)]' : 'text-gray-400'"
          title="Queue"
        >
          <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="3" y1="6" x2="16" y2="6"></line><line x1="3" y1="12" x2="13" y2="12"></line><line x1="3" y1="18" x2="13" y2="18"></line><polygon points="18 14 22 16.5 18 19" fill="currentColor" stroke="none"></polygon><line x1="18" y1="9" x2="18" y2="13"></line></svg>
        </button>

        <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"></polygon><path d="M19.07 4.93a10 10 0 0 1 0 14.14M15.54 8.46a5 5 0 0 1 0 7.07"></path></svg>
        <input 
          type="range" 
          min="0" 
          max="1" 
          step="0.01" 
          :value="store.volume"
          @input="store.setVolume($event.target.value)"
          class="w-24 h-1 bg-gray-600 rounded-lg appearance-none cursor-pointer accent-[var(--accent-color)] hover:accent-white"
        >
      </div>
    </div>
  </div>
</template>

<style scoped>
input[type="range"]::-webkit-slider-thumb {
  -webkit-appearance: none;
  height: 12px;
  width: 12px;
  border-radius: 50%;
  background: currentColor;
  margin-top: -4px;
}
</style>