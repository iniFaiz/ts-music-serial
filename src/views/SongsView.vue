<script setup>
import { ref, watch } from 'vue';
import { store } from '../store';
import { invoke } from '@tauri-apps/api/core';
import SongList from '../components/SongList.vue';

defineOptions({ name: 'SongsView' });

// Windowed pagination: fetch the library one page at a time from SQLite (sorted +
// FTS-filtered server-side) and append as the user scrolls, instead of loading
// the entire library into memory. SongList runs in `server-sort` mode so its
// column headers refetch rather than sorting the partial array.
const PAGE = 300;
const songs = ref([]);
const total = ref(0);
const loading = ref(false);
const offset = ref(0);
const sortBy = ref('title');
const order = ref('asc');
const scrollEl = ref(null);
// Incremented only when a fresh result set replaces the list (search / sort) so
// SongList crossfades it in — not on scroll-append.
const swapSignal = ref(0);

// Debounced search — avoid a DB query on every keystroke.
const search = ref(store.searchQuery);
let searchTimer = null;
watch(
  () => store.searchQuery,
  (v) => {
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      search.value = v;
    }, 150);
  }
);

// Bumped on every reset so a slow in-flight request from a previous query can't
// clobber the results of a newer one.
let reqToken = 0;

async function loadPage(reset = false) {
  if (!reset && loading.value) return;
  if (!reset && total.value > 0 && songs.value.length >= total.value) {
    return; // everything loaded
  }
  const fetchOffset = reset ? 0 : offset.value;
  const myToken = reset ? ++reqToken : reqToken;
  loading.value = true;
  try {
    const page = await invoke('db_tracks_page', {
      sortBy: sortBy.value,
      order: order.value,
      search: search.value || null,
      offset: fetchOffset,
      limit: PAGE,
    });
    // A newer reset superseded this request — drop its (stale) results. Crucially,
    // we only ever swap in the new list *after* it arrives, so the previous
    // results stay on screen (no flash of the "No songs found" empty state).
    if (myToken !== reqToken) return;
    total.value = page.total;
    songs.value = reset ? page.tracks : songs.value.concat(page.tracks);
    offset.value = songs.value.length;
    if (reset) {
      swapSignal.value += 1; // signal SongList to crossfade the new results
      if (scrollEl.value) scrollEl.value.scrollTop = 0;
    }
  } catch (e) {
    console.error('Failed to load songs page', e);
  } finally {
    if (myToken === reqToken) loading.value = false;
  }
}

// Reset + reload when the library, search, or sort changes.
watch([() => store.libraryVersion, search, sortBy, order], () => loadPage(true), {
  immediate: true,
});

function onScroll() {
  const el = scrollEl.value;
  if (!el) return;
  // Load the next page as we approach the bottom.
  if (el.scrollTop + el.clientHeight >= el.scrollHeight - 600) {
    loadPage(false);
  }
}

function onSortChange({ key, order: ord }) {
  sortBy.value = key || 'title';
  order.value = ord || 'asc';
}
</script>

<template>
  <div class="h-full flex flex-col">
    <div class="px-8 pt-8 pb-4">
      <h1 class="text-3xl font-bold tracking-tight text-white">Songs</h1>
      <p v-if="store.searchQuery" class="text-sm text-gray-500 mt-1">
        Searching for "{{ store.searchQuery }}"
      </p>
    </div>
    <div ref="scrollEl" class="flex-1 overflow-auto" @scroll="onScroll">
      <SongList
        :songs="songs"
        server-sort
        :loading="loading"
        :swap-signal="swapSignal"
        @sort-change="onSortChange"
      />
    </div>
  </div>
</template>
