<script setup>
import { ref, onMounted, onUnmounted, nextTick, watch } from 'vue';
import { useRouter } from 'vue-router';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { listen } from '@tauri-apps/api/event';
import { store } from './store';
import PlayerControls from './components/PlayerControls.vue';
import QueuePanel from './components/QueuePanel.vue';
import PlaylistCreateModal from './components/PlaylistCreateModal.vue';
import SmartPlaylistModal from './components/SmartPlaylistModal.vue';
import CommandPalette from './components/CommandPalette.vue';
import PlaylistCover from './components/PlaylistCover.vue';
import TitleBar from './components/TitleBar.vue';
import FullScreenPlayer from './components/FullScreenPlayer.vue';
import MiniPlayer from './components/MiniPlayer.vue';
import LyricsPanel from './components/LyricsPanel.vue';
import { navigateWithTransition, smartBack, goForwardWithTransition } from './viewTransition';

const router = useRouter();

// Navigate to /songs when typing in search
watch(
  () => store.searchQuery,
  (query) => {
    if (query && router.currentRoute.value.path !== '/songs') {
      router.push('/songs');
    }
  }
);

// ---- Global keyboard shortcuts ----
const SEEK_STEP = 5; // seconds for ←/→
const SEEK_STEP_BIG = 10; // seconds for Shift+←/→
const VOLUME_STEP = 0.05; // 5% for ↑/↓
const VOLUME_STEP_BIG = 0.1; // 10% for Shift+↑/↓

// True when the keystroke is headed into a text field / editable area, where the
// single-key media shortcuts must not hijack what the user is typing.
const isTypingTarget = (e) => {
  const el = e.target;
  if (!el) return false;
  const tag = el.tagName;
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable;
};

// Seek relative to the current position, clamped to the track length.
const seekBy = (delta) => {
  if (!store.currentSong) return;
  let target = (store.currentTime || 0) + delta;
  target = Math.max(0, target);
  if (store.duration > 0) target = Math.min(store.duration, target);
  store.seek(target);
};

// Nudge the volume (0..1), rounded to avoid float drift.
const bumpVolume = (delta) => {
  const v = Math.min(1, Math.max(0, Math.round(((store.volume || 0) + delta) * 100) / 100));
  store.setVolume(v);
};

const handleKeydown = (e) => {
  // Window-mode toggles + Escape work everywhere, even inside inputs.
  // Ctrl+Shift+F toggles the fullscreen Now-Playing view and enters native monitor
  // fullscreen. Ignored while the mini player is open (the two window modes conflict).
  if (e.ctrlKey && e.shiftKey && (e.key === 'F' || e.key === 'f')) {
    e.preventDefault();
    if (!store.miniPlayerOpen) store.toggleFullscreen();
    return;
  }
  // Ctrl+Shift+M toggles the Apple-Music-style compact mini player.
  if (e.ctrlKey && e.shiftKey && (e.key === 'M' || e.key === 'm')) {
    e.preventDefault();
    store.toggleMiniPlayer();
    return;
  }
  // Ctrl/Cmd+K opens the command palette (works even inside text inputs).
  if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && (e.key === 'k' || e.key === 'K')) {
    e.preventDefault();
    store.toggleCommandPalette();
    return;
  }
  if (e.key === 'Escape' && store.commandPaletteOpen) {
    store.closeCommandPalette();
    return;
  }
  if (e.key === 'Escape' && store.miniPlayerOpen) {
    store.exitMiniPlayer();
    return;
  }
  if (e.key === 'Escape' && store.fullscreenOpen) {
    store.exitFullscreenWithTransition();
    return;
  }

  // Everything below is a media shortcut — never steal keystrokes from a text box.
  if (isTypingTarget(e)) return;

  // Track navigation: Ctrl/Cmd + ←/→ (hardware media keys are left to the OS/SMTC
  // layer so they aren't handled twice).
  if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key === 'ArrowRight') {
    e.preventDefault();
    store.nextSong(true);
    return;
  }
  if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key === 'ArrowLeft') {
    e.preventDefault();
    store.prevSong();
    return;
  }

  // Past here we only want bare keys (optionally with Shift) — let any other
  // Ctrl/Cmd/Alt chord fall through to the browser/app.
  if (e.ctrlKey || e.metaKey || e.altKey) return;

  // Number row 1..0 → jump to 10%..100% (0 = start). Matches YouTube/most players.
  if (e.code.length === 6 && e.code.startsWith('Digit')) {
    if (store.currentSong && store.duration > 0) {
      e.preventDefault();
      const n = Number(e.code.slice(5));
      store.seek((store.duration * n) / 10);
    }
    return;
  }

  switch (e.code) {
    // Play / pause. K is the YouTube-style alias. Always toggle (and
    // preventDefault) even when a button has focus — otherwise Space activates
    // that button instead. This notably bit when restoring from the taskbar with
    // the minimize/next button still focused: Space would re-minimize / skip.
    // preventDefault stops the focused button from also firing, so no double-action.
    case 'Space':
    case 'KeyK': {
      e.preventDefault();
      store.togglePlay();
      return;
    }
    case 'ArrowRight':
      e.preventDefault();
      seekBy(e.shiftKey ? SEEK_STEP_BIG : SEEK_STEP);
      return;
    case 'ArrowLeft':
      e.preventDefault();
      seekBy(e.shiftKey ? -SEEK_STEP_BIG : -SEEK_STEP);
      return;
    case 'ArrowUp':
      e.preventDefault();
      bumpVolume(e.shiftKey ? VOLUME_STEP_BIG : VOLUME_STEP);
      return;
    case 'ArrowDown':
      e.preventDefault();
      bumpVolume(e.shiftKey ? -VOLUME_STEP_BIG : -VOLUME_STEP);
      return;
    case 'Home':
      if (store.currentSong) {
        e.preventDefault();
        store.seek(0);
      }
      return;
    case 'KeyM':
      e.preventDefault();
      store.toggleMute();
      return;
    case 'KeyS':
      e.preventDefault();
      store.toggleShuffle();
      return;
    case 'KeyR':
      e.preventDefault();
      store.toggleLoop();
      return;
    case 'KeyL':
      // Like / unlike the current track.
      if (store.currentSong) {
        e.preventDefault();
        store.toggleFavorite(store.currentSong.path);
      }
      return;
    default:
      return;
  }
};

