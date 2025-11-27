<script setup>
import { store } from '../store';
import SongList from '../components/SongList.vue';
</script>

<template>
  <div class="h-full flex flex-col">
    <div class="px-8 pt-8 pb-4">
      <h1 class="text-3xl font-bold tracking-tight text-white">Songs</h1>
      <p v-if="store.searchQuery" class="text-sm text-gray-500 mt-1">
        Searching for "{{ store.searchQuery }}"
      </p>
    </div>
    <div class="flex-1 overflow-auto">
      <SongList :songs="store.filteredSongs" />
    </div>
  </div>
</template>