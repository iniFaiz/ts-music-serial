<script setup>
import { ref, computed, watch } from 'vue';
import { store } from '../store';
import { useRouter } from 'vue-router';
import CoverImage from '../components/CoverImage.vue';
import LibraryGridSkeleton from '../components/LibraryGridSkeleton.vue';
import { navigateWithTransition } from '../viewTransition';
import { useQuery } from '../useLibraryData';
import { fetchAlbums, fetchAlbumTracks } from '../libraryQueries';

defineOptions({ name: 'AlbumsView' });

const router = useRouter();

// Albums grouped in SQLite (GROUP BY), mapped to the card shape the template uses.
const { data: albums, loading } = useQuery(fetchAlbums, {
  initial: [],
  cacheKey: 'albums',
});

const searchQuery = ref('');
const sortBy = ref('name');
const sortOrder = ref('asc');

// Smart UX: Default to descending for count and last played, and ascending for alphabetical values.
watch(sortBy, (newVal) => {
  if (newVal === 'lastPlayed' || newVal === 'count') {
    sortOrder.value = 'desc';
  } else {
    sortOrder.value = 'asc';
  }
});

const toggleSortOrder = () => {
  sortOrder.value = sortOrder.value === 'asc' ? 'desc' : 'asc';
};

const filteredAndSortedAlbums = computed(() => {
  let list = albums.value || [];
  const q = searchQuery.value.trim().toLowerCase();
  if (q) {
    list = list.filter(
      (a) =>
        (a.name && a.name.toLowerCase().includes(q)) ||
        (a.artist && a.artist.toLowerCase().includes(q)) ||
        (a.allArtists && a.allArtists.toLowerCase().includes(q))
    );
  }
  return [...list].sort((a, b) => {
    let factor = sortOrder.value === 'asc' ? 1 : -1;
    if (sortBy.value === 'name') {
      return (a.name || '').localeCompare(b.name || '') * factor;
    } else if (sortBy.value === 'artist') {
      return (a.artist || '').localeCompare(b.artist || '') * factor;
    } else if (sortBy.value === 'count') {
      return ((a.count || 0) - (b.count || 0)) * factor;
    } else if (sortBy.value === 'lastPlayed') {
      return ((a.lastPlayed || 0) - (b.lastPlayed || 0)) * factor;
    }
    return 0;
  });
});

function openAlbum(albumName, event) {
  if (event && (event.button !== 0 || event.which === 3)) return;
  store.selectedAlbum = albumName;
  const coverEl = event.currentTarget.querySelector('.cover-image');
  navigateWithTransition(
    () => router.push({ name: 'AlbumDetail', params: { name: albumName } }),
    coverEl,
    'shared-cover',
    'to-album-transition'
  );
}

async function playAlbum(albumName) {
  store.selectedAlbum = albumName;
  const songs = await fetchAlbumTracks(albumName);
  if (songs.length > 0) {
    store.recordRecent('album', albumName);
    store.playSong(songs[0], songs);
  }
}

function goToArtist(artistName, event = null) {
  if (!artistName || artistName === 'Unknown Artist') return;
  const navigate = () => router.push({ name: 'ArtistDetail', params: { name: artistName } });

  const cardEl = event ? event.currentTarget.closest('.group') : null;
  const coverEl = cardEl ? cardEl.querySelector('.cover-image') : null;

  if (coverEl) {
    navigateWithTransition(navigate, coverEl, 'shared-cover', 'to-artist-transition');
  } else {
    navigate();
  }
}

// Context Menu State & Actions
const menuState = ref({
  open: false,
  x: 0,
  y: 0,
  album: null,
});

const openContextMenu = (album, e) => {
  if (e) {
    e.preventDefault();
    e.stopPropagation();
  }
  const menuWidth = 224;
  const menuHeight = 320;
  let x = e ? e.clientX : 0;
  let y = e ? e.clientY : 0;
  if (x + menuWidth > window.innerWidth) {
    x = window.innerWidth - menuWidth - 10;
  }
  if (y + menuHeight > window.innerHeight) {
    y = window.innerHeight - menuHeight - 10;
  }
  menuState.value = {
    open: true,
    x: Math.max(10, x),
    y: Math.max(10, y),
    album,
  };
};

const closeContextMenu = () => {
  menuState.value.open = false;
};

const onWindowClick = (e) => {
  if (!menuState.value.open) return;
  const menuEl = document.querySelector('.context-menu-container');
  if (menuEl && menuEl.contains(e.target)) return;
  closeContextMenu();
};

const onBeforeLeave = (el) => {
  const width = el.offsetWidth;
  const height = el.offsetHeight;
  const left = el.offsetLeft;
  const top = el.offsetTop;

  el.style.width = `${width}px`;
  el.style.height = `${height}px`;
  el.style.left = `${left}px`;
  el.style.top = `${top}px`;
};

import { onMounted, onUnmounted } from 'vue';
onMounted(() => window.addEventListener('click', onWindowClick));
onUnmounted(() => window.removeEventListener('click', onWindowClick));

