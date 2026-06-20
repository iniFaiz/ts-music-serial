<script setup>
import { computed, ref, onMounted, onUnmounted } from 'vue';
import { useRouter } from 'vue-router';
import { invoke } from '@tauri-apps/api/core';
import { store } from '../store';
import CoverImage from './CoverImage.vue';
import { navigateWithTransition } from '../viewTransition';

const props = defineProps({
  songs: { type: Array, required: true },
  // When this list belongs to a playlist, rows offer "Remove from playlist".
  playlistId: { type: String, default: '' },
  isFavorites: { type: Boolean, default: false },
});

const router = useRouter();

const sortKey = ref(null);
const sortOrder = ref('asc');

// Whether drag-to-reorder is available (only for playlists/favorites with no active sort)
const canReorder = computed(() => (!!props.playlistId || props.isFavorites) && !sortKey.value);

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

// ---- Pointer-event based drag-to-reorder (playlist only) ----
// Uses a movement threshold so clicks (play) vs drags (reorder) are distinct.
const plDragIndex = ref(-1);
const plOverIndex = ref(-1);
const plDragActive = ref(false); // true once threshold exceeded
const songListContainer = ref(null);
let plStartY = 0;
let plPendingIndex = -1;
const PL_DRAG_THRESHOLD = 5; // px of vertical movement to start drag
let plDragDidReorder = false;

const getRowIndexFromY = (clientY) => {
  if (!songListContainer.value) return -1;
  const rows = songListContainer.value.querySelectorAll('[data-pl-drag-idx]');
  for (const row of rows) {
    const rect = row.getBoundingClientRect();
    if (clientY >= rect.top && clientY <= rect.bottom) {
      return parseInt(row.dataset.plDragIdx, 10);
    }
  }
  if (rows.length > 0) {
    const firstRect = rows[0].getBoundingClientRect();
    if (clientY < firstRect.top) return 0;
    const lastRect = rows[rows.length - 1].getBoundingClientRect();
    if (clientY > lastRect.bottom) return rows.length - 1;
  }
  return -1;
};

const onPlMouseMove = (e) => {
  if (plPendingIndex === -1) return;
  const dy = Math.abs(e.clientY - plStartY);
  // Activate drag once threshold is exceeded
  if (!plDragActive.value && dy >= PL_DRAG_THRESHOLD) {
    plDragActive.value = true;
    plDragIndex.value = plPendingIndex;
    plOverIndex.value = plPendingIndex;
    document.body.style.userSelect = 'none';
    document.body.style.cursor = 'grabbing';
  }
  if (plDragActive.value) {
    e.preventDefault();
    const idx = getRowIndexFromY(e.clientY);
    if (idx !== -1) plOverIndex.value = idx;
  }
};

const onPlMouseUp = () => {
  if (plDragActive.value && plDragIndex.value !== -1 && plOverIndex.value !== -1 && plDragIndex.value !== plOverIndex.value) {
    if (props.isFavorites) {
      store.moveInFavorites(plDragIndex.value, plOverIndex.value);
    } else {
      store.moveInPlaylist(props.playlistId, plDragIndex.value, plOverIndex.value);
    }
    plDragDidReorder = true;
  }
  plDragIndex.value = -1;
  plOverIndex.value = -1;
  plDragActive.value = false;
  plPendingIndex = -1;
  document.removeEventListener('mousemove', onPlMouseMove);
  document.removeEventListener('mouseup', onPlMouseUp);
  document.body.style.userSelect = '';
  document.body.style.cursor = '';
};

