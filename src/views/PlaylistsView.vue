<script setup>
import { computed } from 'vue';
import { store } from '../store';
import { useRouter } from 'vue-router';
import PlaylistCover from '../components/PlaylistCover.vue';
import { navigateWithTransition } from '../viewTransition';

defineOptions({ name: 'PlaylistsView' });

const router = useRouter();

const playlists = computed(() => store.playlists);

function openPlaylist(id, event) {
  const coverEl = event.currentTarget.querySelector('.cover-image');
  navigateWithTransition(
    () => router.push({ name: 'PlaylistDetail', params: { id } }),
    coverEl,
    'shared-cover',
    'to-album-transition'
  );
}

function playPlaylist(id) {
  const songs = store.playlistSongs(id);
  if (songs.length > 0) {
    store.playSong(songs[0], songs);
  }
}

function newPlaylist() {
  store.openPlaylistModal();
}
</script>

<template>
  <div class="h-full overflow-auto px-8 pt-8 pb-12">
    <div class="flex items-center justify-between mb-6">
      <h1 class="text-3xl font-bold tracking-tight text-white">Playlists</h1>
      <button
        @click="newPlaylist"
        class="bg-[var(--accent-color)] text-white px-5 py-1.5 rounded-[4px] text-xs font-semibold hover:bg-red-500 transition flex items-center gap-1.5 shadow-lg"
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <line x1="12" y1="5" x2="12" y2="19"></line>
          <line x1="5" y1="12" x2="19" y2="12"></line>
        </svg>
        New Playlist
      </button>
    </div>

    <div
      v-if="playlists.length > 0"
      class="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6 gap-x-6 gap-y-10"
    >
      <div
        v-for="pl in playlists"
        :key="pl.id"
        :data-cover-key="pl.id"
        @click="openPlaylist(pl.id, $event)"
        class="cursor-pointer group"
      >
        <!-- Playlist Art -->
        <div
          class="w-full aspect-square mb-3 relative shadow-lg group-hover:scale-[1.02] transition-transform duration-200 ease-out"
        >
          <PlaylistCover
            :name="pl.name"
            :cover="pl.cover"
            :size="200"
            className="w-full h-full rounded-md bg-[#282828] cover-image"
          />
          <!-- Hover overlay -->
          <div
            class="absolute inset-0 bg-black/20 opacity-0 group-hover:opacity-100 transition-opacity rounded-md flex items-end p-3"
          >
            <div
              v-if="pl.paths.length > 0"
              @click.stop="playPlaylist(pl.id)"
              class="bg-[var(--accent-color)] text-white rounded-full p-3 shadow-lg translate-y-2 opacity-0 group-hover:translate-y-0 group-hover:opacity-100 transition-all duration-300 hover:scale-110 hover:bg-red-500"
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="currentColor"
                stroke="none"
              >
                <polygon points="5 3 19 12 5 21 5 3"></polygon>
              </svg>
            </div>
          </div>
        </div>

        <h3 class="text-[13px] font-medium text-white truncate pr-2 leading-snug">
          {{ pl.name }}
        </h3>
        <p class="text-[13px] text-[var(--text-secondary)] truncate">{{ pl.paths.length }} songs</p>
      </div>
    </div>

    <div v-else class="p-20 text-center text-gray-600">
      <div class="text-4xl mb-4 opacity-20">♪</div>
      <p>No playlists created yet.</p>
      <p class="text-xs mt-2">
        Click the "New Playlist" button above or in the sidebar to get started.
      </p>
    </div>
  </div>
</template>
