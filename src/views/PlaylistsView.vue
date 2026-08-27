<script setup>
import { computed, ref, onMounted, onUnmounted } from 'vue';
import { store } from '../store';
import { useRouter } from 'vue-router';
import PlaylistCover from '../components/PlaylistCover.vue';
import LibraryGridSkeleton from '../components/LibraryGridSkeleton.vue';
import { navigateWithTransition } from '../viewTransition';

defineOptions({ name: 'PlaylistsView' });

const router = useRouter();

const searchQuery = ref('');
const playlists = computed(() => store.playlists);

const filteredPlaylists = computed(() => {
  let list = playlists.value || [];
  const q = searchQuery.value.trim().toLowerCase();
  if (q) {
    list = list.filter((pl) => pl.name && pl.name.toLowerCase().includes(q));
  }
  return list;
});

const loading = computed(() => !store.libraryReady);
// db_playlists returns a live track_count for every row (item count for normal
// playlists, evaluated-rule count for smart ones).
const cardCount = (pl) => pl.track_count || 0;

function newSmartPlaylist() {
  store.openSmartModal('create');
}

function playCard(pl) {
  if (store.isSmart(pl)) store.playSmartPlaylist(pl.id);
  else playPlaylist(pl.id);
}

function openPlaylist(pl, event) {
  // Don't navigate if we just finished dragging
  if (dragDidReorder) {
    dragDidReorder = false;
    return;
  }
  const coverEl = event.currentTarget.querySelector('.cover-image');
  const to = store.isSmart(pl)
    ? '/smart/' + pl.id
    : { name: 'PlaylistDetail', params: { id: pl.id } };
  navigateWithTransition(() => router.push(to), coverEl, 'shared-cover', 'to-album-transition');
}

function playPlaylist(id) {
  store.playPlaylist(id);
}

function newPlaylist() {
  store.openPlaylistModal();
}

// Context Menu state & handlers
const menuState = ref({
  open: false,
  x: 0,
  y: 0,
  playlist: null,
});

const openContextMenu = (pl, e) => {
  e.preventDefault();
  e.stopPropagation();
  menuState.value = {
    open: true,
    x: Math.min(e.clientX, window.innerWidth - 200),
    y: Math.min(e.clientY, window.innerHeight - 170),
    playlist: pl,
  };
};

const closeContextMenu = () => {
  menuState.value.open = false;
};

