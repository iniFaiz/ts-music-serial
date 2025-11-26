<script setup>
import { computed, ref } from 'vue';
import CoverImage from './CoverImage.vue';

const props = defineProps({
  songs: { type: Array, required: true }
});

const DEFAULT_SORT_KEY = 'date_added';
const DEFAULT_SORT_ORDER = 'desc';

const sortKey = ref(DEFAULT_SORT_KEY);
const sortOrder = ref(DEFAULT_SORT_ORDER);

const formatDuration = (seconds) => {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${m}:${s.toString().padStart(2, '0')}`;
};

const getSortIcon = (key) => {
  if (sortKey.value !== key) return '↕';
  return sortOrder.value === 'asc' ? '↑' : '↓';
};

const toggleSort = (key) => {
  if (sortKey.value === key) {
    if (sortOrder.value === 'asc') {
      sortOrder.value = 'desc';
    } else {
      sortKey.value = DEFAULT_SORT_KEY;
      sortOrder.value = DEFAULT_SORT_ORDER;
    }
  } else {
    sortKey.value = key;
    sortOrder.value = 'asc';
  }
};

const sortedSongs = computed(() => {
  let items = [...props.songs];

  return items.sort((a, b) => {
    const modifier = sortOrder.value === 'asc' ? 1 : -1;
    
    let valA = a[sortKey.value];
    let valB = b[sortKey.value];

    if (typeof valA === 'string') valA = valA.toLowerCase();
    if (typeof valB === 'string') valB = valB.toLowerCase();

    if (valA < valB) return -1 * modifier;
    if (valA > valB) return 1 * modifier;
    return 0;
  });
});
</script>

<template>
  <div class="w-full text-left text-sm lg:text-base">
    <div 
      class="grid gap-4 bg-gray-800 text-gray-400 text-xs uppercase font-semibold p-3 lg:p-4 sticky top-0 z-10 shadow-sm select-none
             grid-cols-[2fr_1.5fr_1.5fr_0.5fr] 
             xl:grid-cols-[3fr_2fr_2fr_100px]"
    >
      <div @click="toggleSort('title')" class="pl-14 lg:pl-16 flex items-center gap-1 cursor-pointer hover:text-white transition-colors">
        Title <span class="text-[10px] opacity-50">{{ getSortIcon('title') }}</span>
      </div>
      <div @click="toggleSort('artist')" class="flex items-center gap-1 cursor-pointer hover:text-white transition-colors">
        Artist <span class="text-[10px] opacity-50">{{ getSortIcon('artist') }}</span>
      </div>
      <div @click="toggleSort('album')" class="flex items-center gap-1 cursor-pointer hover:text-white transition-colors">
        Album <span class="text-[10px] opacity-50">{{ getSortIcon('album') }}</span>
      </div>
      <div @click="toggleSort('duration_secs')" class="flex items-center justify-end gap-1 cursor-pointer hover:text-white transition-colors text-right">
        Duration <span class="text-[10px] opacity-50">{{ getSortIcon('duration_secs') }}</span>
      </div>
    </div>

    <div class="divide-y divide-gray-800">
      <div 
        v-for="song in sortedSongs" 
        :key="song.path" 
        class="grid gap-4 p-2 lg:p-3 xl:p-4 items-center hover:bg-gray-800/50 transition-colors group
               grid-cols-[2fr_1.5fr_1.5fr_0.5fr] 
               xl:grid-cols-[3fr_2fr_2fr_100px]"
      >
        <div class="flex items-center gap-3 lg:gap-4 overflow-hidden">
          <CoverImage 
            :path="song.path" 
            className="h-10 w-10 lg:h-12 lg:w-12 rounded shadow-sm shrink-0 transition-all" 
          />
          <div class="truncate">
            <div class="font-medium text-gray-200 truncate">{{ song.title }}</div>
          </div>
        </div>
        
        <div class="truncate text-gray-400">{{ song.artist }}</div>
        <div class="truncate text-gray-400">{{ song.album }}</div>
        <div class="text-right text-gray-500 font-mono">{{ formatDuration(song.duration_secs) }}</div>
      </div>
      
      <div v-if="songs.length === 0" class="p-12 text-center text-gray-500 italic">
        No songs found. Select a folder to start scanning.
      </div>
    </div>
  </div>
</template>