const onPlRowMouseDown = (index, e) => {
  // Don't initiate drag from buttons or interactive elements
  if (e.target.closest('button') || e.target.closest('input')) return;
  plPendingIndex = index;
  plStartY = e.clientY;
  plDragDidReorder = false;
  document.addEventListener('mousemove', onPlMouseMove);
  document.addEventListener('mouseup', onPlMouseUp);
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

// ---- Clickable Links & Routing Navigation ----

const findCoverBySongPath = (path) => {
  if (!path) return null;
  const rows = document.querySelectorAll('.song-row');
  for (const row of rows) {
    if (row.dataset.songPath === path) {
      return row.querySelector('.cover-image');
    }
  }
  return null;
};

const navigateToArtist = (artistName, event = null) => {
  if (!artistName || artistName === 'Unknown Artist') return;
  const navigate = () => router.push({ name: 'ArtistDetail', params: { name: artistName } });

  const rowEl = event ? event.currentTarget.closest('.song-row') : null;
  const coverEl = rowEl ? rowEl.querySelector('.cover-image') : null;

  if (coverEl) {
    navigateWithTransition(navigate, coverEl, 'shared-cover', 'to-artist-transition');
  } else {
    navigate();
  }
};

const navigateToAlbum = (albumName, event = null) => {
  if (!albumName || albumName === 'Unknown Album') return;
  const navigate = () => router.push({ name: 'AlbumDetail', params: { name: albumName } });

  const rowEl = event ? event.currentTarget.closest('.song-row') : null;
  const coverEl = rowEl ? rowEl.querySelector('.cover-image') : null;

  if (coverEl) {
    navigateWithTransition(navigate, coverEl, 'shared-cover', 'to-album-transition');
  } else {
    navigate();
  }
};

// ---- Hover Tooltips ----

const tooltip = ref({ show: false, text: '', x: 0, y: 0 });

const showTooltip = (event, text) => {
  const target = event.currentTarget;
  if (target.scrollWidth > target.clientWidth) {
    tooltip.value = {
      show: true,
      text,
      x: event.clientX + 10,
      y: event.clientY + 15,
    };
  }
};

const moveTooltip = (event) => {
  if (tooltip.value.show) {
    tooltip.value.x = event.clientX + 10;
    tooltip.value.y = event.clientY + 15;
  }
};

const hideTooltip = () => {
  tooltip.value.show = false;
};

// ---- File Information Modal ----

const infoModalOpen = ref(false);
const infoSong = ref(null);
const copyStatus = ref('Copy Path');

const showFileInfo = () => {
  infoSong.value = menu.value.song;
  infoModalOpen.value = true;
  closeMenu();
};

const closeInfoModal = () => {
  infoModalOpen.value = false;
  infoSong.value = null;
  copyStatus.value = 'Copy Path';
};

const copyToClipboard = async (text) => {
  try {
    await navigator.clipboard.writeText(text);
    copyStatus.value = 'Copied!';
    setTimeout(() => {
      copyStatus.value = 'Copy Path';
    }, 2000);
  } catch (err) {
    console.error('Failed to copy text:', err);
  }
};

// ---- Multi-select Selection Mode ----

const selectMode = ref(false);
const selectedSongs = ref([]);
const showPlDropdown = ref(false);

const toggleSelectSong = (song) => {
  const idx = selectedSongs.value.indexOf(song.path);
  if (idx >= 0) {
    selectedSongs.value.splice(idx, 1);
  } else {
    selectedSongs.value.push(song.path);
  }
};

const toggleSelectAll = (event) => {
  if (event.target.checked) {
    selectedSongs.value = sortedSongs.value.map((s) => s.path);
  } else {
    selectedSongs.value = [];
  }
};

const startSelectMode = () => {
  selectMode.value = true;
  selectedSongs.value = [menu.value.song.path];
  closeMenu();
};

const cancelSelection = () => {
  selectMode.value = false;
  selectedSongs.value = [];
  showPlDropdown.value = false;
};

const playSelected = () => {
  const tracks = props.songs.filter((s) => selectedSongs.value.includes(s.path));
  if (tracks.length > 0) {
    store.playSong(tracks[0], tracks);
  }
  cancelSelection();
};

const addSelectedToQueue = () => {
  const tracks = props.songs.filter((s) => selectedSongs.value.includes(s.path));
  if (tracks.length > 0) {
    store.addToQueue(tracks);
  }
  cancelSelection();
};

const addSelectedToPlaylist = (playlistId) => {
  store.addToPlaylist(playlistId, selectedSongs.value);
  cancelSelection();
};

const newPlaylistWithSelected = () => {
  store.openPlaylistModal(selectedSongs.value);
  cancelSelection();
};

const removeSelectedFromPlaylist = () => {
  if (props.playlistId) {
    selectedSongs.value.forEach((path) => {
      store.removeFromPlaylist(props.playlistId, path);
    });
  }
  cancelSelection();
};

// ---- Custom Warning Deletion Modal ----

const deleteConfirmModalOpen = ref(false);
const deleteActionType = ref('single'); // 'single' or 'bulk'
const deleteConfirmSong = ref(null); // for single delete target

const triggerDelete = () => {
  deleteConfirmSong.value = menu.value.song;
  deleteActionType.value = 'single';
  deleteConfirmModalOpen.value = true;
  closeMenu();
};

const triggerBulkDelete = () => {
  deleteActionType.value = 'bulk';
  deleteConfirmModalOpen.value = true;
  showPlDropdown.value = false;
};

const closeDeleteConfirm = () => {
  deleteConfirmModalOpen.value = false;
  deleteConfirmSong.value = null;
};

const executeDelete = async (fromDisk) => {
  const paths =
    deleteActionType.value === 'bulk' ? [...selectedSongs.value] : [deleteConfirmSong.value.path];

  for (const path of paths) {
    try {
      if (fromDisk) {
        await store.deleteSong(path);
      } else {
        await store.removeSongFromLibrary(path);
      }
    } catch (err) {
      console.error(`Error deleting/removing track (${path}):`, err);
    }
  }

  deleteConfirmModalOpen.value = false;
  deleteConfirmSong.value = null;
  if (deleteActionType.value === 'bulk') {
    cancelSelection();
  }
};

// ---- Row context menu ----

const menu = ref({ open: false, x: 0, y: 0, maxHeight: 400, song: null });

const openMenu = (song, event) => {
  event.preventDefault();
  const winWidth = window.innerWidth;
  const winHeight = window.innerHeight;
  const menuWidth = 224;
  const menuHeight = props.playlistId ? 450 : 400;

  let x = event.clientX;
  let y = event.clientY;

  if (x + menuWidth > winWidth) {
    x = winWidth - menuWidth - 10;
  }
  x = Math.max(10, x);

  const spaceBelow = winHeight - y;
  const spaceAbove = y;

  let maxHeight = 400;
  let topPosition = y;

  if (spaceBelow >= spaceAbove) {
    topPosition = y;
    maxHeight = spaceBelow - 20;
  } else {
    if (spaceAbove >= menuHeight) {
      topPosition = y - menuHeight;
      maxHeight = menuHeight;
    } else {
      topPosition = 10;
      maxHeight = y - 20;
    }
  }

  maxHeight = Math.max(150, maxHeight);
  topPosition = Math.max(10, topPosition);

  menu.value = { open: true, x, y: topPosition, maxHeight, song };
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

const showArtist = () => {
  const song = menu.value.song;
  if (!song) return;
  const navigate = () => router.push({ name: 'ArtistDetail', params: { name: song.artist } });
  const coverEl = findCoverBySongPath(song.path);
  closeMenu();
  if (coverEl) {
    navigateWithTransition(navigate, coverEl, 'shared-cover', 'to-artist-transition');
  } else {
    navigate();
  }
};

const showAlbum = () => {
  const song = menu.value.song;
  if (!song) return;
  const navigate = () => router.push({ name: 'AlbumDetail', params: { name: song.album } });
  const coverEl = findCoverBySongPath(song.path);
  closeMenu();
  if (coverEl) {
    navigateWithTransition(navigate, coverEl, 'shared-cover', 'to-album-transition');
  } else {
    navigate();
  }
};

const showInFolder = async () => {
  const song = menu.value.song;
  if (!song) return;
  closeMenu();
  try {
    await invoke('player_show_in_folder', { path: song.path });
  } catch (err) {
    console.error('Failed to show in folder:', err);
  }
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

const isHoveringMenu = ref(false);

const closeMenuOnScroll = () => {
  if (isHoveringMenu.value) {
    return;
  }
  closeMenu();
};

onMounted(() => {
  window.addEventListener('click', closeMenu);
  window.addEventListener('scroll', closeMenuOnScroll, true);
  window.addEventListener('resize', closeMenu);
});

onUnmounted(() => {
  window.removeEventListener('click', closeMenu);
  window.removeEventListener('scroll', closeMenuOnScroll, true);
  window.removeEventListener('resize', closeMenu);
  hideTooltip();
  // Cleanup playlist drag listeners
  document.removeEventListener('mousemove', onPlMouseMove);
  document.removeEventListener('mouseup', onPlMouseUp);
  document.body.style.userSelect = '';
  document.body.style.cursor = '';
});
</script>

<template>
  <div ref="songListContainer" class="w-full text-left text-sm 2xl:text-sm px-6 pb-12">
    <!-- Header -->
    <div
      class="grid gap-4 text-[var(--text-secondary)] text-xs font-medium uppercase tracking-wide border-b border-[var(--border-color)] py-2 mb-2 sticky top-0 bg-[var(--app-bg)]/95 backdrop-blur-xl z-10 select-none grid-cols-[20px_3fr_2fr_2fr_120px] 2xl:grid-cols-[30px_4fr_3fr_3fr_120px]"
    >
      <div class="text-center flex justify-center items-center h-full">
        <input
          v-if="selectMode"
          type="checkbox"
          :checked="selectedSongs.length === sortedSongs.length && sortedSongs.length > 0"
          @change="toggleSelectAll"
          class="accent-[var(--accent-color)] h-3.5 w-3.5 rounded cursor-pointer"
        />
        <span v-else>#</span>
      </div>
      <div
        @click="toggleSort('title')"
        class="cursor-pointer hover:text-white flex items-center gap-1"
      >
        Title <span class="text-[8px]">{{ getSortIcon('title') }}</span>
      </div>
      <div
        @click="toggleSort('artist')"
        class="cursor-pointer hover:text-white flex items-center gap-1"
      >
        Artist <span class="text-[8px]">{{ getSortIcon('artist') }}</span>
      </div>
      <div
        @click="toggleSort('album')"
        class="cursor-pointer hover:text-white flex items-center gap-1"
      >
        Album <span class="text-[8px]">{{ getSortIcon('album') }}</span>
      </div>
      <div
        @click="toggleSort('duration_secs')"
        class="cursor-pointer hover:text-white flex items-center justify-end gap-1 text-right"
      >
        Time <span class="text-[8px]">{{ getSortIcon('duration_secs') }}</span>
      </div>
    </div>

    <!-- Rows -->
    <TransitionGroup name="song-list" tag="div" class="space-y-0.5">
      <div
        v-for="(song, index) in sortedSongs"
        :key="song.path"
        :data-song-path="song.path"
        :data-pl-drag-idx="canReorder ? index : undefined"
        @click="plDragDidReorder ? null : (selectMode ? toggleSelectSong(song) : playSong(song))"
        @contextmenu.prevent="openMenu(song, $event)"
        @mousedown="canReorder ? onPlRowMouseDown(index, $event) : null"
        class="song-row grid gap-4 py-2 px-2 rounded-md hover:bg-[#2a2a2a] group items-center transition-colors cursor-pointer grid-cols-[20px_3fr_2fr_2fr_120px] 2xl:grid-cols-[30px_4fr_3fr_3fr_120px] 2xl:py-1.5"
        :class="{
          'bg-[#2a2a2a]': isCurrentSong(song) || (selectMode && selectedSongs.includes(song.path)),
          'opacity-30': canReorder && index === plDragIndex,
          'pl-drop-target-above': canReorder && plOverIndex === index && plDragIndex !== index && plDragIndex > index,
          'pl-drop-target-below': canReorder && plOverIndex === index && plDragIndex !== index && plDragIndex < index,
        }"
      >
        <div class="text-xs text-gray-500 text-center flex justify-center items-center h-full">
          <input
            v-if="selectMode"
            type="checkbox"
            :checked="selectedSongs.includes(song.path)"
            @click.stop="toggleSelectSong(song)"
            class="accent-[var(--accent-color)] h-3.5 w-3.5 rounded cursor-pointer"
          />
          <span
            v-else-if="isCurrentSong(song) && store.isPlaying"
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
        <div class="flex items-center gap-3 overflow-hidden">
          <CoverImage
            :path="song.path"
            className="h-10 w-10 2xl:h-9 2xl:w-9 rounded-[4px] shadow-sm shrink-0 bg-[#333]"
          />
          <div class="truncate">
            <div
              class="text-[13px] font-medium text-white truncate leading-tight"
              :class="{ 'text-[var(--accent-color)]': isCurrentSong(song) }"
              @mouseenter="showTooltip($event, song.title)"
              @mousemove="moveTooltip"
              @mouseleave="hideTooltip"
            >
              {{ song.title }}
            </div>
          </div>
        </div>

        <div
          class="text-[13px] 2xl:text-xs text-[var(--text-secondary)] truncate"
          @mouseenter="showTooltip($event, song.artist)"
          @mousemove="moveTooltip"
          @mouseleave="hideTooltip"
        >
          <span
            @click.stop="navigateToArtist(song.artist, $event)"
            class="hover:text-[var(--accent-color)] hover:underline cursor-pointer transition-colors"
          >
            {{ song.artist }}
          </span>
        </div>
        <div
          class="text-[13px] 2xl:text-xs text-[var(--text-secondary)] truncate"
          @mouseenter="showTooltip($event, song.album)"
          @mousemove="moveTooltip"
          @mouseleave="hideTooltip"
        >
          <span
            @click.stop="navigateToAlbum(song.album, $event)"
            class="hover:text-[var(--accent-color)] hover:underline cursor-pointer transition-colors"
          >
            {{ song.album }}
          </span>
        </div>

        <!-- Actions + time -->
        <div class="flex items-center justify-end gap-2">
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
            class="text-[12px] 2xl:text-xs text-[var(--text-secondary)] font-variant-numeric tabular-nums"
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
    </TransitionGroup>

    <div v-if="songs.length === 0" class="p-20 text-center text-gray-600">
      <div class="text-4xl mb-4 opacity-20">♫</div>
      <p>No songs found.</p>
    </div>

    <!-- Context menu -->
    <Teleport to="body">
      <div
        v-if="menu.open"
        class="fixed z-[100] w-56 bg-[#282828] border border-[#3a3a3a] rounded-md shadow-2xl py-1 text-sm text-white select-none overflow-y-auto scrollbar-thin context-menu-container"
        :style="{ left: menu.x + 'px', top: menu.y + 'px', maxHeight: menu.maxHeight + 'px' }"
        @click.stop
        @contextmenu.prevent
        @mouseenter="isHoveringMenu = true"
        @mouseleave="isHoveringMenu = false"
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
        <button
          @click="startSelectMode"
          class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
        >
          Select Songs
        </button>

        <div class="border-t border-[#3a3a3a] my-1"></div>

        <button
          @click="showArtist"
          class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
        >
          Show Artist
        </button>
        <button
          @click="showAlbum"
          class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
        >
          Show Album
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
        <div class="max-h-40 overflow-auto scrollbar-thin">
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
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors text-red-400"
          >
            Remove from playlist
          </button>
        </template>

        <div class="border-t border-[#3a3a3a] my-1"></div>
        <button
          @click="showFileInfo"
          class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
        >
          File Information
        </button>
        <button
          @click="showInFolder"
          class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
        >
          Show in File Manager
        </button>
        <button
          @click="triggerDelete"
          class="w-full text-left px-4 py-2 text-red-500 hover:bg-[#3a3a3a] transition-colors"
        >
          Delete / Remove
        </button>
      </div>
    </Teleport>

    <!-- Custom tooltip -->
    <Teleport to="body">
      <div
        v-if="tooltip.show"
        class="fixed z-[9999] pointer-events-none bg-[#181818]/95 border border-[#333] px-2.5 py-1.5 rounded shadow-xl text-xs text-white max-w-xs break-all backdrop-blur-md transition-opacity duration-150"
        :style="{ left: tooltip.x + 'px', top: tooltip.y + 'px' }"
      >
        {{ tooltip.text }}
      </div>
    </Teleport>

    <!-- File Information Modal -->
    <Teleport to="body">
      <div
        v-if="infoModalOpen"
        class="fixed inset-0 z-[200] flex items-center justify-center bg-black/75 backdrop-blur-md transition-opacity duration-300"
        @click="closeInfoModal"
      >
        <div
          class="relative w-[90%] max-w-2xl bg-[#1c1c1e] border border-[#2c2c2e] rounded-2xl shadow-2xl overflow-hidden p-6 md:p-8 flex flex-col md:flex-row gap-6 md:gap-8 animate-fade-in"
          @click.stop
        >
          <!-- Close button -->
          <button
            @click="closeInfoModal"
            class="absolute top-4 right-4 text-gray-400 hover:text-white transition-colors"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="20"
              height="20"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <line x1="18" y1="6" x2="6" y2="18"></line>
              <line x1="6" y1="6" x2="18" y2="18"></line>
            </svg>
          </button>

          <!-- Left column: Cover Art -->
          <div class="flex flex-col items-center gap-4 shrink-0">
            <CoverImage
              v-if="infoSong"
              :path="infoSong.path"
              className="w-48 h-48 md:w-56 md:h-56 rounded-xl shadow-2xl object-cover bg-[#282828]"
            />
            <div class="text-xs text-gray-500 font-mono tracking-wider uppercase">
              {{ infoSong ? infoSong.path.split('.').pop().toUpperCase() : '' }} Audio File
            </div>
          </div>

          <!-- Right column: Metadata Grid -->
          <div class="flex-1 flex flex-col justify-between overflow-hidden">
            <div>
              <h2 class="text-xl font-bold text-white mb-4 pr-6 truncate">File Information</h2>

              <div class="space-y-3.5 text-sm">
                <div class="grid grid-cols-[100px_1fr] gap-2 items-baseline">
                  <span class="text-gray-500">Title:</span>
                  <span class="text-white font-medium truncate">{{ infoSong?.title }}</span>
                </div>
                <div class="grid grid-cols-[100px_1fr] gap-2 items-baseline">
                  <span class="text-gray-500">Artist:</span>
                  <span class="text-white font-medium truncate">{{ infoSong?.artist }}</span>
                </div>
                <div class="grid grid-cols-[100px_1fr] gap-2 items-baseline">
                  <span class="text-gray-500">Album:</span>
                  <span class="text-white font-medium truncate">{{ infoSong?.album }}</span>
                </div>
                <div
                  class="grid grid-cols-[100px_1fr] gap-2 items-baseline"
                  v-if="infoSong?.track_number"
                >
                  <span class="text-gray-500">Track:</span>
                  <span class="text-white font-medium">{{ infoSong?.track_number }}</span>
                </div>
                <div class="grid grid-cols-[100px_1fr] gap-2 items-baseline" v-if="infoSong?.year">
                  <span class="text-gray-500">Year:</span>
                  <span class="text-white font-medium">{{ infoSong?.year }}</span>
                </div>
                <div class="grid grid-cols-[100px_1fr] gap-2 items-baseline">
                  <span class="text-gray-500">Duration:</span>
                  <span class="text-white font-medium">{{
                    infoSong ? formatDuration(infoSong.duration_secs) : ''
                  }}</span>
                </div>
                <div class="grid grid-cols-[100px_1fr] gap-2 items-baseline">
                  <span class="text-gray-500">Date Added:</span>
                  <span class="text-white font-medium">
                    {{ infoSong ? new Date(infoSong.date_added * 1000).toLocaleString() : '' }}
                  </span>
                </div>
              </div>
            </div>

            <!-- Path/Copy Section -->
            <div class="mt-6 pt-4 border-t border-[#2c2c2e]">
              <div class="text-[11px] text-gray-500 uppercase tracking-wider mb-2">File Path</div>
              <div
                class="bg-[#121214] border border-[#2c2c2e] p-2.5 rounded-lg flex items-center justify-between gap-3 text-xs"
              >
                <div class="font-mono text-gray-400 select-all truncate flex-1">
                  {{ infoSong?.path }}
                </div>
                <button
                  @click="copyToClipboard(infoSong?.path)"
                  class="shrink-0 bg-[#2c2c2e] hover:bg-[#3a3a3c] text-white px-3 py-1.5 rounded-md font-medium transition-colors"
                >
                  {{ copyStatus }}
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Custom Warning Deletion Modal -->
    <Teleport to="body">
      <div
        v-if="deleteConfirmModalOpen"
        class="fixed inset-0 z-[210] flex items-center justify-center bg-black/80 backdrop-blur-md"
        @click="closeDeleteConfirm"
      >
        <div
          class="relative w-[95%] max-w-md bg-[#1c1c1e] border border-[#2c2c2e] rounded-2xl shadow-2xl p-6 text-center animate-fade-in"
          @click.stop
        >
          <!-- Warning Icon -->
          <div
            class="mx-auto flex items-center justify-center h-12 w-12 rounded-full bg-red-950/50 border border-red-500 mb-4"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="24"
              height="24"
              viewBox="0 0 24 24"
              fill="none"
              stroke="red"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path
                d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z"
              ></path>
              <line x1="12" y1="9" x2="12" y2="13"></line>
              <line x1="12" y1="17" x2="12.01" y2="17"></line>
            </svg>
          </div>

          <h3 class="text-lg font-bold text-white mb-2">Delete or Remove Song</h3>

          <p class="text-sm text-gray-400 mb-6 px-2">
            <span v-if="deleteActionType === 'single'">
              You are about to modify <strong>"{{ deleteConfirmSong?.title }}"</strong>.
            </span>
            <span v-else>
              You are about to modify <strong>{{ selectedSongs.length }}</strong> selected songs.
            </span>
            Choose whether to permanently delete the files from your disk, or just remove them from
            this application library/playlists.
          </p>

          <div class="flex flex-col gap-2.5">
            <!-- Delete Files (Red) -->
            <button
              @click="executeDelete(true)"
              class="w-full bg-red-600 hover:bg-red-700 text-white font-semibold py-2.5 rounded-lg transition-colors shadow-lg"
            >
              Delete Files (Permanently)
            </button>

            <!-- Remove from List (White) -->
            <button
              @click="executeDelete(false)"
              class="w-full bg-white hover:bg-gray-100 text-black font-semibold py-2.5 rounded-lg transition-colors shadow-lg"
            >
              Remove from List
            </button>

            <!-- Cancel -->
            <button
              @click="closeDeleteConfirm"
              class="w-full bg-[#2c2c2e] hover:bg-[#3a3a3c] text-gray-400 hover:text-white font-medium py-2 rounded-lg transition-colors mt-1"
            >
              Cancel
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Floating Action Bar for Multi-select -->
    <Teleport to="body">
      <div
        v-if="selectMode && selectedSongs.length > 0"
        class="fixed bottom-6 left-1/2 -translate-x-1/2 z-[150] bg-[#1e1e1e]/95 border border-[#333] backdrop-blur-md px-6 py-3.5 rounded-full shadow-2xl flex items-center gap-4 text-white text-xs 2xl:text-sm animate-slide-up"
        @click.stop
      >
        <span class="font-semibold text-gray-300"> {{ selectedSongs.length }} selected </span>

        <div class="h-4 w-[1px] bg-gray-700"></div>

        <button
          @click="playSelected"
          class="bg-[var(--accent-color)] hover:bg-red-500 text-white px-3.5 py-1.5 rounded-full font-medium transition"
        >
          Play
        </button>

        <button
          @click="addSelectedToQueue"
          class="bg-[#2c2c2e] hover:bg-[#3a3a3c] text-white px-3.5 py-1.5 rounded-full font-medium transition"
        >
          Add to Queue
        </button>

        <!-- Add to Playlist dropdown trigger -->
        <div class="relative">
          <button
            @click="showPlDropdown = !showPlDropdown"
            class="bg-[#2c2c2e] hover:bg-[#3a3a3c] text-white px-3.5 py-1.5 rounded-full font-medium transition flex items-center gap-1"
          >
            Add to Playlist
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2.5"
            >
              <path d="m6 9 6 6 6-6" />
            </svg>
          </button>

          <!-- Dropdown Menu -->
          <div
            v-if="showPlDropdown"
            class="absolute bottom-12 left-0 z-[160] w-48 bg-[#282828] border border-[#3a3a3a] rounded-md shadow-2xl py-1 text-left text-sm max-h-40 overflow-auto scrollbar-thin"
          >
            <button
              v-for="pl in store.playlists"
              :key="pl.id"
              @click="addSelectedToPlaylist(pl.id)"
              class="w-full text-left px-4 py-2 hover:bg-[#3a3a3c] transition-colors truncate"
            >
              {{ pl.name }}
            </button>
            <div class="border-t border-[#3a3a3a] my-1" v-if="store.playlists.length > 0"></div>
            <button
              @click="newPlaylistWithSelected"
              class="w-full text-left px-4 py-2 text-[var(--accent-color)] hover:bg-[#3a3a3c] transition-colors"
            >
              + New Playlist
            </button>
          </div>
        </div>

        <!-- Remove from playlist (only if playlistId is provided) -->
        <button
          v-if="playlistId"
          @click="removeSelectedFromPlaylist"
          class="bg-red-950/40 hover:bg-red-950/80 text-red-400 px-3.5 py-1.5 rounded-full font-medium border border-red-900/50 transition"
        >
          Remove
        </button>

        <!-- Bulk Delete/Remove trigger -->
        <button
          @click="triggerBulkDelete"
          class="bg-red-600 hover:bg-red-700 text-white px-3.5 py-1.5 rounded-full font-medium transition"
        >
          Delete
        </button>

        <button
          @click="cancelSelection"
          class="text-gray-400 hover:text-white font-medium transition-colors px-2 ml-1"
        >
          Cancel
        </button>
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

/* Playlist drag-to-reorder: drop-target indicators */
.pl-drop-target-above {
  box-shadow: inset 0 2px 0 0 var(--accent-color);
}
.pl-drop-target-below {
  box-shadow: inset 0 -2px 0 0 var(--accent-color);
}

.animate-fade-in {
  animation: fadeIn 0.2s cubic-bezier(0.16, 1, 0.3, 1) forwards;
}

.animate-slide-up {
  animation: slideUp 0.3s cubic-bezier(0.16, 1, 0.3, 1) forwards;
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

@keyframes slideUp {
  from {
    opacity: 0;
    transform: translate(-50%, 20px);
  }
  to {
    opacity: 1;
    transform: translate(-50%, 0);
  }
}

/* Playlist song list reorder FLIP animation */
.song-list-move {
  transition: transform 0.3s cubic-bezier(0.22, 0.61, 0.36, 1);
}
</style>
