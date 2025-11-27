<script setup>
import { ref, watch, onMounted, computed } from 'vue';
import { store } from '../store';
import { convertFileSrc } from '@tauri-apps/api/core';
import CoverImage from './CoverImage.vue';

const audioPlayer = ref(null);
const seekValue = ref(0);

// Computed source URL
const audioSrc = computed(() => {
  if (!store.currentSong) return '';
  return convertFileSrc(store.currentSong.path);
});

// Watch for song changes
watch(() => store.currentSong, async (newSong) => {
  if (newSong && audioPlayer.value) {
    // Wait for DOM update
    setTimeout(async () => {
      try {
        await audioPlayer.value.play();
        store.isPlaying = true;
      } catch (e) {
        console.error("Playback error:", e);
      }
    }, 50);
  }
});

// Watch for play/pause state
watch(() => store.isPlaying, (playing) => {
  if (!audioPlayer.value) return;
  if (playing) audioPlayer.value.play();
  else audioPlayer.value.pause();
});

// Watch for volume changes
watch(() => store.volume, (vol) => {
  if (audioPlayer.value) audioPlayer.value.volume = vol;
});

// Handle time update
const onTimeUpdate = () => {
  if (!audioPlayer.value) return;
  store.currentTime = audioPlayer.value.currentTime;
  seekValue.value = audioPlayer.value.currentTime;
};

// Handle metadata loaded
const onLoadedMetadata = () => {
  if (!audioPlayer.value) return;
  store.duration = audioPlayer.value.duration;
};

// Handle song end
const onEnded = () => {
  store.nextSong();
};

// User seeking
const onSeek = (e) => {
  const time = parseFloat(e.target.value);
  if (audioPlayer.value) {
    audioPlayer.value.currentTime = time;
    store.currentTime = time;
  }
};

// Format time helper
const formatTime = (seconds) => {
  if (!seconds) return "0:00";
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, '0')}`;
};
</script>

<template>
  <div class="h-24 bg-[#181818] border-b border-[#282828] flex items-center justify-between px-4 z-50 select-none">
    
    <!-- Hidden Audio Element -->
    <audio 
      ref="audioPlayer"
      :src="audioSrc"
      @timeupdate="onTimeUpdate"
      @loadedmetadata="onLoadedMetadata"
      @ended="onEnded"
      crossorigin="anonymous"
    ></audio>

    <!-- Left: Controls -->
    <div class="flex items-center gap-4 w-1/4">
      <button @click="store.prevSong()" class="text-gray-300 hover:text-white transition">
        <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor" stroke="none"><polygon points="19 20 9 12 19 4 19 20"></polygon><line x1="5" y1="19" x2="5" y2="5" stroke="currentColor" stroke-width="2"></line></svg>
      </button>

      <button 
        @click="store.togglePlay()" 
        class="bg-white text-black rounded-full p-2 hover:scale-105 transition flex items-center justify-center"
      >
        <svg v-if="store.isPlaying" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor" stroke="none"><rect x="6" y="4" width="4" height="16"></rect><rect x="14" y="4" width="4" height="16"></rect></svg>
        <svg v-else xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor" stroke="none"><polygon points="5 3 19 12 5 21 5 3"></polygon></svg>
      </button>

      <button @click="store.nextSong()" class="text-gray-300 hover:text-white transition">
        <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor" stroke="none"><polygon points="5 4 15 12 5 20 5 4"></polygon><line x1="19" y1="5" x2="19" y2="19" stroke="currentColor" stroke-width="2"></line></svg>
      </button>
    </div>

    <!-- Center: Info & Progress -->
    <div class="flex flex-col items-center w-2/4 max-w-2xl px-4">
      <!-- Song Info -->
      <div v-if="store.currentSong" class="flex items-center gap-4 mb-2 w-full justify-center">
        <CoverImage :path="store.currentSong.path" className="h-10 w-10 rounded shadow-sm bg-[#333]" />
        <div class="flex flex-col overflow-hidden text-center">
          <span class="text-sm font-medium text-white truncate max-w-[300px]">{{ store.currentSong.title }}</span>
          <span class="text-xs text-gray-400 truncate max-w-[300px]">{{ store.currentSong.artist }} — {{ store.currentSong.album }}</span>
        </div>
      </div>
      <div v-else class="h-10 mb-2 flex items-center text-gray-500 text-sm">
        Select a song to play
      </div>

      <!-- Progress Bar -->
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

    <!-- Right: Volume -->
    <div class="flex items-center justify-end gap-3 w-1/4">
      <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"></polygon><path d="M19.07 4.93a10 10 0 0 1 0 14.14M15.54 8.46a5 5 0 0 1 0 7.07"></path></svg>
      <input 
        type="range" 
        min="0" 
        max="1" 
        step="0.01" 
        v-model="store.volume"
        class="w-24 h-1 bg-gray-600 rounded-lg appearance-none cursor-pointer accent-[var(--accent-color)] hover:accent-white"
      >
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
  margin-top: -4px; /* You need to specify a margin in Chrome, but in Firefox and IE it is automatic */
}
</style>
