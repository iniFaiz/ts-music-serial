<script setup>
import { store } from '../store';

const cancel = () => {
  store.closeConfirm();
};

const confirm = () => {
  if (typeof store.confirmModal.onConfirm === 'function') {
    store.confirmModal.onConfirm();
  }
  store.closeConfirm();
};
</script>

<template>
  <Transition name="modal">
    <div
      v-if="store.confirmModal.open"
      class="fixed inset-0 z-[300] flex items-center justify-center bg-black/70 backdrop-blur-md"
      @click.self="cancel"
      @keydown.esc="cancel"
    >
      <div
        class="modal-panel w-[420px] max-w-[92vw] bg-[#1c1c1e] rounded-2xl shadow-2xl border border-[#2c2c2e] p-6"
      >
        <h2 class="text-lg font-bold text-white mb-3">{{ store.confirmModal.title }}</h2>
        <p class="text-sm text-gray-400 mb-6 leading-relaxed">
          {{ store.confirmModal.message }}
        </p>

        <div class="flex justify-end gap-2.5">
          <button
            @click="cancel"
            class="px-4 py-2 rounded-lg text-sm font-medium text-gray-400 hover:text-white bg-[#2c2c2e] hover:bg-[#3a3a3c] transition"
          >
            {{ store.confirmModal.cancelText }}
          </button>
          <button
            @click="confirm"
            class="px-5 py-2 rounded-lg text-sm font-semibold bg-red-600 hover:bg-red-500 text-white transition shadow-lg"
          >
            {{ store.confirmModal.confirmText }}
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>
