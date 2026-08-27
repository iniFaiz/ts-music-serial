<script setup>
import { ref } from 'vue';
import { store } from '../store';
import { useFocusTrap } from '../useFocusTrap';

const modalRef = ref(null);

const cancel = () => {
  store.closeConfirm();
};

const confirm = () => {
  if (typeof store.confirmModal.onConfirm === 'function') {
    store.runMutation(store.confirmModal.onConfirm);
  }
  store.closeConfirm();
};

useFocusTrap(modalRef, () => store.confirmModal.open, {
  onEscape: cancel,
});
</script>

<template>
  <Transition name="modal">
    <div
      v-if="store.confirmModal.open"
      class="fixed inset-0 z-[300] flex items-center justify-center"
    >
      <button
        type="button"
        class="fixed inset-0 bg-black/70 backdrop-blur-md cursor-default border-0 w-full h-full"
        tabindex="-1"
        :aria-label="$t('common.close')"
        @click="cancel"
      ></button>
      <div
        ref="modalRef"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="confirm-modal-title"
        aria-describedby="confirm-modal-desc"
        class="relative z-10 modal-panel w-[420px] max-w-[92vw] bg-[#1c1c1e] rounded-2xl shadow-2xl border border-[#2c2c2e] p-6"
      >
        <h2 id="confirm-modal-title" class="text-lg font-bold text-white mb-3">{{ store.confirmModal.title }}</h2>
        <p id="confirm-modal-desc" class="text-sm text-gray-400 mb-6 leading-relaxed">
          {{ store.confirmModal.message }}
        </p>

        <div class="flex justify-end gap-2.5">
          <button
            type="button"
            @click="cancel"
            class="px-4 py-2 rounded-lg text-sm font-medium text-gray-400 hover:text-white bg-[#2c2c2e] hover:bg-[#3a3a3c] transition"
          >
            {{ store.confirmModal.cancelText || $t('common.cancel') }}
          </button>
          <button
            type="button"
            @click="confirm"
            class="px-5 py-2 rounded-lg text-sm font-semibold bg-red-600 hover:bg-red-500 text-white transition shadow-lg"
          >
            {{ store.confirmModal.confirmText || $t('common.confirm') }}
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>

