import { ref } from 'vue';
import { store } from './store';

// Bulk selection mode for song lists: select rows, then play / queue /
// playlist them as a batch. Extracted from SongList (also usable by other
// list views later).
//
// `songs` is the getter for the currently visible list order (used by
// select-all), `playlistId` is a getter returning the current playlist id when
// the list lives inside a playlist (enables bulk remove), and
// `menu`/`closeMenu` come from the row context menu that starts the mode.
export function useMultiSelect({ songs, playlistId, menu, closeMenu }) {
  const selectMode = ref(false);
  const selectedSongs = ref([]);
  const showPlDropdown = ref(false);

  const toggleSelectSong = (song) => {
    const idx = selectedSongs.value.indexOf(song.path);
    if (idx >= 0) {
      selectedSongs.value.splice(idx, 1);
    } else {
      selectedSongs.value.push(song.path);
    }
  };

  const toggleSelectAll = (event) => {
    if (event.target.checked) {
      selectedSongs.value = songs().map((s) => s.path);
    } else {
      selectedSongs.value = [];
    }
  };

  const startSelectMode = () => {
    selectMode.value = true;
    selectedSongs.value = [menu.value.song.path];
    closeMenu();
  };

  const cancelSelection = () => {
    selectMode.value = false;
    selectedSongs.value = [];
    showPlDropdown.value = false;
  };

  const playSelected = () => {
    const tracks = songs().filter((s) => selectedSongs.value.includes(s.path));
    if (tracks.length > 0) {
      store.playSong(tracks[0], tracks);
    }
    cancelSelection();
  };

  const addSelectedToQueue = () => {
    const tracks = songs().filter((s) => selectedSongs.value.includes(s.path));
    if (tracks.length > 0) {
      store.addToQueue(tracks);
    }
    cancelSelection();
  };

  const addSelectedToPlaylist = (id) => {
    store.runMutation(() => store.addToPlaylist(id, selectedSongs.value));
    cancelSelection();
  };

  const newPlaylistWithSelected = () => {
    store.openPlaylistModal(selectedSongs.value);
    cancelSelection();
  };

  const removeSelectedFromPlaylist = () => {
    const pid = playlistId?.();
    if (pid) {
      const paths = [...selectedSongs.value];
      store.runMutation(() =>
        Promise.all(paths.map((path) => store.removeFromPlaylist(pid, path)))
      );
    }
    cancelSelection();
  };

  return {
    selectMode,
    selectedSongs,
    showPlDropdown,
    toggleSelectSong,
    toggleSelectAll,
    startSelectMode,
    cancelSelection,
    playSelected,
    addSelectedToQueue,
    addSelectedToPlaylist,
    newPlaylistWithSelected,
    removeSelectedFromPlaylist,
  };
}
