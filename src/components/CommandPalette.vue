<script setup>
import { ref, computed, watch, nextTick } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useRouter } from 'vue-router';
import { store } from '../store';
import CoverImage from './CoverImage.vue';

// Ctrl+K command palette. Uses the SQLite FTS5 index for instant song search and
// the GROUP BY album/artist commands, plus the in-memory playlist cache, to jump
// anywhere in the library. Arrow keys to move, Enter to activate, Esc to close.

const router = useRouter();

const query = ref('');
const songs = ref([]);
const albums = ref([]);
const artists = ref([]);
const activeIndex = ref(0);
const inputEl = ref(null);
let debounce = null;

const playlists = computed(() => {
  const q = query.value.trim().toLowerCase();
  if (!q) return [];
  return store.playlists.filter((p) => (p.name || '').toLowerCase().includes(q)).slice(0, 4);
});

// One flat, ordered list so arrow-key navigation and rendering stay in sync.
const items = computed(() => [
  ...songs.value.map((s) => ({ type: 'song', data: s })),
  ...albums.value.map((a) => ({ type: 'album', data: a })),
  ...artists.value.map((a) => ({ type: 'artist', data: a })),
  ...playlists.value.map((p) => ({ type: 'playlist', data: p })),
]);

async function runSearch() {
  const q = query.value.trim();
  if (!q) {
    songs.value = [];
    albums.value = [];
    artists.value = [];
    activeIndex.value = 0;
    return;
  }
  try {
    const [s, al, ar] = await Promise.all([
      invoke('db_search', { query: q, limit: 6 }),
      invoke('db_albums', { search: q }),
      invoke('db_artists', { search: q }),
    ]);
    songs.value = s || [];
    albums.value = (al || []).slice(0, 4);
    artists.value = (ar || []).slice(0, 4);
    activeIndex.value = 0;
  } catch (e) {
    console.error('Command palette search failed', e);
  }
}

watch(query, () => {
  if (debounce) clearTimeout(debounce);
  debounce = setTimeout(runSearch, 140);
});

watch(
  () => store.commandPaletteOpen,
  (open) => {
    if (open) {
      query.value = '';
      songs.value = [];
      albums.value = [];
      artists.value = [];
      activeIndex.value = 0;
      nextTick(() => inputEl.value && inputEl.value.focus());
    }
  }
);

function move(delta) {
  const n = items.value.length;
  if (n === 0) return;
  activeIndex.value = (activeIndex.value + delta + n) % n;
}

function activate(item) {
  if (!item) return;
  store.closeCommandPalette();
  if (item.type === 'song') {
    store.playSong(item.data, [item.data]);
  } else if (item.type === 'album') {
    router.push({ name: 'AlbumDetail', params: { name: item.data.album } });
  } else if (item.type === 'artist') {
    router.push({ name: 'ArtistDetail', params: { name: item.data.artist } });
  } else if (item.type === 'playlist') {
    router.push(
      item.data.is_smart ? '/smart/' + item.data.id : { name: 'PlaylistDetail', params: { id: item.data.id } }
    );
  }
}

function onKeydown(e) {
  if (e.key === 'ArrowDown') {
    e.preventDefault();
    move(1);
  } else if (e.key === 'ArrowUp') {
    e.preventDefault();
    move(-1);
  } else if (e.key === 'Enter') {
    e.preventDefault();
    activate(items.value[activeIndex.value]);
  } else if (e.key === 'Escape') {
    e.preventDefault();
    store.closeCommandPalette();
  }
}

const groupLabel = { song: 'Songs', album: 'Albums', artist: 'Artists', playlist: 'Playlists' };
// Whether a flat-list row is the first of its group (to render a header above it).
function isGroupStart(i) {
  return i === 0 || items.value[i].type !== items.value[i - 1].type;
}
</script>

