<script setup>
import { ref, onMounted, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';

const props = defineProps({
  path: { type: String, required: true },
  className: { type: String, default: "h-10 w-10 rounded" }
});

const imageData = ref(null);
const loaded = ref(false);

async function loadCover() {
  if (!props.path || loaded.value) return;
  
  try {
    // Calls Rust command
    const result = await invoke('get_track_cover', { path: props.path });
    imageData.value = result;
  } catch (e) {
    console.error("Cover load failed", e);
  } finally {
    loaded.value = true;
  }
}

onMounted(loadCover);
watch(() => props.path, () => {
  loaded.value = false;
  loadCover();
});
</script>

<template>
  <div :class="[className, 'bg-gray-800 flex items-center justify-center overflow-hidden shrink-0']">
    <img v-if="imageData" :src="imageData" class="w-full h-full object-cover" alt="Cover" loading="lazy" />
    <span v-else class="text-gray-600 text-xs">🎵</span>
  </div>
</template>