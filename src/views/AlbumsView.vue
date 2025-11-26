<script setup>
import { computed } from 'vue';
import { store } from '../store';
import { useRouter } from 'vue-router';
import CoverImage from '../components/CoverImage.vue';

const router = useRouter();

const albums = computed(() => {
  const map = new Map();
  store.songs.forEach(song => {
    if (!map.has(song.album)) {
      map.set(song.album, { 
        name: song.album, 
        artist: song.artist, 
        count: 0,
        coverPath: song.path 
      });
    }
    map.get(song.album).count++;
  });
  return Array.from(map.values()).sort((a, b) => a.name.localeCompare(b.name));
});

function openAlbum(albumName) {
  router.push({ name: 'AlbumDetail', params: { name: albumName } });
}
</script>

<template>
  <div class="h-full overflow-auto p-6 relative">
    <h2 class="text-xl font-bold mb-4 text-gray-300 bg-gray-900 z-10 py-2">
      Albums ({{ albums.length }})
    </h2>
    
    <div class="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 gap-6">
      <div 
        v-for="album in albums" 
        :key="album.name"
        @click="openAlbum(album.name)"
        class="bg-gray-800 hover:bg-gray-700 p-4 rounded-xl cursor-pointer transition-all hover:scale-105 group shadow-lg border border-gray-700 hover:border-gray-600"
      >
        <CoverImage 
          :path="album.coverPath" 
          className="w-full aspect-square bg-gray-900 rounded-lg mb-3 shadow-md group-hover:shadow-xl transition-shadow"
        />
        
        <h3 class="font-bold text-gray-200 truncate">{{ album.name }}</h3>
        <p class="text-xs text-gray-400 truncate">{{ album.artist }}</p>
        <p class="text-xs text-gray-500 mt-1">{{ album.count }} songs</p>
      </div>
    </div>
  </div>
</template>