const deletePlaylistConfirm = (pl) => {
  closeContextMenu();
  if (!pl) return;
  const isSmart = store.isSmart(pl);
  store.showConfirm({
    title: isSmart ? 'Delete Smart Playlist' : 'Delete Playlist',
    message: `Are you sure you want to delete "${pl.name}"? This action cannot be undone.`,
    confirmText: 'Delete',
    cancelText: 'Cancel',
    onConfirm: () => {
      if (isSmart) {
        return store.deleteSmartPlaylist(pl.id);
      } else {
        return store.deletePlaylist(pl.id);
      }
    },
  });
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

onMounted(() => {
  window.addEventListener('click', closeContextMenu);
});

// ---- Drag-to-reorder playlists in the grid ----
const dragIndex = ref(-1);
const overIndex = ref(-1);
const dragActive = ref(false);
const gridContainer = ref(null);
let startX = 0;
let startY = 0;
let pendingIdx = -1;
let dragDidReorder = false;
const DRAG_THRESHOLD = 8;

const getCardIndex = (clientX, clientY) => {
  const el = gridContainer.value?.$el || gridContainer.value;
  if (!el) return -1;
  const cards = el.querySelectorAll('[data-pl-grid-idx]');
  for (const card of cards) {
    const rect = card.getBoundingClientRect();
    if (
      clientX >= rect.left &&
      clientX <= rect.right &&
      clientY >= rect.top &&
      clientY <= rect.bottom
    ) {
      return parseInt(card.dataset.plGridIdx, 10);
    }
  }
  return -1;
};

const onMouseMove = (e) => {
  if (pendingIdx === -1) return;
  const dx = Math.abs(e.clientX - startX);
  const dy = Math.abs(e.clientY - startY);
  if (!dragActive.value && (dx >= DRAG_THRESHOLD || dy >= DRAG_THRESHOLD)) {
    dragActive.value = true;
    dragIndex.value = pendingIdx;
    overIndex.value = pendingIdx;
    document.body.style.userSelect = 'none';
    document.body.style.cursor = 'grabbing';
  }
  if (dragActive.value) {
    e.preventDefault();
    const idx = getCardIndex(e.clientX, e.clientY);
    if (idx !== -1) overIndex.value = idx;
  }
};

const onMouseUp = () => {
  if (
    dragActive.value &&
    dragIndex.value !== -1 &&
    overIndex.value !== -1 &&
    dragIndex.value !== overIndex.value
  ) {
    store.runMutation(() => store.movePlaylistOrder(dragIndex.value, overIndex.value));
    dragDidReorder = true;
  }
  dragIndex.value = -1;
  overIndex.value = -1;
  dragActive.value = false;
  pendingIdx = -1;
  document.removeEventListener('mousemove', onMouseMove);
  document.removeEventListener('mouseup', onMouseUp);
  document.body.style.userSelect = '';
  document.body.style.cursor = '';
  setTimeout(() => {
    dragDidReorder = false;
  }, 50);
};

const onCardMouseDown = (index, e) => {
  // Don't interfere with play button clicks
  if (e.target.closest('[data-play-btn]')) return;
  if (e.target.closest('button')) return;
  pendingIdx = index;
  startX = e.clientX;
  startY = e.clientY;
  dragDidReorder = false;
  document.addEventListener('mousemove', onMouseMove);
  document.addEventListener('mouseup', onMouseUp);
};

onUnmounted(() => {
  window.removeEventListener('click', closeContextMenu);
  document.removeEventListener('mousemove', onMouseMove);
  document.removeEventListener('mouseup', onMouseUp);
  document.body.style.userSelect = '';
  document.body.style.cursor = '';
});
</script>

<template>
  <div class="h-full overflow-auto px-8 pt-8 pb-12">
    <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-4 mb-6">
      <h1 class="text-3xl font-bold tracking-tight text-white">{{ $t('nav.playlists') }}</h1>
      <div class="flex items-center gap-2.5 flex-wrap">
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
            :aria-label="$t('views.playlists.searchPlaceholder')"
            :placeholder="$t('views.playlists.searchPlaceholder')"
            class="w-full h-[32px] bg-[#2a2a2a] text-xs text-white rounded-md pl-9 pr-8 focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] placeholder-gray-500"
          />
          <button
            v-if="searchQuery"
            type="button"
            @click="searchQuery = ''"
            :aria-label="$t('common.clear')"
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
              aria-hidden="true"
            >
              <line x1="18" y1="6" x2="6" y2="18"></line>
              <line x1="6" y1="6" x2="18" y2="18"></line>
            </svg>
          </button>
        </div>

        <button
          type="button"
          @click="newSmartPlaylist"
          class="h-[32px] bg-[#2c2c2e] text-white px-4 rounded-md text-xs font-semibold hover:bg-[#3a3a3c] transition inline-flex items-center gap-1.5 shadow-lg shrink-0"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="currentColor"
            stroke="none"
            class="text-[var(--accent-color)]"
            aria-hidden="true"
          >
            <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"></polygon>
          </svg>
          {{ $t('nav.newSmartPlaylist') }}
        </button>
        <button
          type="button"
          @click="newPlaylist"
          class="h-[32px] bg-[var(--accent-color)] text-white px-4 rounded-md text-xs font-semibold hover:bg-red-500 transition inline-flex items-center gap-1.5 shadow-lg shrink-0"
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
            aria-hidden="true"
          >
            <line x1="12" y1="5" x2="12" y2="19"></line>
            <line x1="5" y1="12" x2="19" y2="12"></line>
          </svg>
          {{ $t('nav.newPlaylist') }}
        </button>
      </div>
    </div>

    <LibraryGridSkeleton v-if="loading" label="Loading playlists" />

    <TransitionGroup
      v-else-if="filteredPlaylists.length > 0"
      ref="gridContainer"
      name="plgrid"
      tag="div"
      @before-leave="onBeforeLeave"
      class="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6 gap-x-6 gap-y-10 relative"
    >
      <div
        v-for="(pl, plIdx) in filteredPlaylists"
        :key="pl.id"
        :data-cover-key="pl.id"
        :data-pl-grid-idx="plIdx"
        role="button"
        tabindex="0"
        :aria-label="pl.name"
        @click="openPlaylist(pl, $event)"
        @keydown.enter="openPlaylist(pl, $event)"
        @keydown.space.prevent="openPlaylist(pl, $event)"
        @contextmenu="openContextMenu(pl, $event)"
        @mousedown="onCardMouseDown(plIdx, $event)"
        class="cursor-pointer group transition-all duration-200 focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent-color)] rounded-md"
        :class="{
          'opacity-30 scale-95': plIdx === dragIndex,
          'plgrid-drop-target': overIndex === plIdx && dragIndex !== plIdx && dragIndex !== -1,
        }"
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
            class="absolute inset-0 bg-black/20 opacity-0 group-hover:opacity-100 transition-opacity rounded-md flex items-end p-3 z-10"
          >
            <button
              v-if="cardCount(pl) > 0"
              type="button"
              data-play-btn
              @click.stop="playCard(pl)"
              aria-label="Play playlist"
              class="bg-[var(--accent-color)] text-white rounded-full p-3 shadow-lg translate-y-2 opacity-0 group-hover:translate-y-0 group-hover:opacity-100 transition-all duration-300 hover:scale-110 hover:bg-red-500"
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="currentColor"
                stroke="none"
                aria-hidden="true"
              >
                <polygon points="5 3 19 12 5 21 5 3"></polygon>
              </svg>
            </button>
          </div>
        </div>

        <h3
          class="text-[13px] font-medium text-white truncate pr-2 leading-snug flex items-center gap-1.5"
        >
          <span class="truncate">{{ pl.name }}</span>
          <svg
            v-if="store.isSmart(pl)"
            xmlns="http://www.w3.org/2000/svg"
            width="11"
            height="11"
            viewBox="0 0 24 24"
            fill="currentColor"
            stroke="none"
            class="text-[var(--accent-color)] shrink-0"
          >
            <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"></polygon>
          </svg>
        </h3>
        <p class="text-[13px] text-[var(--text-secondary)] truncate">{{ $t('views.playlists.songsCount', { count: cardCount(pl) }) }}</p>
      </div>
    </TransitionGroup>

    <div
      v-else-if="playlists.length > 0 && filteredPlaylists.length === 0"
      class="p-20 text-center text-gray-600"
    >
      <div class="text-4xl mb-4 opacity-20">🔍</div>
      <p>{{ $t('views.playlists.emptySearch', { query: searchQuery }) }}</p>
    </div>

    <div v-else class="p-20 text-center text-gray-600">
      <div class="text-4xl mb-4 opacity-20">♪</div>
      <p>{{ $t('views.playlists.empty') }}</p>
      <p class="text-xs mt-2">{{ $t('views.playlists.emptySubtext') }}</p>
    </div>

    <!-- Right-click Context Menu -->
    <div
      v-if="menuState.open"
      class="fixed z-[250] w-48 rounded-lg bg-[#282828] border border-[#3a3a3a] py-1.5 shadow-2xl text-xs text-white"
      :style="{ top: `${menuState.y}px`, left: `${menuState.x}px` }"
      @click.stop
    >
      <button
        @click="
          openPlaylist(menuState.playlist, $event);
          closeContextMenu();
        "
        class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
      >
        {{ $t('views.playlists.open') }}
      </button>
      <button
        @click="
          playCard(menuState.playlist);
          closeContextMenu();
        "
        class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
      >
        {{ $t('views.playlists.play') }}
      </button>
      <button
        @click="
          store.isSmart(menuState.playlist)
            ? store.openSmartModal('edit', menuState.playlist.id)
            : store.openPlaylistModal(null, 'edit', menuState.playlist.id);
          closeContextMenu();
        "
        class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
      >
        {{ $t('views.playlists.edit') }}
      </button>
      <div class="border-t border-[#3a3a3a] my-1"></div>
      <button
        @click="deletePlaylistConfirm(menuState.playlist)"
        class="w-full text-left px-4 py-2 text-red-500 hover:bg-[#3a3a3a] transition-colors font-medium"
      >
        {{ $t('views.playlists.delete') }}
      </button>
    </div>
  </div>
</template>

<style scoped>
/* Drop target highlight */
.plgrid-drop-target {
  outline: 2px solid var(--accent-color);
  outline-offset: 4px;
  border-radius: 8px;
}

/* FLIP reorder & enter/leave animations for grid cards */
.plgrid-move {
  transition: transform 0.4s cubic-bezier(0.16, 1, 0.3, 1);
}

.plgrid-enter-active {
  transition: all 0.4s cubic-bezier(0.16, 1, 0.3, 1);
}

.plgrid-leave-active {
  transition:
    opacity 0.38s cubic-bezier(0.16, 1, 0.3, 1),
    transform 0.38s cubic-bezier(0.16, 1, 0.3, 1) !important;
  position: absolute !important;
  z-index: 0;
  pointer-events: none;
}

.plgrid-enter-from {
  opacity: 0;
  transform: scale(0.9) translateY(12px);
}

.plgrid-leave-to {
  opacity: 0;
  transform: scale(0.85);
}
</style>
