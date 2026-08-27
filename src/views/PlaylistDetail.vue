<script setup>
import { computed, ref, watch, onMounted, onUnmounted } from 'vue';
import { invokeCommand as invoke } from '../generated/ipc';
import { useRoute, useRouter } from 'vue-router';
import { store } from '../store';
import SongList from '../components/SongList.vue';
import PlaylistCover from '../components/PlaylistCover.vue';
import CoverImage from '../components/CoverImage.vue';
import { useQuery } from '../useLibraryData';
import { fetchPlaylistTracks } from '../libraryQueries';

const route = useRoute();
const router = useRouter();

defineOptions({ name: 'PlaylistDetail' });

const playlistId = computed(() => route.params.id);
const playlist = computed(() => store.getPlaylist(playlistId.value));
// Playlist tracks (in order) fetched from the DB; re-runs on library changes or
// when the route id changes.
const { data: songs, loading } = useQuery(() => fetchPlaylistTracks(playlistId.value), {
  deps: [() => store.playlistsVersion, () => playlistId.value],
  initial: [],
  cacheKey: () => `playlist:${playlistId.value}`,
});

const suggestedSongs = ref([]);
const isRefreshingSuggestions = ref(false);
const isBulkRefreshing = ref(false);
const addingSongPath = ref(null);
const recentlyAddedPath = ref(null);

const playAll = () => {
  if (songs.value.length > 0) {
    store.recordRecent('playlist', playlistId.value);
    store.playSong(songs.value[0], songs.value);
  }
};

const removePlaylist = () => {
  menuOpen.value = false;
  if (!playlist.value) return;
  store.showConfirm({
    title: 'Delete Playlist',
    message: `Are you sure you want to delete "${playlist.value.name}"? This action cannot be undone.`,
    confirmText: 'Delete',
    cancelText: 'Cancel',
    onConfirm: async () => {
      await store.deletePlaylist(playlist.value.id);
      router.push('/playlists');
    },
  });
};

const menuOpen = ref(false);

const closeMenu = (e) => {
  if (e && e.target.closest('.playlist-menu-container')) return;
  menuOpen.value = false;
};

onMounted(() => {
  window.addEventListener('click', closeMenu);
});

onUnmounted(() => {
  window.removeEventListener('click', closeMenu);
});

const shufflePlaylist = async () => {
  if (songs.value.length > 0) {
    store.recordRecent('playlist', playlistId.value);
    await store.playRandom(songs.value);
  }
};

const playNextPlaylist = () => {
  if (songs.value.length > 0) {
    store.playNextSongs(songs.value);
  }
};

const playLastPlaylist = () => {
  if (songs.value.length > 0) {
    store.addToQueue(songs.value);
  }
};

const exportM3u = (portable = false) => {
  menuOpen.value = false;
  store.exportPlaylistM3u(playlistId.value, { relativePaths: portable });
};

// Suggest a few random library tracks not already in this playlist, sampled from DB.
const getSuggestions = async () => {
  if (isRefreshingSuggestions.value) return;
  isRefreshingSuggestions.value = true;
  isBulkRefreshing.value = true;
  try {
    const currentPaths = new Set((songs.value || []).map((s) => s.path));
    const seen = new Set(currentPaths);
    const picks = [];
    for (let i = 0; i < 30 && picks.length < 5; i++) {
      let t = null;
      try {
        t = await invoke('db_random_track', { exclude: null });
      } catch {
        t = null;
      }
      if (t && !seen.has(t.path)) {
        seen.add(t.path);
        picks.push(t);
      }
    }
    suggestedSongs.value = picks;
  } finally {
    setTimeout(() => {
      isRefreshingSuggestions.value = false;
    }, 400);
    setTimeout(() => {
      isBulkRefreshing.value = false;
    }, 450);
  }
};

watch(
  [() => playlistId.value, loading],
  ([newId, isLoading], [oldId]) => {
    if (!isLoading && (newId !== oldId || suggestedSongs.value.length === 0)) {
      getSuggestions();
    }
  },
  { immediate: true }
);

const suggestionsClosed = ref(false);

watch(playlistId, () => {
  suggestionsClosed.value = false;
});

