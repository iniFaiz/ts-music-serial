<script setup>
import { store } from './store';
import PlayerControls from './components/PlayerControls.vue';
import QueuePanel from './components/QueuePanel.vue';
import PlaylistCreateModal from './components/PlaylistCreateModal.vue';

function newPlaylist() {
  store.openPlaylistModal();
}
</script>

<template>
  <div class="flex h-screen bg-[var(--app-bg)] text-[var(--text-primary)] font-sans overflow-hidden select-none">
    
    <!-- Sidebar -->
    <nav class="w-64 bg-[var(--sidebar-bg)] border-r border-[var(--border-color)] flex flex-col shrink-0 pt-8 pb-4 px-4 gap-6">
      <!-- Search -->
      <div class="relative">
        <span class="absolute text-gray-500 -translate-y-1/2 left-3 top-1/2">
          <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"></circle><line x1="21" y1="21" x2="16.65" y2="16.65"></line></svg>
        </span>
        <input 
          v-model="store.searchQuery"
          type="text" 
          placeholder="Search" 
          class="w-full bg-[#282828] text-sm text-white rounded-lg py-1.5 pl-9 pr-3 focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] placeholder-gray-500"
        />
      </div>

      <!-- Library -->
      <div class="space-y-1">
        <div class="px-3 mb-2 text-xs font-semibold tracking-wider text-gray-500 uppercase">Library</div>
        
        <router-link to="/songs" active-class="bg-[#282828] text-[var(--accent-color)] font-medium" class="flex items-center gap-3 px-3 py-2 rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors">
          <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18V5l12-2v13"></path><circle cx="6" cy="18" r="3"></circle><circle cx="18" cy="16" r="3"></circle></svg>
          Songs
        </router-link>
        
        <router-link to="/albums" active-class="bg-[#282828] text-[var(--accent-color)] font-medium" class="flex items-center gap-3 px-3 py-2 rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors">
          <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="4" width="16" height="16" rx="2" ry="2"></rect><circle cx="12" cy="12" r="4"></circle><line x1="12" y1="12" x2="12" y2="4"></line></svg>
          Albums
        </router-link>
        
        <router-link to="/artists" active-class="bg-[#282828] text-[var(--accent-color)] font-medium" class="flex items-center gap-3 px-3 py-2 rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors">
          <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z"></path></svg>
          Artists
        </router-link>

        <router-link to="/favorites" active-class="bg-[#282828] text-[var(--accent-color)] font-medium" class="flex items-center gap-3 px-3 py-2 rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors">
          <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="currentColor" stroke="none"><path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"></path></svg>
          Liked Songs
        </router-link>

        <router-link to="/settings" active-class="bg-[#282828] text-[var(--accent-color)] font-medium" class="flex items-center gap-3 px-3 py-2 rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors">
          <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="4" y1="21" x2="4" y2="14"></line><line x1="4" y1="10" x2="4" y2="3"></line><line x1="12" y1="21" x2="12" y2="12"></line><line x1="12" y1="8" x2="12" y2="3"></line><line x1="20" y1="21" x2="20" y2="16"></line><line x1="20" y1="12" x2="20" y2="3"></line><line x1="1" y1="14" x2="7" y2="14"></line><line x1="9" y1="8" x2="15" y2="8"></line><line x1="17" y1="16" x2="23" y2="16"></line></svg>
          Settings
        </router-link>
      </div>

      <!-- Playlists -->
      <div class="space-y-1 overflow-hidden flex flex-col min-h-0 flex-1">
        <div class="flex items-center justify-between px-3 mb-1">
          <span class="text-xs font-semibold tracking-wider text-gray-500 uppercase">Playlists</span>
          <button @click="newPlaylist" class="text-gray-500 hover:text-white transition" title="New playlist">
            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"></line><line x1="5" y1="12" x2="19" y2="12"></line></svg>
          </button>
        </div>
        <div class="overflow-auto flex-1 -mr-1 pr-1">
          <router-link
            v-for="pl in store.playlists"
            :key="pl.id"
            :to="'/playlists/' + pl.id"
            active-class="bg-[#282828] text-white"
            class="flex items-center gap-3 px-3 py-1.5 rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="shrink-0"><path d="M9 18V5l12-2v13"></path><circle cx="6" cy="18" r="3"></circle><circle cx="18" cy="16" r="3"></circle></svg>
            <span class="truncate">{{ pl.name }}</span>
          </router-link>
          <div v-if="store.playlists.length === 0" class="px-3 py-1 text-[11px] text-gray-600">
            No playlists yet
          </div>
        </div>
      </div>

      <div class="px-3 mt-auto mb-4">
        <button 
          @click="store.selectAndScan()" 
          :disabled="store.loading"
          class="w-full group flex items-center gap-3 px-3 py-2 rounded-md text-sm font-medium text-[var(--accent-color)] hover:bg-[#282828] transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <div class="flex items-center justify-center w-5 h-5 rounded bg-[var(--accent-color)]/10 group-hover:bg-[var(--accent-color)]/20 transition-colors">
            <svg v-if="store.loading" class="w-3 h-3 animate-spin" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"></path>
            </svg>
            <svg v-else xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"></line><line x1="5" y1="12" x2="19" y2="12"></line></svg>
          </div>
          
          <span>{{ store.loading ? 'Scanning...' : 'Add Folder' }}</span>
        </button>
        
        <div class="text-[10px] text-gray-600 mt-1 px-3 truncate opacity-70">
          {{ store.statusMessage }}
        </div>
      </div>
    </nav>

    <!-- Main Content -->
    <div class="flex flex-col flex-1 overflow-hidden">
      <!-- Player Controls -->
      <PlayerControls />

      <main class="flex-1 relative overflow-hidden flex flex-col bg-[var(--app-bg)]">
        <div class="flex-1 overflow-auto scroll-smooth">
          <router-view v-slot="{ Component }">
            <keep-alive :include="['SongsView', 'AlbumsView', 'ArtistsView']">
              <component :is="Component" />
            </keep-alive>
          </router-view>
        </div>

        <!-- Up-next queue drawer -->
        <QueuePanel />
      </main>
    </div>

    <!-- Create-playlist modal (global overlay) -->
    <PlaylistCreateModal />
  </div>
</template>