<script setup>
import { ref, watch } from 'vue';
import {
  loadCover,
  loadCoverDataUrl,
  evictCover,
  getCachedCover,
  hasCachedCover,
  coverVersion,
} from '../coverCache';

const props = defineProps({
  path: { type: String, required: true },
  className: { type: String, default: 'h-10 w-10 rounded' },
  // When set, tags this cover with a view-transition-name so it can morph
  // to/from a matching cover on another page (shared-element transition).
  transitionName: { type: String, default: '' },
});

// Hydrate synchronously from the shared cache so a previously seen cover renders
// immediately (no flash) when the component is recreated on page navigation.
const initialCached = props.path && hasCachedCover(props.path) ? getCachedCover(props.path) : null;
const imageData = ref(initialCached);
const isLoaded = ref(!!initialCached);
const fallbackAttempted = ref(false);

async function resolveCover(path) {
  if (!path) {
    imageData.value = null;
    isLoaded.value = false;
    return;
  }
  if (hasCachedCover(path)) {
    const cached = getCachedCover(path);
    imageData.value = cached;
    isLoaded.value = !!cached;
    return;
  }
  const result = await loadCover(path);
  // Guard against a race: the path prop may have changed while awaiting.
  if (props.path === path) {
    imageData.value = result;
    if (!result) isLoaded.value = false;
  }
}

function onImgLoad() {
  isLoaded.value = true;
}

async function handleImageError() {
  const path = props.path;
  if (!path || fallbackAttempted.value) {
    imageData.value = null;
    isLoaded.value = false;
    return;
  }

  fallbackAttempted.value = true;
  evictCover(path);
  imageData.value = null;
  isLoaded.value = false;
  const fallback = await loadCoverDataUrl(path);
  if (props.path === path) {
    imageData.value = fallback;
    if (!fallback) isLoaded.value = false;
  }
}

watch(
  () => props.path,
  (path) => {
    fallbackAttempted.value = false;
    if (path && hasCachedCover(path)) {
      const cached = getCachedCover(path);
      imageData.value = cached;
      isLoaded.value = !!cached;
    } else {
      imageData.value = null;
      isLoaded.value = false;
    }
    resolveCover(path);
  },
  { immediate: true }
);

// Re-resolve after a cover invalidation (tag editor changed the embedded art)
// — the path prop stays the same, so the watcher above wouldn't refire.
watch(coverVersion, () => {
  if (!imageData.value) fallbackAttempted.value = false;
  resolveCover(props.path);
});
</script>

<template>
  <div
    :class="[
      className,
      'cover-image flex items-center justify-center overflow-hidden shrink-0 relative border border-white/5 bg-[#282828]',
    ]"
    :style="transitionName ? { viewTransitionName: transitionName } : null"
  >
    <!-- Placeholder SVG when image is loading or missing -->
    <div
      v-if="!isLoaded"
      class="absolute inset-0 bg-gradient-to-br from-gray-700/60 to-gray-800/60 flex items-center justify-center pointer-events-none"
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        class="w-1/2 h-1/2 text-gray-500 opacity-40"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <path d="M9 18V5l12-2v13"></path>
        <circle cx="6" cy="18" r="3"></circle>
        <circle cx="18" cy="16" r="3"></circle>
      </svg>
    </div>

    <img
      v-if="imageData"
      :src="imageData"
      class="w-full h-full object-cover relative z-10 transition-opacity duration-200"
      :class="isLoaded ? 'opacity-100' : 'opacity-0'"
      alt=""
      draggable="false"
      @load="onImgLoad"
      @error="handleImageError"
    />
  </div>
</template>
