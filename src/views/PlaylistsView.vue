<script setup>
import { computed, ref, onUnmounted } from 'vue';
import { store } from '../store';
import { useRouter } from 'vue-router';
import PlaylistCover from '../components/PlaylistCover.vue';
import { navigateWithTransition } from '../viewTransition';

defineOptions({ name: 'PlaylistsView' });

const router = useRouter();

const playlists = computed(() => store.playlists);
const smartCount = (pl) => store.smartSongs(pl.id).length;
const cardCount = (pl) => (store.isSmart(pl) ? smartCount(pl) : pl.paths.length);

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
  const to = store.isSmart(pl) ? '/smart/' + pl.id : { name: 'PlaylistDetail', params: { id: pl.id } };
  navigateWithTransition(() => router.push(to), coverEl, 'shared-cover', 'to-album-transition');
}

function playPlaylist(id) {
  store.playPlaylist(id);
}

function newPlaylist() {
  store.openPlaylistModal();
}

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
    if (clientX >= rect.left && clientX <= rect.right && clientY >= rect.top && clientY <= rect.bottom) {
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
  if (dragActive.value && dragIndex.value !== -1 && overIndex.value !== -1 && dragIndex.value !== overIndex.value) {
    store.movePlaylistOrder(dragIndex.value, overIndex.value);
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
  document.removeEventListener('mousemove', onMouseMove);
  document.removeEventListener('mouseup', onMouseUp);
  document.body.style.userSelect = '';
  document.body.style.cursor = '';
});
</script>

<template>
  <div class="h-full overflow-auto px-8 pt-8 pb-12">
    <div class="flex items-center justify-between mb-6">
      <h1 class="text-3xl font-bold tracking-tight text-white">Playlists</h1>
      <div class="flex items-center gap-2.5">
        <button
          @click="newSmartPlaylist"
          class="bg-[#2c2c2e] text-white px-5 py-1.5 rounded-[4px] text-xs font-semibold hover:bg-[#3a3a3c] transition flex items-center gap-1.5 shadow-lg"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="13"
            height="13"
            viewBox="0 0 24 24"
            fill="currentColor"
            stroke="none"
            class="text-[var(--accent-color)]"
          >
            <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"></polygon>
          </svg>
          New Smart Playlist
        </button>
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
    </div>

    <TransitionGroup
      v-if="playlists.length > 0"
      ref="gridContainer"
      name="plgrid"
      tag="div"
      class="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6 gap-x-6 gap-y-10"
    >
      <div
        v-for="(pl, plIdx) in playlists"
        :key="pl.id"
        :data-cover-key="pl.id"
        :data-pl-grid-idx="plIdx"
        @click="openPlaylist(pl, $event)"
        @mousedown="onCardMouseDown(plIdx, $event)"
        class="cursor-pointer group transition-all duration-200"
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
            class="absolute inset-0 bg-black/20 opacity-0 group-hover:opacity-100 transition-opacity rounded-md flex items-end p-3"
          >
            <div
              v-if="cardCount(pl) > 0"
              data-play-btn
              @click.stop="playCard(pl)"
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

        <h3 class="text-[13px] font-medium text-white truncate pr-2 leading-snug flex items-center gap-1.5">
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
        <p class="text-[13px] text-[var(--text-secondary)] truncate">{{ cardCount(pl) }} songs</p>
      </div>
    </TransitionGroup>

    <div v-if="playlists.length === 0" class="p-20 text-center text-gray-600">
      <div class="text-4xl mb-4 opacity-20">♪</div>
      <p>No playlists created yet.</p>
      <p class="text-xs mt-2">
        Click "New Playlist" or "New Smart Playlist" above to get started.
      </p>
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

/* FLIP reorder animation for grid cards */
.plgrid-move {
  transition: transform 0.35s cubic-bezier(0.22, 0.61, 0.36, 1);
}
</style>
