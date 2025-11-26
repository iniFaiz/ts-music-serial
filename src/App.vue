<script setup>
import { store } from './store';
</script>

<template>
  <div class="flex flex-col h-screen bg-gray-900 text-gray-100 font-sans overflow-hidden">
    <!-- Header -->
    <header class="bg-gray-800 px-4 py-3 flex justify-between items-center shrink-0 shadow-md z-20">
      <div class="flex items-center gap-3">
        <span class="font-bold text-blue-400 tracking-wide">Ts-Music</span>
      </div>
      <div class="flex items-center gap-4">
        <span class="text-xs text-gray-500">{{ store.statusMessage }}</span>
        <button 
          @click="store.selectAndScan()" 
          :disabled="store.loading"
          class="bg-blue-600 hover:bg-blue-500 disabled:bg-gray-700 disabled:cursor-wait text-white text-xs px-3 py-1.5 rounded font-medium transition-colors"
        >
          {{ store.loading ? 'Scanning...' : 'Change Folder' }}
        </button>
      </div>
    </header>

    <!-- Layout -->
    <div class="flex flex-1 overflow-hidden">
      <!-- Sidebar -->
      <nav class="w-48 bg-gray-900 border-r border-gray-800 flex flex-col shrink-0">
        <div class="p-4 space-y-1">
          <router-link to="/songs" active-class="bg-gray-800 text-blue-400" class="block px-3 py-2 rounded text-sm font-medium text-gray-400 hover:text-gray-200 hover:bg-gray-800 transition-colors">
            Songs
          </router-link>
          <router-link to="/albums" active-class="bg-gray-800 text-blue-400" class="block px-3 py-2 rounded text-sm font-medium text-gray-400 hover:text-gray-200 hover:bg-gray-800 transition-colors">
            Albums
          </router-link>
          <router-link to="/artists" active-class="bg-gray-800 text-blue-400" class="block px-3 py-2 rounded text-sm font-medium text-gray-400 hover:text-gray-200 hover:bg-gray-800 transition-colors">
            Artists
          </router-link>
        </div>
      </nav>

      <!-- Content -->
      <main class="flex-1 relative bg-gray-900">
        <router-view></router-view>
      </main>
    </div>
  </div>
</template>