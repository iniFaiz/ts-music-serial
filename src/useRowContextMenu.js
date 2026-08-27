import { ref } from 'vue';
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

  const openMenu = (song, event) => {
    event.preventDefault();
    const winWidth = window.innerWidth;
    const winHeight = window.innerHeight;
    const menuWidth = 224;
    const menuHeight = playlistId?.() ? 450 : 400;

    let x = event.clientX;
    let y = event.clientY;

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
    menu.value.open = false;
  };

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
  };
}
