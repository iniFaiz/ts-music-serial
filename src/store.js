import { reactive } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { idbGet, idbSet, idbDelete } from './libraryStore';
import { sortTracks } from './sortTracks';

// Directory portion of a file path (handles both / and \ separators).
function dirName(path) {
  const idx = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  return idx > 0 ? path.slice(0, idx) : path;
}

export const store = reactive({
  songs: [],
  roots: [],
  loading: false,
  statusMessage: "Ready to scan",
  selectedPath: "",
  searchQuery: "",
  useParallelism: true,
  scanComplete: false,
  scanDuration: "0",
  scanCount: 0,

  currentSong: null,
  isPlaying: false,
  isBuffering: false,
  volume: 1.0,
  currentTime: 0,
  duration: 0,
  queue: [],
  loopMode: 0,
  shuffleMode: false,

  // Liked songs (array of file paths) and user playlists.
  favorites: [],
  playlists: [], // [{ id, name, paths: [] }]

  // Hand-off to PlayerControls' load watcher: where to start the next load and
  // whether it should auto-play. Used by "resume on launch" (seek + paused).
  pendingSeek: null,
  pendingAutoplay: true,
  queuePanelOpen: false,

  // Persist the current library + scanned roots to IndexedDB. JSON round-trips
  // strip Vue's reactive proxies so the values can be structured-cloned.
  async persist() {
    try {
      await idbSet('library', JSON.parse(JSON.stringify(this.songs)));
      await idbSet('roots', [...this.roots]);
    } catch (e) {
      console.error("Failed to persist library", e);
    }
  },

  // Persist lightweight app state (likes, playlists, and what/where was
  // playing) so the next launch can restore it. Called on every relevant
  // change and on an interval to checkpoint the playback position.
  async persistState() {
    try {
      await idbSet('app_state', {
        favorites: [...this.favorites],
        playlists: JSON.parse(JSON.stringify(this.playlists)),
        playback: {
          songPath: this.currentSong ? this.currentSong.path : null,
          positionSecs: this.currentTime || 0,
          queuePaths: this.queue.map((s) => s.path),
          volume: this.volume,
          loopMode: this.loopMode,
          shuffleMode: this.shuffleMode,
        },
      });
    } catch (e) {
      console.error("Failed to persist app state", e);
    }
  },

  async loadLibrary() {
    try {
      // One-time migration from the old localStorage cache.
      const legacy = localStorage.getItem('music_library');
      if (legacy) {
        try {
          await idbSet('library', JSON.parse(legacy));
        } catch (e) {
          console.error("Failed to migrate legacy library", e);
        }
        localStorage.removeItem('music_library');
      }

      const [songs, roots] = await Promise.all([idbGet('library'), idbGet('roots')]);
      this.songs = Array.isArray(songs) ? songs : [];

      // Roots authorize streaming/cover access. If unknown (e.g. migrated
      // data), fall back to each track's containing folder.
      let resolvedRoots = Array.isArray(roots) ? roots : [];
      if (resolvedRoots.length === 0 && this.songs.length > 0) {
        resolvedRoots = [...new Set(this.songs.map((s) => dirName(s.path)))];
        await idbSet('roots', resolvedRoots);
      }
      this.roots = resolvedRoots;

      if (this.roots.length > 0) {
        try {
          await invoke('restore_roots', { roots: this.roots });
        } catch (e) {
          console.error("Failed to restore roots", e);
        }
      }

      if (this.songs.length > 0) {
        this.scanCount = this.songs.length;
        this.statusMessage = `Loaded ${this.songs.length} songs`;
        this.scanComplete = true;
      }

      // Restore likes, playlists and the last playback session.
      await this.restoreState();
    } catch (e) {
      console.error("Failed to load library", e);
    }
  },

  async restoreState() {
    let state;
    try {
      state = await idbGet('app_state');
    } catch (e) {
      console.error("Failed to read app state", e);
      return;
    }
    if (!state) return;

    this.favorites = Array.isArray(state.favorites) ? state.favorites : [];
    this.playlists = Array.isArray(state.playlists) ? state.playlists : [];

    const pb = state.playback;
    if (!pb) return;

    if (typeof pb.volume === 'number') this.volume = pb.volume;
    this.loopMode = pb.loopMode || 0;
    this.shuffleMode = !!pb.shuffleMode;

    const byPath = new Map(this.songs.map((s) => [s.path, s]));
    if (Array.isArray(pb.queuePaths)) {
      this.queue = pb.queuePaths.map((p) => byPath.get(p)).filter(Boolean);
    }

    // Re-load the last track but leave it paused at the saved position; the
    // PlayerControls watcher reads pendingSeek/pendingAutoplay when it loads.
    if (pb.songPath && byPath.has(pb.songPath)) {
      this.pendingSeek = pb.positionSecs || 0;
      this.pendingAutoplay = false;
      this.currentTime = pb.positionSecs || 0;
      this.isPlaying = false;
      this.currentSong = byPath.get(pb.songPath);
    }
  },

  async resetLibrary() {
    this.songs = [];
    this.roots = [];
    this.scanCount = 0;
    this.currentSong = null;
    this.queue = [];
    this.isPlaying = false;
    this.scanComplete = false;
    this.favorites = [];
    this.playlists = [];
    this.statusMessage = "Library reset";
    try {
      await idbDelete('library');
      await idbDelete('roots');
      await idbDelete('app_state');
    } catch (e) {
      console.error("Failed to reset library", e);
    }
  },

  async selectAndScan() {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        recursive: true,
      });

      if (selected) {
        this.selectedPath = selected;
        await this.scanMusic(selected);
      }
    } catch (err) {
      console.error(err);
      this.statusMessage = "Error opening dialog";
    }
  },

  async scanMusic(path) {
    this.loading = true;
    this.scanComplete = false;
    this.statusMessage = "Scanning...";

    const startTime = performance.now();

    try {
      const result = await invoke("scan_music_folder", { 
        path, 
        useParallelism: this.useParallelism 
      });
      const endTime = performance.now();
      
      const existingPaths = new Set(this.songs.map(s => s.path));
      const newSongs = result.filter(s => !existingPaths.has(s.path));

      this.songs = sortTracks([...this.songs, ...newSongs]);

      if (!this.roots.includes(path)) {
        this.roots = [...this.roots, path];
      }
      await this.persist();

      const timeSeconds = ((endTime - startTime) / 1000).toFixed(2);
      this.statusMessage = `Added ${newSongs.length} new tracks in ${timeSeconds}s`;
      
      this.scanDuration = timeSeconds;
      this.scanCount = this.songs.length;
      this.scanComplete = true;
    } catch (error) {
      this.statusMessage = `Error: ${error}`;
    } finally {
      this.loading = false;
    }
  },

  closePopup() {
    this.scanComplete = false;
  },

  playSong(song, newQueue = null) {
    if (newQueue && newQueue.length > 0) {
      this.queue = [...newQueue];
    } else if (this.queue.length === 0) {
        this.queue = [...this.songs];
    }
    this.pendingSeek = null;
    this.pendingAutoplay = true;
    this.currentSong = song;
    this.isPlaying = true;
    this.persistState();
  },

  // ---- Queue management -------------------------------------------------

  currentQueueIndex() {
    if (!this.currentSong) return -1;
    return this.queue.findIndex((s) => s.path === this.currentSong.path);
  },

  // Insert right after the current track so it plays next.
  playNext(song) {
    if (this.queue.length === 0) {
      this.queue = this.currentSong ? [this.currentSong, song] : [song];
    } else {
      const idx = this.currentQueueIndex();
      this.queue.splice(idx + 1, 0, song);
    }
    this.persistState();
  },

  addToQueue(songs) {
    const list = Array.isArray(songs) ? songs : [songs];
    if (this.queue.length === 0 && this.currentSong) {
      this.queue = [this.currentSong];
    }
    this.queue.push(...list);
    this.persistState();
  },

  removeFromQueue(index) {
    if (index < 0 || index >= this.queue.length) return;
    this.queue.splice(index, 1);
    this.persistState();
  },

  moveInQueue(from, to) {
    if (from === to) return;
    if (from < 0 || from >= this.queue.length) return;
    if (to < 0 || to >= this.queue.length) return;
    const [item] = this.queue.splice(from, 1);
    this.queue.splice(to, 0, item);
    this.persistState();
  },

  playQueueIndex(index) {
    if (index < 0 || index >= this.queue.length) return;
    this.pendingSeek = null;
    this.pendingAutoplay = true;
    this.currentSong = this.queue[index];
    this.isPlaying = true;
    this.persistState();
  },

  clearQueue() {
    this.queue = this.currentSong ? [this.currentSong] : [];
    this.persistState();
  },

  // ---- Favorites --------------------------------------------------------

  isFavorite(path) {
    return this.favorites.includes(path);
  },

  toggleFavorite(path) {
    if (!path) return;
    const idx = this.favorites.indexOf(path);
    if (idx >= 0) this.favorites.splice(idx, 1);
    else this.favorites.push(path);
    this.persistState();
  },

  get favoriteSongs() {
    const set = new Set(this.favorites);
    return this.songs.filter((s) => set.has(s.path));
  },

  // ---- Playlists --------------------------------------------------------

  createPlaylist(name, description = '', cover = null) {
    const id = 'pl_' + Date.now().toString(36) + Math.random().toString(36).slice(2, 7);
    const playlist = {
      id,
      name: (name || '').trim() || 'New Playlist',
      description: (description || '').trim(),
      cover: cover || null,
      paths: [],
    };
    this.playlists.push(playlist);
    this.persistState();
    return playlist;
  },

  // Playlist-create modal (opened from the sidebar "+" and song context menu).
  playlistModal: { open: false, pendingSongPath: null },

  openPlaylistModal(pendingSongPath = null) {
    this.playlistModal.pendingSongPath = pendingSongPath;
    this.playlistModal.open = true;
  },

  closePlaylistModal() {
    this.playlistModal.open = false;
    this.playlistModal.pendingSongPath = null;
  },

  deletePlaylist(id) {
    const idx = this.playlists.findIndex((p) => p.id === id);
    if (idx >= 0) {
      this.playlists.splice(idx, 1);
      this.persistState();
    }
  },

  renamePlaylist(id, name) {
    const pl = this.playlists.find((p) => p.id === id);
    if (pl && name && name.trim()) {
      pl.name = name.trim();
      this.persistState();
    }
  },

  getPlaylist(id) {
    return this.playlists.find((p) => p.id === id);
  },

  addToPlaylist(id, paths) {
    const pl = this.getPlaylist(id);
    if (!pl) return;
    const list = Array.isArray(paths) ? paths : [paths];
    for (const path of list) {
      if (!pl.paths.includes(path)) pl.paths.push(path);
    }
    this.persistState();
  },

  removeFromPlaylist(id, path) {
    const pl = this.getPlaylist(id);
    if (!pl) return;
    const idx = pl.paths.indexOf(path);
    if (idx >= 0) {
      pl.paths.splice(idx, 1);
      this.persistState();
    }
  },

  playlistSongs(id) {
    const pl = this.getPlaylist(id);
    if (!pl) return [];
    const byPath = new Map(this.songs.map((s) => [s.path, s]));
    return pl.paths.map((p) => byPath.get(p)).filter(Boolean);
  },

  playPlaylist(id) {
    const list = this.playlistSongs(id);
    if (list.length > 0) this.playSong(list[0], list);
  },

  togglePlay() {
    this.isPlaying = !this.isPlaying;
  },

  toggleLoop() {
    this.loopMode = (this.loopMode + 1) % 3;
    this.persistState();
  },

  nextSong(userTriggered = false) {
    if (!this.currentSong || this.queue.length === 0) return;

    if (this.loopMode === 2 && !userTriggered) {
      return; 
    }
    
    let nextIndex;
    const currentIndex = this.queue.findIndex(s => s.path === this.currentSong.path);

    if (this.shuffleMode) {
      nextIndex = Math.floor(Math.random() * this.queue.length);
    } else {
      nextIndex = currentIndex + 1;
    }

    if (nextIndex >= this.queue.length) {
      if (this.loopMode === 1) {
        nextIndex = 0;
      } else {
        this.isPlaying = false;
        return;
      }
    }

    this.pendingSeek = null;
    this.pendingAutoplay = true;
    this.currentSong = this.queue[nextIndex];
    this.isPlaying = true;
    this.persistState();
  },

  prevSong() {
    if (!this.currentSong || this.queue.length === 0) return;
    
    if (this.currentTime > 3) {
      this.currentTime = 0;
      return; 
    }

    let prevIndex;
    const currentIndex = this.queue.findIndex(s => s.path === this.currentSong.path);

    if (this.shuffleMode) {
      prevIndex = Math.floor(Math.random() * this.queue.length);
    } else {
      prevIndex = currentIndex - 1;
    }

    if (prevIndex < 0) {
       if (this.loopMode === 1) {
         prevIndex = this.queue.length - 1;
       } else {
         prevIndex = 0;
       }
    }

    this.pendingSeek = null;
    this.pendingAutoplay = true;
    this.currentSong = this.queue[prevIndex];
    this.isPlaying = true;
    this.persistState();
  },

  setVolume(val) {
    this.volume = parseFloat(val);
  },

  setParallelism(val) {
    this.useParallelism = val;
  },

  toggleShuffle() {
    this.shuffleMode = !this.shuffleMode;
    this.persistState();
  },

  get filteredSongs() {
    if (!this.searchQuery) return this.songs;
    const lower = this.searchQuery.toLowerCase();
    return this.songs.filter(song => 
      (song.title && song.title.toLowerCase().includes(lower)) ||
      (song.artist && song.artist.toLowerCase().includes(lower)) ||
      (song.album && song.album.toLowerCase().includes(lower))
    );
  }
});

store.loadLibrary();