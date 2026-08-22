<script setup>
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { store } from '../store';
import { useQueueReorder } from '../useQueueReorder';
import { useRouter } from 'vue-router';
import CoverImage from './CoverImage.vue';
import { navigateWithTransition } from '../viewTransition';

const router = useRouter();

// Stable per-entry key (by object identity) so TransitionGroup can FLIP-animate
// the reorder. The same references stay in the array across a reorder — only
// their order changes — so each row keeps its key and slides to its new slot.
const {
  dragIndex,
  overIndex,
  listContainer,
  disableQueueTransition,
  keyFor,
  onQueueLeave,
  onGripMouseDown,
} = useQueueReorder(
  () => store.queue.length,
  (from, to) => store.moveInQueue(from, to)
);

// Autoplay keeps appending batches forever, so the queue can grow into the
// thousands within one session. Render only the visible window (+buffer) with
// padding that preserves scroll geometry; rows carry their REAL queue index so
// play/remove/reorder targets stay correct. Drag-to-reorder needs every row in
// the DOM for hit testing, so the grip is disabled once windowing kicks in —
// at that size dragging is impractical anyway.
const QUEUE_VIRT_THRESHOLD = 300;
const BUFFER_ROWS = 8;
const rowPitch = ref(58); // remeasured from real rows on first render
const viewStart = ref(0);
const viewEnd = ref(40);
let rafPending = false;

const virtualizeQueue = computed(() => store.queue.length > QUEUE_VIRT_THRESHOLD);

// Rows to render, each carrying its real index in the full queue.
const renderQueue = computed(() => {
  const queue = store.queue;
  if (!virtualizeQueue.value) {
    return queue.map((song, index) => ({ song, index }));
  }
  const total = queue.length;
  const start = Math.max(0, Math.min(viewStart.value, total));
  const end = Math.min(viewEnd.value, total);
  const out = [];
  for (let i = start; i < end; i++) out.push({ song: queue[i], index: i });
  return out;
});

// Pad the scroll content so the rendered slice sits at the right offset and the
// scrollbar reflects the full queue height (null when not windowing).
const virtualPadStyle = computed(() => {
  if (!virtualizeQueue.value) return null;
  const total = store.queue.length;
  const start = Math.max(0, Math.min(viewStart.value, total));
  const end = Math.min(viewEnd.value, total);
  return {
    paddingTop: `${start * rowPitch.value}px`,
    paddingBottom: `${(total - end) * rowPitch.value}px`,
  };
});

const measureRowPitch = () => {
  const el = listContainer.value;
  if (!el || typeof el.querySelectorAll !== 'function') return;
  const rows = el.querySelectorAll('.queue-row');
  if (rows.length >= 2) {
    const d = rows[1].getBoundingClientRect().top - rows[0].getBoundingClientRect().top;
    if (d > 10) rowPitch.value = d;
  } else if (rows.length === 1) {
    const h = rows[0].getBoundingClientRect().height;
    if (h > 10) rowPitch.value = h + 4; // + space-y-1 gap
  }
};

const updateWindow = () => {
  const total = store.queue.length;
  const el = listContainer.value;
  if (!virtualizeQueue.value || !el) return;
  // The padded wrapper starts at the container's content origin, so the number
  // of scrolled-past pixels is simply the container's scrollTop.
  const pitch = rowPitch.value || 58;
  const start = Math.max(0, Math.floor(el.scrollTop / pitch) - BUFFER_ROWS);
  const visible = Math.ceil(el.clientHeight / pitch) + BUFFER_ROWS * 2;
  viewStart.value = start;
  viewEnd.value = Math.min(total, start + visible);
};

const scheduleWindowUpdate = () => {
  if (rafPending) return;
  rafPending = true;
  requestAnimationFrame(() => {
    rafPending = false;
    measureRowPitch();
    updateWindow();
  });
};

watch(
  () => [store.queue.length, virtualizeQueue.value, store.queuePanelOpen],
  () => nextTick(scheduleWindowUpdate)
);

onMounted(() => {
  const el = listContainer.value;
  if (el) el.addEventListener('scroll', scheduleWindowUpdate, { passive: true });
});

onUnmounted(() => {
  const el = listContainer.value;
  if (el) el.removeEventListener('scroll', scheduleWindowUpdate);
  if (rafPending) {
    cancelAnimationFrame(rafPending);
    rafPending = false;
  }
});

const isCurrent = (song) =>
  store.currentSong &&
  (store.currentSong.queueId && song.queueId
    ? store.currentSong.queueId === song.queueId
    : store.currentSong.path === song.path);

