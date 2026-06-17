<script setup>
import { ref, onMounted, onUnmounted, nextTick } from 'vue';
import { useRouter } from 'vue-router';
import { store } from './store';
import PlayerControls from './components/PlayerControls.vue';
import QueuePanel from './components/QueuePanel.vue';
import PlaylistCreateModal from './components/PlaylistCreateModal.vue';
import PlaylistCover from './components/PlaylistCover.vue';
import TitleBar from './components/TitleBar.vue';
import { goBackWithTransition } from './viewTransition';

const router = useRouter();

const scrollContainer = ref(null);
const scrollPositions = new Map();

router.beforeEach((to, from) => {
  if (scrollContainer.value) {
    const container =
      scrollContainer.value.querySelector('.overflow-auto') || scrollContainer.value;
    scrollPositions.set(from.fullPath, container.scrollTop);
  }
});

router.afterEach((to) => {
  nextTick(() => {
    if (scrollContainer.value) {
      const container =
        scrollContainer.value.querySelector('.overflow-auto') || scrollContainer.value;
      const pos = scrollPositions.get(to.fullPath) || 0;
      const originalBehavior = container.style.scrollBehavior;
      container.style.scrollBehavior = 'auto';
      container.scrollTop = pos;
      container.style.scrollBehavior = originalBehavior;
    }
  });
});

// Collapse the sidebar to an icon-only rail when the window gets too narrow.
// Changed from 860 to 980 so the sidebar collapses before title bar text/logo collide.
const COMPACT_BREAKPOINT = 980;
const compact = ref(false);
const updateCompact = () => {
  compact.value = window.innerWidth < COMPACT_BREAKPOINT;
};

const handleMouseUp = (e) => {
  if (e.button === 3) {
    e.preventDefault();
    const canGoBack = !!(window.history.state && window.history.state.back);
    if (canGoBack) {
      goBackWithTransition(router);
    }
  }
};

const handleMouseDown = (e) => {
  if (e.button === 3) {
    e.preventDefault();
  }
};

const handleAuxClick = (e) => {
  if (e.button === 3) {
    e.preventDefault();
  }
};

onMounted(() => {
  updateCompact();
  window.addEventListener('resize', updateCompact);
  window.addEventListener('mouseup', handleMouseUp);
  window.addEventListener('mousedown', handleMouseDown);
  window.addEventListener('auxclick', handleAuxClick);
});

onUnmounted(() => {
  window.removeEventListener('resize', updateCompact);
  window.removeEventListener('mouseup', handleMouseUp);
  window.removeEventListener('mousedown', handleMouseDown);
  window.removeEventListener('auxclick', handleAuxClick);
});

function newPlaylist() {
  store.openPlaylistModal();
}
</script>

