<script setup>
import { ref, onMounted, onUnmounted, nextTick } from 'vue';

defineProps({
  title: { type: String, required: true },
  subtitle: { type: String, default: '' },
  // Optional router-link target for the heading + chevron ("see all").
  to: { type: [String, Object], default: null },
});

const scroller = ref(null);
const canLeft = ref(false);
const canRight = ref(false);

const updateArrows = () => {
  const el = scroller.value;
  if (!el) return;
  canLeft.value = el.scrollLeft > 4;
  canRight.value = el.scrollLeft + el.clientWidth < el.scrollWidth - 4;
};

const scrollBy = (dir) => {
  const el = scroller.value;
  if (!el) return;
  el.scrollBy({ left: dir * el.clientWidth * 0.85, behavior: 'smooth' });
};

let ro = null;
onMounted(async () => {
  await nextTick();
  updateArrows();
  if (window.ResizeObserver && scroller.value) {
    ro = new ResizeObserver(updateArrows);
    ro.observe(scroller.value);
  }
});
onUnmounted(() => {
  if (ro) ro.disconnect();
});
</script>

<template>
  <section class="mb-9 group/shelf">
    <div class="flex items-end justify-between mb-4 px-8">
      <component
        :is="to ? 'router-link' : 'div'"
        :to="to || undefined"
        class="group/head inline-flex items-center gap-1.5 text-left"
        :class="to ? 'cursor-pointer' : ''"
      >
        <h2 class="text-xl font-bold tracking-tight text-white group-hover/head:text-[var(--accent-color)] transition-colors">
          {{ title }}
        </h2>
        <svg
          v-if="to"
          xmlns="http://www.w3.org/2000/svg"
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
          class="text-gray-500 group-hover/head:text-[var(--accent-color)] group-hover/head:translate-x-0.5 transition-all"
        >
          <polyline points="9 18 15 12 9 6" />
        </svg>
      </component>

      <!-- Arrow controls (fade in on shelf hover) -->
      <div class="flex items-center gap-2 opacity-0 group-hover/shelf:opacity-100 transition-opacity duration-200">
        <button
          @click="scrollBy(-1)"
          :disabled="!canLeft"
          class="h-7 w-7 rounded-full bg-[#2a2a2a] hover:bg-[#3a3a3a] text-white flex items-center justify-center transition disabled:opacity-30 disabled:cursor-default"
          title="Scroll left"
        >
          <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="15 18 9 12 15 6" />
          </svg>
        </button>
        <button
          @click="scrollBy(1)"
          :disabled="!canRight"
          class="h-7 w-7 rounded-full bg-[#2a2a2a] hover:bg-[#3a3a3a] text-white flex items-center justify-center transition disabled:opacity-30 disabled:cursor-default"
          title="Scroll right"
        >
          <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6" />
          </svg>
        </button>
      </div>
    </div>
    <p v-if="subtitle" class="text-sm text-gray-500 -mt-3 mb-4 px-8">{{ subtitle }}</p>

    <div ref="scroller" @scroll="updateArrows" class="flex gap-5 overflow-x-auto px-8 pb-2 shelf-row">
      <slot />
    </div>
  </section>
</template>

<style scoped>
/* Hide the horizontal scrollbar; arrows + drag handle navigation. Vertical
   wheel is intentionally NOT hijacked, so page scrolling always works. */
.shelf-row {
  scrollbar-width: none;
  scroll-behavior: smooth;
}
.shelf-row::-webkit-scrollbar {
  display: none;
}
</style>