<template>
  <Teleport to="body">
    <Transition name="cmdk">
      <div
        v-if="store.commandPaletteOpen"
        class="fixed inset-0 z-[400] flex items-start justify-center pt-[12vh] bg-black/60 backdrop-blur-sm"
        @click.self="store.closeCommandPalette()"
      >
        <div
          class="cmdk-panel w-[600px] max-w-[92vw] bg-[#1c1c1e] rounded-2xl shadow-2xl border border-[#2c2c2e] overflow-hidden"
        >
          <!-- Search input -->
          <div class="flex items-center gap-3 px-4 py-3 border-b border-[#2c2c2e]">
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              class="text-gray-500 shrink-0"
            >
              <circle cx="11" cy="11" r="8" />
              <line x1="21" y1="21" x2="16.65" y2="16.65" />
            </svg>
            <input
              ref="inputEl"
              v-model="query"
              @keydown="onKeydown"
              type="text"
              placeholder="Search songs, albums, artists, playlists…"
              class="flex-1 bg-transparent text-white text-[15px] focus:outline-none placeholder-gray-600"
            />
            <kbd class="text-[10px] text-gray-500 border border-[#3a3a3a] rounded px-1.5 py-0.5">Esc</kbd>
          </div>

          <!-- Results -->
          <div class="max-h-[52vh] overflow-y-auto py-2">
            <template v-if="items.length">
              <template v-for="(item, i) in items" :key="item.type + '-' + i">
                <div
                  v-if="isGroupStart(i)"
                  class="px-4 pt-2 pb-1 text-[11px] font-semibold uppercase tracking-wider text-gray-500"
                >
                  {{ groupLabel[item.type] }}
                </div>
                <button
                  class="w-full flex items-center gap-3 px-4 py-2 text-left transition-colors"
                  :class="i === activeIndex ? 'bg-[var(--accent-color)]/15' : 'hover:bg-white/5'"
                  @click="activate(item)"
                  @mousemove="activeIndex = i"
                >
                  <!-- Art -->
                  <CoverImage
                    v-if="item.type === 'song'"
                    :path="item.data.path"
                    className="w-9 h-9 rounded bg-[#282828] shrink-0"
                  />
                  <CoverImage
                    v-else-if="item.type === 'album'"
                    :path="item.data.cover_path"
                    className="w-9 h-9 rounded bg-[#282828] shrink-0"
                  />
                  <CoverImage
                    v-else-if="item.type === 'artist'"
                    :path="item.data.cover_path"
                    className="w-9 h-9 rounded-full bg-[#282828] shrink-0"
                  />
                  <div
                    v-else
                    class="w-9 h-9 rounded bg-[#282828] shrink-0 flex items-center justify-center text-gray-500"
                  >
                    <svg
                      xmlns="http://www.w3.org/2000/svg"
                      width="16"
                      height="16"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="2"
                    >
                      <line x1="8" y1="6" x2="21" y2="6" />
                      <line x1="8" y1="12" x2="21" y2="12" />
                      <line x1="8" y1="18" x2="13" y2="18" />
                    </svg>
                  </div>

                  <div class="min-w-0 flex-1">
                    <div class="text-sm text-white truncate">
                      {{
                        item.type === 'song'
                          ? item.data.title
                          : item.type === 'album'
                            ? item.data.album
                            : item.type === 'artist'
                              ? item.data.artist
                              : item.data.name
                      }}
                    </div>
                    <div class="text-xs text-gray-500 truncate">
                      {{
                        item.type === 'song'
                          ? item.data.artist
                          : item.type === 'album'
                            ? item.data.artist
                            : item.type === 'artist'
                              ? item.data.track_count + ' songs'
                              : item.data.is_smart
                                ? 'Smart playlist'
                                : 'Playlist'
                      }}
                    </div>
                  </div>
                  <span
                    v-if="item.type === 'song'"
                    class="text-[11px] text-gray-600 shrink-0"
                    >Play</span
                  >
                </button>
              </template>
            </template>
            <div v-else-if="query.trim()" class="px-4 py-10 text-center text-sm text-gray-500">
              No results for "{{ query }}"
            </div>
            <div v-else class="px-4 py-10 text-center text-sm text-gray-600">
              Type to search your library
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.cmdk-enter-active,
.cmdk-leave-active {
  transition: opacity 0.18s ease;
}
.cmdk-enter-from,
.cmdk-leave-to {
  opacity: 0;
}
.cmdk-enter-active .cmdk-panel,
.cmdk-leave-active .cmdk-panel {
  transition: transform 0.2s cubic-bezier(0.22, 1, 0.36, 1);
}
.cmdk-enter-from .cmdk-panel,
.cmdk-leave-to .cmdk-panel {
  transform: translateY(-12px) scale(0.98);
}
</style>