// ---- Drag & drop folders/files onto the window ----
let unlistenDrop = null;
// ---- Filesystem watcher → debounced library refresh ----
let unlistenLibraryChanged = null;
let unlistenExclusiveErr = null;
let refreshTimer = null;

const scrollContainer = ref(null);
const scrollPositions = new Map();
const horizontalScrollPositions = new Map();

router.beforeEach((to, from) => {
  if (scrollContainer.value) {
    const container =
      scrollContainer.value.querySelector('.overflow-auto') || scrollContainer.value;
    scrollPositions.set(from.fullPath, container.scrollTop);

    // Save horizontal scroll positions
    const horizontalShelves = scrollContainer.value.querySelectorAll('.shelf-row');
    const horizPos = [];
    horizontalShelves.forEach((el) => {
      const section = el.closest('section');
      const titleEl = section ? section.querySelector('h2') : null;
      const title = titleEl ? titleEl.textContent.trim() : '';
      horizPos.push({
        title,
        scrollLeft: el.scrollLeft,
      });
    });
    horizontalScrollPositions.set(from.fullPath, horizPos);
  }
});

router.afterEach((to) => {
  nextTick(() => {
    if (scrollContainer.value) {
      const container =
        scrollContainer.value.querySelector('.overflow-auto') || scrollContainer.value;

      // Detail pages should always start scrolled to the top
      const isDetailPage = [
        'AlbumDetail',
        'ArtistDetail',
        'PlaylistDetail',
        'SmartPlaylistDetail',
        'CollectionDetail',
      ].includes(to.name);
      const pos = isDetailPage ? 0 : scrollPositions.get(to.fullPath) || 0;

      const originalBehavior = container.style.scrollBehavior;
      container.style.scrollBehavior = 'auto';
      container.scrollTop = pos;
      container.style.scrollBehavior = originalBehavior;

      // Restore horizontal scroll positions if not a detail page
      if (!isDetailPage) {
        const horizPos = horizontalScrollPositions.get(to.fullPath);
        if (horizPos && horizPos.length > 0) {
          nextTick(() => {
            const horizontalShelves = scrollContainer.value.querySelectorAll('.shelf-row');
            horizontalShelves.forEach((el, index) => {
              const section = el.closest('section');
              const titleEl = section ? section.querySelector('h2') : null;
              const title = titleEl ? titleEl.textContent.trim() : '';
              const match = horizPos.find((p) => p.title === title) || horizPos[index];
              if (match) {
                const orig = el.style.scrollBehavior;
                el.style.scrollBehavior = 'auto';
                el.scrollLeft = match.scrollLeft;
                el.style.scrollBehavior = orig;
              }
            });
          });
        }
      }
    }
  });
});

