<script setup>
import { ref, watch, nextTick } from 'vue';
import { useRouter } from 'vue-router';
import { invokeCommand as invoke } from '../generated/ipc';
import { store } from '../store';
import { useFocusTrap } from '../useFocusTrap';

const router = useRouter();

const title = ref('');
const description = ref('');
const cover = ref(null); // downscaled JPEG data URL
const titleField = ref(null);
const modalRef = ref(null);
const saving = ref(false);

const cancel = () => store.closePlaylistModal();

useFocusTrap(modalRef, () => store.playlistModal.open, {
  onEscape: cancel,
  initialFocus: titleField,
});

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
  },
  { immediate: true }
);

const pickImage = async () => {
  try {
    const selected = await invoke('pick_playlist_cover');
    if (selected) cover.value = selected;
  } catch (error) {
    console.error('Failed to process playlist cover', error);
    store.showToast(`Could not process cover: ${error}`, { type: 'error' });
  }
};

const save = async () => {
  if (saving.value) return;
  saving.value = true;
  try {
    if (store.playlistModal.mode === 'edit' && store.playlistModal.playlistId) {
      await store.updatePlaylist(
        store.playlistModal.playlistId,
        title.value,
        description.value,
        cover.value
      );
      store.closePlaylistModal();
    } else {
      const pending = store.playlistModal.pendingSongPath;
      const pl = await store.createPlaylist(title.value, description.value, cover.value);
      if (pl && pending) await store.addToPlaylist(pl.id, pending);
      store.closePlaylistModal();
      if (pl) router.push('/playlists/' + pl.id);
    }
  } catch {
    // The store has already rolled back/reloaded and displayed the error toast.
  } finally {
    saving.value = false;
  }
};
</script>

<template>
  <Transition name="modal">
    <div
      v-if="store.playlistModal.open"
      class="fixed inset-0 z-[200] flex items-center justify-center"
    >
      <button
        type="button"
        class="fixed inset-0 bg-black/70 backdrop-blur-md cursor-default border-0 w-full h-full"
        tabindex="-1"
        aria-label="Close playlist dialog"
        @click="cancel"
      ></button>
      <div
        ref="modalRef"
        role="dialog"
        aria-modal="true"
        aria-labelledby="playlist-modal-title"
        class="relative z-10 modal-panel w-[520px] max-w-[92vw] bg-[#1c1c1e] rounded-2xl shadow-2xl border border-[#2c2c2e] p-6"
      >
        <h2 id="playlist-modal-title" class="text-xl font-bold text-white mb-5">
          {{ store.playlistModal.mode === 'edit' ? $t('playlistModal.editTitle') : $t('playlistModal.createTitle') }}
        </h2>

        <div class="flex gap-5">
          <!-- Cover picker -->
          <button
            type="button"
            @click="pickImage"
            class="group relative h-40 w-40 shrink-0 rounded-md overflow-hidden bg-[#2a2a2a] border border-dashed border-[#4a4a4a] hover:border-[var(--accent-color)] transition-colors flex items-center justify-center"
            :aria-label="$t('playlistModal.addCover')"
            :title="$t('playlistModal.addCover')"
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
                aria-hidden="true"
              >
                <line x1="12" y1="5" x2="12" y2="19"></line>
                <line x1="5" y1="12" x2="19" y2="12"></line>
              </svg>
              <span class="text-[11px]">{{ $t('playlistModal.addCover') }}</span>
            </div>
            <div
              v-if="cover"
              class="absolute inset-0 bg-black/40 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center text-white text-xs"
            >
              {{ $t('playlistModal.changeCover') }}
            </div>
          </button>

          <!-- Fields -->
          <div class="flex flex-col gap-3 flex-1 min-w-0">
            <input
              ref="titleField"
              v-model="title"
              type="text"
              :placeholder="$t('playlistModal.namePlaceholder')"
              :aria-label="$t('playlistModal.namePlaceholder')"
              maxlength="80"
              @keyup.enter="save"
              class="w-full bg-[#2a2a2a] text-white rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] placeholder-gray-500"
            />
            <textarea
              v-model="description"
              :placeholder="$t('playlistModal.descPlaceholder')"
              :aria-label="$t('playlistModal.descPlaceholder')"
              rows="5"
              maxlength="300"
              class="w-full flex-1 resize-none bg-[#2a2a2a] text-white rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] placeholder-gray-500"
            ></textarea>
          </div>
        </div>

        <div class="flex justify-end gap-2.5 mt-6">
          <button
            type="button"
            @click="cancel"
            class="px-4 py-2 rounded-lg text-sm font-medium text-gray-400 hover:text-white bg-[#2c2c2e] hover:bg-[#3a3a3c] transition"
          >
            {{ $t('common.cancel') }}
          </button>
          <button
            type="button"
            @click="save"
            :disabled="saving"
            class="px-5 py-2 rounded-lg text-sm font-semibold bg-[var(--accent-color)] text-white hover:bg-red-500 transition shadow-lg disabled:cursor-not-allowed disabled:opacity-50"
          >
            {{ saving ? $t('common.loading') : store.playlistModal.mode === 'edit' ? $t('common.save') : $t('common.create') }}
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>
