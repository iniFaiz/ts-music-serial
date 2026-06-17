<script setup>
import { ref, watch, onMounted, onUnmounted } from 'vue';
import { useRouter, useRoute } from 'vue-router';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { goBackWithTransition } from '../viewTransition';
import TsLogo from './TsLogo.vue';

const router = useRouter();
const route = useRoute();
const appWindow = getCurrentWindow();

const isMaximized = ref(false);
const canGoBack = ref(false);

// vue-router stores nav history in window.history.state; `back` is null at the
// root of the stack, so we can grey out the button when there's nowhere to go.
const refreshCanGoBack = () => {
  canGoBack.value = !!(window.history.state && window.history.state.back);
};
watch(() => route.fullPath, refreshCanGoBack, { immediate: true });

let unlistenResize = null;
const refreshMaxState = async () => {
  try {
    isMaximized.value = await appWindow.isMaximized();
  } catch {
    /* ignore */
  }
};

onMounted(async () => {
  refreshCanGoBack();
  await refreshMaxState();
  try {
    unlistenResize = await appWindow.onResized(() => refreshMaxState());
  } catch {
    /* window events are best-effort */
  }
});

onUnmounted(() => {
  if (unlistenResize) unlistenResize();
});

const goBack = () => {
  if (canGoBack.value) goBackWithTransition(router);
};

const minimize = () => appWindow.minimize().catch(() => {});
const toggleMaximize = () => appWindow.toggleMaximize().catch(() => {});
const close = () => appWindow.close().catch(() => {});
</script>

<template>
  <!-- Custom title bar: draggable, with the brand on the left and the OS window
       controls on the right (the native frame is disabled in tauri.conf.json). -->
  <div
    data-tauri-drag-region
    class="h-10 shrink-0 flex items-center justify-between bg-[var(--sidebar-bg)] border-b border-[var(--border-color)] select-none"
    style="view-transition-name: title-bar"
  >
    <!-- Brand + back -->
    <div class="flex items-center gap-1 pl-2 pr-4">
      <button
        @click="goBack"
        :disabled="!canGoBack"
        class="p-1.5 rounded-md transition-colors"
        :class="
          canGoBack
            ? 'text-gray-300 hover:text-white hover:bg-white/10'
            : 'text-gray-600 cursor-default'
        "
        title="Back"
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <line x1="19" y1="12" x2="5" y2="12"></line>
          <polyline points="12 19 5 12 12 5"></polyline>
        </svg>
      </button>
      <div class="flex items-center gap-2 pl-1 pointer-events-none">
        <TsLogo :size="20" />
        <span class="text-sm font-semibold text-white tracking-tight">ts-music</span>
      </div>
    </div>

    <!-- Window controls -->
    <div class="flex items-center h-full">
      <button
        @click="minimize"
        class="h-full w-11 flex items-center justify-center text-gray-300 hover:bg-white/10 hover:text-white transition-colors"
        title="Minimize"
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <line x1="5" y1="12" x2="19" y2="12"></line>
        </svg>
      </button>
      <button
        @click="toggleMaximize"
        class="h-full w-11 flex items-center justify-center text-gray-300 hover:bg-white/10 hover:text-white transition-colors"
        :title="isMaximized ? 'Restore' : 'Maximize'"
      >
        <svg
          v-if="isMaximized"
          xmlns="http://www.w3.org/2000/svg"
          width="13"
          height="13"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <rect x="8" y="3" width="13" height="13" rx="1"></rect>
          <path d="M16 16v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h3"></path>
        </svg>
        <svg
          v-else
          xmlns="http://www.w3.org/2000/svg"
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <rect x="4" y="4" width="16" height="16" rx="1"></rect>
        </svg>
      </button>
      <button
        @click="close"
        class="h-full w-11 flex items-center justify-center text-gray-300 hover:bg-[#e81123] hover:text-white transition-colors"
        title="Close"
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
        >
          <line x1="18" y1="6" x2="6" y2="18"></line>
          <line x1="6" y1="6" x2="18" y2="18"></line>
        </svg>
      </button>
    </div>
  </div>
</template>
