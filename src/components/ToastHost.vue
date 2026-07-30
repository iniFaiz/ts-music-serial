<script setup>
import { store } from '../store';

const iconFor = {
  success: '✓',
  error: '!',
  warning: '!',
  info: 'i',
};
</script>

<template>
  <Teleport to="body">
    <TransitionGroup
      name="toast"
      tag="div"
      class="fixed top-12 right-4 z-[1000000] flex w-[min(380px,calc(100vw-2rem))] flex-col gap-2 pointer-events-none"
      aria-live="polite"
      aria-atomic="false"
    >
      <div
        v-for="toast in store.toasts"
        :key="toast.id"
        class="pointer-events-auto flex items-start gap-3 rounded-xl border border-white/10 bg-[#252527]/95 px-4 py-3 text-sm text-white shadow-2xl backdrop-blur-md"
        :class="{
          'border-red-500/40': toast.type === 'error',
          'border-amber-400/40': toast.type === 'warning',
          'border-emerald-500/40': toast.type === 'success',
        }"
        :role="toast.type === 'error' ? 'alert' : 'status'"
      >
        <span
          class="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-xs font-bold"
          :class="{
            'bg-red-500/20 text-red-300': toast.type === 'error',
            'bg-amber-400/20 text-amber-200': toast.type === 'warning',
            'bg-emerald-500/20 text-emerald-300': toast.type === 'success',
            'bg-sky-500/20 text-sky-300': toast.type === 'info',
          }"
          aria-hidden="true"
        >
          {{ iconFor[toast.type] }}
        </span>
        <span class="min-w-0 flex-1 leading-5">{{ toast.message }}</span>
        <button
          type="button"
          class="-mr-1 rounded p-1 text-gray-400 hover:bg-white/10 hover:text-white"
          aria-label="Dismiss notification"
          @click="store.dismissToast(toast.id)"
        >
          ×
        </button>
      </div>
    </TransitionGroup>
  </Teleport>
</template>

<style scoped>
.toast-enter-active,
.toast-leave-active {
  transition:
    opacity 0.18s ease,
    transform 0.18s ease;
}

.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateX(18px);
}
</style>
