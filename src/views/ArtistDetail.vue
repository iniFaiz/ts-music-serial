<script setup>
import { computed } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { store } from '../store';
import SongList from '../components/SongList.vue';
import CoverImage from '../components/CoverImage.vue';

const route = useRoute();
const router = useRouter();
const artistName = route.params.name;

// filter + sort
const artistSongs = computed(() => {
  const songs = store.songs.filter(s => s.artist === artistName);
  
  // sort
  return songs.sort((a, b) => {
    const albumA = (a.album || "").toLowerCase();
    const albumB = (b.album || "").toLowerCase();
    if (albumA < albumB) return -1;
    if (albumA > albumB) return 1;

    const trackA = a.track_number || 0;
    const trackB = b.track_number || 0;
    return trackA - trackB;
  });
});

const representativePath = computed(() => 
  artistSongs.value.length > 0 ? artistSongs.value[0].path : ""
);

const playArtist = () => {
  if (artistSongs.value.length > 0) {
    store.playSong(artistSongs.value[0], artistSongs.value);
  }
};
</script>

<template>
  <div class="h-full flex flex-col overflow-auto">
    <!-- Header -->
    <div class="h-[40vh] w-full relative overflow-hidden">
      <!-- Background Image -->
      <div class="absolute inset-0">
         <CoverImage 
          :path="representativePath" 
          className="w-full h-full object-cover blur-2xl opacity-40 scale-110"
        />
        <div class="absolute inset-0 bg-gradient-to-t from-[var(--app-bg)] to-transparent"></div>
      </div>

      <div class="absolute bottom-0 left-0 p-8 flex items-end gap-6 w-full">
        <div class="h-40 w-40 rounded-full overflow-hidden shadow-2xl border-4 border-[var(--app-bg)] shrink-0">
           <CoverImage 
            :path="representativePath" 
            className="h-full w-full object-cover"
          />
        </div>
        
        <div class="mb-4">
          <h1 class="text-5xl font-bold text-white tracking-tight drop-shadow-lg">{{ artistName }}</h1>
          
          <button 
            @click="playArtist"
            class="mt-4 bg-[var(--accent-color)] text-white px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-red-500 transition flex items-center gap-2 shadow-lg"
          >
             <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="currentColor" stroke="none"><polygon points="5 3 19 12 5 21 5 3"></polygon></svg>
             Play
           </button>
        </div>
      </div>
    </div>

    <div class="px-2 py-6">
      <h2 class="px-6 text-xl font-bold text-white mb-4">Popular Songs</h2>
      <SongList :songs="artistSongs" />
    </div>
  </div>
</template>