const handleMenuPlayAll = async () => {
  const album = menuState.value.album;
  closeContextMenu();
  if (!album) return;
  await playAlbum(album.name);
};

const handleMenuShuffle = async () => {
  const album = menuState.value.album;
  closeContextMenu();
  if (!album) return;
  const songs = await fetchAlbumTracks(album.name);
  if (songs.length > 0) {
    store.recordRecent('album', album.name);
    await store.playRandom(songs);
  }
};

const handleMenuPlayNext = async () => {
  const album = menuState.value.album;
  closeContextMenu();
  if (!album) return;
  const songs = await fetchAlbumTracks(album.name);
  if (songs.length > 0) {
    store.playNextSongs(songs);
  }
};

const handleMenuAddToQueue = async () => {
  const album = menuState.value.album;
  closeContextMenu();
  if (!album) return;
  const songs = await fetchAlbumTracks(album.name);
  if (songs.length > 0) {
    store.addToQueue(songs);
  }
};

const handleMenuNewPlaylist = async () => {
  const album = menuState.value.album;
  closeContextMenu();
  if (!album) return;
  const songs = await fetchAlbumTracks(album.name);
  const paths = songs.map((s) => s.path);
  store.openPlaylistModal(paths);
};

const handleMenuAddToExistingPlaylist = async (playlistId) => {
  const album = menuState.value.album;
  closeContextMenu();
  if (!album) return;
  const songs = await fetchAlbumTracks(album.name);
  const paths = songs.map((s) => s.path);
  store.runMutation(() => store.addToPlaylist(playlistId, paths));
};

const handleMenuDelete = async () => {
  const album = menuState.value.album;
  closeContextMenu();
  if (!album) return;
  store.showConfirm({
    title: 'Delete Album',
    message: `Are you sure you want to delete all songs from "${album.name}"?`,
    confirmText: 'Delete',
    cancelText: 'Cancel',
    onConfirm: async () => {
      const songs = await fetchAlbumTracks(album.name);
      const consentToken = await store.requestDeleteConsent(songs.map((song) => song.path));
      if (!consentToken) return;
      for (const song of songs) {
        await store.deleteSong(song.path, consentToken).catch(() => {});
      }
    },
  });
};
</script>

