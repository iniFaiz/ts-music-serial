<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { store } from '../store';
import SongList from '../components/SongList.vue';
import CoverImage from '../components/CoverImage.vue';
import { navigateWithTransition } from '../viewTransition';

const route = useRoute();
const router = useRouter();
const albumName = route.params.name;
const coverRef = ref(null);

const menuOpen = ref(false);

const closeMenu = (e) => {
  if (e && e.target.closest('.playlist-menu-container')) return;
  menuOpen.value = false;
};

onMounted(() => {
  store.selectedAlbum = albumName;
  window.addEventListener('click', closeMenu);
});

onUnmounted(() => {
  window.removeEventListener('click', closeMenu);
});

const albumSongs = computed(() => {
  return store.songs
    .filter((s) => s.album === albumName)
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
    year: first.year ? first.year.toString() : 'Unknown Year',
    totalTime: `${mins} minutes`,
    count: albumSongs.value.length,
    coverPath: first.path,
  };
});

const playAlbum = () => {
  if (albumSongs.value.length > 0) {
    store.recordRecent('album', albumName);
    store.playSong(albumSongs.value[0], albumSongs.value);
  }
};

const shuffleAlbum = () => {
  if (albumSongs.value.length > 0) {
    store.recordRecent('album', albumName);
    store.shuffleMode = true;
    const randomIndex = Math.floor(Math.random() * albumSongs.value.length);
    store.playSong(albumSongs.value[randomIndex], albumSongs.value);
  }
};

const playNextAlbum = () => {
  if (albumSongs.value.length > 0) {
    store.playNextSongs(albumSongs.value);
  }
};

const playLastAlbum = () => {
  if (albumSongs.value.length > 0) {
    store.addToQueue(albumSongs.value);
  }
};

const deleteAlbumFromLibrary = async () => {
  if (confirm(`Are you sure you want to remove the album "${albumName}" from library?`)) {
    for (const song of albumSongs.value) {
      await store.removeSongFromLibrary(song.path);
    }
    router.push('/albums');
  }
};

const goToArtist = () => {
  if (albumInfo.value.artist) {
    const el = coverRef.value
      ? coverRef.value.querySelector('.cover-image') || coverRef.value
      : null;
    if (el) {
      navigateWithTransition(
        () => router.push({ name: 'ArtistDetail', params: { name: albumInfo.value.artist } }),
        el,
        'shared-cover',
        'to-artist-transition'
      );
    } else {
      router.push({ name: 'ArtistDetail', params: { name: albumInfo.value.artist } });
    }
  }
};
</script>

<template>
  <div class="flex flex-col h-full overflow-auto">
    <!-- Header -->
    <div class="p-8 flex gap-8 items-end bg-gradient-to-b from-[#2a2a2a] to-[var(--app-bg)]">
      <!-- Cover -->
      <div ref="coverRef" class="relative shadow-2xl h-52 w-52 shrink-0 group">
        <CoverImage
          :path="albumInfo.coverPath"
          className="h-full w-full rounded-md shadow-lg"
          transitionName="shared-cover"
        />
      </div>

      <!-- Info -->
      <div class="flex flex-col gap-1 pb-2 overflow-hidden flex-1">
        <h4 class="text-sm font-bold text-[var(--accent-color)] uppercase tracking-wider mb-1">
          Album
        </h4>
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

        <div class="flex gap-3 mt-6 items-center">
          <button
            @click="playAlbum"
            class="bg-[var(--accent-color)] text-white px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-red-500 transition flex items-center gap-2 shadow-lg"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="currentColor"
              stroke="none"
            >
              <polygon points="5 3 19 12 5 21 5 3"></polygon>
            </svg>
            Play
          </button>

          <button
            @click="shuffleAlbum"
            class="bg-[#3a3a3a] text-[var(--accent-color)] px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-[#444] transition flex items-center gap-2 shadow-lg"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M16 3h5v5M4 20L21 3M21 16v5h-5M15 15l6 6M4 4l5 5" />
            </svg>
            Shuffle
          </button>
        </div>
      </div>

      <!-- Ellipsis Options Menu at the far right end -->
      <div class="relative pb-2 self-end playlist-menu-container">
        <button
          @click.stop="menuOpen = !menuOpen"
          class="text-red-500 hover:text-red-400 p-2 rounded-full hover:bg-white/5 transition-colors flex items-center justify-center"
          title="More options"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="currentColor"
            stroke="none"
          >
            <circle cx="5" cy="12" r="2"></circle>
            <circle cx="12" cy="12" r="2"></circle>
            <circle cx="19" cy="12" r="2"></circle>
          </svg>
        </button>

        <!-- Options Dropdown -->
        <div
          v-if="menuOpen"
          class="absolute right-0 mt-2 z-50 w-56 rounded-lg bg-[#282828] border border-[#3a3a3a] py-1.5 shadow-2xl text-sm text-white"
        >
          <button
            @click="playAlbum"
            :disabled="albumSongs.length === 0"
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors disabled:opacity-40"
          >
            Play "{{ albumName }}"
          </button>
          <button
            @click="shuffleAlbum"
            :disabled="albumSongs.length === 0"
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors disabled:opacity-40"
          >
            Shuffle "{{ albumName }}"
          </button>
          <button
            @click="playNextAlbum"
            :disabled="albumSongs.length === 0"
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors disabled:opacity-40"
          >
            Play next
          </button>
          <button
            @click="playLastAlbum"
            :disabled="albumSongs.length === 0"
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors disabled:opacity-40"
          >
            Play last
          </button>
          <div class="border-t border-[#3a3a3a] my-1"></div>
          <button
            @click="deleteAlbumFromLibrary"
            class="w-full text-left px-4 py-2 text-red-500 hover:bg-[#3a3a3a] transition-colors"
          >
            Delete from library
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
