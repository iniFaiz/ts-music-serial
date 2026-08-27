<script setup>
// Queue sheet for the mini player — mirrors the main QueuePanel's row UX
// (drag-to-reorder via grip, dblclick/click to play, remove, clear, autoplay
// toggle) as a frosted overlay over the mini player content.
//
// Visibility is owned by the parent (`open`); the slide transition is reported
// back through `after-leave` so the parent can unmount bookkeeping.
import { store } from '../store';
import CoverImage from './CoverImage.vue';
import { useQueueReorder } from '../useQueueReorder';

defineProps({ open: { type: Boolean, default: false } });
const emit = defineEmits(['after-leave', 'navigate-artist']);

const {
  listContainer,
  dragIndex,
  overIndex,
  keyFor,
  onQueueLeave,
  disableQueueTransition,
  onGripMouseDown,
} = useQueueReorder(
  () => store.queue.length,
  (from, to) => store.moveInQueue(from, to)
);

const isCurrent = (s) =>
  store.currentSong &&
  (store.currentSong.queueId && s.queueId
    ? store.currentSong.queueId === s.queueId
    : store.currentSong.path === s.path);

// Artist rows route through the main window, so hand navigation to the parent.
const goToArtist = (artistName) => {
  if (!artistName || artistName === 'Unknown Artist') return;
  emit('navigate-artist', artistName);
};
</script>

<template>
  <Transition name="mini-queue" @after-leave="emit('after-leave')">
    <div v-if="open" class="absolute inset-0 z-10 flex flex-col mini-queue-panel">
      <div class="flex items-center justify-between px-4 py-2.5 shrink-0">
        <h2 class="text-sm font-bold">{{ $t('player.queue') }}</h2>
        <div class="flex items-center gap-3">
          <button
            v-if="store.queue.length > 1"
            @click="store.clearQueue()"
            class="text-xs text-[var(--text-secondary)] hover:text-white transition"
            :title="$t('queue.clear')"
          >
            {{ $t('common.clear') }}
          </button>
          <button
            @click="store.toggleAutoplay()"
            class="transition"
            :class="
              store.autoplayMode
                ? 'text-[var(--accent-color)]'
                : 'text-gray-400 hover:text-white'
            "
            :title="store.autoplayMode ? 'Autoplay on' : 'Autoplay off'"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path
                d="M12 12c-2-2.67-4-4-6-4a4 4 0 1 0 0 8c2 0 4-1.33 6-4Zm0 0c2 2.67 4 4 6 4a4 4 0 0 0 0-8c-2 0-4 1.33-6 4Z"
              />
            </svg>
          </button>
        </div>
      </div>
      <div
        ref="listContainer"
        class="relative flex-1 px-2 pt-1 pb-36 overflow-auto mini-scroll"
      >
        <div v-if="store.queue.length === 0" class="p-8 text-sm text-center text-gray-600">
          {{ $t('queue.empty') }}
        </div>
        <TransitionGroup
          v-else
          name="queue"
          :css="!disableQueueTransition"
          @leave="onQueueLeave"
          tag="div"
          class="space-y-1"
        >
          <div
            v-for="(qsong, index) in store.queue"
            :key="keyFor(qsong)"
            :data-queue-idx="index"
            role="row"
            tabindex="0"
            @dblclick="store.playQueueIndex(index)"
            @keydown.enter="store.playQueueIndex(index)"
            class="queue-row group flex items-center gap-2 p-1.5 rounded-md hover:bg-white/10 transition-colors focus:outline-none focus-visible:ring-1 focus-visible:ring-[var(--accent-color)]"
            :class="{
              'bg-white/10': isCurrent(qsong),
              'opacity-30': index === dragIndex,
              'drop-target-above':
                overIndex === index && dragIndex !== index && dragIndex > index,
              'drop-target-below':
                overIndex === index && dragIndex !== index && dragIndex < index,
            }"
          >
            <button
              type="button"
              class="shrink-0 cursor-grab active:cursor-grabbing text-gray-500 hover:text-gray-200 transition-colors bg-transparent border-0 p-0"
              @mousedown="onGripMouseDown(index, $event)"
              :aria-label="$t('queue.dragToReorder')"
              :title="$t('queue.dragToReorder')"
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="currentColor"
                stroke="none"
                aria-hidden="true"
              >
                <circle cx="9" cy="5" r="1.5"></circle>
                <circle cx="15" cy="5" r="1.5"></circle>
                <circle cx="9" cy="12" r="1.5"></circle>
                <circle cx="15" cy="12" r="1.5"></circle>
                <circle cx="9" cy="19" r="1.5"></circle>
                <circle cx="15" cy="19" r="1.5"></circle>
              </svg>
            </button>
            <CoverImage :path="qsong.path" className="h-9 w-9 rounded shrink-0 bg-[#333]" />
            <div class="flex-1 min-w-0">
              <button
                type="button"
                @click="store.playQueueIndex(index)"
                class="text-left w-full truncate bg-transparent border-0 p-0 block"
              >
                <span
                  class="text-[12px] font-medium truncate leading-tight block"
                  :class="isCurrent(qsong) ? 'text-[var(--accent-color)]' : 'text-white'"
                >
                  {{ qsong.title }}
                </span>
              </button>
              <button
                type="button"
                @click.stop="goToArtist(qsong.artist)"
                class="text-left text-[11px] text-[var(--text-secondary)] hover:text-[var(--accent-color)] hover:underline cursor-pointer truncate transition-colors bg-transparent border-0 p-0 block max-w-full"
              >
                {{ qsong.artist }}
              </button>
            </div>
            <button
              type="button"
              @click.stop="store.removeFromQueue(index)"
              class="text-gray-400 transition opacity-0 group-hover:opacity-100 hover:text-white shrink-0"
              :aria-label="$t('queue.removeFromQueue')"
              :title="$t('queue.removeFromQueue')"
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="15"
                height="15"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                aria-hidden="true"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <line x1="18" y1="6" x2="6" y2="18"></line>
                <line x1="6" y1="6" x2="18" y2="18"></line>
              </svg>
            </button>
          </div>
        </TransitionGroup>
      </div>
    </div>
  </Transition>
</template>
