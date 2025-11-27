<script setup>
import { ref, watch, onUnmounted, nextTick } from 'vue';
import { store } from '../store';
import { readFile } from '@tauri-apps/plugin-fs';
import CoverImage from './CoverImage.vue';

const audioPlayer = ref(null);
const seekValue = ref(0);
const debugError = ref(null);
const debugStatus = ref('Idle');
let currentObjectUrl = null;

const getMimeType = (path) => {
  const ext = path.split('.').pop().toLowerCase();
  const types = {
    'mp3': 'audio/mpeg',
    'wav': 'audio/wav',
    'ogg': 'audio/ogg',
    'm4a': 'audio/mp4',
    'flac': 'audio/flac',
    'aac': 'audio/aac'
  };
  return types[ext] || 'application/octet-stream';
};

watch(() => store.currentSong, async (newSong) => {
  if (!newSong) return;

  if (currentObjectUrl) {
    URL.revokeObjectURL(currentObjectUrl);
    currentObjectUrl = null;
  }

  debugStatus.value = `Loading...`;
  debugError.value = null;
  store.isPlaying = false;

  try {
    const fileBytes = await readFile(newSong.path);
    const mimeType = getMimeType(newSong.path);
    const blob = new Blob([fileBytes], { type: mimeType });
    currentObjectUrl = URL.createObjectURL(blob);

    if (audioPlayer.value) {
      audioPlayer.value.src = currentObjectUrl;
      audioPlayer.value.load();
      const playPromise = audioPlayer.value.play();
      if (playPromise !== undefined) {
        playPromise
          .then(() => {
            store.isPlaying = true;
            debugStatus.value = `Playing`;
          })
          .catch(e => debugError.value = e.message);
      }
    }
  } catch (err) {
    debugError.value = `Error: ${err}`; 
  }
});

watch(() => store.isPlaying, (playing) => {
  if (!audioPlayer.value) return;
  if (playing) audioPlayer.value.play().catch(e => console.error(e));
  else audioPlayer.value.pause();
});

watch(() => store.volume, (vol) => {
  if (audioPlayer.value) audioPlayer.value.volume = vol;
});

watch(() => store.currentTime, (time) => {
  if (audioPlayer.value && Math.abs(audioPlayer.value.currentTime - time) > 1) {
     audioPlayer.value.currentTime = time;
  }
});

onUnmounted(() => {
  if (currentObjectUrl) URL.revokeObjectURL(currentObjectUrl);
});

const onTimeUpdate = () => {
  if (!audioPlayer.value) return;
  if (Math.abs(store.currentTime - audioPlayer.value.currentTime) > 1) {
      store.currentTime = audioPlayer.value.currentTime;
  }
  seekValue.value = audioPlayer.value.currentTime;
};

const onLoadedMetadata = () => {
  if (!audioPlayer.value) return;
  store.duration = audioPlayer.value.duration;
  audioPlayer.value.volume = store.volume;
};

const onEnded = () => {
  if (store.loopMode !== 2) {
    store.nextSong(false);
  }
};

const onError = (e) => {
  debugError.value = `Error ${e.target.error?.code}`;
};

const onSeek = (e) => {
  const time = parseFloat(e.target.value);
  if (audioPlayer.value) {
    audioPlayer.value.currentTime = time;
    store.currentTime = time;
  }
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
    
    <div v-if="debugError" class="bg-red-900/50 text-[10px] text-red-200 p-1 px-4 text-center">
      {{ debugError }}
    </div>

    <div class="h-24 flex items-center justify-between px-4">
      <audio 
        ref="audioPlayer"
        @timeupdate="onTimeUpdate"
        @loadedmetadata="onLoadedMetadata"
        @ended="onEnded"
        @error="onError"
        :loop="store.loopMode === 2"
        crossorigin="anonymous"
      ></audio>

      <!-- Controls -->
      <div class="flex items-center gap-5 w-1/3 pl-4">
        <!-- Shuffle -->
        <button 
          @click="store.toggleShuffle()" 
          class="transition hover:text-white"
          :class="store.shuffleMode ? 'text-[var(--accent-color)]' : 'text-gray-400'"
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
          <svg v-if="store.isPlaying" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor" stroke="none"><rect x="6" y="4" width="4" height="16"></rect><rect x="14" y="4" width="4" height="16"></rect></svg>
          <svg v-else xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor" stroke="none"><polygon points="5 3 19 12 5 21 5 3"></polygon></svg>
        </button>

        <!-- Next -->
        <button @click="store.nextSong(true)" class="text-gray-300 hover:text-white transition">
          <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor" stroke="none"><polygon points="5 4 15 12 5 20 5 4"></polygon><line x1="19" y1="5" x2="19" y2="19" stroke="currentColor" stroke-width="2"></line></svg>
        </button>

        <!-- Loop -->
        <button 
          @click="store.toggleLoop()" 
          class="transition hover:text-white relative"
          :class="store.loopMode > 0 ? 'text-[var(--accent-color)]' : 'text-gray-400'"
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
            <span class="text-sm font-medium text-white truncate max-w-[300px]">{{ store.currentSong.title }}</span>
            <span class="text-xs text-gray-400 truncate max-w-[300px]">{{ store.currentSong.artist }}</span>
          </div>
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
            @input="onSeek"
            class="w-full h-1 bg-gray-600 rounded-lg appearance-none cursor-pointer accent-[var(--accent-color)] hover:accent-white"
          >
          <span>{{ formatTime(store.duration) }}</span>
        </div>
      </div>

      <!-- Volume -->
      <div class="flex items-center justify-end gap-3 w-1/3 pr-4">
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