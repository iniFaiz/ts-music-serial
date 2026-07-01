<script setup>
import { store } from '../store';
import { invoke } from '@tauri-apps/api/core';
import SongList from '../components/SongList.vue';
import { useQuery } from '../useLibraryData';

defineOptions({ name: 'SongsView' });

// All songs (or the FTS5 search matches), fetched from SQLite and re-run when the
// search box or the library changes. SongList virtualises the rows.
const { data: songs } = useQuery(
  async () => {
    const page = await invoke('db_tracks_page', {
      sortBy: 'title',
      order: 'asc',
      search: store.searchQuery || null,
      offset: 0,
      limit: 500000,
    });
    return page.tracks;
  },
  { deps: [() => store.searchQuery], initial: [] }
);
</script>

<template>
  <div class="h-full flex flex-col">
    <div class="px-8 pt-8 pb-4">
      <h1 class="text-3xl font-bold tracking-tight text-white">Songs</h1>
      <p v-if="store.searchQuery" class="text-sm text-gray-500 mt-1">
        Searching for "{{ store.searchQuery }}"
      </p>
    </div>
    <div class="flex-1 overflow-auto">
      <SongList :songs="songs" />
    </div>
  </div>
</template>