<template>
  <div
    class="flex flex-col h-screen bg-[var(--app-bg)] text-[var(--text-primary)] font-sans overflow-hidden select-none"
  >
    <!-- Custom window title bar (back, brand, window controls) -->
    <TitleBar />

    <div class="flex flex-1 overflow-hidden">
      <!-- Sidebar -->
      <nav
        class="bg-[var(--sidebar-bg)] border-r border-[var(--border-color)] flex flex-col shrink-0 pt-4 pb-4 gap-5 transition-[width] duration-200 ease-out"
        :class="compact ? 'w-16 px-2' : 'w-64 px-4'"
        @click="store.queuePanelOpen = false"
      >
        <!-- Search -->
        <div v-if="!compact" class="relative">
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
            v-model="store.searchQuery"
            type="text"
            placeholder="Search"
            class="w-full bg-[#282828] text-sm text-white rounded-lg py-1.5 pl-9 pr-3 focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] placeholder-gray-500"
          />
        </div>
        <div v-else class="flex justify-center py-1 text-gray-500" title="Search">
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2.5"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <circle cx="11" cy="11" r="8"></circle>
            <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
          </svg>
        </div>

        <!-- Library -->
        <div class="space-y-1">
          <div
            v-if="!compact"
            class="px-3 mb-2 text-xs font-semibold tracking-wider text-gray-500 uppercase"
          >
            Library
          </div>

          <router-link
            to="/songs"
            active-class="bg-[#282828] text-[var(--accent-color)] font-medium"
            class="flex items-center rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors"
            :class="compact ? 'justify-center py-2.5' : 'gap-3 px-3 py-2'"
            :title="compact ? 'Songs' : null"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M9 18V5l12-2v13"></path>
              <circle cx="6" cy="18" r="3"></circle>
              <circle cx="18" cy="16" r="3"></circle>
            </svg>
            <span v-if="!compact">Songs</span>
          </router-link>

          <router-link
            to="/albums"
            active-class="bg-[#282828] text-[var(--accent-color)] font-medium"
            class="flex items-center rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors"
            :class="compact ? 'justify-center py-2.5' : 'gap-3 px-3 py-2'"
            :title="compact ? 'Albums' : null"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <rect x="4" y="4" width="16" height="16" rx="2" ry="2"></rect>
              <circle cx="12" cy="12" r="4"></circle>
              <line x1="12" y1="12" x2="12" y2="4"></line>
            </svg>
            <span v-if="!compact">Albums</span>
          </router-link>

          <router-link
            to="/artists"
            active-class="bg-[#282828] text-[var(--accent-color)] font-medium"
            class="flex items-center rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors"
            :class="compact ? 'justify-center py-2.5' : 'gap-3 px-3 py-2'"
            :title="compact ? 'Artists' : null"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path
                d="M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z"
              ></path>
            </svg>
            <span v-if="!compact">Artists</span>
          </router-link>

          <router-link
            to="/favorites"
            active-class="bg-[#282828] text-[var(--accent-color)] font-medium"
            class="flex items-center rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors"
            :class="compact ? 'justify-center py-2.5' : 'gap-3 px-3 py-2'"
            :title="compact ? 'Liked Songs' : null"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="currentColor"
              stroke="none"
            >
              <path
                d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"
              ></path>
            </svg>
            <span v-if="!compact">Liked Songs</span>
          </router-link>

          <router-link
            to="/settings"
            active-class="bg-[#282828] text-[var(--accent-color)] font-medium"
            class="flex items-center rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors"
            :class="compact ? 'justify-center py-2.5' : 'gap-3 px-3 py-2'"
            :title="compact ? 'Settings' : null"
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
              <line x1="4" y1="21" x2="4" y2="14"></line>
              <line x1="4" y1="10" x2="4" y2="3"></line>
              <line x1="12" y1="21" x2="12" y2="12"></line>
              <line x1="12" y1="8" x2="12" y2="3"></line>
              <line x1="20" y1="21" x2="20" y2="16"></line>
              <line x1="20" y1="12" x2="20" y2="3"></line>
              <line x1="1" y1="14" x2="7" y2="14"></line>
              <line x1="9" y1="8" x2="15" y2="8"></line>
              <line x1="17" y1="16" x2="23" y2="16"></line>
            </svg>
            <span v-if="!compact">Settings</span>
          </router-link>
        </div>

        <!-- Playlists -->
        <div class="space-y-1 overflow-hidden flex flex-col min-h-0 flex-1">
          <div
            class="flex items-center mb-1"
            :class="compact ? 'justify-center px-1' : 'justify-between px-3'"
          >
            <span
              v-if="!compact"
              class="text-xs font-semibold tracking-wider text-gray-500 uppercase"
              >Playlists</span
            >
            <button
              @click="newPlaylist"
              class="text-gray-500 hover:text-white transition"
              title="New playlist"
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="16"
                height="16"
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
            </button>
          </div>
          <div class="overflow-auto flex-1 -mr-1 pr-1">
            <router-link
              v-for="pl in store.playlists"
              :key="pl.id"
              :to="'/playlists/' + pl.id"
              active-class="bg-[#282828] text-white"
              class="flex items-center rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors"
              :class="compact ? 'justify-center py-1.5' : 'gap-3 px-2 py-1.5'"
              :title="compact ? pl.name : null"
            >
              <PlaylistCover
                :name="pl.name"
                :cover="pl.cover"
                :size="28"
                className="h-7 w-7 rounded shrink-0"
              />
              <span v-if="!compact" class="truncate">{{ pl.name }}</span>
            </router-link>
            <div
              v-if="store.playlists.length === 0 && !compact"
              class="px-3 py-1 text-[11px] text-gray-600"
            >
              No playlists yet
            </div>
          </div>
        </div>

        <div class="mt-auto" :class="compact ? '' : 'px-3 mb-4'">
          <button
            @click="store.selectAndScan()"
            :disabled="store.loading"
            class="w-full group flex items-center rounded-md text-sm font-medium text-[var(--accent-color)] hover:bg-[#282828] transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed"
            :class="compact ? 'justify-center py-2' : 'gap-3 px-3 py-2'"
            :title="compact ? 'Add Folder' : null"
          >
            <div
              class="flex items-center justify-center w-5 h-5 rounded bg-[var(--accent-color)]/10 group-hover:bg-[var(--accent-color)]/20 transition-colors"
            >
              <svg
                v-if="store.loading"
                class="w-3 h-3 animate-spin"
                xmlns="http://www.w3.org/2000/svg"
                fill="none"
                viewBox="0 0 24 24"
              >
                <circle
                  class="opacity-25"
                  cx="12"
                  cy="12"
                  r="10"
                  stroke="currentColor"
                  stroke-width="4"
                ></circle>
                <path
                  class="opacity-75"
                  fill="currentColor"
                  d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
                ></path>
              </svg>
              <svg
                v-else
                xmlns="http://www.w3.org/2000/svg"
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="3"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <line x1="12" y1="5" x2="12" y2="19"></line>
                <line x1="5" y1="12" x2="19" y2="12"></line>
              </svg>
            </div>

            <span v-if="!compact">{{ store.loading ? 'Scanning...' : 'Add Folder' }}</span>
          </button>

          <div v-if="!compact" class="text-[10px] text-gray-600 mt-1 px-3 truncate opacity-70">
            {{ store.statusMessage }}
          </div>
        </div>
      </nav>

      <!-- Main Content -->
      <div class="flex flex-col flex-1 overflow-hidden">
        <!-- Player Controls -->
        <PlayerControls />

        <main class="flex-1 relative overflow-hidden flex flex-col bg-[var(--app-bg)]">
          <div
            ref="scrollContainer"
            class="flex-1 overflow-auto scroll-smooth"
            @click="store.queuePanelOpen = false"
          >
            <router-view v-slot="{ Component }">
              <keep-alive :include="['SongsView', 'AlbumsView', 'ArtistsView']">
                <component :is="Component" />
              </keep-alive>
            </router-view>
          </div>

          <!-- Up-next queue drawer -->
          <QueuePanel @click.stop />
        </main>
      </div>
    </div>

    <!-- Create-playlist modal (global overlay) -->
    <PlaylistCreateModal />
  </div>
</template>
