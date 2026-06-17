<script setup>
import { ref } from 'vue';
import { store } from '../store';
import CoverImage from './CoverImage.vue';

// Index currently being dragged, for reordering.
const dragIndex = ref(-1);
const overIndex = ref(-1);

const onDragStart = (index) => {
  dragIndex.value = index;
};

const onDragOver = (index, event) => {
  event.preventDefault();
  overIndex.value = index;
};

const onDrop = (index) => {
  if (dragIndex.value >= 0 && dragIndex.value !== index) {
    store.moveInQueue(dragIndex.value, index);
  }
  dragIndex.value = -1;
  overIndex.value = -1;
};

const onDragEnd = () => {
  dragIndex.value = -1;
  overIndex.value = -1;
};

const isCurrent = (song) =>
  store.currentSong && store.currentSong.path === song.path;
</script>

<template>
  <Transition name="slide">
    <aside
      v-if="store.queuePanelOpen"
      class="absolute top-0 right-0 h-full w-80 bg-[#181818] border-l border-[var(--border-color)] flex flex-col shadow-2xl z-40"
    >
      <!-- Header -->
      <div class="flex items-center justify-between px-4 py-4 border-b border-[var(--border-color)]">
        <h2 class="text-base font-bold text-white">Queue</h2>
        <div class="flex items-center gap-2">
          <button
            v-if="store.queue.length > 1"
            @click="store.clearQueue()"
            class="text-xs text-[var(--text-secondary)] hover:text-white transition"
            title="Clear queue"
          >
            Clear
          </button>
          <button
            @click="store.queuePanelOpen = false"
            class="text-gray-400 hover:text-white transition"
            title="Close"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>
          </button>
        </div>
      </div>

      <!-- List -->
      <div class="flex-1 overflow-auto p-2">
        <div v-if="store.queue.length === 0" class="p-8 text-center text-gray-600 text-sm">
          The queue is empty.
        </div>

        <div
          v-for="(song, index) in store.queue"
          :key="song.path + '-' + index"
          draggable="true"
          @dragstart="onDragStart(index)"
          @dragover="onDragOver(index, $event)"
          @drop="onDrop(index)"
          @dragend="onDragEnd"
          @dblclick="store.playQueueIndex(index)"
          class="group flex items-center gap-3 p-2 rounded-md hover:bg-[#2a2a2a] cursor-grab active:cursor-grabbing transition-colors"
          :class="{
            'bg-[#2a2a2a]': isCurrent(song),
            'ring-1 ring-[var(--accent-color)]': overIndex === index && dragIndex !== index,
          }"
        >
          <CoverImage :path="song.path" className="h-10 w-10 rounded shrink-0 bg-[#333]" />
          <div class="flex-1 min-w-0" @click="store.playQueueIndex(index)">
            <div
              class="text-[13px] font-medium truncate leading-tight"
              :class="isCurrent(song) ? 'text-[var(--accent-color)]' : 'text-white'"
            >
              {{ song.title }}
            </div>
            <div class="text-xs text-[var(--text-secondary)] truncate">{{ song.artist }}</div>
          </div>
          <button
            @click.stop="store.removeFromQueue(index)"
            class="opacity-0 group-hover:opacity-100 text-gray-400 hover:text-white transition shrink-0"
            title="Remove from queue"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>
          </button>
        </div>
      </div>

      <div class="px-4 py-2 text-[11px] text-gray-600 border-t border-[var(--border-color)]">
        Drag to reorder · double-click to play
      </div>
    </aside>
  </Transition>
</template>

<style scoped>
.slide-enter-active,
.slide-leave-active {
  transition: transform 0.28s cubic-bezier(0.4, 0, 0.2, 1);
}
.slide-enter-from,
.slide-leave-to {
  transform: translateX(100%);
}
</style>