<template>
  <div class="h-full overflow-auto px-8 pt-8 pb-12">
    <!-- Header with controls -->
    <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-4 mb-8">
      <h1 class="text-3xl font-bold tracking-tight text-white">Albums</h1>

      <div class="flex items-center gap-3 w-full sm:w-auto">
        <!-- Search Bar -->
        <div class="relative flex-1 sm:w-60 sm:flex-none">
          <span class="absolute text-gray-500 -translate-y-1/2 left-3 top-1/2">
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="3"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <circle cx="11" cy="11" r="8"></circle>
              <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
            </svg>
          </span>
          <input
            v-model="searchQuery"
            type="text"
            placeholder="Search albums..."
            class="w-full bg-[#2a2a2a] text-xs text-white rounded-md py-2 pl-9 pr-8 focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] placeholder-gray-500"
          />
          <button
            v-if="searchQuery"
            @click="searchQuery = ''"
            class="absolute right-2.5 top-1/2 -translate-y-1/2 text-gray-500 hover:text-white"
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
              <line x1="18" y1="6" x2="6" y2="18"></line>
              <line x1="6" y1="6" x2="18" y2="18"></line>
            </svg>
          </button>
        </div>

        <!-- Sort selector -->
        <div class="relative">
          <select
            v-model="sortBy"
            class="appearance-none bg-[#2a2a2a] border-none text-xs text-white rounded-md py-2 pl-3 pr-8 focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] cursor-pointer"
          >
            <option value="name">Title</option>
            <option value="artist">Artist</option>
            <option value="count">Tracks</option>
            <option value="lastPlayed">Last Played</option>
          </select>
          <span
            class="absolute right-2.5 top-1/2 -translate-y-1/2 text-gray-500 pointer-events-none"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="10"
              height="10"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="3"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <polyline points="6 9 12 15 18 9"></polyline>
            </svg>
          </span>
        </div>

        <!-- Sort Order Button -->
        <button
          @click="toggleSortOrder"
          class="bg-[#2a2a2a] hover:bg-[#3a3a3a] text-white p-2 rounded-md transition-colors flex items-center justify-center h-[32px] w-[32px]"
          :title="sortOrder === 'asc' ? 'Sort Ascending' : 'Sort Descending'"
        >
          <svg
            v-if="sortOrder === 'asc'"
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
            <polyline points="19 12 12 19 5 12"></polyline>
          </svg>
          <svg
            v-else
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
            <line x1="12" y1="19" x2="12" y2="5"></line>
            <polyline points="5 12 12 5 19 12"></polyline>
          </svg>
        </button>
      </div>
    </div>

    <!-- Grid List -->
    <LibraryGridSkeleton v-if="loading" label="Loading albums" />

    <TransitionGroup
      v-else
      name="grid"
      tag="div"
      @before-leave="onBeforeLeave"
      class="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6 gap-x-6 gap-y-10 relative"
    >
      <div
        v-for="album in filteredAndSortedAlbums"
        :key="album.name"
        :data-cover-key="album.name"
        :data-artist-key="album.artist"
        @click="openAlbum(album.name, $event)"
        @contextmenu.prevent.stop="openContextMenu(album, $event)"
        @mousedown.right.prevent.stop="openContextMenu(album, $event)"
        class="cursor-pointer group"
      >
        <!-- Album Art -->
        <div
          class="w-full aspect-square mb-3 relative shadow-lg group-hover:scale-[1.02] transition-transform duration-200 ease-out"
        >
          <CoverImage
            :path="album.coverPath"
            className="w-full h-full rounded-md bg-[#282828] cover-image"
          />
          <!-- Hover -->
          <div
            class="absolute inset-0 bg-black/20 opacity-0 group-hover:opacity-100 transition-opacity rounded-md flex items-end p-3"
          >
            <div
              @click.stop="playAlbum(album.name)"
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
          {{ album.name }}
        </h3>
        <p
          @click.stop="goToArtist(album.artist, $event)"
          class="text-[13px] text-[var(--text-secondary)] truncate hover:text-[var(--accent-color)] hover:underline cursor-pointer inline-block max-w-full transition-colors"
        >
          {{ album.artist }}
        </p>
      </div>
    </TransitionGroup>

    <!-- Empty State -->
    <div
      v-if="!loading && filteredAndSortedAlbums.length === 0"
      class="py-20 text-center text-gray-500 animate-fade-in flex flex-col items-center"
    >
      <svg
        class="h-16 w-16 text-gray-500 opacity-40 mb-3"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
      >
        <circle cx="12" cy="12" r="10" />
        <circle cx="12" cy="12" r="3" />
        <path d="M12 2a10 10 0 0 1 10 10" stroke-dasharray="1 3" />
        <path d="M12 6a6 6 0 0 1 6 6" />
      </svg>
      <p class="text-sm font-medium text-white/80">No albums found</p>
      <p class="text-xs text-gray-500 mt-1">Try searching for something else</p>
    </div>

    <!-- Right-click Context Menu -->
    <Teleport to="body">
      <div
        v-if="menuState.open"
        class="fixed z-[250] w-56 bg-[#282828] border border-[#3a3a3a] rounded-md shadow-2xl py-1 text-sm text-white select-none context-menu-container"
        :style="{ left: menuState.x + 'px', top: menuState.y + 'px' }"
        @click.stop
        @contextmenu.prevent
      >
        <button
          @click="handleMenuPlayAll"
          class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
        >
          Play all
        </button>
        <button
          @click="handleMenuShuffle"
          class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
        >
          Shuffle
        </button>
        <button
          @click="handleMenuPlayNext"
          class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
        >
          Play next
        </button>
        <button
          @click="handleMenuAddToQueue"
          class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
        >
          Add to queue
        </button>

        <div class="border-t border-[#3a3a3a] my-1"></div>

        <div class="px-4 py-1 text-[11px] uppercase tracking-wide text-gray-500 font-medium">
          Add to playlist
        </div>
        <div class="max-h-40 overflow-auto scrollbar-thin">
          <button
            v-for="pl in store.normalPlaylists"
            :key="pl.id"
            @click="handleMenuAddToExistingPlaylist(pl.id)"
            class="w-full text-left px-4 py-1.5 hover:bg-[#3a3a3a] transition-colors truncate"
          >
            {{ pl.name }}
          </button>
        </div>
        <button
          @click="handleMenuNewPlaylist"
          class="w-full text-left px-4 py-2 text-[var(--accent-color)] hover:bg-[#3a3a3a] transition-colors font-medium"
        >
          + New playlist
        </button>

        <div class="border-t border-[#3a3a3a] my-1"></div>

        <button
          @click="handleMenuDelete"
          class="w-full text-left px-4 py-2 text-red-500 hover:bg-[#3a3a3a] transition-colors font-medium"
        >
          Delete Album
        </button>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.grid-move {
  transition: transform 0.3s cubic-bezier(0.16, 1, 0.3, 1);
}

.grid-enter-active {
  transition: opacity 0.25s cubic-bezier(0.16, 1, 0.3, 1);
}

.grid-leave-active {
  transition: opacity 0.25s cubic-bezier(0.16, 1, 0.3, 1) !important;
  position: absolute !important;
  z-index: 0;
  pointer-events: none;
}

.grid-enter-from {
  opacity: 0;
}

.grid-leave-to {
  opacity: 0;
}

.animate-fade-in {
  animation: fadeIn 0.3s cubic-bezier(0.16, 1, 0.3, 1) forwards;
}
@keyframes fadeIn {
  from {
    opacity: 0;
    transform: scale(0.96);
  }
  to {
    opacity: 1;
    transform: scale(1);
  }
}
</style>
