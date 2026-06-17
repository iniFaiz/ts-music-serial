<script setup>
import { computed, ref, onMounted, onUnmounted } from 'vue';
import { store } from '../store';
import CoverImage from './CoverImage.vue';

const props = defineProps({
  songs: { type: Array, required: true },
  // When this list belongs to a playlist, rows offer "Remove from this playlist".
  playlistId: { type: String, default: '' },
});

const sortKey = ref(null);
const sortOrder = ref('asc');

const toggleSort = (key) => {
  if (sortKey.value === key) {
    if (sortOrder.value === 'asc') {
      sortOrder.value = 'desc';
    } else {
      sortKey.value = null;
      sortOrder.value = 'asc';
    }
  } else {
    sortKey.value = key;
    sortOrder.value = 'asc';
  }
};

const sortedSongs = computed(() => {
  let items = [...props.songs];

  if (!sortKey.value) {
    return items;
  }

  return items.sort((a, b) => {
    const modifier = sortOrder.value === 'asc' ? 1 : -1;
    let valA = a[sortKey.value];
    let valB = b[sortKey.value];

    if (typeof valA === 'string') valA = valA.toLowerCase();
    if (typeof valB === 'string') valB = valB.toLowerCase();

    if (valA === undefined || valA === null) valA = 0;
    if (valB === undefined || valB === null) valB = 0;

    if (valA < valB) return -1 * modifier;
    if (valA > valB) return 1 * modifier;
    return 0;
  });
});

const playSong = (song) => {
  store.playSong(song, sortedSongs.value);
};

const isCurrentSong = (song) => {
  return store.currentSong && store.currentSong.path === song.path;
};

