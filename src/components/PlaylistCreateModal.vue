<script setup>
import { ref, watch, nextTick } from 'vue';
import { useRouter } from 'vue-router';
import { store } from '../store';

const router = useRouter();

const title = ref('');
const description = ref('');
const cover = ref(null); // downscaled JPEG data URL
const fileInput = ref(null);
const titleField = ref(null);

// Focus the title when the modal opens; clear fields when it closes.
// Focus the title when the modal opens; pre-fill fields if editing, or clear if creating.
watch(
  () => store.playlistModal.open,
  async (open) => {
    if (open) {
      if (store.playlistModal.mode === 'edit' && store.playlistModal.playlistId) {
        const pl = store.getPlaylist(store.playlistModal.playlistId);
        if (pl) {
          title.value = pl.name;
          description.value = pl.description || '';
          cover.value = pl.cover || null;
        }
      }
      await nextTick();
      titleField.value?.focus();
    } else {
      title.value = '';
      description.value = '';
      cover.value = null;
    }
  }
);

const pickImage = () => fileInput.value?.click();

const onFile = (e) => {
  const file = e.target.files && e.target.files[0];
  e.target.value = '';
  if (!file) return;
  const reader = new FileReader();
  reader.onload = () => downscale(String(reader.result));
  reader.readAsDataURL(file);
};

// Shrink the picked image to a reasonable thumbnail so it stays small in storage.
const downscale = (dataUrl) => {
  const img = new Image();
  img.onload = () => {
    const max = 400;
    const scale = Math.min(1, max / Math.max(img.width, img.height));
    const w = Math.round(img.width * scale);
    const h = Math.round(img.height * scale);
    const canvas = document.createElement('canvas');
    canvas.width = w;
    canvas.height = h;
    canvas.getContext('2d').drawImage(img, 0, 0, w, h);
    cover.value = canvas.toDataURL('image/jpeg', 0.85);
  };
  img.src = dataUrl;
};

const cancel = () => store.closePlaylistModal();

const save = () => {
  if (store.playlistModal.mode === 'edit' && store.playlistModal.playlistId) {
    store.updatePlaylist(store.playlistModal.playlistId, title.value, description.value, cover.value);
    store.closePlaylistModal();
  } else {
    const pl = store.createPlaylist(title.value, description.value, cover.value);
    const pending = store.playlistModal.pendingSongPath;
    if (pending) store.addToPlaylist(pl.id, pending);
    store.closePlaylistModal();
    router.push('/playlists/' + pl.id);
  }
};
</script>

<template>
  <Transition name="modal">
    <div
      v-if="store.playlistModal.open"
      class="fixed inset-0 z-[200] flex items-center justify-center bg-black/70 backdrop-blur-md"
      @click.self="cancel"
      @keydown.esc="cancel"
    >
      <div
        class="modal-panel w-[520px] max-w-[92vw] bg-[#1c1c1e] rounded-2xl shadow-2xl border border-[#2c2c2e] p-6"
      >
        <h2 class="text-xl font-bold text-white mb-5">{{ store.playlistModal.mode === 'edit' ? 'Edit playlist' : 'Create playlist' }}</h2>

        <div class="flex gap-5">
          <!-- Cover picker -->
          <button
            @click="pickImage"
            class="group relative h-40 w-40 shrink-0 rounded-md overflow-hidden bg-[#2a2a2a] border border-dashed border-[#4a4a4a] hover:border-[var(--accent-color)] transition-colors flex items-center justify-center"
            title="Choose a cover image"
          >
            <img v-if="cover" :src="cover" class="w-full h-full object-cover" alt="" />
            <div
              v-else
              class="flex flex-col items-center gap-2 text-gray-500 group-hover:text-[var(--accent-color)] transition-colors"
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="32"
                height="32"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <line x1="12" y1="5" x2="12" y2="19"></line>
                <line x1="5" y1="12" x2="19" y2="12"></line>
              </svg>
              <span class="text-[11px]">Add cover</span>
            </div>
            <div
              v-if="cover"
              class="absolute inset-0 bg-black/40 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center text-white text-xs"
            >
              Change
            </div>
          </button>

          <!-- Fields -->
          <div class="flex flex-col gap-3 flex-1 min-w-0">
            <input
              ref="titleField"
              v-model="title"
              type="text"
              placeholder="Playlist title"
              maxlength="80"
              @keyup.enter="save"
              class="w-full bg-[#2a2a2a] text-white rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] placeholder-gray-500"
            />
            <textarea
              v-model="description"
              placeholder="Description (optional)"
              rows="5"
              maxlength="300"
              class="w-full flex-1 resize-none bg-[#2a2a2a] text-white rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] placeholder-gray-500"
            ></textarea>
          </div>
        </div>

        <input ref="fileInput" type="file" accept="image/*" class="hidden" @change="onFile" />

        <div class="flex justify-end gap-2.5 mt-6">
          <button
            @click="cancel"
            class="px-4 py-2 rounded-lg text-sm font-medium text-gray-400 hover:text-white bg-[#2c2c2e] hover:bg-[#3a3a3c] transition"
          >
            Cancel
          </button>
          <button
            @click="save"
            class="px-5 py-2 rounded-lg text-sm font-semibold bg-[var(--accent-color)] text-white hover:bg-red-500 transition shadow-lg"
          >
            {{ store.playlistModal.mode === 'edit' ? 'Save' : 'Create' }}
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>
