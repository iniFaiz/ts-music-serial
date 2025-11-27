<script setup>
import { computed } from 'vue';
import { store } from '../store';
import { useRouter } from 'vue-router';
import CoverImage from '../components/CoverImage.vue';

const router = useRouter();

const artists = computed(() => {
  const map = new Map();
  store.songs.forEach(song => {
    if (!map.has(song.artist)) {
      map.set(song.artist, { 
        name: song.artist, 
        count: 0, 
        albums: new Set(),
        coverPath: song.path
      });
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
  <div class="h-full overflow-auto px-8 pt-8 pb-12">
    <h1 class="text-3xl font-bold tracking-tight text-white mb-6">Artists</h1>
    
    <div class="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 gap-x-6 gap-y-10">
      <div 
        v-for="artist in artists" 
        :key="artist.name"
        @click="openArtist(artist.name)"
        class="cursor-pointer group text-center"
      >
        <!-- Artist Image -->
        <div class="w-full aspect-square mb-4 mx-auto max-w-[200px] relative">
           <CoverImage 
            :path="artist.coverPath" 
            className="w-full h-full rounded-full shadow-lg object-cover bg-[#282828] group-hover:scale-[1.02] transition-transform duration-200"
          />
        </div>
        
        <h3 class="text-[15px] font-medium text-white truncate">{{ artist.name }}</h3>
      </div>
    </div>
  </div>
</template>