const formatDuration = (seconds) => {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${m}:${s.toString().padStart(2, '0')}`;
};

const getSortIcon = (key) => {
  if (sortKey.value !== key) return '';
  return sortOrder.value === 'asc' ? '▲' : '▼';
};

// ---- Row context menu ---------------------------------------------------

const menu = ref({ open: false, x: 0, y: 0, song: null });

const openMenu = (song, event) => {
  // Clamp so the menu stays on-screen.
  const x = Math.min(event.clientX, window.innerWidth - 230);
  const y = Math.min(event.clientY, window.innerHeight - 320);
  menu.value = { open: true, x, y, song };
};

const closeMenu = () => {
  menu.value.open = false;
};

const playNext = () => {
  store.playNext(menu.value.song);
  closeMenu();
};

const addToQueue = () => {
  store.addToQueue(menu.value.song);
  closeMenu();
};

const toggleLike = () => {
  store.toggleFavorite(menu.value.song.path);
  closeMenu();
};

const addToPlaylist = (id) => {
  store.addToPlaylist(id, menu.value.song.path);
  closeMenu();
};

const newPlaylistWithSong = () => {
  store.openPlaylistModal(menu.value.song.path);
  closeMenu();
};

const removeFromThisPlaylist = () => {
  if (props.playlistId) store.removeFromPlaylist(props.playlistId, menu.value.song.path);
  closeMenu();
};

onMounted(() => {
  window.addEventListener('click', closeMenu);
  window.addEventListener('scroll', closeMenu, true);
  window.addEventListener('resize', closeMenu);
});

onUnmounted(() => {
  window.removeEventListener('click', closeMenu);
  window.removeEventListener('scroll', closeMenu, true);
  window.removeEventListener('resize', closeMenu);
});
</script>

<template>
  <div class="w-full text-left text-sm 2xl:text-lg px-6 pb-12">
    <!-- Header -->
    <div
      class="grid gap-4 text-[var(--text-secondary)] text-xs 2xl:text-sm font-medium uppercase tracking-wide border-b border-[var(--border-color)] py-2 mb-2 sticky top-0 bg-[var(--app-bg)]/95 backdrop-blur-xl z-10 select-none grid-cols-[20px_3fr_2fr_2fr_120px] 2xl:grid-cols-[40px_4fr_3fr_3fr_150px]"
    >
      <div class="text-center">#</div>
      <div
        @click="toggleSort('title')"
        class="cursor-pointer hover:text-white flex items-center gap-1"
      >
        Title <span class="text-[8px] 2xl:text-[10px]">{{ getSortIcon('title') }}</span>
      </div>
      <div
        @click="toggleSort('artist')"
        class="cursor-pointer hover:text-white flex items-center gap-1"
      >
        Artist <span class="text-[8px] 2xl:text-[10px]">{{ getSortIcon('artist') }}</span>
      </div>
      <div
        @click="toggleSort('album')"
        class="cursor-pointer hover:text-white flex items-center gap-1"
      >
        Album <span class="text-[8px] 2xl:text-[10px]">{{ getSortIcon('album') }}</span>
      </div>
      <div
        @click="toggleSort('duration_secs')"
        class="cursor-pointer hover:text-white flex items-center justify-end gap-1 text-right"
      >
        Time <span class="text-[8px] 2xl:text-[10px]">{{ getSortIcon('duration_secs') }}</span>
      </div>
    </div>

    <!-- Rows -->
    <div class="space-y-0.5">
      <div
        v-for="(song, index) in sortedSongs"
        :key="song.path"
        @click="playSong(song)"
        @contextmenu.prevent="openMenu(song, $event)"
        class="song-row grid gap-4 py-2 px-2 rounded-md hover:bg-[#2a2a2a] group items-center transition-colors cursor-pointer grid-cols-[20px_3fr_2fr_2fr_120px] 2xl:grid-cols-[40px_4fr_3fr_3fr_150px] 2xl:py-3"
        :class="{ 'bg-[#2a2a2a]': isCurrentSong(song) }"
      >
        <div
          class="text-xs 2xl:text-sm text-gray-500 text-center flex justify-center items-center h-full"
        >
          <span
            v-if="isCurrentSong(song) && store.isPlaying"
            class="text-[var(--accent-color)] animate-pulse"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="currentColor"
              stroke="none"
            >
              <polygon points="5 3 19 12 5 21 5 3"></polygon>
            </svg>
          </span>
          <span
            v-else-if="isCurrentSong(song) && !store.isPlaying"
            class="text-[var(--accent-color)]"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="currentColor"
              stroke="none"
            >
              <rect x="6" y="4" width="4" height="16"></rect>
              <rect x="14" y="4" width="4" height="16"></rect>
            </svg>
          </span>
          <span v-else>{{ index + 1 }}</span>
        </div>

        <!-- Title & Cover -->
        <div class="flex items-center gap-3 2xl:gap-5 overflow-hidden">
          <CoverImage
            :path="song.path"
            className="h-10 w-10 2xl:h-14 2xl:w-14 rounded-[4px] shadow-sm shrink-0 bg-[#333]"
          />
          <div class="truncate">
            <div
              class="text-[13px] 2xl:text-[16px] font-medium text-white truncate leading-tight"
              :class="{ 'text-[var(--accent-color)]': isCurrentSong(song) }"
            >
              {{ song.title }}
            </div>
          </div>
        </div>

        <div class="text-[13px] 2xl:text-[15px] text-[var(--text-secondary)] truncate">
          {{ song.artist }}
        </div>
        <div class="text-[13px] 2xl:text-[15px] text-[var(--text-secondary)] truncate">
          {{ song.album }}
        </div>

        <!-- Actions + time -->
        <div class="flex items-center justify-end gap-2 2xl:gap-3">
          <button
            @click.stop="store.toggleFavorite(song.path)"
            class="transition hover:scale-110"
            :class="
              store.isFavorite(song.path)
                ? 'text-[var(--accent-color)] opacity-100'
                : 'text-gray-400 opacity-0 group-hover:opacity-100 hover:text-white'
            "
            :title="store.isFavorite(song.path) ? 'Remove from Liked Songs' : 'Add to Liked Songs'"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="15"
              height="15"
              viewBox="0 0 24 24"
              :fill="store.isFavorite(song.path) ? 'currentColor' : 'none'"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path
                d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"
              ></path>
            </svg>
          </button>

          <span
            class="text-[12px] 2xl:text-[14px] text-[var(--text-secondary)] font-variant-numeric tabular-nums"
            >{{ formatDuration(song.duration_secs) }}</span
          >

          <button
            @click.stop="openMenu(song, $event)"
            class="text-gray-400 opacity-0 group-hover:opacity-100 hover:text-white transition"
            title="More"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="currentColor"
              stroke="none"
            >
              <circle cx="5" cy="12" r="1.6"></circle>
              <circle cx="12" cy="12" r="1.6"></circle>
              <circle cx="19" cy="12" r="1.6"></circle>
            </svg>
          </button>
        </div>
      </div>

      <div v-if="songs.length === 0" class="p-20 text-center text-gray-600">
        <div class="text-4xl mb-4 opacity-20">♫</div>
        <p>No songs found.</p>
      </div>
    </div>

    <!-- Context menu -->
    <Teleport to="body">
      <div
        v-if="menu.open"
        class="fixed z-[100] w-56 bg-[#282828] border border-[#3a3a3a] rounded-md shadow-2xl py-1 text-sm text-white select-none"
        :style="{ left: menu.x + 'px', top: menu.y + 'px' }"
        @click.stop
        @contextmenu.prevent
      >
        <button
          @click="playNext"
          class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
        >
          Play next
        </button>
        <button
          @click="addToQueue"
          class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
        >
          Add to queue
        </button>

        <div class="border-t border-[#3a3a3a] my-1"></div>

        <button
          @click="toggleLike"
          class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
        >
          {{
            menu.song && store.isFavorite(menu.song.path)
              ? 'Remove from Liked Songs'
              : 'Add to Liked Songs'
          }}
        </button>

        <div class="border-t border-[#3a3a3a] my-1"></div>

        <div class="px-4 py-1 text-[11px] uppercase tracking-wide text-gray-500">
          Add to playlist
        </div>
        <div class="max-h-40 overflow-auto">
          <button
            v-for="pl in store.playlists"
            :key="pl.id"
            @click="addToPlaylist(pl.id)"
            class="w-full text-left px-4 py-1.5 hover:bg-[#3a3a3a] transition-colors truncate"
          >
            {{ pl.name }}
          </button>
        </div>
        <button
          @click="newPlaylistWithSong"
          class="w-full text-left px-4 py-2 text-[var(--accent-color)] hover:bg-[#3a3a3a] transition-colors"
        >
          + New playlist
        </button>

        <template v-if="playlistId">
          <div class="border-t border-[#3a3a3a] my-1"></div>
          <button
            @click="removeFromThisPlaylist"
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
          >
            Remove from this playlist
          </button>
        </template>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
/* Skip rendering rows that are scrolled off-screen. Cheap, browser-native
   culling that keeps large libraries (thousands of tracks) smooth without a
   full virtual-scroll rewrite. The intrinsic size hint preserves scrollbar
   geometry for unrendered rows. */
.song-row {
  content-visibility: auto;
  contain-intrinsic-size: auto 56px;
}
</style>
