<script setup>
import { computed, ref, nextTick, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { store } from '../store';
import SongList from '../components/SongList.vue';
import PlaylistCover from '../components/PlaylistCover.vue';
import CoverImage from '../components/CoverImage.vue';

const route = useRoute();
const router = useRouter();

const playlistId = computed(() => route.params.id);
const playlist = computed(() => store.getPlaylist(playlistId.value));
const songs = computed(() => store.playlistSongs(playlistId.value));

const editing = ref(false);
const nameInput = ref('');
const nameField = ref(null);

const suggestedSongs = ref([]);

const startRename = async () => {
  if (!playlist.value) return;
  nameInput.value = playlist.value.name;
  editing.value = true;
  await nextTick();
  nameField.value?.focus();
  nameField.value?.select();
};

const commitRename = () => {
  if (editing.value && playlist.value) {
    store.renamePlaylist(playlist.value.id, nameInput.value);
  }
  editing.value = false;
};

const playAll = () => {
  if (songs.value.length > 0) store.playSong(songs.value[0], songs.value);
};

const removePlaylist = () => {
  if (playlist.value) {
    store.deletePlaylist(playlist.value.id);
    router.push('/songs');
  }
};

const getSuggestions = () => {
  if (!store.songs || store.songs.length === 0) {
    suggestedSongs.value = [];
    return;
  }
  const currentPaths = new Set(playlist.value?.paths || []);
  const availableSongs = store.songs.filter((s) => !currentPaths.has(s.path));

  if (availableSongs.length === 0) {
    suggestedSongs.value = [];
    return;
  }

  const shuffled = [...availableSongs].sort(() => 0.5 - Math.random());
  suggestedSongs.value = shuffled.slice(0, 5);
};

watch(
  [() => store.songs, () => playlistId.value],
  () => {
    getSuggestions();
  },
  { immediate: true }
);
</script>

<template>
  <div v-if="playlist" class="flex flex-col h-full overflow-auto">
    <!-- Header -->
    <div class="p-8 flex gap-8 items-end bg-gradient-to-b from-[#2a2a2a] to-[var(--app-bg)]">
      <PlaylistCover
        :name="playlist.name"
        :cover="playlist.cover"
        :size="208"
        className="h-52 w-52 rounded-md shadow-2xl"
      />

      <div class="flex flex-col gap-1 pb-2 overflow-hidden flex-1">
        <h4 class="text-sm font-bold text-[var(--accent-color)] uppercase tracking-wider mb-1">
          Playlist
        </h4>

        <input
          v-if="editing"
          ref="nameField"
          v-model="nameInput"
          @blur="commitRename"
          @keyup.enter="commitRename"
          @keyup.esc="editing = false"
          class="text-4xl font-bold tracking-tight text-white bg-transparent border-b border-[var(--accent-color)] focus:outline-none w-full"
        />
        <h1
          v-else
          @click="startRename"
          class="text-4xl font-bold tracking-tight text-white truncate cursor-text hover:underline decoration-dotted"
          title="Click to rename"
        >
          {{ playlist.name }}
        </h1>

        <p
          v-if="playlist.description"
          class="text-sm text-[var(--text-secondary)] mt-2 line-clamp-2 max-w-xl"
        >
          {{ playlist.description }}
        </p>
        <p class="text-xs text-[var(--text-secondary)] font-medium mt-2">
          {{ songs.length }} songs
        </p>

        <div class="flex gap-3 mt-6">
          <button
            @click="playAll"
            :disabled="songs.length === 0"
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
            Play
          </button>
          <button
            @click="removePlaylist"
            class="bg-[#3a3a3a] text-[var(--text-secondary)] px-6 py-2 rounded-[4px] text-sm font-semibold hover:bg-[#444] hover:text-white transition flex items-center gap-2"
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
              <polyline points="3 6 5 6 21 6"></polyline>
              <path
                d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"
              ></path>
            </svg>
            Delete
          </button>
        </div>
      </div>
    </div>

    <div class="px-2 pb-12">
      <SongList v-if="songs.length > 0" :songs="songs" :playlist-id="playlist.id" />
      <div v-else class="py-12 px-6 text-center text-gray-500">
        <div class="text-4xl mb-3 opacity-20">♫</div>
        <p class="text-sm font-medium text-white/80">This playlist is empty.</p>
        <p class="text-xs text-gray-500 mt-1 max-w-sm mx-auto">
          Right-click a song anywhere and choose "Add to playlist".
        </p>

        <!-- Suggested Songs Widget -->
        <div
          class="mt-10 max-w-lg mx-auto text-left bg-[#1d1d1f] border border-[#2d2d2f] rounded-xl p-5 shadow-2xl relative"
        >
          <div class="flex items-center justify-between mb-4 border-b border-[#2d2d2f] pb-3">
            <div>
              <h3 class="text-xs font-semibold text-white uppercase tracking-wider">
                Recommended Songs
              </h3>
              <p class="text-[11px] text-gray-500 mt-0.5">Quick add to your playlist</p>
            </div>
            <button
              @click="getSuggestions"
              class="text-gray-400 hover:text-white transition flex items-center gap-1.5 text-[11px] font-medium bg-[#282828] hover:bg-[#333] px-2.5 py-1 rounded-md border border-[#3a3a3a]"
              title="Refresh suggestions"
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
                class="hover:rotate-180 transition-transform duration-500"
              >
                <path d="M21.5 2v6h-6M21.34 15.57a10 10 0 1 1-.57-8.38l5.67-5.67" />
              </svg>
              Refresh
            </button>
          </div>

          <div v-if="suggestedSongs.length > 0" class="space-y-2">
            <div
              v-for="song in suggestedSongs"
              :key="song.path"
              class="flex items-center justify-between p-2 rounded-lg hover:bg-white/5 transition duration-150 group"
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
                @click="store.addToPlaylist(playlist.id, song.path)"
                class="bg-[#282828] hover:bg-[var(--accent-color)] text-gray-300 hover:text-white border border-[#3a3a3a] hover:border-transparent px-3 py-1 rounded-full text-[11px] font-semibold transition-all duration-150 flex items-center gap-1 shrink-0"
              >
                <svg
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
                Add
              </button>
            </div>
          </div>
          <div v-else class="text-center py-6 text-xs text-gray-600">
            No suggestions available. Try adding more songs to your library.
          </div>
        </div>
      </div>
    </div>
  </div>

  <div v-else class="p-20 text-center text-gray-600">
    <p>Playlist not found.</p>
  </div>
</template>
