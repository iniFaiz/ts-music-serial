<script setup>
import { computed, ref } from 'vue';
import { store } from '../store';
import CoverImage from './CoverImage.vue';

const props = defineProps({
  songs: { type: Array, required: true }
});

const sortKey = ref(null);
const sortOrder = ref('asc');

const toggleSort = (key) => {
  if (sortKey.value === key) {
    if (sortOrder.value === 'asc') {
      sortOrder.value = 'desc';
    } else {
      sortKey.value = null;
      sortOrder.value = 'asc';
    }
  } else {
    sortKey.value = key;
    sortOrder.value = 'asc';
  }
};

const sortedSongs = computed(() => {
  let items = [...props.songs];

  if (!sortKey.value) {
    return items;
  }

  return items.sort((a, b) => {
    const modifier = sortOrder.value === 'asc' ? 1 : -1;
    let valA = a[sortKey.value];
    let valB = b[sortKey.value];
    
    if (typeof valA === 'string') valA = valA.toLowerCase();
    if (typeof valB === 'string') valB = valB.toLowerCase();
    
    if (valA === undefined || valA === null) valA = 0;
    if (valB === undefined || valB === null) valB = 0;

    if (valA < valB) return -1 * modifier;
    if (valA > valB) return 1 * modifier;
    return 0;
  });
});

const playSong = (song) => {
  store.playSong(song, sortedSongs.value);
};

const isCurrentSong = (song) => {
  return store.currentSong && store.currentSong.path === song.path;
};

const formatDuration = (seconds) => {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${m}:${s.toString().padStart(2, '0')}`;
};

const getSortIcon = (key) => {
  if (sortKey.value !== key) return '';
  return sortOrder.value === 'asc' ? '▲' : '▼';
};
</script>

<template>
  <div class="w-full text-left text-sm 2xl:text-lg px-6 pb-12">
    <!-- Header -->
    <div 
      class="grid gap-4 text-[var(--text-secondary)] text-xs 2xl:text-sm font-medium uppercase tracking-wide border-b border-[var(--border-color)] py-2 mb-2 sticky top-0 bg-[var(--app-bg)]/95 backdrop-blur-xl z-10 select-none
             grid-cols-[20px_3fr_2fr_2fr_60px]
             2xl:grid-cols-[40px_4fr_3fr_3fr_80px]"
    >
      <div class="text-center">#</div>
      <div @click="toggleSort('title')" class="cursor-pointer hover:text-white flex items-center gap-1">
        Title <span class="text-[8px] 2xl:text-[10px]">{{ getSortIcon('title') }}</span>
      </div>
      <div @click="toggleSort('artist')" class="cursor-pointer hover:text-white flex items-center gap-1">
        Artist <span class="text-[8px] 2xl:text-[10px]">{{ getSortIcon('artist') }}</span>
      </div>
      <div @click="toggleSort('album')" class="cursor-pointer hover:text-white flex items-center gap-1">
        Album <span class="text-[8px] 2xl:text-[10px]">{{ getSortIcon('album') }}</span>
      </div>
      <div @click="toggleSort('duration_secs')" class="cursor-pointer hover:text-white flex items-center justify-end gap-1 text-right">
        Time <span class="text-[8px] 2xl:text-[10px]">{{ getSortIcon('duration_secs') }}</span>
      </div>
    </div>

    <!-- Rows -->
    <div class="space-y-0.5">
      <div 
        v-for="(song, index) in sortedSongs" 
        :key="song.path" 
        @click="playSong(song)"
        class="grid gap-4 py-2 px-2 rounded-md hover:bg-[#2a2a2a] group items-center transition-colors cursor-pointer
               grid-cols-[20px_3fr_2fr_2fr_60px]
               2xl:grid-cols-[40px_4fr_3fr_3fr_80px]
               2xl:py-3"
        :class="{ 'bg-[#2a2a2a]': isCurrentSong(song) }"
      >
        <div class="text-xs 2xl:text-sm text-gray-500 text-center">
          <span v-if="isCurrentSong(song) && store.isPlaying" class="text-[var(--accent-color)]">▶</span>
          <span v-else>{{ song.track_number || index + 1 }}</span>
        </div>

        <!-- Title & Cover -->
        <div class="flex items-center gap-3 2xl:gap-5 overflow-hidden">
          <CoverImage :path="song.path" className="h-10 w-10 2xl:h-14 2xl:w-14 rounded-[4px] shadow-sm shrink-0 bg-[#333]" />
          <div class="truncate">
            <div 
              class="text-[13px] 2xl:text-[16px] font-medium text-white truncate leading-tight"
              :class="{ 'text-[var(--accent-color)]': isCurrentSong(song) }"
            >
              {{ song.title }}
            </div>
          </div>
        </div>
        
        <div class="text-[13px] 2xl:text-[15px] text-[var(--text-secondary)] truncate">{{ song.artist }}</div>
        <div class="text-[13px] 2xl:text-[15px] text-[var(--text-secondary)] truncate">{{ song.album }}</div>
        <div class="text-[12px] 2xl:text-[14px] text-[var(--text-secondary)] text-right font-variant-numeric tabular-nums">{{ formatDuration(song.duration_secs) }}</div>
      </div>
      
      <div v-if="songs.length === 0" class="p-20 text-center text-gray-600">
        <div class="text-4xl mb-4 opacity-20">♫</div>
        <p>No songs found.</p>
      </div>
    </div>
  </div>
</template>