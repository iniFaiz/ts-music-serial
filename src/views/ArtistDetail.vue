<script setup>
import { computed } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useRoute } from 'vue-router';
import { store } from '../store';
import SongList from '../components/SongList.vue';
import CoverImage from '../components/CoverImage.vue';
import { useQuery } from '../useLibraryData';

const route = useRoute();
const artistName = route.params.name;

// Artist tracks, ordered by album then track number in SQL.
const { data: artistSongs } = useQuery(
  () => invoke('db_artist_tracks', { artist: artistName }),
  { initial: [] }
);

const representativePath = computed(() => {
  const withCover = artistSongs.value.find((s) => s.has_cover);
  return withCover ? withCover.path : artistSongs.value.length > 0 ? artistSongs.value[0].path : '';
});

const playArtist = () => {
  if (artistSongs.value.length > 0) {
    store.playSong(artistSongs.value[0], artistSongs.value);
  }
};

const shuffleArtist = () => {
  if (artistSongs.value.length > 0) {
    store.shuffleMode = true;
    const randomIndex = Math.floor(Math.random() * artistSongs.value.length);
    store.playSong(artistSongs.value[randomIndex], artistSongs.value);
  }
};
</script>

<template>
  <div class="h-full flex flex-col overflow-auto">
    <!-- Header -->
    <div class="h-[40vh] w-full relative overflow-hidden shrink-0">
      <!-- Background Image -->
      <div class="absolute inset-0">
        <CoverImage
          :path="representativePath"
          className="w-full h-full object-cover blur-2xl opacity-40 scale-110"
        />
        <div class="absolute inset-0 bg-gradient-to-t from-[var(--app-bg)] to-transparent"></div>
      </div>

      <div class="absolute bottom-0 left-0 p-8 flex items-end gap-6 w-full">
        <div
          class="h-40 w-40 rounded-full overflow-hidden shadow-2xl border-4 border-[var(--app-bg)] shrink-0"
          style="view-transition-name: shared-cover"
        >
          <CoverImage :path="representativePath" className="h-full w-full object-cover" />
        </div>

        <div class="mb-4">
          <h1 class="text-5xl font-bold text-white tracking-tight drop-shadow-lg">
            {{ artistName }}
          </h1>

          <div class="flex gap-3 mt-4">
            <button
              @click="playArtist"
              class="bg-[var(--accent-color)] text-white px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-red-500 transition flex items-center gap-2 shadow-lg"
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="currentColor"
                stroke="none"
              >
                <polygon points="5 3 19 12 5 21 5 3"></polygon>
              </svg>
              Play
            </button>

            <button
              @click="shuffleArtist"
              class="bg-[#3a3a3a] text-[var(--accent-color)] px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-[#444] transition flex items-center gap-2 shadow-lg"
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M16 3h5v5M4 20L21 3M21 16v5h-5M15 15l6 6M4 4l5 5" />
              </svg>
              Shuffle
            </button>
          </div>
        </div>
      </div>
    </div>

    <div class="px-2 py-6">
      <h2 class="px-6 text-xl font-bold text-white mb-4">Popular Songs</h2>
      <SongList :songs="artistSongs" />
    </div>
  </div>
</template>