// Collapse the sidebar to an icon-only rail when the window gets too narrow.
// Changed from 980 to 1024 to collapse the sidebar earlier and give player controls more room on medium widths.
const COMPACT_BREAKPOINT = 1125;
const compact = ref(false);
const updateCompact = () => {
  compact.value = window.innerWidth < COMPACT_BREAKPOINT;
};

const handleMouseUp = (e) => {
  if (e.button === 3) {
    e.preventDefault();
    const canGoBack = !!(window.history.state && window.history.state.back);
    if (canGoBack) {
      smartBack(router);
    }
  } else if (e.button === 4) {
    e.preventDefault();
    const canGoForward = !!(window.history.state && window.history.state.forward);
    if (canGoForward) {
      goForwardWithTransition(router);
    }
  }
};

const handleMouseDown = (e) => {
  if (e.button === 3 || e.button === 4) {
    e.preventDefault();
  }
};

const handleAuxClick = (e) => {
  if (e.button === 3 || e.button === 4) {
    e.preventDefault();
  }
};

onMounted(async () => {
  updateCompact();
  window.addEventListener('resize', updateCompact);
  window.addEventListener('mouseup', handleMouseUp);
  window.addEventListener('mousedown', handleMouseDown);
  window.addEventListener('auxclick', handleAuxClick);
  window.addEventListener('keydown', handleKeydown);

  // Drag & drop: highlight while hovering, add the dropped paths on release.
  try {
    unlistenDrop = await getCurrentWebview().onDragDropEvent((event) => {
      const t = event.payload.type;
      if (t === 'enter' || t === 'over') {
        store.dragActive = true;
      } else if (t === 'leave') {
        store.dragActive = false;
      } else if (t === 'drop') {
        store.dragActive = false;
        if (event.payload.paths && event.payload.paths.length) {
          store.addPaths(event.payload.paths);
        }
      }
    });
  } catch {
    // drag-drop best-effort
  }

  // Auto-refresh the library when watched folders change on disk (debounced
  // again on the JS side so several backend events collapse into one refresh).
  try {
    unlistenLibraryChanged = await listen('library-changed', () => {
      if (refreshTimer) clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => {
        if (!store.loading) store.refreshLibrary();
      }, 600);
    });
  } catch {
    // watcher best-effort
  }

  // Surface WASAPI-exclusive fallback so the user knows it dropped to shared mode.
  try {
    unlistenExclusiveErr = await listen('wasapi-exclusive-error', (e) => {
      const msg = e && e.payload ? `: ${e.payload}` : '';
      store.statusMessage = `WASAPI exclusive unavailable — using shared mode${msg}`;
      // The backend has already disabled exclusive mode; sync the frontend.
      if (store.wasapiExclusive) {
        store.wasapiExclusive = false;
        store.persistState();
      }
    });
  } catch {
    // best-effort
  }
});

onUnmounted(() => {
  window.removeEventListener('resize', updateCompact);
  window.removeEventListener('mouseup', handleMouseUp);
  window.removeEventListener('mousedown', handleMouseDown);
  window.removeEventListener('auxclick', handleAuxClick);
  window.removeEventListener('keydown', handleKeydown);
  if (unlistenDrop) unlistenDrop();
  if (unlistenLibraryChanged) unlistenLibraryChanged();
  if (unlistenExclusiveErr) unlistenExclusiveErr();
  if (refreshTimer) clearTimeout(refreshTimer);
  // Cleanup sidebar playlist drag
  document.removeEventListener('mousemove', onSidebarPlMouseMove);
  document.removeEventListener('mouseup', onSidebarPlMouseUp);
  document.body.style.userSelect = '';
  document.body.style.cursor = '';
});

function newPlaylist() {
  store.openPlaylistModal();
}

// ---- Sidebar playlist drag-to-reorder ----
const sidebarPlDragIndex = ref(-1);
const sidebarPlOverIndex = ref(-1);
const sidebarPlDragActive = ref(false);
const sidebarPlContainer = ref(null);
let sidebarPlStartY = 0;
let sidebarPlPendingIdx = -1;
const SIDEBAR_DRAG_THRESHOLD = 5;
let sidebarPlDragDidReorder = false;