const addAndRemoveFromSuggestions = async (songPath) => {
  if (addingSongPath.value) return;
  addingSongPath.value = songPath;
  try {
    await store.addToPlaylist(playlist.value.id, songPath);
  } catch {
    addingSongPath.value = null;
    return;
  }
  recentlyAddedPath.value = songPath;

  setTimeout(() => {
    if (recentlyAddedPath.value === songPath) {
      recentlyAddedPath.value = null;
    }
  }, 2500);

  // Smoothly remove added song from recommendations list
  suggestedSongs.value = suggestedSongs.value.filter((s) => s.path !== songPath);

  // Fetch 1 replacement track to maintain 5 suggestions
  try {
    const currentPlaylistPaths = new Set((songs.value || []).map((s) => s.path));
    const currentSuggestedPaths = new Set(suggestedSongs.value.map((s) => s.path));

    let replacement = null;
    for (let i = 0; i < 25; i++) {
      const t = await invoke('db_random_track', { exclude: null }).catch(() => null);
      if (t && !currentPlaylistPaths.has(t.path) && !currentSuggestedPaths.has(t.path)) {
        replacement = t;
        break;
      }
    }

    if (replacement) {
      await new Promise((r) => setTimeout(r, 120));
      suggestedSongs.value.push(replacement);
    } else if (suggestedSongs.value.length === 0) {
      getSuggestions();
    }
  } finally {
    addingSongPath.value = null;
  }
};
</script>

