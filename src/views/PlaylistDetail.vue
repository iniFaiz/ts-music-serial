<script setup>
import { computed, ref, nextTick } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { store } from '../store';
import SongList from '../components/SongList.vue';
import CoverImage from '../components/CoverImage.vue';

const route = useRoute();
const router = useRouter();

const playlistId = computed(() => route.params.id);
const playlist = computed(() => store.getPlaylist(playlistId.value));
const songs = computed(() => store.playlistSongs(playlistId.value));

const editing = ref(false);
const nameInput = ref('');
const nameField = ref(null);

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
</script>

<template>
  <div v-if="playlist" class="flex flex-col h-full overflow-auto">
    <!-- Header -->
    <div class="p-8 flex gap-8 items-end bg-gradient-to-b from-[#2a2a2a] to-[var(--app-bg)]">
      <div class="h-52 w-52 shrink-0 rounded-md shadow-2xl overflow-hidden bg-gradient-to-br from-[#4a4a4a] to-[#1f1f1f] flex items-center justify-center">
        <img v-if="playlist.cover" :src="playlist.cover" class="w-full h-full object-cover" alt="" />
        <CoverImage v-else-if="songs.length > 0" :path="songs[0].path" className="w-full h-full" />
        <svg v-else xmlns="http://www.w3.org/2000/svg" width="72" height="72" viewBox="0 0 24 24" fill="none" stroke="#888" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18V5l12-2v13"></path><circle cx="6" cy="18" r="3"></circle><circle cx="18" cy="16" r="3"></circle></svg>
      </div>

      <div class="flex flex-col gap-1 pb-2 overflow-hidden flex-1">
        <h4 class="text-sm font-bold text-[var(--accent-color)] uppercase tracking-wider mb-1">Playlist</h4>

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

        <p v-if="playlist.description" class="text-sm text-[var(--text-secondary)] mt-2 line-clamp-2 max-w-xl">{{ playlist.description }}</p>
        <p class="text-xs text-[var(--text-secondary)] font-medium mt-2">{{ songs.length }} songs</p>

        <div class="flex gap-3 mt-6">
          <button
            @click="playAll"
            :disabled="songs.length === 0"
            class="bg-[var(--accent-color)] text-white px-8 py-2 rounded-[4px] text-sm font-semibold hover:bg-red-500 transition flex items-center gap-2 shadow-lg disabled:opacity-40"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="currentColor" stroke="none"><polygon points="5 3 19 12 5 21 5 3"></polygon></svg>
            Play
          </button>
          <button
            @click="removePlaylist"
            class="bg-[#3a3a3a] text-[var(--text-secondary)] px-6 py-2 rounded-[4px] text-sm font-semibold hover:bg-[#444] hover:text-white transition flex items-center gap-2"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"></polyline><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path></svg>
            Delete
          </button>
        </div>
      </div>
    </div>

    <div class="px-2 pb-12">
      <SongList v-if="songs.length > 0" :songs="songs" :playlist-id="playlist.id" />
      <div v-else class="p-20 text-center text-gray-600">
        <div class="text-4xl mb-4 opacity-20">♫</div>
        <p>This playlist is empty.</p>
        <p class="text-xs mt-2">Right-click a song anywhere and choose "Add to playlist".</p>
      </div>
    </div>
  </div>

  <div v-else class="p-20 text-center text-gray-600">
    <p>Playlist not found.</p>
  </div>
</template>