const getSidebarPlRowIndex = (clientY) => {
  if (!sidebarPlContainer.value) return -1;
  const rows = sidebarPlContainer.value.querySelectorAll('[data-sidebar-pl-idx]');
  for (const row of rows) {
    const rect = row.getBoundingClientRect();
    if (clientY >= rect.top && clientY <= rect.bottom) {
      return parseInt(row.dataset.sidebarPlIdx, 10);
    }
  }
  if (rows.length > 0) {
    const firstRect = rows[0].getBoundingClientRect();
    if (clientY < firstRect.top) return 0;
    const lastRect = rows[rows.length - 1].getBoundingClientRect();
    if (clientY > lastRect.bottom) return rows.length - 1;
  }
  return -1;
};

const onSidebarPlMouseMove = (e) => {
  if (sidebarPlPendingIdx === -1) return;
  const dy = Math.abs(e.clientY - sidebarPlStartY);
  if (!sidebarPlDragActive.value && dy >= SIDEBAR_DRAG_THRESHOLD) {
    sidebarPlDragActive.value = true;
    sidebarPlDragIndex.value = sidebarPlPendingIdx;
    sidebarPlOverIndex.value = sidebarPlPendingIdx;
    sidebarPlDragDidReorder = true;
    document.body.style.userSelect = 'none';
    document.body.style.cursor = 'grabbing';
  }
  if (sidebarPlDragActive.value) {
    e.preventDefault();
    const idx = getSidebarPlRowIndex(e.clientY);
    if (idx !== -1) sidebarPlOverIndex.value = idx;
  }
};

const onSidebarPlMouseUp = () => {
  if (sidebarPlDragActive.value && sidebarPlDragIndex.value !== -1 && sidebarPlOverIndex.value !== -1 && sidebarPlDragIndex.value !== sidebarPlOverIndex.value) {
    store.movePlaylistOrder(sidebarPlDragIndex.value, sidebarPlOverIndex.value);
  }
  sidebarPlDragIndex.value = -1;
  sidebarPlOverIndex.value = -1;
  sidebarPlDragActive.value = false;
  sidebarPlPendingIdx = -1;
  document.removeEventListener('mousemove', onSidebarPlMouseMove);
  document.removeEventListener('mouseup', onSidebarPlMouseUp);
  document.body.style.userSelect = '';
  document.body.style.cursor = '';
  setTimeout(() => {
    sidebarPlDragDidReorder = false;
  }, 50);
};

const onSidebarPlMouseDown = (index, e) => {
  if (e.target.closest('button')) return;
  sidebarPlPendingIdx = index;
  sidebarPlStartY = e.clientY;
  sidebarPlDragDidReorder = false;
  document.addEventListener('mousemove', onSidebarPlMouseMove);
  document.addEventListener('mouseup', onSidebarPlMouseUp);
};

const navigatePlaylist = (pl, event) => {
  if (sidebarPlDragDidReorder) {
    sidebarPlDragDidReorder = false;
    return;
  }
  const coverEl = event.currentTarget.querySelector('.h-7') || event.currentTarget.querySelector('img') || event.currentTarget.firstElementChild;
  const path = store.isSmart(pl) ? '/smart/' + pl.id : '/playlists/' + pl.id;
  navigateWithTransition(() => router.push(path), coverEl, 'shared-cover', 'to-album-transition');
};

</script>