<template>
  <div
    v-if="!store.libraryReady"
    class="h-full flex items-center justify-center text-sm text-gray-500"
    role="status"
  >
    <span class="animate-pulse">Loading playlist…</span>
  </div>

  <div v-else-if="playlist" class="flex flex-col h-full overflow-auto">
    <!-- Header -->
    <div class="p-8 flex gap-8 items-end bg-gradient-to-b from-[#2a2a2a] to-[var(--app-bg)]">
      <PlaylistCover
        :name="playlist.name"
        :cover="playlist.cover"
        :size="208"
        className="h-52 w-52 rounded-2xl shadow-2xl"
        style="view-transition-name: shared-cover"
      />

      <div class="flex flex-col gap-1 pb-2 overflow-hidden flex-1">
        <h4 class="text-sm font-bold text-[var(--accent-color)] uppercase tracking-wider mb-1">
          Playlist
        </h4>

        <h1 class="text-4xl font-bold tracking-tight text-white truncate">
          {{ playlist.name }}
        </h1>

        <p
          v-if="playlist.description"
          class="text-sm text-[var(--text-secondary)] mt-2 line-clamp-2 max-w-xl"
        >
          {{ playlist.description }}
        </p>
        <p class="text-xs text-[var(--text-secondary)] font-medium mt-2">
          {{ loading ? $t('common.loading') : $t('views.playlists.songsCount', { count: songs.length }) }}
        </p>

        <div class="flex gap-3 mt-6 items-center">
          <button
            @click="playAll"
            :disabled="loading || songs.length === 0"
            class="bg-[var(--accent-color)] text-white px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-red-500 transition flex items-center gap-2 shadow-lg disabled:opacity-40"
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
            {{ $t('common.play') }}
          </button>
          <button
            @click="shufflePlaylist"
            :disabled="loading || songs.length === 0"
            class="bg-[#3a3a3a] text-[var(--accent-color)] px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-[#444] transition flex items-center gap-2 shadow-lg disabled:opacity-40"
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
            {{ $t('common.shuffle') }}
          </button>
        </div>
      </div>

      <!-- Ellipsis Options Menu at the far right end -->
      <div class="relative pb-2 self-end playlist-menu-container">
        <button
          @click.stop="menuOpen = !menuOpen"
          class="text-red-500 hover:text-red-400 p-2 rounded-full hover:bg-white/5 transition-colors flex items-center justify-center"
          :title="$t('common.options')"
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
            @click="store.openPlaylistModal(null, 'edit', playlist.id)"
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors"
          >
            {{ $t('common.edit') }}
          </button>
          <button
            @click="playAll"
            :disabled="loading || songs.length === 0"
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors disabled:opacity-40"
          >
            {{ $t('common.play') }} "{{ playlist.name }}"
          </button>
          <button
            @click="shufflePlaylist"
            :disabled="loading || songs.length === 0"
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors disabled:opacity-40"
          >
            {{ $t('common.shuffle') }} "{{ playlist.name }}"
          </button>
          <button
            @click="playNextPlaylist"
            :disabled="loading || songs.length === 0"
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors disabled:opacity-40"
          >
            {{ $t('songList.menu.playNext') }}
          </button>
          <button
            @click="playLastPlaylist"
            :disabled="loading || songs.length === 0"
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors disabled:opacity-40"
          >
            {{ $t('views.albums.addToQueue') }}
          </button>
          <div class="border-t border-[#3a3a3a] my-1"></div>
          <button
            @click="exportM3u(false)"
            :disabled="loading || songs.length === 0"
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors disabled:opacity-40"
          >
            {{ $t('views.playlistDetail.exportM3u') }}
          </button>
          <button
            @click="exportM3u(true)"
            :disabled="loading || songs.length === 0"
            class="w-full text-left px-4 py-2 hover:bg-[#3a3a3a] transition-colors disabled:opacity-40"
            title="Relative paths — keep this file next to your music folder to share it"
          >
            {{ $t('views.playlistDetail.exportPortableM3u') }}
          </button>
          <div class="border-t border-[#3a3a3a] my-1"></div>
          <button
            @click="removePlaylist"
            class="w-full text-left px-4 py-2 text-red-500 hover:bg-[#3a3a3a] transition-colors"
          >
            {{ $t('views.playlistDetail.delete') }}
          </button>
        </div>
      </div>
    </div>

    <div class="px-2 pb-12">
      <SongList
        v-if="loading || songs.length > 0"
        :songs="songs"
        :playlist-id="playlist.id"
        :loading="loading"
        :highlight-path="recentlyAddedPath"
      />
      <div v-else class="py-12 px-6 text-center text-gray-500">
        <div class="text-4xl mb-3 opacity-20">♫</div>
        <p class="text-sm font-medium text-white/80">{{ $t('views.playlistDetail.empty') }}</p>
        <p class="text-xs text-gray-500 mt-1 max-w-sm mx-auto">
          {{ $t('views.playlistDetail.addSongs') }}
        </p>
      </div>

      <!-- Suggested Songs Widget -->
      <Transition name="widget-fade">
        <div
          v-if="!loading && songs.length < 25 && !suggestionsClosed"
          class="mt-10 max-w-lg mx-auto text-left bg-[#1d1d1f] border border-[#2d2d2f] rounded-xl p-5 shadow-2xl relative overflow-hidden"
        >
          <div class="flex items-center justify-between mb-4 border-b border-[#2d2d2f] pb-3">
            <div>
              <h3 class="text-xs font-semibold text-white uppercase tracking-wider">
                {{ $t('views.playlistDetail.recommendedSongs') }}
              </h3>
            </div>
            <div class="flex items-center gap-2">
              <button
                @click="getSuggestions"
                :disabled="isRefreshingSuggestions"
                class="text-gray-400 hover:text-white transition flex items-center gap-1.5 text-[11px] font-medium bg-[#282828] hover:bg-[#333] px-2.5 py-1 rounded-md border border-[#3a3a3a] disabled:opacity-50"
                :title="$t('views.playlistDetail.refreshSuggestions')"
              >
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  width="11"
                  height="11"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  :class="{ 'animate-spin': isRefreshingSuggestions }"
                  class="transition-transform duration-500"
                >
                  <path d="M21.5 2v6h-6M21.34 15.57a10 10 0 1 1-.57-8.38l5.67-5.67" />
                </svg>
                {{ $t('common.refresh') }}
              </button>
              <button
                @click="suggestionsClosed = true"
                class="text-gray-400 hover:text-white transition flex items-center justify-center bg-[#282828] hover:bg-[#333] h-[25px] w-[25px] rounded-md border border-[#3a3a3a]"
                :title="$t('common.close')"
              >
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  width="12"
                  height="12"
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
          </div>

          <TransitionGroup
            v-if="suggestedSongs.length > 0"
            name="rec-song"
            tag="div"
            class="space-y-2 relative"
            :class="{ 'is-bulk-refreshing': isBulkRefreshing }"
          >
            <div
              v-for="song in suggestedSongs"
              :key="song.path"
              class="flex items-center justify-between p-2 rounded-lg hover:bg-white/5 transition duration-150 group rec-song-item"
            >
              <div class="flex items-center gap-3 overflow-hidden flex-1 min-w-0 pr-3">
                <CoverImage
                  :path="song.path"
                  className="h-9 w-9 rounded-[4px] shadow-md bg-[#333] shrink-0"
                />
                <div class="truncate">
                  <div class="text-xs font-medium text-white truncate leading-none mb-1">
                    {{ song.title }}
                  </div>
                  <div class="text-[10px] text-gray-400 truncate leading-none">
                    {{ song.artist }} • <span class="opacity-60">{{ song.album }}</span>
                  </div>
                </div>
              </div>

              <button
                @click="addAndRemoveFromSuggestions(song.path)"
                :disabled="addingSongPath === song.path"
                class="bg-[#282828] hover:bg-[var(--accent-color)] text-gray-300 hover:text-white border border-[#3a3a3a] hover:border-transparent px-3 py-1 rounded-full text-[11px] font-semibold transition-all duration-200 flex items-center gap-1 shrink-0 active:scale-95 disabled:opacity-50"
              >
                <svg
                  v-if="addingSongPath !== song.path"
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
                  <line x1="12" y1="5" x2="12" y2="19"></line>
                  <line x1="5" y1="12" x2="19" y2="12"></line>
                </svg>
                <svg
                  v-else
                  class="animate-spin h-2.5 w-2.5 text-white"
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
                    d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                  ></path>
                </svg>
                {{ addingSongPath === song.path ? 'Adding' : 'Add' }}
              </button>
            </div>
          </TransitionGroup>
          <div v-else class="text-center py-6 text-xs text-gray-600">
            No suggestions available. Try adding more songs to your library.
          </div>
        </div>
      </Transition>
    </div>
  </div>

  <div v-else class="p-20 text-center text-gray-600">
    <p>Playlist not found.</p>
  </div>
