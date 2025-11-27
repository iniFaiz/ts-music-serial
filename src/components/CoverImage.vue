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
    const result = await invoke('get_track_cover', { path: props.path });
    imageData.value = result;
  } catch (e) {
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
  <div :class="[className, 'flex items-center justify-center overflow-hidden shrink-0 relative border border-white/5']">
    <img 
      v-if="imageData" 
      :src="imageData" 
      class="w-full h-full object-cover"
      alt="" 
      loading="lazy"
    />
    <div v-else class="w-full h-full bg-gradient-to-br from-gray-700 to-gray-800 flex items-center justify-center">
      <svg xmlns="[http://www.w3.org/2000/svg](http://www.w3.org/2000/svg)" class="w-1/2 h-1/2 text-gray-500 opacity-50" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18V5l12-2v13"></path><circle cx="6" cy="18" r="3"></circle><circle cx="18" cy="16" r="3"></circle></svg>
    </div>
  </div>
</template>