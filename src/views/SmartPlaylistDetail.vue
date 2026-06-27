<script setup>
import { computed, ref, onMounted, onUnmounted } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { store } from '../store';
import SongList from '../components/SongList.vue';
import PlaylistCover from '../components/PlaylistCover.vue';

const route = useRoute();
const router = useRouter();

const smartId = computed(() => route.params.id);
const sp = computed(() => store.getSmartPlaylist(smartId.value));
const songs = computed(() => store.smartSongs(smartId.value));

const playAll = () => {
  if (songs.value.length > 0) {
    store.recordRecent('smart', smartId.value);
    store.playSong(songs.value[0], songs.value);
  }
};

const shuffleAll = () => {
  if (songs.value.length === 0) return;
  store.recordRecent('smart', smartId.value);
  store.shuffleMode = true;
  const i = Math.floor(Math.random() * songs.value.length);
  store.playSong(songs.value[i], songs.value);
};

const edit = () => store.openSmartModal('edit', smartId.value);

const removeSmart = () => {
  if (sp.value) {
    store.deleteSmartPlaylist(sp.value.id);
    router.push('/home');
  }
};

const playNext = () => {
  if (songs.value.length > 0) store.playNextSongs(songs.value);
  menuOpen.value = false;
};
const playLast = () => {
  if (songs.value.length > 0) store.addToQueue(songs.value);
  menuOpen.value = false;
};

const menuOpen = ref(false);
const closeMenu = (e) => {
  if (e && e.target.closest('.smart-menu-container')) return;
  menuOpen.value = false;
};
onMounted(() => window.addEventListener('click', closeMenu));
onUnmounted(() => window.removeEventListener('click', closeMenu));
</script>

<template>
  <div v-if="sp" class="flex flex-col h-full overflow-auto">
    <!-- Header -->
    <div class="p-8 flex gap-8 items-end bg-gradient-to-b from-[#2a2a2a] to-[var(--app-bg)]">
      <PlaylistCover
        :name="sp.name"
        :cover="sp.cover"
        :size="208"
        className="h-52 w-52 rounded-md shadow-2xl shrink-0"
        style="view-transition-name: shared-cover"
      />

      <div class="flex flex-col gap-1 pb-2 overflow-hidden flex-1">
        <h4 class="text-sm font-bold text-[var(--accent-color)] uppercase tracking-wider mb-1 flex items-center gap-1.5">
          <svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="currentColor" stroke="none">
            <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
          </svg>
          Smart Playlist
        </h4>
        <h1 class="text-4xl font-bold tracking-tight text-white truncate">{{ sp.name }}</h1>
        <p v-if="sp.description" class="text-sm text-[var(--text-secondary)] mt-2 line-clamp-2 max-w-xl">
          {{ sp.description }}
        </p>
        <p class="text-xs text-[var(--text-secondary)] font-medium mt-2">{{ songs.length }} songs</p>

        <div class="flex gap-3 mt-6 items-center">
          <button
            @click="playAll"
            :disabled="songs.length === 0"
            class="bg-[var(--accent-color)] text-white px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-red-500 transition flex items-center gap-2 shadow-lg disabled:opacity-40"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="currentColor" stroke="none">
              <polygon points="5 3 19 12 5 21 5 3" />
            </svg>
            Play
          </button>
          <button
            @click="shuffleAll"
            :disabled="songs.length === 0"
            class="bg-[#3a3a3a] text-[var(--accent-color)] px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-[#444] transition flex items-center gap-2 shadow-lg disabled:opacity-40"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M16 3h5v5M4 20L21 3M21 16v5h-5M15 15l6 6M4 4l5 5" />
            </svg>
            Shuffle
          </button>
        </div>
      </div>

      <!-- Options menu -->
      <div class="relative pb-2 self-end smart-menu-container">
        <button
          @click.stop="menuOpen = !menuOpen"
          class="text-red-500 hover:text-red-400 p-2 rounded-full hover:bg-white/5 transition-colors flex items-center justify-center"
          title="More options"
        >
          <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="currentColor" stroke="none">
            <circle cx="5" cy="12" r="2" /><circle cx="12" cy="12" r="2" /><circle cx="19" cy="12" r="2" />
          </svg>
        </button>
        <div
          v-if="menuOpen"
          class="absolute right-0 mt-2 z-50 w-56 rounded-lg bg-[#282828] border border-[#3a3a3a] py-1.5 shadow-2xl text-sm text-white"
        >
          <button @click="edit" class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors">Edit</button>
          <button @click="playNext" :disabled="songs.length === 0" class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors disabled:opacity-40">Play next</button>
          <button @click="playLast" :disabled="songs.length === 0" class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors disabled:opacity-40">Play last</button>
          <div class="border-t border-[#3a3a3a] my-1"></div>
          <button @click="removeSmart" class="w-full text-left px-4 py-2 text-red-500 hover:bg-[#3a3a3a] transition-colors">Delete smart playlist</button>
        </div>
      </div>
    </div>

    <div class="px-2 pb-12">
      <SongList v-if="songs.length > 0" :songs="songs" />
      <div v-else class="py-16 px-6 text-center text-gray-500">
        <div class="text-4xl mb-3 opacity-20">⚡</div>
        <p class="text-sm font-medium text-white/80">No songs match these rules yet.</p>
        <p class="text-xs text-gray-500 mt-1 max-w-sm mx-auto">
          Loosen the rules or keep listening — the playlist fills in automatically.
        </p>
        <button
          @click="edit"
          class="mt-4 bg-[#282828] hover:bg-[#333] text-white text-xs font-semibold px-4 py-2 rounded-lg border border-[#3a3a3a] transition"
        >
          Edit Rules
        </button>
      </div>
    </div>
  </div>

  <div v-else class="p-20 text-center text-gray-600">
    <p>Smart playlist not found.</p>
  </div>
</template>