</template>

<style scoped>
/* Recommended songs transition */
.rec-song-move {
  transition: transform 0.35s cubic-bezier(0.16, 1, 0.3, 1);
}

.rec-song-enter-active {
  transition: all 0.4s cubic-bezier(0.16, 1, 0.3, 1);
  overflow: hidden;
  max-height: 60px;
}

.rec-song-leave-active {
  transition: all 0.35s cubic-bezier(0.16, 1, 0.3, 1);
  overflow: hidden;
  max-height: 60px;
  pointer-events: none;
}

.rec-song-enter-from {
  opacity: 0;
  max-height: 0px;
  padding-top: 0px;
  padding-bottom: 0px;
  margin-top: 0px;
  margin-bottom: 0px;
  transform: translateY(12px) scale(0.96);
}

.rec-song-leave-to {
  opacity: 0;
  max-height: 0px;
  padding-top: 0px;
  padding-bottom: 0px;
  margin-top: 0px;
  margin-bottom: 0px;
  transform: translateX(24px) scale(0.94);
}

/* Bulk refresh in-place crossfade mode */
.is-bulk-refreshing .rec-song-leave-active {
  position: absolute !important;
  left: 0;
  right: 0;
  width: 100%;
  max-height: none !important;
}

.is-bulk-refreshing .rec-song-enter-active {
  max-height: none !important;
}

.is-bulk-refreshing .rec-song-enter-from {
  max-height: none !important;
  padding-top: 8px !important;
  padding-bottom: 8px !important;
  margin-top: 0px !important;
}

.is-bulk-refreshing .rec-song-leave-to {
  max-height: none !important;
  padding-top: 8px !important;
  padding-bottom: 8px !important;
  margin-top: 0px !important;
  transform: translateY(-6px) scale(0.96) !important;
}

/* Recommended Widget Fade/Scale transition */
.widget-fade-enter-active,
.widget-fade-leave-active {
  transition: all 0.35s cubic-bezier(0.16, 1, 0.3, 1);
}

.widget-fade-enter-from,
.widget-fade-leave-to {
  opacity: 0;
  transform: translateY(16px) scale(0.97);
}
</style>
