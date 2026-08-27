import { ref, watch, nextTick } from 'vue';
import { useRouter } from 'vue-router';
import { invokeCommand as invoke } from './generated/ipc';
import { store } from './store';
import { navigateWithTransition } from './viewTransition';

// Row context menu for song lists, extracted from SongList.
//
// Owns the menu state (viewport-aware positioning that flips up/down based on
// the click point), every action it offers, and the hover-guard used to keep
// scrolling under an open menu from closing it.
//
// Options:
//   playlistId          getter — enables "remove from this playlist" and a
//                       taller menu estimate when the list lives in a playlist
//   findCoverBySongPath fn(path) → element — cover lookup for shared-element
//                       view transitions on artist/album navigation
export function useRowContextMenu({ playlistId, findCoverBySongPath }) {
  const router = useRouter();

  const menu = ref({ open: false, x: 0, y: 0, maxHeight: 400, song: null });
  let triggerElement = null;

  const openMenu = (song, event) => {
    if (event?.preventDefault) event.preventDefault();
    triggerElement =
      (event?.target && event.target.closest?.('button, [tabindex="0"]')) || document.activeElement;
    const winWidth = window.innerWidth;
    const winHeight = window.innerHeight;
    const menuWidth = 224;
    const menuHeight = playlistId?.() ? 450 : 400;

    let x = event?.clientX ?? 100;
    let y = event?.clientY ?? 100;

    if (x + menuWidth > winWidth) {
      x = winWidth - menuWidth - 10;
    }
    x = Math.max(10, x);

    const spaceBelow = winHeight - y;
    const spaceAbove = y;

    let maxHeight;
    let topPosition;

    if (spaceBelow >= spaceAbove) {
      topPosition = y;
      maxHeight = spaceBelow - 20;
    } else {
      if (spaceAbove >= menuHeight) {
        topPosition = y - menuHeight;
        maxHeight = menuHeight;
      } else {
        topPosition = 10;
        maxHeight = y - 20;
      }
    }

    maxHeight = Math.max(150, maxHeight);
    topPosition = Math.max(10, topPosition);

    menu.value = { open: true, x, y: topPosition, maxHeight, song };
  };

  const closeMenu = () => {
    if (!menu.value.open) return;
    menu.value.open = false;
    if (triggerElement && typeof triggerElement.focus === 'function') {
      try {
        triggerElement.focus();
      } catch {}
      triggerElement = null;
    }
  };

  const handleMenuKeyDown = (event) => {
    if (!menu.value.open) return;
    const container = event.currentTarget;
    if (!container) return;

    if (event.key === 'Escape') {
      event.preventDefault();
      closeMenu();
      return;
    }

    const items = Array.from(container.querySelectorAll('button:not([disabled])'));
    if (items.length === 0) return;

    const currentIndex = items.indexOf(document.activeElement);

    if (event.key === 'ArrowDown') {
      event.preventDefault();
      const nextIndex = currentIndex < items.length - 1 ? currentIndex + 1 : 0;
      items[nextIndex]?.focus();
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      const prevIndex = currentIndex > 0 ? currentIndex - 1 : items.length - 1;
      items[prevIndex]?.focus();
    } else if (event.key === 'Home') {
      event.preventDefault();
      items[0]?.focus();
    } else if (event.key === 'End') {
      event.preventDefault();
      items[items.length - 1]?.focus();
    }
  };

  watch(
    () => menu.value.open,
    async (isOpen) => {
      if (isOpen) {
        await nextTick();
        const menuEl = document.querySelector('.context-menu-container');
        const firstBtn = menuEl?.querySelector('button:not([disabled])');
        firstBtn?.focus();
      }
    }
  );

  const playNext = () => {
    store.playNext(menu.value.song);
    closeMenu();
  };

  const addToQueue = () => {
    store.addToQueue(menu.value.song);
    closeMenu();
  };

  const showArtist = () => {
    const song = menu.value.song;
    if (!song) return;
    const navigate = () => router.push({ name: 'ArtistDetail', params: { name: song.artist } });
    const coverEl = findCoverBySongPath(song.path);
    closeMenu();
    if (coverEl) {
      navigateWithTransition(navigate, coverEl, 'shared-cover', 'to-artist-transition');
    } else {
      navigate();
    }
  };

  const showAlbum = () => {
    const song = menu.value.song;
    if (!song) return;
    const navigate = () => router.push({ name: 'AlbumDetail', params: { name: song.album } });
    const coverEl = findCoverBySongPath(song.path);
    closeMenu();
    if (coverEl) {
      navigateWithTransition(navigate, coverEl, 'shared-cover', 'to-album-transition');
    } else {
      navigate();
    }
  };

  const showInFolder = async () => {
    const song = menu.value.song;
    if (!song) return;
    closeMenu();
    try {
      await invoke('player_show_in_folder', { path: song.path });
    } catch (err) {
      console.error('Failed to show in folder:', err);
    }
  };

  const toggleLike = () => {
    store.runMutation(() => store.toggleFavorite(menu.value.song.path));
    closeMenu();
  };

  const addToPlaylist = (id) => {
    store.runMutation(() => store.addToPlaylist(id, menu.value.song.path));
    closeMenu();
  };

  const newPlaylistWithSong = () => {
    store.openPlaylistModal(menu.value.song.path);
    closeMenu();
  };

  const removeFromThisPlaylist = () => {
    const pid = playlistId?.();
    if (pid) {
      store.runMutation(() => store.removeFromPlaylist(pid, menu.value.song.path));
    }
    closeMenu();
  };

  // Scrolling the list under an open menu closes it — unless the pointer is
  // hovering the menu itself (wheel-over-menu must not dismiss).
  const isHoveringMenu = ref(false);

  const closeMenuOnScroll = () => {
    if (isHoveringMenu.value) {
      return;
    }
    closeMenu();
  };

  return {
    menu,
    openMenu,
    closeMenu,
    playNext,
    addToQueue,
    showArtist,
    showAlbum,
    showInFolder,
    toggleLike,
    addToPlaylist,
    newPlaylistWithSong,
    removeFromThisPlaylist,
    isHoveringMenu,
    closeMenuOnScroll,
    handleMenuKeyDown,
  };
}
