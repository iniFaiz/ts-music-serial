<script setup>
import { invoke } from '@tauri-apps/api/core';
import { useRouter } from 'vue-router';
import CoverImage from '../components/CoverImage.vue';
import { navigateWithTransition } from '../viewTransition';
import { useQuery } from '../useLibraryData';

defineOptions({ name: 'ArtistsView' });

const router = useRouter();

// Artists grouped in SQLite (GROUP BY), mapped to the card shape the template uses.
const { data: artists } = useQuery(
  async () => {
    const rows = await invoke('db_artists', { search: null });
    return rows.map((r) => ({
      name: r.artist,
      count: r.track_count,
      albums: r.album_count,
      coverPath: r.cover_path,
    }));
  },
  { initial: [] }
);

function openArtist(artistName, event) {
  const coverEl = event.currentTarget.querySelector('.cover-image');
  navigateWithTransition(
    () => router.push({ name: 'ArtistDetail', params: { name: artistName } }),
    coverEl,
    'shared-cover',
    'to-artist-transition'
  );
}
</script>

<template>
  <div class="h-full overflow-auto px-8 pt-8 pb-12">
    <h1 class="text-3xl font-bold tracking-tight text-white mb-6">Artists</h1>

    <div
      class="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6 gap-x-6 gap-y-10"
    >
      <div
        v-for="artist in artists"
        :key="artist.name"
        :data-cover-key="artist.name"
        @click="openArtist(artist.name, $event)"
        class="cursor-pointer group text-center"
      >
        <!-- Artist Image -->
        <div class="w-full aspect-square mb-4 mx-auto max-w-[200px] relative">
          <CoverImage
            :path="artist.coverPath"
            className="w-full h-full rounded-full shadow-lg object-cover bg-[#282828] group-hover:scale-[1.02] transition-transform duration-200"
          />
        </div>

        <h3 class="text-[15px] font-medium text-white truncate">{{ artist.name }}</h3>
      </div>
    </div>
  </div>
</template>
