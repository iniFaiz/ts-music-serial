<script setup>
import { computed, ref } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { store } from '../store';
import { getCollection } from '../collections';
import { getMorphCollectionKey } from '../viewTransition';
import { useQuery } from '../useLibraryData';
import SongList from '../components/SongList.vue';
import SmartCover from '../components/SmartCover.vue';

const route = useRoute();
const router = useRouter();

// Only carry the shared-element cover name when this page was opened from a cover
// card (Top Picks / recent card). Header "see all" links clear the morph key, so
// those opens/closes just cross-fade instead of morphing into an unrelated card.
// The page is recreated per navigation (not kept alive), so reading this once at
// setup is correct.
const morphable = ref(getMorphCollectionKey() === route.params.key);

const collection = computed(() => getCollection(route.params.key));
// Fetch the collection's live tracks from the DB; refetch on library/stats change.
const { data: songs } = useQuery(
  () => (collection.value ? collection.value.fetch(store) : Promise.resolve([])),
  { deps: [() => route.params.key], watchStats: true, initial: [] }
);

const playAll = () => {
  if (songs.value.length > 0) {
    store.recordRecent('collection', route.params.key);
    store.playSong(songs.value[0], songs.value);
  }
};

const shuffleAll = () => {
  if (songs.value.length === 0) return;
  store.recordRecent('collection', route.params.key);
  store.shuffleMode = true;
  const i = Math.floor(Math.random() * songs.value.length);
  store.playSong(songs.value[i], songs.value);
};

// Turn this live insight into a real, editable Smart Playlist.
const saveAsSmart = async () => {
  const c = collection.value;
  if (!c) return;
  const sp = await store.createSmartPlaylist({
    name: c.title,
    description: c.subtitle,
    color: c.color,
    rules: JSON.parse(JSON.stringify(c.rules)),
    sortBy: c.sortBy,
    sortOrder: c.sortOrder,
    limit: 0,
  });
  if (sp) router.push('/smart/' + sp.id);
};
</script>

<template>
  <div v-if="collection" class="flex flex-col h-full overflow-auto">
    <!-- Header -->
    <div class="p-8 flex gap-8 items-end bg-gradient-to-b from-[#2a2a2a] to-[var(--app-bg)]">
      <SmartCover
        :title="collection.title"
        :color="collection.color"
        :icon="collection.icon"
        :show-title="false"
        className="h-52 w-52 rounded-md shadow-2xl shrink-0"
        :style="morphable ? 'view-transition-name: shared-cover' : undefined"
      />

      <div class="flex flex-col gap-1 pb-2 overflow-hidden flex-1">
        <h4 class="text-sm font-bold text-[var(--accent-color)] uppercase tracking-wider mb-1">Insight Mix</h4>
        <h1 class="text-4xl font-bold tracking-tight text-white">{{ collection.title }}</h1>
        <p class="text-sm text-[var(--text-secondary)] mt-2">{{ collection.subtitle }}</p>
        <p class="text-xs text-[var(--text-secondary)] font-medium mt-2">{{ songs.length }} songs</p>

        <div class="flex flex-wrap gap-3 mt-6 items-center">
          <button
            @click="playAll"
            :disabled="songs.length === 0"
            class="bg-[var(--accent-color)] text-white px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-red-500 transition flex items-center gap-2 shadow-lg disabled:opacity-40"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="currentColor" stroke="none">
              <polygon points="5 3 19 12 5 21 5 3" />
            </svg>
            Play
          </button>
          <button
            @click="shuffleAll"
            :disabled="songs.length === 0"
            class="bg-[#3a3a3a] text-[var(--accent-color)] px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-[#444] transition flex items-center gap-2 shadow-lg disabled:opacity-40"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M16 3h5v5M4 20L21 3M21 16v5h-5M15 15l6 6M4 4l5 5" />
            </svg>
            Shuffle
          </button>
          <button
            @click="saveAsSmart"
            class="bg-transparent text-gray-300 border border-[#3a3a3a] px-5 py-2 rounded-[4px] text-sm font-semibold hover:text-white hover:border-white/40 transition flex items-center gap-2"
            title="Create an editable Smart Playlist from these rules"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
            </svg>
            Save as Smart Playlist
          </button>
        </div>
      </div>
    </div>

    <div class="px-2 pb-12">
      <SongList v-if="songs.length > 0" :songs="songs" />
      <div v-else class="py-16 px-6 text-center text-gray-500">
        <div class="text-4xl mb-3 opacity-20">♫</div>
        <p class="text-sm font-medium text-white/80">Nothing here yet.</p>
        <p class="text-xs text-gray-500 mt-1 max-w-sm mx-auto">
          Keep listening — this mix fills in automatically as you play music.
        </p>
      </div>
    </div>
  </div>

  <div v-else class="p-20 text-center text-gray-600">
    <p>Collection not found.</p>
  </div>
</template>
