<script setup>
import { computed } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { store } from '../store';
import SongList from '../components/SongList.vue';
import CoverImage from '../components/CoverImage.vue';

const route = useRoute();
const router = useRouter();
const albumName = route.params.name;

const albumSongs = computed(() => {
  return store.songs
    .filter(s => s.album === albumName)
    .sort((a, b) => {
      const tA = a.track_number || 0;
      const tB = b.track_number || 0;
      return tA - tB;
    });
});

const albumInfo = computed(() => {
  if (albumSongs.value.length === 0) return {};
  
  const first = albumSongs.value[0];
  const totalSecs = albumSongs.value.reduce((acc, s) => acc + s.duration_secs, 0);
  const mins = Math.floor(totalSecs / 60);
  
  return {
    artist: first.artist,
    year: first.year ? first.year.toString() : "Unknown Year", 
    totalTime: `${mins} minutes`,
    count: albumSongs.value.length,
    coverPath: first.path
  };
});

const playAlbum = () => {
  if (albumSongs.value.length > 0) {
    store.playSong(albumSongs.value[0], albumSongs.value);
  }
};

const shuffleAlbum = () => {
  if (albumSongs.value.length > 0) {
    store.shuffleMode = true;
    const randomIndex = Math.floor(Math.random() * albumSongs.value.length);
    store.playSong(albumSongs.value[randomIndex], albumSongs.value);
  }
};

const goToArtist = () => {
  if (albumInfo.value.artist) {
    router.push({ name: 'ArtistDetail', params: { name: albumInfo.value.artist } });
  }
};
</script>

<template>
  <div class="flex flex-col h-full overflow-auto">
    <!-- Header -->
    <div class="p-8 flex gap-8 items-end bg-gradient-to-b from-[#2a2a2a] to-[var(--app-bg)]">
      <!-- Cover -->
      <div class="relative shadow-2xl h-52 w-52 shrink-0 group">
        <CoverImage 
          :path="albumInfo.coverPath" 
          className="h-full w-full rounded-md shadow-lg"
        />
      </div>

      <!-- Info -->
      <div class="flex flex-col gap-1 pb-2 overflow-hidden">
        <h4 class="text-sm font-bold text-[var(--accent-color)] uppercase tracking-wider mb-1">Album</h4>
        <h1 class="text-4xl font-bold tracking-tight text-white truncate">{{ albumName }}</h1>
        <h2 
          @click="goToArtist"
          class="text-xl font-medium text-[var(--accent-color)] truncate cursor-pointer hover:underline"
        >
          {{ albumInfo.artist }}
        </h2>
        
        <p class="text-xs text-[var(--text-secondary)] font-medium uppercase mt-2 tracking-wide">
          {{ albumInfo.year }}
        </p>
        
        <div class="flex gap-3 mt-6">
           <button @click="playAlbum" class="bg-[var(--accent-color)] text-white px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-red-500 transition flex items-center gap-2 shadow-lg">
             <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="currentColor" stroke="none"><polygon points="5 3 19 12 5 21 5 3"></polygon></svg>
             Play
           </button>
           
           <button @click="shuffleAlbum" class="bg-[#3a3a3a] text-[var(--accent-color)] px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-[#444] transition flex items-center gap-2 shadow-lg">
             <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 3h5v5M4 20L21 3M21 16v5h-5M15 15l6 6M4 4l5 5"/></svg>
             Shuffle
           </button>
        </div>
      </div>
    </div>

    <div class="px-2 pb-12">
      <SongList :songs="albumSongs" />
      <div class="px-8 py-4 text-xs text-[var(--text-secondary)]">
        {{ albumInfo.count }} Songs, {{ albumInfo.totalTime }}
      </div>
    </div>
  </div>
</template>