const navigateToArtist = (artistName) => {
  if (!artistName || artistName === 'Unknown Artist') return;
  const navigate = () => router.push({ name: 'ArtistDetail', params: { name: artistName } });

  store.queuePanelOpen = false;
  navigateWithTransition(navigate, null);
};
</script>

<template>
  <Transition name="slide">
    <aside
      v-if="store.queuePanelOpen"
      class="absolute top-0 right-0 h-full w-80 bg-[#181818] border-l border-[var(--border-color)] flex flex-col shadow-2xl z-40"
      style="view-transition-name: queue-panel"
    >
      <!-- Header -->
      <div
        class="flex items-center justify-between px-4 py-4 border-b border-[var(--border-color)]"
      >
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
            :class="
              store.autoplayMode ? 'text-[var(--accent-color)]' : 'text-gray-400 hover:text-white'
            "
            :title="store.autoplayMode ? 'Autoplay on — keep playing random songs' : 'Autoplay off'"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="20"
              height="20"
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

      <!-- List -->
      <div ref="listContainer" class="flex-1 overflow-auto p-2 relative">
        <div v-if="store.queue.length === 0" class="p-8 text-center text-gray-600 text-sm">
          The queue is empty.
        </div>

        <TransitionGroup
          v-else
          name="queue"
          :css="!disableQueueTransition"
          @leave="onQueueLeave"
          tag="div"
          class="space-y-1"
          :style="virtualPadStyle"
        >
          <div
            v-for="{ song, index } in renderQueue"
            :key="keyFor(song)"
            :data-queue-idx="index"
            @dblclick="store.playQueueIndex(index)"
            class="queue-row group flex items-center gap-2 p-2 rounded-md hover:bg-[#2a2a2a] transition-colors"
            :class="{
              'bg-[#2a2a2a]': isCurrent(song),
              'opacity-30': index === dragIndex,
              'drop-target-above': overIndex === index && dragIndex !== index && dragIndex > index,
              'drop-target-below': overIndex === index && dragIndex !== index && dragIndex < index,
            }"
          >
            <!-- Drag grip handle -->
            <div
              v-if="!virtualizeQueue"
              class="shrink-0 cursor-grab active:cursor-grabbing text-gray-600 hover:text-gray-300 transition-colors drag-grip"
              @mousedown="onGripMouseDown(index, $event)"
              title="Drag to reorder"
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="currentColor"
                stroke="none"
              >
                <circle cx="9" cy="5" r="1.5"></circle>
                <circle cx="15" cy="5" r="1.5"></circle>
                <circle cx="9" cy="12" r="1.5"></circle>
                <circle cx="15" cy="12" r="1.5"></circle>
                <circle cx="9" cy="19" r="1.5"></circle>
                <circle cx="15" cy="19" r="1.5"></circle>
              </svg>
            </div>
            <CoverImage :path="song.path" className="h-10 w-10 rounded shrink-0 bg-[#333]" />
            <div class="flex-1 min-w-0" @click="store.playQueueIndex(index)">
              <div
                class="text-[13px] font-medium truncate leading-tight"
                :class="isCurrent(song) ? 'text-[var(--accent-color)]' : 'text-white'"
              >
                {{ song.title }}
              </div>
              <div
                @click.stop="navigateToArtist(song.artist)"
                class="text-xs text-[var(--text-secondary)] hover:text-[var(--accent-color)] hover:underline cursor-pointer truncate transition-colors"
              >
                {{ song.artist }}
              </div>
            </div>
            <button
              @click.stop="store.removeFromQueue(index)"
              class="opacity-0 group-hover:opacity-100 text-gray-400 hover:text-white transition shrink-0"
              title="Remove from queue"
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
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

/* Drop-target indicator: accent line showing where the item will land. */
.drop-target-above {
  box-shadow: inset 0 2px 0 0 var(--accent-color);
}
.drop-target-below {
  box-shadow: inset 0 -2px 0 0 var(--accent-color);
}

/* Drag grip pulse on hover */
.drag-grip:hover {
  animation: grip-pulse 0.6s ease-in-out;
}

@keyframes grip-pulse {
  0%,
  100% {
    transform: scale(1);
  }
  50% {
    transform: scale(1.15);
  }
}

/* Reorder: every displaced row glides to its new position (one clean pass). */
.queue-move {
  transition: transform 0.3s cubic-bezier(0.22, 0.61, 0.36, 1);
}
/* Removing a row: it fades and leaves the flow so the rest close the gap. */
.queue-leave-active {
  transition: opacity 0.2s ease;
  position: absolute;
}
.queue-leave-to {
  opacity: 0;
}
</style>
