import { reactive } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
import { idbGet, idbSet, idbDelete } from './libraryStore';
import { sortTracks } from './sortTracks';
import { evaluateSmartPlaylist, newSmartPlaylist } from './smartPlaylists';
import { EQ_PRESETS, EQ_BAND_COUNT, EQ_MIN_DB, EQ_MAX_DB, matchPreset } from './equalizer';

const appWindow = getCurrentWindow();

// Mini player window sizes per sub-view (logical px). The window resizes between
// these as the user switches lyrics / album-art / compact, Apple-Music style.
const MINI_SIZES = {
  lyrics: { w: 360, h: 620 },
  compact: { w: 360, h: 150 },
  artwork: { w: 360, h: 360 },
};
const MINI_MIN_WIDTH = 360;
const MINI_MIN_HEIGHT = 150;
// The main window's original minimum + default size (mirrors tauri.conf.json).
// The default is the fallback restore size if the saved size is ever missing.
const MAIN_MIN_WIDTH = 1000;
const MAIN_MIN_HEIGHT = 480;
const MAIN_DEFAULT_WIDTH = 1200;
const MAIN_DEFAULT_HEIGHT = 700;

// Window geometry saved when entering the mini player, restored on exit. Kept at
// module scope (not in the reactive store) so it's never persisted/serialized.
let savedWindowSize = null; // PhysicalSize from outerSize()
let savedWindowMaximized = false;

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
  preselectedNextSong: null,
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
  // 10-band graphic equalizer (DSP runs in Rust as a Source filter). `eqBands`
  // holds per-band gains in dB for the ISO octave bands 31Hz..16kHz; `eqPreset`
  // is the selected preset id, or 'custom' once a band is hand-adjusted.
  eqEnabled: false,
  eqPreampDb: 0,
  eqBands: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  eqPreset: 'flat',
  // Optional Musixmatch community user token for the lyrics fallback chain.
  musixmatchToken: '',
  // Selected lyrics provider: 'lrclib' | 'local' | 'netease' | 'musixmatch' | 'none'
  lyricsSource: 'netease',
  // Show the romanization (romaji) sub-line under lyrics when one is available.
  // Shared by the fullscreen player and the lyrics sidebar toggles.
  showRomaji: false,

  // Fullscreen Now-Playing overlay (Apple Music style cover + synced lyrics).
  fullscreenOpen: false,
  fullscreenOverlayVisible: false,

  // Apple-Music-style compact mini player. Toggled with Ctrl+Shift+M; shrinks the
  // window and shows a synced-lyrics overlay. `miniAlwaysOnTop` (persisted) keeps
  // the mini window floating above other apps while it's open.
  miniPlayerOpen: false,
  miniAlwaysOnTop: false,
  // Currently selected/active album for keyboard shortcut actions.
  selectedAlbum: null,
  // Drop-overlay visibility while a drag is over the window.
  dragActive: false,

  // Liked songs (array of file paths) and user playlists.
  favorites: [],
  playlists: [], // [{ id, name, paths: [] }]

  // Per-track play statistics: { [path]: { playCount, lastPlayed, skipCount } }.
  // `lastPlayed` is a ms epoch. Drives Home insights and smart playlists.
  stats: {},
  // Smart playlists live in the same `playlists` array as normal ones, flagged
  // by the presence of a `rules` object, so they share ordering, drag-reorder
  // and rendering. `smartPlaylists` / `normalPlaylists` getters split them.
  // Smart-playlist create/edit modal state.
  smartModal: { open: false, mode: 'create', smartId: null },

  // Recently played *containers* (not individual songs) for the Home shelf:
  // [{ type: 'playlist'|'smart'|'station'|'album'|'collection', key, ts }].
  // Most-recent first, de-duplicated by type+key.
  recents: [],

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
        stats: JSON.parse(JSON.stringify(this.stats)),
        recents: JSON.parse(JSON.stringify(this.recents)),
        settings: {
          outputDevice: this.outputDevice,
          normalizationEnabled: this.normalizationEnabled,
          normalizationPreampDb: this.normalizationPreampDb,
          transitionMode: this.transitionMode,
          crossfadeSecs: this.crossfadeSecs,
          eqEnabled: this.eqEnabled,
          eqPreampDb: this.eqPreampDb,
          eqBands: [...this.eqBands],
          eqPreset: this.eqPreset,
          musixmatchToken: this.musixmatchToken,
          lyricsSource: this.lyricsSource,
          showRomaji: this.showRomaji,
          miniAlwaysOnTop: this.miniAlwaysOnTop,
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
    // Migrate legacy separately-stored smart playlists into the unified array.
    if (Array.isArray(state.smartPlaylists) && state.smartPlaylists.length) {
      const seen = new Set(this.playlists.map((p) => p.id));
      for (const sp of state.smartPlaylists) {
        if (!seen.has(sp.id)) this.playlists.push(sp);
      }
    }
    // Every playlist (smart included) needs a `paths` array so library-pruning
    // code can filter it unconditionally.
    this.playlists.forEach((p) => {
      if (!Array.isArray(p.paths)) p.paths = [];
    });
    this.stats = state.stats && typeof state.stats === 'object' ? state.stats : {};
    this.recents = Array.isArray(state.recents) ? state.recents : [];

    const s = state.settings;
    if (s) {
      if (typeof s.outputDevice !== 'undefined') this.outputDevice = s.outputDevice;
      if (typeof s.normalizationEnabled === 'boolean')
        this.normalizationEnabled = s.normalizationEnabled;
      if (typeof s.normalizationPreampDb === 'number')
        this.normalizationPreampDb = s.normalizationPreampDb;
      if (typeof s.transitionMode === 'string') this.transitionMode = s.transitionMode;
      if (typeof s.crossfadeSecs === 'number') this.crossfadeSecs = s.crossfadeSecs;
      if (typeof s.eqEnabled === 'boolean') this.eqEnabled = s.eqEnabled;
      if (typeof s.eqPreampDb === 'number') this.eqPreampDb = s.eqPreampDb;
      if (Array.isArray(s.eqBands) && s.eqBands.length === EQ_BAND_COUNT)
        this.eqBands = s.eqBands.map((n) => Number(n) || 0);
      if (typeof s.eqPreset === 'string') this.eqPreset = s.eqPreset;
      if (typeof s.musixmatchToken === 'string') this.musixmatchToken = s.musixmatchToken;
      if (typeof s.lyricsSource === 'string') this.lyricsSource = s.lyricsSource;
      if (typeof s.showRomaji === 'boolean') this.showRomaji = s.showRomaji;
      if (typeof s.miniAlwaysOnTop === 'boolean') this.miniAlwaysOnTop = s.miniAlwaysOnTop;

      // Re-select the saved output device (the audio thread starts on default).
      if (this.outputDevice) {
        invoke('set_output_device', { name: this.outputDevice }).catch(() => {});
      }
    }

    // Sync restored/default transition and normalization settings with backend
    invoke('player_set_transition', {
      mode: this.transitionMode,
      crossfadeSecs: this.crossfadeSecs
    }).catch(() => {});
    invoke('player_set_normalization_settings', {
      enabled: this.normalizationEnabled,
      preampDb: this.normalizationPreampDb
    }).catch(() => {});
    this.syncEqualizer();

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
    this.currentTime = 0;
    this.duration = 0;
    this.queue = [];
    this.isPlaying = false;
    this.scanComplete = false;
    this.favorites = [];
    this.playlists = [];
    this.stats = {};
    this.recents = [];
    this.statusMessage = 'Library reset';
    try {
      await invoke('player_stop');
    } catch (e) {
      console.error('Failed to stop player during reset', e);
    }
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
    invoke('player_set_normalization_settings', {
      enabled: this.normalizationEnabled,
      preampDb: this.normalizationPreampDb,
    }).catch(() => {});
  },
  setNormalizationPreamp(v) {
    this.normalizationPreampDb = Number(v) || 0;
    this.persistState();
    invoke('player_set_normalization_settings', {
      enabled: this.normalizationEnabled,
      preampDb: this.normalizationPreampDb,
    }).catch(() => {});
  },
  setTransitionMode(v) {
    this.transitionMode = v;
    this.persistState();
    invoke('player_set_transition', {
      mode: this.transitionMode,
      crossfadeSecs: this.crossfadeSecs,
    }).catch(() => {});
  },
  setCrossfadeSecs(v) {
    this.crossfadeSecs = Math.max(1, Math.min(12, Number(v) || 6));
    this.persistState();
    invoke('player_set_transition', {
      mode: this.transitionMode,
      crossfadeSecs: this.crossfadeSecs,
    }).catch(() => {});
  },
  // ---- Equalizer ---------------------------------------------------------

  // Push the current EQ state to the Rust DSP filter. Called on every change and
  // once on startup (so a saved EQ is applied to the restored track).
  syncEqualizer() {
    invoke('player_set_equalizer', {
      enabled: this.eqEnabled,
      gains: [...this.eqBands],
      preampDb: this.eqPreampDb,
    }).catch(() => {});
  },
  setEqEnabled(v) {
    this.eqEnabled = !!v;
    this.persistState();
    this.syncEqualizer();
  },
  setEqBand(i, v) {
    if (i < 0 || i >= this.eqBands.length) return;
    const bands = [...this.eqBands];
    bands[i] = Math.max(EQ_MIN_DB, Math.min(EQ_MAX_DB, Number(v) || 0));
    this.eqBands = bands;
    // Hand-editing a band switches the selection to the matching preset (if the
    // curve happens to equal one) or 'custom'.
    this.eqPreset = matchPreset(bands);
    this.persistState();
    this.syncEqualizer();
  },
  setEqPreamp(v) {
    this.eqPreampDb = Math.max(EQ_MIN_DB, Math.min(EQ_MAX_DB, Number(v) || 0));
    this.persistState();
    this.syncEqualizer();
  },
  applyEqPreset(id) {
    const preset = EQ_PRESETS[id];
    if (!preset) return;
    this.eqBands = [...preset.gains];
    this.eqPreset = id;
    this.persistState();
    this.syncEqualizer();
  },
  resetEq() {
    this.eqBands = [...EQ_PRESETS.flat.gains];
    this.eqPreampDb = 0;
    this.eqPreset = 'flat';
    this.persistState();
    this.syncEqualizer();
  },

  setMusixmatchToken(v) {
    this.musixmatchToken = String(v || '').trim();
    this.persistState();
  },
  setLyricsSource(v) {
    this.lyricsSource = String(v || 'netease');
    this.persistState();
  },
  toggleRomaji() {
    this.showRomaji = !this.showRomaji;
    this.persistState();
  },

  // ---- Fullscreen Now-Playing --------------------------------------------

  async enterFullscreenWithTransition() {
    if (!this.currentSong || this.fullscreenOpen) return;
    this.fullscreenOverlayVisible = true;
    
    // Wait for the fade-in transition (300ms)
    await new Promise(r => setTimeout(r, 300));
    
    this.fullscreenOpen = true;
    try {
      await appWindow.setFullscreen(true);
    } catch (err) {
      console.warn("Tauri fullscreen error:", err);
    }
    
    // Wait slightly for OS window sizing transition to settle
    await new Promise(r => setTimeout(r, 150));
    
    this.fullscreenOverlayVisible = false;
  },

  async exitFullscreenWithTransition() {
    if (!this.fullscreenOpen) return;
    this.fullscreenOverlayVisible = true;
    
    // Wait for the fade-in transition (300ms)
    await new Promise(r => setTimeout(r, 300));
    
    this.fullscreenOpen = false;
    try {
      await appWindow.setFullscreen(false);
    } catch (err) {
      console.warn("Tauri fullscreen restore error:", err);
    }
    
    // Wait slightly for OS window sizing transition to settle
    await new Promise(r => setTimeout(r, 150));
    
    this.fullscreenOverlayVisible = false;
  },

  openFullscreen() {
    this.enterFullscreenWithTransition();
  },

  closeFullscreen() {
    this.exitFullscreenWithTransition();
  },

  toggleFullscreen() {
    if (this.fullscreenOpen) {
      this.exitFullscreenWithTransition();
    } else {
      this.enterFullscreenWithTransition();
    }
  },

  // ---- Mini player (Apple Music compact window) --------------------------

  // Shrink the window into the compact mini player. The overlay is shown first
  // so the resize never flashes the squished full UI. The original size and
  // maximized state are saved for an exact restore.
  async enterMiniPlayer() {
    if (this.miniPlayerOpen) return;
    // The mini player and native fullscreen are mutually exclusive.
    if (this.fullscreenOpen) await this.exitFullscreenWithTransition();
    this.miniPlayerOpen = true;
    try {
      savedWindowMaximized = await appWindow.isMaximized();
      if (savedWindowMaximized) await appWindow.unmaximize();
      savedWindowSize = await appWindow.outerSize();
      // Lower the min size before shrinking, or the OS clamps the new size.
      await appWindow.setMinSize(new LogicalSize(MINI_MIN_WIDTH, MINI_MIN_HEIGHT));
      await this.applyMiniViewSize('lyrics');
      await appWindow.setResizable(false);
      await appWindow.setAlwaysOnTop(this.miniAlwaysOnTop);
    } catch (e) {
      console.warn('Failed to enter mini player', e);
    }
  },

  // Resize the mini window to fit a sub-view ('lyrics' | 'compact' | 'artwork').
  // Driven by MiniPlayer.vue as the user toggles views.
  async applyMiniViewSize(view) {
    if (!this.miniPlayerOpen) return;
    const s = MINI_SIZES[view] || MINI_SIZES.lyrics;
    try {
      await appWindow.setSize(new LogicalSize(s.w, s.h));
    } catch (e) {
      console.warn('Failed to resize mini player', e);
    }
  },

  // Resize to an explicit logical size — used by the compact bar, which measures
  // its own content height so it fits exactly (no stray gap / clipping).
  async applyMiniSize(width, height) {
    if (!this.miniPlayerOpen) return;
    try {
      await appWindow.setSize(new LogicalSize(Math.round(width), Math.round(height)));
    } catch (e) {
      console.warn('Failed to resize mini player', e);
    }
  },

  // Restore the full window. `miniPlayerOpen` is flipped to false FIRST so any
  // pending mini view-resize (applyMiniViewSize/applyMiniSize/fitCompact, some
  // scheduled via nextTick) becomes a no-op and can't shrink the window back to
  // the mini size after we restore it — that race was leaving the main window
  // stuck at the mini size on exit. The leave transition fades the overlay over
  // the (fast) grow-back so there's no flash of the squished full UI.
  async exitMiniPlayer() {
    if (!this.miniPlayerOpen) return;
    this.miniPlayerOpen = false;
    try {
      await appWindow.setAlwaysOnTop(false);
      await appWindow.setResizable(true);
      await appWindow.setMinSize(new LogicalSize(MAIN_MIN_WIDTH, MAIN_MIN_HEIGHT));
      // Fall back to the default size if the saved size is somehow missing, so the
      // window is never left stuck at the mini size.
      const target = savedWindowSize || new LogicalSize(MAIN_DEFAULT_WIDTH, MAIN_DEFAULT_HEIGHT);
      await appWindow.setSize(target);
      if (savedWindowMaximized) await appWindow.maximize();
    } catch (e) {
      console.warn('Failed to exit mini player', e);
    }
    savedWindowSize = null;
    savedWindowMaximized = false;
  },

  toggleMiniPlayer() {
    if (this.miniPlayerOpen) this.exitMiniPlayer();
    else this.enterMiniPlayer();
  },

  setMiniAlwaysOnTop(v) {
    this.miniAlwaysOnTop = !!v;
    this.persistState();
    if (this.miniPlayerOpen) {
      appWindow.setAlwaysOnTop(this.miniAlwaysOnTop).catch(() => {});
    }
  },

  closePopup() {
    this.scanComplete = false;
  },

  playSong(song, newQueue = null) {
    this.preselectedNextSong = null;
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
    this.preselectedNextSong = null;
    this.persistState();
  },

  playNextSongs(songs) {
    const list = songs.map((s) => ({ ...s }));
    if (this.queue.length === 0) {
      this.queue = this.currentSong ? [this.currentSong, ...list] : [...list];
    } else {
      const idx = this.currentQueueIndex();
      this.queue.splice(idx + 1, 0, ...list);
    }
    this.preselectedNextSong = null;
    this.persistState();
  },

  addToQueue(songs) {
    const list = (Array.isArray(songs) ? songs : [songs]).map((s) => ({ ...s }));
    if (this.queue.length === 0 && this.currentSong) {
      this.queue = [this.currentSong];
    }
    this.queue.push(...list);
    this.preselectedNextSong = null;
    this.persistState();
  },

  removeFromQueue(index) {
    if (index < 0 || index >= this.queue.length) return;
    this.queue.splice(index, 1);
    this.preselectedNextSong = null;
    this.persistState();
  },

  moveInQueue(from, to) {
    if (from === to) return;
    if (from < 0 || from >= this.queue.length) return;
    if (to < 0 || to >= this.queue.length) return;
    const [item] = this.queue.splice(from, 1);
    this.queue.splice(to, 0, item);
    this.preselectedNextSong = null;
    this.persistState();
  },

  playQueueIndex(index) {
    if (index < 0 || index >= this.queue.length) return;
    this.preselectedNextSong = null;
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
    this.preselectedNextSong = null;
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
    const byPath = new Map(this.songs.map((s) => [s.path, s]));
    return this.favorites.map((p) => byPath.get(p)).filter(Boolean);
  },

  moveInFavorites(from, to) {
    if (from === to) return;
    if (from < 0 || from >= this.favorites.length) return;
    if (to < 0 || to >= this.favorites.length) return;
    const [item] = this.favorites.splice(from, 1);
    this.favorites.splice(to, 0, item);
    this.persistState();
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

  // Playlist-create/edit modal.
  playlistModal: { open: false, pendingSongPath: null, mode: 'create', playlistId: null },

  openPlaylistModal(pendingSongPath = null, mode = 'create', playlistId = null) {
    this.playlistModal.pendingSongPath = pendingSongPath;
    this.playlistModal.mode = mode;
    this.playlistModal.playlistId = playlistId;
    this.playlistModal.open = true;
  },

  closePlaylistModal() {
    this.playlistModal.open = false;
    this.playlistModal.pendingSongPath = null;
    this.playlistModal.mode = 'create';
    this.playlistModal.playlistId = null;
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

  updatePlaylist(id, name, description, cover) {
    const pl = this.playlists.find((p) => p.id === id);
    if (pl) {
      pl.name = (name || '').trim() || 'New Playlist';
      pl.description = (description || '').trim();
      if (cover !== undefined) {
        pl.cover = cover;
      }
      this.persistState();
    }
  },

  getPlaylist(id) {
    return this.playlists.find((p) => p.id === id);
  },

  movePlaylistOrder(from, to) {
    if (from === to) return;
    if (from < 0 || from >= this.playlists.length) return;
    if (to < 0 || to >= this.playlists.length) return;
    const [item] = this.playlists.splice(from, 1);
    this.playlists.splice(to, 0, item);
    this.persistState();
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

  moveInPlaylist(id, from, to) {
    if (from === to) return;
    const pl = this.getPlaylist(id);
    if (!pl) return;
    if (from < 0 || from >= pl.paths.length) return;
    if (to < 0 || to >= pl.paths.length) return;
    const [item] = pl.paths.splice(from, 1);
    pl.paths.splice(to, 0, item);
    this.persistState();
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
    delete this.stats[path];

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
    delete this.stats[path];

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
    if (list.length > 0) {
      this.recordRecent('playlist', id);
      this.playSong(list[0], list);
    }
  },

  // ---- Play statistics --------------------------------------------------

  // Stats for a path, with safe defaults so callers never see undefined.
  statFor(path) {
    return this.stats[path] || { playCount: 0, lastPlayed: 0, skipCount: 0 };
  },

  // Mark that playback of a track has begun. Updates "last played" (drives the
  // Recently Played row) but not the play count — that only lands once the
  // listener has heard enough of the track (see recordPlay).
  recordPlayStart(path) {
    if (!path) return;
    const cur = this.stats[path] || { playCount: 0, lastPlayed: 0, skipCount: 0 };
    this.stats[path] = { ...cur, lastPlayed: Date.now() };
    this.persistState();
  },

  // Count a completed/substantial listen toward the play count.
  recordPlay(path) {
    if (!path) return;
    const cur = this.stats[path] || { playCount: 0, lastPlayed: 0, skipCount: 0 };
    this.stats[path] = { ...cur, playCount: (cur.playCount || 0) + 1, lastPlayed: Date.now() };
    this.persistState();
  },

  // Count a skip (track abandoned early). Lightweight signal, not surfaced
  // prominently but kept for future "less interested" heuristics.
  recordSkip(path) {
    if (!path) return;
    const cur = this.stats[path] || { playCount: 0, lastPlayed: 0, skipCount: 0 };
    this.stats[path] = { ...cur, skipCount: (cur.skipCount || 0) + 1 };
    this.persistState();
  },

  // Total number of plays across the library (for the Home summary).
  get totalPlayCount() {
    let n = 0;
    for (const k in this.stats) n += this.stats[k].playCount || 0;
    return n;
  },

  // Approximate total listening time in seconds (playCount × track duration).
  get totalListenSeconds() {
    const byPath = new Map(this.songs.map((s) => [s.path, s]));
    let secs = 0;
    for (const k in this.stats) {
      const song = byPath.get(k);
      if (song) secs += (this.stats[k].playCount || 0) * (song.duration_secs || 0);
    }
    return secs;
  },

  // ---- Insight collections (live, derived from the library + stats) ------
  // Each getter returns songs newest/strongest first. Views slice for display.

  // Adapter passed to the smart-playlist engine and reused by the getters.
  get insightCtx() {
    return {
      now: Date.now(),
      stat: (p) => this.stats[p] || { playCount: 0, lastPlayed: 0, skipCount: 0 },
      isFavorite: (p) => this.favorites.includes(p),
    };
  },

  get recentlyPlayedSongs() {
    return this.songs
      .filter((s) => this.statFor(s.path).lastPlayed > 0)
      .sort((a, b) => this.statFor(b.path).lastPlayed - this.statFor(a.path).lastPlayed);
  },

  get mostPlayedSongs() {
    return this.songs
      .filter((s) => this.statFor(s.path).playCount > 0)
      .sort((a, b) => this.statFor(b.path).playCount - this.statFor(a.path).playCount);
  },

  // "On Repeat": tracks played repeatedly and recently (last 45 days). Scored by
  // play count, with ties broken by recency.
  get onRepeatSongs() {
    const cutoff = Date.now() - 45 * 86400000;
    return this.songs
      .filter((s) => {
        const st = this.statFor(s.path);
        return st.playCount >= 2 && st.lastPlayed >= cutoff;
      })
      .sort((a, b) => {
        const sa = this.statFor(a.path);
        const sb = this.statFor(b.path);
        return sb.playCount - sa.playCount || sb.lastPlayed - sa.lastPlayed;
      });
  },

  get recentlyAddedSongs() {
    return [...this.songs].sort((a, b) => (b.date_added || 0) - (a.date_added || 0));
  },

  // "Rediscover": liked songs not played in the last 60 days (or never), shuffled.
  get rediscoverSongs() {
    const cutoff = Date.now() - 60 * 86400000;
    const list = this.songs.filter((s) => {
      if (!this.favorites.includes(s.path)) return false;
      const lp = this.statFor(s.path).lastPlayed;
      return lp === 0 || lp < cutoff;
    });
    for (let i = list.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [list[i], list[j]] = [list[j], list[i]];
    }
    return list;
  },

  // Top artists by play count (falling back to track count), for Stations.
  get topArtists() {
    const map = new Map();
    for (const s of this.songs) {
      if (!s.artist || s.artist === 'Unknown Artist') continue;
      const cur = map.get(s.artist) || { name: s.artist, plays: 0, tracks: 0, coverPath: s.path };
      cur.plays += this.statFor(s.path).playCount || 0;
      cur.tracks += 1;
      map.set(s.artist, cur);
    }
    return [...map.values()].sort((a, b) => b.plays - a.plays || b.tracks - a.tracks);
  },

  // Top genres by play count (falling back to track count), for Stations.
  get topGenres() {
    const map = new Map();
    for (const s of this.songs) {
      if (!s.genre) continue;
      const cur = map.get(s.genre) || { name: s.genre, plays: 0, tracks: 0, coverPath: s.path };
      cur.plays += this.statFor(s.path).playCount || 0;
      cur.tracks += 1;
      map.set(s.genre, cur);
    }
    return [...map.values()].sort((a, b) => b.plays - a.plays || b.tracks - a.tracks);
  },

  // ---- Recently-played containers ---------------------------------------

  // Record that a container (playlist/smart/station/album/collection) was
  // played, so the Home "Recently Played" shelf can surface it.
  recordRecent(type, key) {
    if (!type || !key) return;
    this.recents = this.recents.filter((r) => !(r.type === type && r.key === key));
    this.recents.unshift({ type, key, ts: Date.now() });
    if (this.recents.length > 40) this.recents.length = 40;
    this.persistState();
  },

  // ---- Stations ---------------------------------------------------------

  // Play a shuffled "station" built from every track by an artist or genre.
  playStation(type, key) {
    let pool;
    if (type === 'genre') pool = this.songs.filter((s) => s.genre === key);
    else pool = this.songs.filter((s) => s.artist === key);
    if (pool.length === 0) return;
    const list = [...pool];
    for (let i = list.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [list[i], list[j]] = [list[j], list[i]];
    }
    this.shuffleMode = false; // queue is already shuffled; play it straight
    this.autoplayMode = true; // keep it going like a radio station
    this.recordRecent('station', `${type}:${key}`);
    this.playSong(list[0], list);
  },

  // ---- Smart playlists --------------------------------------------------

  // Smart playlists are entries in `playlists` that carry a `rules` object;
  // normal ones don't. These getters split the unified array for views.
  get smartPlaylists() {
    return this.playlists.filter((p) => p && p.rules);
  },
  get normalPlaylists() {
    return this.playlists.filter((p) => p && !p.rules);
  },
  isSmart(pl) {
    return !!(pl && pl.rules);
  },

  getSmartPlaylist(id) {
    return this.playlists.find((p) => p.id === id && p.rules);
  },

  // Evaluate a smart playlist's rules against the live library.
  smartSongs(id) {
    const sp = this.getSmartPlaylist(id);
    if (!sp) return [];
    return evaluateSmartPlaylist(sp, this.songs, this.insightCtx);
  },

  playSmartPlaylist(id) {
    const list = this.smartSongs(id);
    if (list.length > 0) {
      this.recordRecent('smart', id);
      this.playSong(list[0], list);
    }
  },

  createSmartPlaylist(data) {
    const sp = newSmartPlaylist(data || {});
    sp.paths = []; // keep a paths array so library-pruning code stays uniform
    this.playlists.push(sp);
    this.persistState();
    return sp;
  },

  updateSmartPlaylist(id, data) {
    const idx = this.playlists.findIndex((p) => p.id === id && p.rules);
    if (idx >= 0) {
      this.playlists[idx] = { ...this.playlists[idx], ...data, id };
      this.persistState();
    }
  },

  deleteSmartPlaylist(id) {
    const idx = this.playlists.findIndex((p) => p.id === id && p.rules);
    if (idx >= 0) {
      this.playlists.splice(idx, 1);
      this.persistState();
    }
  },

  openSmartModal(mode = 'create', smartId = null) {
    this.smartModal.mode = mode;
    this.smartModal.smartId = smartId;
    this.smartModal.open = true;
  },

  closeSmartModal() {
    this.smartModal.open = false;
    this.smartModal.smartId = null;
    this.smartModal.mode = 'create';
  },

  togglePlay() {
    if (!this.currentSong) return;
    if (this.playbackFinished && this.currentSong) {
      this.playSong(this.currentSong);
    } else {
      this.isPlaying = !this.isPlaying;
    }
  },

  toggleLoop() {
    if (!this.currentSong) return;
    this.loopMode = (this.loopMode + 1) % 3;
    this.preselectedNextSong = null;
    this.persistState();
  },

  nextSong(userTriggered = false) {
    if (!this.currentSong || this.queue.length === 0) return;

    if (this.loopMode === 2 && !userTriggered) {
      return;
    }

    let nextIndex;
    const currentIndex = this.queue.findIndex((s) => s.path === this.currentSong.path);

    if (this.preselectedNextSong) {
      const idx = this.queue.findIndex((s) => s.path === this.preselectedNextSong.path);
      if (idx >= 0) {
        nextIndex = idx;
      } else if (this.autoplayMode) {
        this.queue.push(this.preselectedNextSong);
        nextIndex = this.queue.length - 1;
      }
      this.preselectedNextSong = null; // consume
    }

    if (nextIndex === undefined) {
      if (this.shuffleMode) {
        nextIndex = Math.floor(Math.random() * this.queue.length);
      } else {
        nextIndex = currentIndex + 1;
      }
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
    this.preselectedNextSong = null;

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
    if (!this.currentSong) return;
    this.shuffleMode = !this.shuffleMode;
    this.preselectedNextSong = null;
    this.persistState();
  },

  toggleAutoplay() {
    this.autoplayMode = !this.autoplayMode;
    this.preselectedNextSong = null;
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
    
    if (this.preselectedNextSong) {
      return this.preselectedNextSong.path;
    }
    
    if (this.shuffleMode) {
      const currentIndex = this.queue.findIndex((s) => s.path === this.currentSong.path);
      let nextIndex;
      if (this.queue.length > 1) {
        let tries = 0;
        do {
          nextIndex = Math.floor(Math.random() * this.queue.length);
        } while (nextIndex === currentIndex && ++tries < 10);
      } else {
        nextIndex = 0;
      }
      this.preselectedNextSong = this.queue[nextIndex];
      return this.preselectedNextSong.path;
    }
    
    const i = this.queue.findIndex((s) => s.path === this.currentSong.path);
    if (i < 0) return null;
    let n = i + 1;
    if (n >= this.queue.length) {
      if (this.loopMode === 1) {
        n = 0;
      } else if (this.autoplayMode) {
        const song = this.pickRandomSong();
        if (song) {
          this.preselectedNextSong = { ...song };
          return this.preselectedNextSong.path;
        }
        return null;
      } else {
        return null;
      }
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
