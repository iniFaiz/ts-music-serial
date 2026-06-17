<script setup>
import { computed } from 'vue';
import { store } from '../store';
import SongList from '../components/SongList.vue';

const songs = computed(() => store.favoriteSongs);

const playAll = () => {
  if (songs.value.length > 0) store.playSong(songs.value[0], songs.value);
};
</script>

<template>
  <div class="flex flex-col h-full overflow-auto">
    <!-- Header -->
    <div class="p-8 flex gap-8 items-end bg-gradient-to-b from-[#3a2030] to-[var(--app-bg)]">
      <div class="h-52 w-52 shrink-0 rounded-md shadow-2xl bg-gradient-to-br from-[var(--accent-color)] to-[#7a1020] flex items-center justify-center">
        <svg xmlns="http://www.w3.org/2000/svg" width="80" height="80" viewBox="0 0 24 24" fill="#fff" stroke="none"><path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"></path></svg>
      </div>

      <div class="flex flex-col gap-1 pb-2 overflow-hidden">
        <h4 class="text-sm font-bold text-[var(--accent-color)] uppercase tracking-wider mb-1">Playlist</h4>
        <h1 class="text-5xl font-bold tracking-tight text-white">Liked Songs</h1>
        <p class="text-xs text-[var(--text-secondary)] font-medium mt-2">{{ songs.length }} songs</p>

        <div class="flex gap-3 mt-6">
          <button
            @click="playAll"
            :disabled="songs.length === 0"
            class="bg-[var(--accent-color)] text-white px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-red-500 transition flex items-center gap-2 shadow-lg disabled:opacity-40"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="currentColor" stroke="none"><polygon points="5 3 19 12 5 21 5 3"></polygon></svg>
            Play
          </button>
        </div>
      </div>
    </div>

    <div class="px-2 pb-12">
      <SongList :songs="songs" />
    </div>
  </div>
</template>
