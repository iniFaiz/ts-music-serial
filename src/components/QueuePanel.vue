<script setup>
import { ref } from 'vue';
import { store } from '../store';
import CoverImage from './CoverImage.vue';

// Stable per-entry key (by object identity) so TransitionGroup can FLIP-animate
// the reorder. The same references stay in the array across a reorder — only
// their order changes — so each row keeps its key and slides to its new slot.
const keyMap = new WeakMap();
let keySeq = 0;
const keyFor = (item) => {
  let k = keyMap.get(item);
  if (k === undefined) {
    k = ++keySeq;
    keyMap.set(item, k);
  }
  return k;
};

const dragIndex = ref(-1); // row being dragged
const overIndex = ref(-1); // row the cursor is currently over (drop target)

const onDragStart = (index, event) => {
  dragIndex.value = index;
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = 'move';
    event.dataTransfer.setData('text/plain', String(index));
  }
};

// Only update the target indicator while dragging — do NOT mutate the list here.
// Reordering live causes the cursor to land on a shifted element and re-fire
// dragenter, which makes the animation loop/stutter.
const onDragEnter = (index) => {
  if (dragIndex.value !== -1) overIndex.value = index;
};

// Reorder exactly once, on drop. TransitionGroup then animates the single
// change: the dragged row glides to its slot and the others fall into place.
const onDrop = (index) => {
  if (dragIndex.value !== -1 && dragIndex.value !== index) {
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
        <div class="flex items-center gap-3">
          <button
            v-if="store.queue.length > 1"
            @click="store.clearQueue()"
            class="text-xs text-[var(--text-secondary)] hover:text-white transition"
            title="Clear queue"
          >
            Clear
          </button>
          <!-- Unlimited queue / autoplay toggle (∞) -->
          <button
            @click="store.toggleAutoplay()"
            class="transition"
            :class="store.autoplayMode ? 'text-[var(--accent-color)]' : 'text-gray-400 hover:text-white'"
            :title="store.autoplayMode ? 'Autoplay on — keep playing random songs' : 'Autoplay off'"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 12c-2-2.67-4-4-6-4a4 4 0 1 0 0 8c2 0 4-1.33 6-4Zm0 0c2 2.67 4 4 6 4a4 4 0 0 0 0-8c-2 0-4 1.33-6 4Z"/></svg>
          </button>
        </div>
      </div>

      <!-- List -->
      <div class="flex-1 overflow-auto p-2 relative">
        <div v-if="store.queue.length === 0" class="p-8 text-center text-gray-600 text-sm">
          The queue is empty.
        </div>

        <TransitionGroup v-else name="queue" tag="div" class="space-y-1">
          <div
            v-for="(song, index) in store.queue"
            :key="keyFor(song)"
            draggable="true"
            @dragstart="onDragStart(index, $event)"
            @dragenter.prevent="onDragEnter(index)"
            @dragover.prevent
            @drop="onDrop(index)"
            @dragend="onDragEnd"
            @dblclick="store.playQueueIndex(index)"
            class="queue-row group flex items-center gap-3 p-2 rounded-md hover:bg-[#2a2a2a] cursor-grab active:cursor-grabbing transition-colors"
            :class="{
              'bg-[#2a2a2a]': isCurrent(song),
              'opacity-30': index === dragIndex,
              'drop-target': overIndex === index && dragIndex !== index,
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
        </TransitionGroup>
      </div>

      <div class="px-4 py-2 text-[11px] border-t border-[var(--border-color)]">
        <div v-if="store.autoplayMode" class="flex items-center gap-1.5 text-[var(--accent-color)]">
          <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 12c-2-2.67-4-4-6-4a4 4 0 1 0 0 8c2 0 4-1.33 6-4Zm0 0c2 2.67 4 4 6 4a4 4 0 0 0 0-8c-2 0-4 1.33-6 4Z"/></svg>
          <span>Autoplay on · random songs keep playing</span>
        </div>
        <span v-else class="text-gray-600">Drag to reorder · double-click to play</span>
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

/* Drop-target indicator: an accent line above the row the item will land on. */
.drop-target {
  box-shadow: inset 0 2px 0 0 var(--accent-color);
}

/* Reorder: every displaced row glides to its new position (one clean pass). */
.queue-move {
  transition: transform 0.3s cubic-bezier(0.22, 0.61, 0.36, 1);
}
/* Removing a row: it fades and leaves the flow so the rest close the gap. */
.queue-leave-active {
  transition: opacity 0.2s ease;
  position: absolute;
  width: calc(100% - 1rem);
}
.queue-leave-to {
  opacity: 0;
}
</style>
