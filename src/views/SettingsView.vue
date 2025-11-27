<template>
  <div class="p-6 max-w-2xl mx-auto">
    <h1 class="text-2xl font-bold text-white mb-6">Settings</h1>
    
    <div class="bg-gray-800 rounded-lg p-6 shadow-lg">
      <h2 class="text-xl font-semibold text-white mb-4">Performance</h2>
      
      <div class="space-y-4">
        <ToggleInt 
          v-model="store.useParallelism" 
          label="Use Parallel Processing (Faster)" 
        />
        
        <p class="text-sm text-gray-400 mt-2">
          When enabled, the application will use multiple CPU threads to scan and parse music files. 
          Disable this if you experience system instability or high CPU usage during scans.
        </p>
      </div>
    </div>

    <div class="bg-gray-800 rounded-lg p-6 shadow-lg mt-6">
      <h2 class="text-xl font-semibold text-white mb-4">Library Management</h2>
      
      <div class="space-y-4">
        <div class="flex items-center justify-between">
          <div>
            <h3 class="text-white font-medium">Reset Library</h3>
            <p class="text-sm text-gray-400">Clear all songs, albums, and artists from the database.</p>
          </div>
          <button 
            @click="confirmReset" 
            class="px-4 py-2 bg-red-600 hover:bg-red-700 text-white rounded-md transition-colors text-sm font-medium"
          >
            Reset Library
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { store } from '../store';
import ToggleInt from '../components/settings/ToggleInt.vue';
import { confirm } from '@tauri-apps/plugin-dialog';

const confirmReset = async () => {
  const yes = await confirm(
    'Are you sure you want to delete all library data? This cannot be undone.',
    { title: 'Reset Library', kind: 'warning' }
  );
  
  if (yes) {
    store.resetLibrary();
  }
};
</script>