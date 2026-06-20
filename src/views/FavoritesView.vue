<script setup>
import { computed, ref, onMounted, onUnmounted } from 'vue';
import { store } from '../store';
import SongList from '../components/SongList.vue';

const songs = computed(() => store.favoriteSongs);

const playAll = () => {
  if (songs.value.length > 0) store.playSong(songs.value[0], songs.value);
};

const menuOpen = ref(false);

const closeMenu = (e) => {
  if (e && e.target.closest('.playlist-menu-container')) return;
  menuOpen.value = false;
};

onMounted(() => {
  window.addEventListener('click', closeMenu);
});

onUnmounted(() => {
  window.removeEventListener('click', closeMenu);
});

const shuffleFavorites = () => {
  if (songs.value.length > 0) {
    store.shuffleMode = true;
    const randomIndex = Math.floor(Math.random() * songs.value.length);
    store.playSong(songs.value[randomIndex], songs.value);
  }
};

const playNextFavorites = () => {
  if (songs.value.length > 0) {
    store.playNextSongs(songs.value);
  }
};

const playLastFavorites = () => {
  if (songs.value.length > 0) {
    store.addToQueue(songs.value);
  }
};
</script>

<template>
  <div class="flex flex-col h-full overflow-auto">
    <!-- Header -->
    <div class="p-8 flex gap-8 items-end bg-gradient-to-b from-[#3a2030] to-[var(--app-bg)] relative">
      <div
        class="h-52 w-52 shrink-0 rounded-md shadow-2xl bg-gradient-to-br from-[var(--accent-color)] to-[#7a1020] flex items-center justify-center"
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="80"
          height="80"
          viewBox="0 0 24 24"
          fill="#fff"
          stroke="none"
        >
          <path
            d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"
          ></path>
        </svg>
      </div>

      <div class="flex flex-col gap-1 pb-2 overflow-hidden flex-1">
        <h4 class="text-sm font-bold text-[var(--accent-color)] uppercase tracking-wider mb-1">
          Playlist
        </h4>
        <h1 class="text-5xl font-bold tracking-tight text-white">Liked Songs</h1>
        <p class="text-xs text-[var(--text-secondary)] font-medium mt-2">
          {{ songs.length }} songs
        </p>

        <div class="flex gap-3 mt-6 items-center">
          <button
            @click="playAll"
            :disabled="songs.length === 0"
            class="bg-[var(--accent-color)] text-white px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-red-500 transition flex items-center gap-2 shadow-lg disabled:opacity-40"
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
            @click="shuffleFavorites"
            :disabled="songs.length === 0"
            class="bg-[#3a3a3a] text-[var(--accent-color)] px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-[#444] transition flex items-center gap-2 shadow-lg disabled:opacity-40"
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

      <!-- Ellipsis Options Menu at the far right end -->
      <div class="relative pb-2 self-end playlist-menu-container">
        <button
          @click.stop="menuOpen = !menuOpen"
          class="text-red-500 hover:text-red-400 p-2 rounded-full hover:bg-white/5 transition-colors flex items-center justify-center"
          title="More options"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="currentColor"
            stroke="none"
          >
            <circle cx="5" cy="12" r="2"></circle>
            <circle cx="12" cy="12" r="2"></circle>
            <circle cx="19" cy="12" r="2"></circle>
          </svg>
        </button>

        <!-- Options Dropdown -->
        <div
          v-if="menuOpen"
          class="absolute right-0 mt-2 z-50 w-56 rounded-lg bg-[#282828] border border-[#3a3a3a] py-1.5 shadow-2xl text-sm text-white"
        >
          <button
            @click="playAll"
            :disabled="songs.length === 0"
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors disabled:opacity-40"
          >
            Play "Liked Songs"
          </button>
          <button
            @click="shuffleFavorites"
            :disabled="songs.length === 0"
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors disabled:opacity-40"
          >
            Shuffle "Liked Songs"
          </button>
          <button
            @click="playNextFavorites"
            :disabled="songs.length === 0"
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors disabled:opacity-40"
          >
            Play next
          </button>
          <button
            @click="playLastFavorites"
            :disabled="songs.length === 0"
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors disabled:opacity-40"
          >
            Play last
          </button>
        </div>
      </div>
    </div>

    <div class="px-2 pb-12">
      <SongList :songs="songs" :is-favorites="true" />
    </div>
  </div>
</template>
