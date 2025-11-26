<script setup>
import { computed } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { store } from '../store';
import SongList from '../components/SongList.vue';

const route = useRoute();
const router = useRouter();
const albumName = route.params.name;

const albumSongs = computed(() => 
  store.songs.filter(s => s.album === albumName)
);
</script>

<template>
  <div class="h-full flex flex-col">
    <div class="p-4 bg-gray-800 flex items-center gap-4">
      <button @click="router.back()" class="text-gray-400 hover:text-white">
        ← Back
      </button>
      <h2 class="text-xl font-bold text-white">Album: {{ albumName }}</h2>
    </div>
    <div class="flex-1 overflow-auto">
      <SongList :songs="albumSongs" />
    </div>
  </div>
</template>