<template>
  <div
    class="flex flex-col h-screen bg-[var(--app-bg)] text-[var(--text-primary)] font-sans overflow-hidden select-none"
  >
    <!-- Custom window title bar (back, brand, window controls) -->
    <TitleBar />

    <div class="flex flex-1 overflow-hidden">
      <!-- Sidebar -->
      <nav
        class="bg-[var(--sidebar-bg)] border-r border-[var(--border-color)] flex flex-col shrink-0 pt-4 pb-4 gap-5 transition-[width] duration-200 ease-out"
        :class="compact ? 'w-16 px-2' : 'w-64 px-4'"
        @click="store.queuePanelOpen = false; store.lyricsPanelOpen = false"
      >
        <!-- Search -->
        <div v-if="!compact" class="relative">
          <span class="absolute text-gray-500 -translate-y-1/2 left-3 top-1/2">
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="3"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <circle cx="11" cy="11" r="8"></circle>
              <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
            </svg>
          </span>
          <input
            v-model="store.searchQuery"
            type="text"
            placeholder="Search"
            class="w-full bg-[#282828] text-sm text-white rounded-lg py-1.5 pl-9 pr-3 focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] placeholder-gray-500"
          />
        </div>
        <div v-else class="flex justify-center py-1 text-gray-500" title="Search">
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2.5"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <circle cx="11" cy="11" r="8"></circle>
            <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
          </svg>
        </div>

        <!-- Top-level navigation -->
        <div class="space-y-1">
          <router-link
            to="/home"
            active-class="bg-[#282828] text-[var(--accent-color)] font-medium"
            class="flex items-center rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors"
            :class="compact ? 'justify-center py-2.5' : 'gap-3 px-3 py-2'"
            :title="compact ? 'Home' : null"
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
              <path d="M3 9.5L12 3l9 6.5"></path>
              <path d="M5 10v10a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V10"></path>
              <path d="M9 21v-6h6v6"></path>
            </svg>
            <span v-if="!compact">Home</span>
          </router-link>
        </div>

        <!-- Library -->
        <div class="space-y-1">
          <div
            v-if="!compact"
            class="px-3 mb-2 text-xs font-semibold tracking-wider text-gray-500 uppercase"
          >
            Library
          </div>

          <router-link
            to="/songs"
            active-class="bg-[#282828] text-[var(--accent-color)] font-medium"
            class="flex items-center rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors"
            :class="compact ? 'justify-center py-2.5' : 'gap-3 px-3 py-2'"
            :title="compact ? 'Songs' : null"
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
              <path d="M9 18V5l12-2v13"></path>
              <circle cx="6" cy="18" r="3"></circle>
              <circle cx="18" cy="16" r="3"></circle>
            </svg>
            <span v-if="!compact">Songs</span>
          </router-link>

          <router-link
            to="/albums"
            active-class="bg-[#282828] text-[var(--accent-color)] font-medium"
            class="flex items-center rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors"
            :class="compact ? 'justify-center py-2.5' : 'gap-3 px-3 py-2'"
            :title="compact ? 'Albums' : null"
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
              <rect x="4" y="4" width="16" height="16" rx="2" ry="2"></rect>
              <circle cx="12" cy="12" r="4"></circle>
              <line x1="12" y1="12" x2="12" y2="4"></line>
            </svg>
            <span v-if="!compact">Albums</span>
          </router-link>

          <router-link
            to="/artists"
            active-class="bg-[#282828] text-[var(--accent-color)] font-medium"
            class="flex items-center rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors"
            :class="compact ? 'justify-center py-2.5' : 'gap-3 px-3 py-2'"
            :title="compact ? 'Artists' : null"
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
                d="M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z"
              ></path>
            </svg>
            <span v-if="!compact">Artists</span>
          </router-link>

          <router-link
            to="/favorites"
            active-class="bg-[#282828] text-[var(--accent-color)] font-medium"
            class="flex items-center rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors"
            :class="compact ? 'justify-center py-2.5' : 'gap-3 px-3 py-2'"
            :title="compact ? 'Liked Songs' : null"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="currentColor"
              stroke="none"
            >
              <path
                d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"
              ></path>
            </svg>
            <span v-if="!compact">Liked Songs</span>
          </router-link>

          <router-link
            to="/settings"
            active-class="bg-[#282828] text-[var(--accent-color)] font-medium"
            class="flex items-center rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors"
            :class="compact ? 'justify-center py-2.5' : 'gap-3 px-3 py-2'"
            :title="compact ? 'Settings' : null"
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
              <line x1="4" y1="21" x2="4" y2="14"></line>
              <line x1="4" y1="10" x2="4" y2="3"></line>
              <line x1="12" y1="21" x2="12" y2="12"></line>
              <line x1="12" y1="8" x2="12" y2="3"></line>
              <line x1="20" y1="21" x2="20" y2="16"></line>
              <line x1="20" y1="12" x2="20" y2="3"></line>
              <line x1="1" y1="14" x2="7" y2="14"></line>
              <line x1="9" y1="8" x2="15" y2="8"></line>
              <line x1="17" y1="16" x2="23" y2="16"></line>
            </svg>
            <span v-if="!compact">Settings</span>
          </router-link>
        </div>

        <!-- Playlists -->
        <div class="flex flex-col flex-1 min-h-0 space-y-1 overflow-hidden">
          <div
            class="flex items-center mb-1"
            :class="compact ? 'justify-center px-1' : 'justify-between px-3'"
          >
            <span
              v-if="!compact"
              class="text-xs font-semibold tracking-wider text-gray-500 uppercase"
              >Playlists</span
            >
            <button
              @click="newPlaylist"
              class="text-gray-500 transition hover:text-white"
              title="New playlist"
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2.5"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <line x1="12" y1="5" x2="12" y2="19"></line>
                <line x1="5" y1="12" x2="19" y2="12"></line>
              </svg>
            </button>
          </div>
          <div ref="sidebarPlContainer" class="flex-1 pr-1 -mr-1 overflow-auto">
            <!-- Link to All Playlists -->
            <router-link
              to="/playlists"
              active-class="bg-[#282828] text-[var(--accent-color)] font-medium"
              class="flex items-center rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors mb-1"
              :class="compact ? 'justify-center py-1.5' : 'gap-3 px-2 py-1.5'"
              :title="compact ? 'All Playlists' : null"
            >
              <div
                class="h-7 w-7 rounded shrink-0 flex items-center justify-center bg-[#282828] text-gray-400 group-hover:text-white"
              >
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  width="14"
                  height="14"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <line x1="8" y1="6" x2="21" y2="6"></line>
                  <line x1="8" y1="12" x2="21" y2="12"></line>
                  <line x1="8" y1="18" x2="21" y2="18"></line>
                  <line x1="3" y1="6" x2="3.01" y2="6"></line>
                  <line x1="3" y1="12" x2="3.01" y2="12"></line>
                  <line x1="3" y1="18" x2="3.01" y2="18"></line>
                </svg>
              </div>
              <span v-if="!compact" class="truncate">All Playlists</span>
            </router-link>

            <TransitionGroup name="sidebar-pl" tag="div">
            <router-link
              v-for="(pl, plIdx) in store.playlists"
              :key="pl.id"
              :data-sidebar-pl-idx="plIdx"
              :to="store.isSmart(pl) ? '/smart/' + pl.id : '/playlists/' + pl.id"
              active-class="bg-[#282828] text-white"
              class="flex items-center rounded-md text-sm text-[var(--text-secondary)] hover:text-white hover:bg-[#282828] transition-colors"
              :class="[
                compact ? 'justify-center py-1.5' : 'gap-3 px-2 py-1.5',
                {
                  'opacity-30': plIdx === sidebarPlDragIndex,
                  'sidebar-pl-drop-above': sidebarPlOverIndex === plIdx && sidebarPlDragIndex !== plIdx && sidebarPlDragIndex > plIdx,
                  'sidebar-pl-drop-below': sidebarPlOverIndex === plIdx && sidebarPlDragIndex !== plIdx && sidebarPlDragIndex < plIdx,
                }
              ]"
              :title="compact ? pl.name : null"
              draggable="false"
              @dragstart.prevent
              @mousedown="onSidebarPlMouseDown(plIdx, $event)"
              @click.prevent="navigatePlaylist(pl, $event)"
            >
              <PlaylistCover
                :name="pl.name"
                :cover="pl.cover"
                :size="28"
                className="h-7 w-7 rounded shrink-0"
              />
              <span v-if="!compact" class="truncate flex-1 flex items-center gap-1.5 min-w-0">
                <span class="truncate">{{ pl.name }}</span>
                <svg
                  v-if="store.isSmart(pl)"
                  xmlns="http://www.w3.org/2000/svg"
                  width="10"
                  height="10"
                  viewBox="0 0 24 24"
                  fill="currentColor"
                  stroke="none"
                  class="text-[var(--accent-color)] shrink-0 opacity-80"
                >
                  <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"></polygon>
                </svg>
              </span>
            </router-link>
            </TransitionGroup>
            <div
              v-if="store.playlists.length === 0 && !compact"
              class="px-3 py-1 text-[11px] text-gray-600"
            >
              No playlists yet
            </div>
          </div>
        </div>

        <div class="mt-auto" :class="compact ? '' : 'px-3 mb-4'">
          <button
            @click="store.selectAndScan()"
            :disabled="store.loading"
            class="w-full group flex items-center rounded-md text-sm font-medium text-[var(--accent-color)] hover:bg-[#282828] transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed"
            :class="compact ? 'justify-center py-2' : 'gap-3 px-3 py-2'"
            :title="compact ? 'Add Folder' : null"
          >
            <div
              class="flex items-center justify-center w-5 h-5 rounded bg-[var(--accent-color)]/10 group-hover:bg-[var(--accent-color)]/20 transition-colors"
            >
              <svg
                v-if="store.loading"
                class="w-3 h-3 animate-spin"
                xmlns="http://www.w3.org/2000/svg"
                fill="none"
                viewBox="0 0 24 24"
              >
                <circle
                  class="opacity-25"
                  cx="12"
                  cy="12"
                  r="10"
                  stroke="currentColor"
                  stroke-width="4"
                ></circle>
                <path
                  class="opacity-75"
                  fill="currentColor"
                  d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
                ></path>
              </svg>
              <svg
                v-else
                xmlns="http://www.w3.org/2000/svg"
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="3"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <line x1="12" y1="5" x2="12" y2="19"></line>
                <line x1="5" y1="12" x2="19" y2="12"></line>
              </svg>
            </div>

            <span v-if="!compact">{{ store.loading ? 'Scanning...' : 'Add Folder' }}</span>
          </button>

          <div v-if="!compact" class="text-[10px] text-gray-600 mt-1 px-3 truncate opacity-70">
            {{ store.statusMessage }}
          </div>
        </div>
      </nav>

      <!-- Main Content -->
      <div class="flex flex-col flex-1 overflow-hidden">
        <!-- Player Controls -->
        <PlayerControls />

        <main class="flex-1 relative overflow-hidden flex flex-col bg-[var(--app-bg)]">
          <div
            ref="scrollContainer"
            class="flex-1 overflow-auto scroll-smooth"
            @click="store.queuePanelOpen = false; store.lyricsPanelOpen = false"
          >
            <router-view v-slot="{ Component }">
              <keep-alive :include="['HomeView', 'SongsView', 'AlbumsView', 'ArtistsView']">
                <component :is="Component" :key="$route.fullPath" />
              </keep-alive>
            </router-view>
          </div>

          <!-- Up-next queue drawer -->
          <QueuePanel @click.stop />

          <!-- Lyrics panel -->
          <LyricsPanel @click.stop />
        </main>
      </div>
    </div>

    <!-- Create-playlist modal (global overlay) -->
    <PlaylistCreateModal />

    <!-- Smart-playlist rule editor (global overlay) -->
    <SmartPlaylistModal />

    <!-- Command palette (global overlay, Ctrl+K) -->
    <CommandPalette />

    <!-- Fullscreen Now-Playing + lyrics (global overlay) -->
    <FullScreenPlayer />

    <!-- Compact mini player (global overlay, Ctrl+Shift+M) -->
    <MiniPlayer />

    <!-- Fullscreen black transition overlay -->
    <Transition name="fullscreen-fade">
      <div
        v-if="store.fullscreenOverlayVisible"
        class="fixed inset-0 bg-black z-[999999] pointer-events-none"
      ></div>
    </Transition>

    <!-- Drag & drop hint -->
    <Transition name="fade">
      <div
        v-if="store.dragActive"
        class="fixed inset-0 z-[150] flex items-center justify-center bg-black/60 backdrop-blur-sm pointer-events-none"
      >
        <div
          class="border-2 border-dashed border-[var(--accent-color)] rounded-2xl px-12 py-10 text-center"
        >
          <div class="mb-1 text-2xl font-bold text-white">Drop to add music</div>
          <div class="text-sm text-gray-300">Folders and audio files are added to your library</div>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style>
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.15s ease;
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

.fullscreen-fade-enter-active,
.fullscreen-fade-leave-active {
  transition: opacity 0.3s ease;
}
.fullscreen-fade-enter-from,
.fullscreen-fade-leave-to {
  opacity: 0;
}

/* Sidebar playlist drag-to-reorder indicators */
.sidebar-pl-drop-above {
  box-shadow: inset 0 2px 0 0 var(--accent-color);
}
.sidebar-pl-drop-below {
  box-shadow: inset 0 -2px 0 0 var(--accent-color);
}

/* Sidebar playlist reorder FLIP animation */
.sidebar-pl-move {
  transition: transform 0.3s cubic-bezier(0.22, 0.61, 0.36, 1);
}
</style>
