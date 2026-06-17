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

// Normalize a path for prefix comparison: unify separators and lowercase (paths
// are case-insensitive on Windows). Strips a trailing separator.
function normPath(path) {
  return String(path || '')
    .replace(/\\/g, '/')
    .replace(/\/+$/, '')
    .toLowerCase();
}

// True when `filePath` lives inside the directory `root` (or equals it).
function isUnderRoot(filePath, root) {
  const f = normPath(filePath);
  const r = normPath(root);
  return f === r || f.startsWith(r + '/');
}

export const store = reactive({
  songs: [],
  roots: [],
  loading: false,
  statusMessage: 'Ready to scan',
  selectedPath: '',
  searchQuery: '',
  useParallelism: true,
  scanComplete: false,
  scanDuration: '0',
  scanCount: 0,

  currentSong: null,
  currentSampleRate: null,
  currentBitDepth: null,
  isPlaying: false,
  isBuffering: false,
  volume: 1.0,
  isMuted: false,
  currentTime: 0,
  duration: 0,
  // Timestamp of the last user seek; the PlayerControls poll uses it to avoid
  // snapping the slider back to a stale position. Shared so the fullscreen
  // player and lyric-click seeks suppress the poll too. Not persisted.
  lastSeekAt: 0,
  queue: [],
  loopMode: 0,
  shuffleMode: false,
  // Unlimited queue (autoplay): when the queue is exhausted, keep playing by
  // appending random tracks from the library — like Apple Music's ∞ autoplay.
  autoplayMode: false,
  // Real-time 6-bar audio visualizer in the player bar (default on). Mirrored to
  // the Rust backend so the FFT analysis only runs when it's actually shown.
  visualizerEnabled: true,
  playbackFinished: false,

  // --- Advanced playback / library settings (persisted) ---
  // Selected audio output device name; null = system default.
  outputDevice: null,
  // Volume normalization (Sound Check): level out loudness across tracks using
  // ReplayGain tags, falling back to a lazily-computed EBU R128 gain.
  normalizationEnabled: false,
  normalizationPreampDb: 0,
  // Track-to-track transition: 'off' | 'gapless' | 'crossfade'.
  transitionMode: 'off',
  crossfadeSecs: 6,
  // Optional Musixmatch community user token for the lyrics fallback chain.
  musixmatchToken: '',
  // Selected lyrics provider: 'lrclib' | 'local' | 'netease' | 'musixmatch' | 'none'
  lyricsSource: 'lrclib',

  // Fullscreen Now-Playing overlay (Apple Music style cover + synced lyrics).
  fullscreenOpen: false,
  // Currently selected/active album for keyboard shortcut actions.
  selectedAlbum: null,
  // Drop-overlay visibility while a drag is over the window.
  dragActive: false,

  // Liked songs (array of file paths) and user playlists.
  favorites: [],
  playlists: [], // [{ id, name, paths: [] }]

  // Hand-off to PlayerControls' load watcher: where to start the next load and
  // whether it should auto-play. Used by "resume on launch" (seek + paused).
  pendingSeek: null,
  pendingAutoplay: true,
  queuePanelOpen: false,
  lyricsPanelOpen: false,

  // Persist the current library + scanned roots to IndexedDB. JSON round-trips
  // strip Vue's reactive proxies so the values can be structured-cloned.
  async persist() {
    try {
      await idbSet('library', JSON.parse(JSON.stringify(this.songs)));
      await idbSet('roots', [...this.roots]);
    } catch (e) {
      console.error('Failed to persist library', e);
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
        settings: {
          outputDevice: this.outputDevice,
          normalizationEnabled: this.normalizationEnabled,
          normalizationPreampDb: this.normalizationPreampDb,
          transitionMode: this.transitionMode,
          crossfadeSecs: this.crossfadeSecs,
          musixmatchToken: this.musixmatchToken,
          lyricsSource: this.lyricsSource,
        },
        playback: {
          songPath: this.currentSong ? this.currentSong.path : null,
          positionSecs: this.currentTime || 0,
          queuePaths: this.queue.map((s) => s.path),
          volume: this.volume,
          isMuted: this.isMuted,
          loopMode: this.loopMode,
          shuffleMode: this.shuffleMode,
          autoplayMode: this.autoplayMode,
          visualizerEnabled: this.visualizerEnabled,
        },
      });
    } catch (e) {
      console.error('Failed to persist app state', e);
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
          console.error('Failed to migrate legacy library', e);
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
          console.error('Failed to restore roots', e);
        }
        // Start watching folders so the library auto-updates on disk changes.
        this.watchRoots();
      }

      if (this.songs.length > 0) {
        this.scanCount = this.songs.length;
        this.statusMessage = `Loaded ${this.songs.length} songs`;
        this.scanComplete = true;
      }

      // Restore likes, playlists and the last playback session.
      await this.restoreState();
    } catch (e) {
      console.error('Failed to load library', e);
    }
  },

  async restoreState() {
    let state;
    try {
      state = await idbGet('app_state');
    } catch (e) {
      console.error('Failed to read app state', e);
      return;
    }
    if (!state) return;

    this.favorites = Array.isArray(state.favorites) ? state.favorites : [];
    this.playlists = Array.isArray(state.playlists) ? state.playlists : [];

    const s = state.settings;
    if (s) {
      if (typeof s.outputDevice !== 'undefined') this.outputDevice = s.outputDevice;
      if (typeof s.normalizationEnabled === 'boolean')
        this.normalizationEnabled = s.normalizationEnabled;
      if (typeof s.normalizationPreampDb === 'number')
        this.normalizationPreampDb = s.normalizationPreampDb;
      if (typeof s.transitionMode === 'string') this.transitionMode = s.transitionMode;
      if (typeof s.crossfadeSecs === 'number') this.crossfadeSecs = s.crossfadeSecs;
      if (typeof s.musixmatchToken === 'string') this.musixmatchToken = s.musixmatchToken;
      if (typeof s.lyricsSource === 'string') this.lyricsSource = s.lyricsSource;

      // Re-select the saved output device (the audio thread starts on default).
      if (this.outputDevice) {
        invoke('set_output_device', { name: this.outputDevice }).catch(() => {});
      }
    }

    const pb = state.playback;
    if (!pb) return;

    if (typeof pb.volume === 'number') this.volume = pb.volume;
    if (typeof pb.isMuted === 'boolean') this.isMuted = pb.isMuted;
    this.loopMode = pb.loopMode || 0;
    this.shuffleMode = !!pb.shuffleMode;
    this.autoplayMode = !!pb.autoplayMode;
    if (typeof pb.visualizerEnabled === 'boolean') this.visualizerEnabled = pb.visualizerEnabled;
    this.syncVisualizer();

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
    this.statusMessage = 'Library reset';
    try {
      await idbDelete('library');
      await idbDelete('roots');
      await idbDelete('app_state');
    } catch (e) {
      console.error('Failed to reset library', e);
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
      this.statusMessage = 'Error opening dialog';
    }
  },

  async scanMusic(path) {
    this.loading = true;
    this.scanComplete = false;
    this.statusMessage = 'Scanning...';

    const startTime = performance.now();

    try {
      const result = await invoke('scan_music_folder', {
        path,
        useParallelism: this.useParallelism,
      });
      const endTime = performance.now();

      const existingPaths = new Set(this.songs.map((s) => s.path));
      const newSongs = result.filter((s) => !existingPaths.has(s.path));

      this.songs = sortTracks([...this.songs, ...newSongs]);

      if (!this.roots.includes(path)) {
        this.roots = [...this.roots, path];
        this.watchRoots();
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

  // Add audio from arbitrary dropped paths (files and/or folders). Folders are
  // registered as roots so their audio can be streamed; lone files register
  // their containing folder. Mirrors scanMusic's incremental merge.
  async addPaths(paths) {
    const list = (Array.isArray(paths) ? paths : [paths]).filter(Boolean);
    if (list.length === 0) return;
    this.loading = true;
    this.statusMessage = 'Adding dropped items...';
    try {
      const result = await invoke('scan_paths', { paths: list });
      const existingPaths = new Set(this.songs.map((s) => s.path));
      const newSongs = result.filter((s) => !existingPaths.has(s.path));
      this.songs = sortTracks([...this.songs, ...newSongs]);

      // Register new roots so the watcher and streaming scope cover them. A
      // dropped path that contains new tracks is treated as a folder root;
      // otherwise fall back to each new track's containing folder.
      const newRoots = [];
      for (const p of list) {
        if (
          !this.roots.some((r) => isUnderRoot(p, r)) &&
          !newRoots.includes(p) &&
          newSongs.some((s) => isUnderRoot(s.path, p))
        ) {
          newRoots.push(p);
        }
      }
      for (const d of new Set(newSongs.map((s) => dirName(s.path)))) {
        if (!this.roots.some((r) => isUnderRoot(d, r)) && !newRoots.some((r) => isUnderRoot(d, r))) {
          newRoots.push(d);
        }
      }
      if (newRoots.length) {
        this.roots = [...this.roots, ...newRoots];
        this.watchRoots();
      }
      await this.persist();
      this.scanCount = this.songs.length;
      this.scanComplete = true;
      this.statusMessage = `Added ${newSongs.length} new tracks`;
    } catch (e) {
      this.statusMessage = `Error: ${e}`;
    } finally {
      this.loading = false;
    }
  },

  // ---- Multi-folder library management -----------------------------------

  // Remove a scanned folder and every track that lives inside it.
  async removeRoot(root) {
    this.roots = this.roots.filter((r) => normPath(r) !== normPath(root));
    const removed = this.songs.filter((s) => isUnderRoot(s.path, root));
    if (removed.length) {
      const removedSet = new Set(removed.map((s) => s.path));
      // Stop playback if the current track is being removed.
      if (this.currentSong && removedSet.has(this.currentSong.path)) {
        this.isPlaying = false;
        this.currentSong = null;
        this.currentTime = 0;
        this.duration = 0;
        try {
          await invoke('player_stop');
        } catch {
          /* ignore */
        }
      }
      this.queue = this.queue.filter((s) => !removedSet.has(s.path));
      this.songs = this.songs.filter((s) => !removedSet.has(s.path));
      this.favorites = this.favorites.filter((p) => !removedSet.has(p));
      this.playlists.forEach((pl) => {
        pl.paths = pl.paths.filter((p) => !removedSet.has(p));
      });
    }
    await this.persist();
    await this.persistState();
    this.scanCount = this.songs.length;
    this.statusMessage = `Removed folder: ${root}`;
    this.watchRoots();
  },

  // Re-scan all roots, merging any newly-added files and pruning files that no
  // longer exist on disk. Lightweight — does not touch unchanged tracks.
  async refreshLibrary() {
    if (this.roots.length === 0) return;
    this.loading = true;
    this.statusMessage = 'Refreshing library...';
    try {
      const existingPaths = new Set(this.songs.map((s) => s.path));
      let merged = this.songs;
      for (const root of this.roots) {
        const result = await invoke('scan_music_folder', {
          path: root,
          useParallelism: this.useParallelism,
        });
        const newSongs = result.filter((s) => !existingPaths.has(s.path));
        newSongs.forEach((s) => existingPaths.add(s.path));
        if (newSongs.length) merged = [...merged, ...newSongs];
      }

      // Prune tracks whose files were deleted.
      try {
        const alive = new Set(await invoke('filter_existing', { paths: merged.map((s) => s.path) }));
        merged = merged.filter((s) => alive.has(s.path));
      } catch {
        /* backend prune unavailable — keep all */
      }

      this.songs = sortTracks(merged);
      this.queue = this.queue.filter((s) => this.songs.some((x) => x.path === s.path));
      await this.persist();
      this.scanCount = this.songs.length;
      this.scanComplete = true;
      this.statusMessage = `Library refreshed — ${this.songs.length} tracks`;
    } catch (e) {
      this.statusMessage = `Error: ${e}`;
    } finally {
      this.loading = false;
    }
  },

  // Wipe and rebuild the library from scratch by re-scanning every root.
  async reindexLibrary() {
    if (this.roots.length === 0) return;
    this.loading = true;
    this.statusMessage = 'Reindexing...';
    const startTime = performance.now();
    try {
      const collected = [];
      const seen = new Set();
      for (const root of this.roots) {
        const result = await invoke('scan_music_folder', {
          path: root,
          useParallelism: this.useParallelism,
        });
        for (const s of result) {
          if (!seen.has(s.path)) {
            seen.add(s.path);
            collected.push(s);
          }
        }
      }
      this.songs = sortTracks(collected);
      this.queue = this.queue.filter((s) => seen.has(s.path));
      await this.persist();
      const secs = ((performance.now() - startTime) / 1000).toFixed(2);
      this.scanCount = this.songs.length;
      this.scanComplete = true;
      this.statusMessage = `Reindexed ${this.songs.length} tracks in ${secs}s`;
    } catch (e) {
      this.statusMessage = `Error: ${e}`;
    } finally {
      this.loading = false;
    }
  },

  // (Re)configure the native filesystem watcher to cover the current roots so
  // the library auto-updates when files change outside the app.
  watchRoots() {
    invoke('watch_roots', { roots: [...this.roots] }).catch(() => {});
  },

  // ---- Audio output device ----------------------------------------------

  async setOutputDevice(name) {
    this.outputDevice = name || null;
    try {
      await invoke('set_output_device', { name: this.outputDevice });
    } catch (e) {
      console.error('Failed to set output device', e);
    }
    this.persistState();
    // Reload the current track on the new device, preserving position/play state.
    if (this.currentSong) {
      this.pendingSeek = this.currentTime || 0;
      this.pendingAutoplay = this.isPlaying;
      this.currentSong = { ...this.currentSong };
    }
  },

  // ---- Normalization / transition settings -------------------------------

  setNormalizationEnabled(v) {
    this.normalizationEnabled = !!v;
    this.persistState();
  },
  setNormalizationPreamp(v) {
    this.normalizationPreampDb = Number(v) || 0;
    this.persistState();
  },
  setTransitionMode(v) {
    this.transitionMode = v;
    this.persistState();
  },
  setCrossfadeSecs(v) {
    this.crossfadeSecs = Math.max(1, Math.min(12, Number(v) || 6));
    this.persistState();
  },
  setMusixmatchToken(v) {
    this.musixmatchToken = String(v || '').trim();
    this.persistState();
  },
  setLyricsSource(v) {
    this.lyricsSource = String(v || 'lrclib');
    this.persistState();
  },

  // ---- Fullscreen Now-Playing --------------------------------------------

  openFullscreen() {
    if (this.currentSong) this.fullscreenOpen = true;
  },
  closeFullscreen() {
    this.fullscreenOpen = false;
  },
  toggleFullscreen() {
    if (this.fullscreenOpen) this.fullscreenOpen = false;
    else this.openFullscreen();
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
    // Preserve pendingSeek if already set (e.g. by onSeekCommit on a finished track)
    if (this.pendingSeek === null) {
      this.pendingSeek = null;
    }
    this.pendingAutoplay = true;
    this.currentSong = song ? { ...song } : null;
    this.isPlaying = true;
    this.playbackFinished = false;
    this.persistState();
  },

  // ---- Queue management -------------------------------------------------

  currentQueueIndex() {
    if (!this.currentSong) return -1;
    return this.queue.findIndex((s) => s.path === this.currentSong.path);
  },

  // Insert right after the current track so it plays next. Clone so the same
  // song queued twice stays a distinct entry (stable, unique key for the UI).
  playNext(song) {
    const entry = { ...song };
    if (this.queue.length === 0) {
      this.queue = this.currentSong ? [this.currentSong, entry] : [entry];
    } else {
      const idx = this.currentQueueIndex();
      this.queue.splice(idx + 1, 0, entry);
    }
    this.persistState();
  },

  addToQueue(songs) {
    const list = (Array.isArray(songs) ? songs : [songs]).map((s) => ({ ...s }));
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
    if (this.pendingSeek === null) {
      this.pendingSeek = null;
    }
    this.pendingAutoplay = true;
    this.currentSong = { ...this.queue[index] };
    this.isPlaying = true;
    this.playbackFinished = false;
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

  async deleteSong(path) {
    try {
      await invoke('player_delete_file', { path });
    } catch (e) {
      console.error('Failed to delete file from disk:', e);
      throw e;
    }

    if (this.currentSong && this.currentSong.path === path) {
      if (this.queue.length <= 1) {
        this.isPlaying = false;
        this.currentSong = null;
        this.currentTime = 0;
        this.duration = 0;
        try {
          await invoke('player_stop');
        } catch (err) {
          console.error(err);
        }
      } else {
        this.nextSong(true);
      }
    }

    this.queue = this.queue.filter((s) => s.path !== path);
    this.songs = this.songs.filter((s) => s.path !== path);

    const favIdx = this.favorites.indexOf(path);
    if (favIdx >= 0) {
      this.favorites.splice(favIdx, 1);
    }

    this.playlists.forEach((pl) => {
      pl.paths = pl.paths.filter((p) => p !== path);
    });

    await this.persist();
    await this.persistState();

    this.scanCount = this.songs.length;
    this.statusMessage = `Deleted file: ${path}`;
  },

  async removeSongFromLibrary(path) {
    if (this.currentSong && this.currentSong.path === path) {
      if (this.queue.length <= 1) {
        this.isPlaying = false;
        this.currentSong = null;
        this.currentTime = 0;
        this.duration = 0;
        try {
          await invoke('player_stop');
        } catch (err) {
          console.error(err);
        }
      } else {
        this.nextSong(true);
      }
    }

    this.queue = this.queue.filter((s) => s.path !== path);
    this.songs = this.songs.filter((s) => s.path !== path);

    const favIdx = this.favorites.indexOf(path);
    if (favIdx >= 0) {
      this.favorites.splice(favIdx, 1);
    }

    this.playlists.forEach((pl) => {
      pl.paths = pl.paths.filter((p) => p !== path);
    });

    await this.persist();
    await this.persistState();

    this.scanCount = this.songs.length;
    this.statusMessage = `Removed file from list: ${path}`;
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
    if (this.playbackFinished && this.currentSong) {
      this.playSong(this.currentSong);
    } else {
      this.isPlaying = !this.isPlaying;
    }
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
    const currentIndex = this.queue.findIndex((s) => s.path === this.currentSong.path);

    if (this.shuffleMode) {
      nextIndex = Math.floor(Math.random() * this.queue.length);
    } else {
      nextIndex = currentIndex + 1;
    }

    if (nextIndex >= this.queue.length) {
      if (this.loopMode === 1) {
        nextIndex = 0;
      } else if (this.autoplayMode) {
        // Unlimited queue: append a random track and continue into it.
        const song = this.pickRandomSong();
        if (!song) {
          this.isPlaying = false;
          return;
        }
        this.queue.push({ ...song });
        nextIndex = this.queue.length - 1;
      } else {
        if (userTriggered) {
          nextIndex = 0;
        } else {
          this.isPlaying = false;
          this.playbackFinished = true;
          return;
        }
      }
    }

    this.pendingSeek = null;
    this.pendingAutoplay = true;
    this.currentSong = { ...this.queue[nextIndex] };
    this.isPlaying = true;
    this.playbackFinished = false;
    this.persistState();
  },

  prevSong() {
    if (!this.currentSong || this.queue.length === 0) return;

    if (this.currentTime > 3) {
      this.pendingSeek = 0;
      this.pendingAutoplay = true;
      this.currentSong = { ...this.currentSong };
      this.isPlaying = true;
      this.playbackFinished = false;
      return;
    }

    let prevIndex;
    const currentIndex = this.queue.findIndex((s) => s.path === this.currentSong.path);

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
    this.currentSong = { ...this.queue[prevIndex] };
    this.isPlaying = true;
    this.playbackFinished = false;
    this.persistState();
  },

  // Seek the current track to `time` seconds. Shared by the player bar, the
  // fullscreen player and lyric-line clicks. Handles the finished-track case by
  // reloading at the requested position.
  async seek(time) {
    const t = Math.max(0, Number(time) || 0);
    this.currentTime = t;
    this.lastSeekAt = Date.now();
    if (this.playbackFinished && this.currentSong) {
      this.pendingSeek = t;
      this.pendingAutoplay = true;
      this.playSong(this.currentSong);
    } else {
      try {
        await invoke('player_seek', { position: t });
      } catch (e) {
        console.error('Seek failed', e);
      }
    }
  },

  setVolume(val) {
    const num = parseFloat(val);
    this.volume = num;
    if (num > 0 && this.isMuted) {
      this.isMuted = false;
    } else if (num === 0 && !this.isMuted) {
      this.isMuted = true;
    }
  },

  toggleMute() {
    this.isMuted = !this.isMuted;
    if (!this.isMuted && this.volume === 0) {
      this.volume = 0.25; // Default volume when unmuting from 0
    }
    this.persistState();
  },

  setParallelism(val) {
    this.useParallelism = val;
  },

  toggleShuffle() {
    this.shuffleMode = !this.shuffleMode;
    this.persistState();
  },

  toggleAutoplay() {
    this.autoplayMode = !this.autoplayMode;
    this.persistState();
  },

  // Push the visualizer on/off state to the Rust backend so the FFT analysis is
  // gated by the Settings toggle (no wasted work when it's hidden).
  syncVisualizer() {
    invoke('player_set_spectrum_enabled', { enabled: this.visualizerEnabled }).catch(() => {});
  },

  setVisualizerEnabled(val) {
    this.visualizerEnabled = !!val;
    this.syncVisualizer();
    this.persistState();
  },

  // Pick a random track from the library for unlimited-queue autoplay. Avoids
  // immediately repeating the current song when the library has alternatives.
  pickRandomSong() {
    if (this.songs.length === 0) return null;
    if (this.songs.length === 1) return this.songs[0];
    const currentPath = this.currentSong ? this.currentSong.path : null;
    let pick;
    let tries = 0;
    do {
      pick = this.songs[Math.floor(Math.random() * this.songs.length)];
    } while (currentPath && pick.path === currentPath && ++tries < 10);
    return pick;
  },

  // Path of the track that will play next under the current queue/loop settings,
  // or null when it's unpredictable (shuffle / autoplay-random). Used to
  // pre-decode the next track for gapless playback.
  nextUpPath() {
    if (!this.currentSong || this.queue.length === 0) return null;
    if (this.loopMode === 2) return this.currentSong.path; // repeat-one
    if (this.shuffleMode) return null;
    const i = this.queue.findIndex((s) => s.path === this.currentSong.path);
    if (i < 0) return null;
    let n = i + 1;
    if (n >= this.queue.length) {
      if (this.loopMode === 1) n = 0;
      else return null; // end of queue (autoplay picks a random track)
    }
    return this.queue[n] ? this.queue[n].path : null;
  },

  get filteredSongs() {
    if (!this.searchQuery) return this.songs;
    const lower = this.searchQuery.toLowerCase();
    return this.songs.filter(
      (song) =>
        (song.title && song.title.toLowerCase().includes(lower)) ||
        (song.artist && song.artist.toLowerCase().includes(lower)) ||
        (song.album && song.album.toLowerCase().includes(lower))
    );
  },
});

store.loadLibrary();
