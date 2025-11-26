<script setup>
import { computed } from 'vue';
import { store } from '../store';
import { useRouter } from 'vue-router';

const router = useRouter();

const artists = computed(() => {
  const map = new Map();
  store.songs.forEach(song => {
    if (!map.has(song.artist)) {
      map.set(song.artist, { name: song.artist, count: 0, albums: new Set() });
    }
    const entry = map.get(song.artist);
    entry.count++;
    entry.albums.add(song.album);
  });
  return Array.from(map.values()).sort((a, b) => a.name.localeCompare(b.name));
});

function openArtist(artistName) {
  router.push({ name: 'ArtistDetail', params: { name: artistName } });
}
</script>

<template>
  <div class="h-full overflow-auto p-6">
    <h2 class="text-xl font-bold mb-4 text-gray-300">Artists ({{ artists.length }})</h2>
    <div class="flex flex-col gap-2">
      <div 
        v-for="artist in artists" 
        :key="artist.name"
        @click="openArtist(artist.name)"
        class="bg-gray-800 hover:bg-gray-750 p-3 rounded flex justify-between items-center cursor-pointer border-l-4 border-transparent hover:border-blue-500 transition-colors"
      >
        <div class="font-medium text-gray-200">{{ artist.name }}</div>
        <div class="text-xs text-gray-500 flex gap-3">
          <span>{{ artist.albums.size }} Albums</span>
          <span>{{ artist.count }} Songs</span>
        </div>
      </div>
    </div>
  </div>
</template>