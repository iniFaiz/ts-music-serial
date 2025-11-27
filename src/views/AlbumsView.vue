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

function playAlbum(albumName) {
  const songs = store.songs.filter(s => s.album === albumName);
  songs.sort((a, b) => (a.track_number || 0) - (b.track_number || 0));
  if (songs.length > 0) {
    store.playSong(songs[0], songs);
  }
}
</script>

<template>
  <div class="h-full overflow-auto px-8 pt-8 pb-12">
    <h1 class="text-3xl font-bold tracking-tight text-white mb-6">Albums</h1>
    
    <div class="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6 gap-x-6 gap-y-10">
      <div 
        v-for="album in albums" 
        :key="album.name"
        @click="openAlbum(album.name)"
        class="cursor-pointer group"
      >
        <!-- Album Art -->
        <div class="w-full aspect-square mb-3 relative shadow-lg group-hover:scale-[1.02] transition-transform duration-200 ease-out">
           <CoverImage 
            :path="album.coverPath" 
            className="w-full h-full rounded-md bg-[#282828]"
          />
          <!-- Hover -->
          <div class="absolute inset-0 bg-black/20 opacity-0 group-hover:opacity-100 transition-opacity rounded-md flex items-end p-3">
             <div 
               @click.stop="playAlbum(album.name)"
               class="bg-[var(--accent-color)] text-white rounded-full p-3 shadow-lg translate-y-2 opacity-0 group-hover:translate-y-0 group-hover:opacity-100 transition-all duration-300 hover:scale-110 hover:bg-red-500"
             >
               <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="currentColor" stroke="none"><polygon points="5 3 19 12 5 21 5 3"></polygon></svg>
             </div>
          </div>
        </div>
        
        <h3 class="text-[13px] font-medium text-white truncate pr-2 leading-snug">{{ album.name }}</h3>
        <p class="text-[13px] text-[var(--text-secondary)] truncate">{{ album.artist }}</p>
      </div>
    </div>
  </div>
</template>