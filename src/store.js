import { reactive } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
import { idbGet, idbDelete } from './libraryStore';
import { newSmartPlaylist } from './smartPlaylists';
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

// Debounce handle for persistState() — bursty callers (track change + stat bump +
// play/pause often fire together) coalesce into a single deep-clone + IDB write.
let persistStateTimer = null;
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
  // Query-driven library: the full track list lives in SQLite (Rust), not here.
  // Views fetch what they render via db_* commands and re-run whenever these
  // counters bump. `libraryVersion` = structural changes (tracks / favorites /
  // playlists); `statsVersion` = play-count / last-played updates (so Home
  // insights refresh without forcing every list to reload).
  libraryVersion: 0,
  statsVersion: 0,
  libraryReady: false,
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
  // WASAPI exclusive output (Windows): routes playback to a dedicated exclusive
  // engine that bypasses the OS audio mixer. Falls back to shared mode if the
  // device refuses exclusive access. Transitions are ignored while it's on.
  wasapiExclusive: false,
  // Discord Rich Presence: shows the current track as your Discord status.
  // The Application ID is hardcoded in the backend — the user only toggles this.
  discordEnabled: false,
  // Runtime cache of resolved album-art URLs for Discord, keyed by artist|album
  // (or artist|title). Not persisted; just avoids re-querying on play/pause.
  discordCoverCache: {},
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
  // Global lyric timing offset in ms. Positive = show lines earlier (shift the
  // playhead the lyrics see forward); negative = later. For sources that are
  // systematically off. Applied to every lyric view's current-time.
  lyricsOffsetMs: 0,
  // Waveform seek bar: replace the plain seek slider with a precomputed amplitude
  // waveform (peaks decoded + cached in Rust). Off by default — the first play of
  // each track triggers a one-time full decode to build its waveform.
  waveformEnabled: false,

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

  // Play statistics now live in the SQLite `stats` table (see db.rs); the
  // frontend records plays/skips via db_record_* and reads them back through the
  // insight/query commands rather than holding a per-path map here.
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

  // ---- Query-driven library helpers --------------------------------------

  // Bump the structural version so every useQuery view refetches. Call after any
  // change to tracks / favorites / playlists.
  bumpLibrary() {
    this.libraryVersion++;
  },
  // Bump only the stats version (Home insights) so play/skip accounting doesn't
  // force every track list in the app to reload.
  bumpStats() {
    this.statsVersion++;
  },

  // Reload the small caches the UI reads synchronously: favorite paths (for
  // isFavorite checks), playlist metadata, and recent containers. The heavy
  // track lists are always fetched on demand by views.
  async refreshFavorites() {
    try {
      this.favorites = await invoke('db_favorite_paths');
    } catch (e) {
      console.error('Failed to load favorites', e);
    }
  },
  async refreshPlaylists() {
    try {
      this.playlists = await invoke('db_playlists');
    } catch (e) {
      console.error('Failed to load playlists', e);
    }
  },
  async refreshRecents() {
    try {
      this.recents = await invoke('db_recents');
    } catch (e) {
      console.error('Failed to load recents', e);
    }
  },

  // Fetch a single track object by path (queue hydration / gapless prepare).
  async getTrackByPath(path) {
    if (!path) return null;
    try {
      return await invoke('db_track', { path });
    } catch {
      return null;
    }
  },

  // Persist lightweight app state (likes, playlists, and what/where was
  // playing) so the next launch can restore it. Called on every relevant
  // change and on an interval to checkpoint the playback position.
  //
  // Debounced: the write deep-clones playlists/stats/recents, so coalesce the
  // bursts of callers that fire together. `flushState()` forces it immediately
  // (used on window close, where we can't wait for the debounce window).
  persistState() {
    if (persistStateTimer) clearTimeout(persistStateTimer);
    persistStateTimer = setTimeout(() => {
      persistStateTimer = null;
      this._writeAppState();
    }, 400);
  },

  async flushState() {
    if (persistStateTimer) {
      clearTimeout(persistStateTimer);
      persistStateTimer = null;
    }
    return this._writeAppState();
  },

  async _writeAppState() {
    // Favorites, playlists, stats and recents each live in their own SQLite
    // table (written directly by their mutations), so app-state persistence is
    // now just the settings + the live playback session, stored as two kv rows.
    try {
      await invoke('db_kv_set', {
        key: 'settings',
        value: {
          outputDevice: this.outputDevice,
          normalizationEnabled: this.normalizationEnabled,
          normalizationPreampDb: this.normalizationPreampDb,
          transitionMode: this.transitionMode,
          crossfadeSecs: this.crossfadeSecs,
          wasapiExclusive: this.wasapiExclusive,
          discordEnabled: this.discordEnabled,
          eqEnabled: this.eqEnabled,
          eqPreampDb: this.eqPreampDb,
          eqBands: [...this.eqBands],
          eqPreset: this.eqPreset,
          musixmatchToken: this.musixmatchToken,
          lyricsSource: this.lyricsSource,
          showRomaji: this.showRomaji,
          lyricsOffsetMs: this.lyricsOffsetMs,
          miniAlwaysOnTop: this.miniAlwaysOnTop,
          waveformEnabled: this.waveformEnabled,
        },
      });
      await invoke('db_kv_set', {
        key: 'playback',
        value: {
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
      // One-time migration of the legacy IndexedDB blob into SQLite, the first
      // time the DB is empty. After this the DB is the sole source of truth.
      if ((await invoke('db_count')) === 0) {
        await this.migrateFromIndexedDb();
      }

      this.roots = await invoke('db_roots');
      this.scanCount = await invoke('db_count');

      if (this.roots.length > 0) {
        try {
          await invoke('restore_roots', { roots: this.roots });
        } catch (e) {
          console.error('Failed to restore roots', e);
        }
        // Start watching folders so the library auto-updates on disk changes.
        this.watchRoots();
      }

      if (this.scanCount > 0) {
        this.statusMessage = `Loaded ${this.scanCount} songs`;
        this.scanComplete = true;
      }

      // Load the small synchronous caches (favorite paths, playlist metadata,
      // recents), then restore settings + the last playback session.
      await this.refreshFavorites();
      await this.refreshPlaylists();
      await this.refreshRecents();
      this.libraryReady = true;
      this.bumpLibrary();
      await this.restoreState();
    } catch (e) {
      console.error('Failed to load library', e);
    }
  },

  // Seed SQLite from the old IndexedDB (or localStorage) data, once. Reads the
  // legacy library / roots / app_state blobs and hands them to db_import, which
  // populates the tracks, stats, favorites, playlists and recents tables plus
  // the settings/playback kv rows.
  async migrateFromIndexedDb() {
    try {
      let library = await idbGet('library');
      if (!Array.isArray(library)) {
        const legacy = localStorage.getItem('music_library');
        if (legacy) {
          try {
            library = JSON.parse(legacy);
          } catch {
            library = null;
          }
        }
      }
      const roots = (await idbGet('roots')) || [];
      const state = (await idbGet('app_state')) || {};
      const tracks = Array.isArray(library) ? library : [];

      // Fall back to each track's folder as a root when none were recorded.
      let resolvedRoots = Array.isArray(roots) ? roots : [];
      if (resolvedRoots.length === 0 && tracks.length > 0) {
        resolvedRoots = [...new Set(tracks.map((s) => dirName(s.path)))];
      }

      if (tracks.length > 0 || state.favorites || state.playlists || state.settings) {
        await invoke('db_import', { tracks, roots: resolvedRoots, state });
        if (tracks.length > 0) {
          this.statusMessage = `Migrated ${tracks.length} songs to database`;
        }
      }

      // Drop the big legacy library blob (keep app_state harmlessly for safety).
      try {
        await idbDelete('library');
        localStorage.removeItem('music_library');
      } catch {
        /* ignore */
      }
    } catch (e) {
      console.error('IndexedDB → SQLite migration failed', e);
    }
  },

  async restoreState() {
    let s, pb;
    try {
      s = await invoke('db_kv_get', { key: 'settings' });
      pb = await invoke('db_kv_get', { key: 'playback' });
    } catch (e) {
      console.error('Failed to read app state', e);
      return;
    }

    if (s) {
      if (typeof s.outputDevice !== 'undefined') this.outputDevice = s.outputDevice;
      if (typeof s.normalizationEnabled === 'boolean')
        this.normalizationEnabled = s.normalizationEnabled;
      if (typeof s.normalizationPreampDb === 'number')
        this.normalizationPreampDb = s.normalizationPreampDb;
      if (typeof s.transitionMode === 'string') this.transitionMode = s.transitionMode;
      if (typeof s.crossfadeSecs === 'number') this.crossfadeSecs = s.crossfadeSecs;
      if (typeof s.wasapiExclusive === 'boolean') this.wasapiExclusive = s.wasapiExclusive;
      if (typeof s.discordEnabled === 'boolean') this.discordEnabled = s.discordEnabled;
      if (typeof s.eqEnabled === 'boolean') this.eqEnabled = s.eqEnabled;
      if (typeof s.eqPreampDb === 'number') this.eqPreampDb = s.eqPreampDb;
      if (Array.isArray(s.eqBands) && s.eqBands.length === EQ_BAND_COUNT)
        this.eqBands = s.eqBands.map((n) => Number(n) || 0);
      if (typeof s.eqPreset === 'string') this.eqPreset = s.eqPreset;
      if (typeof s.musixmatchToken === 'string') this.musixmatchToken = s.musixmatchToken;
      if (typeof s.lyricsSource === 'string') this.lyricsSource = s.lyricsSource;
      if (typeof s.showRomaji === 'boolean') this.showRomaji = s.showRomaji;
      if (typeof s.lyricsOffsetMs === 'number') this.lyricsOffsetMs = s.lyricsOffsetMs;
      if (typeof s.miniAlwaysOnTop === 'boolean') this.miniAlwaysOnTop = s.miniAlwaysOnTop;
      if (typeof s.waveformEnabled === 'boolean') this.waveformEnabled = s.waveformEnabled;

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
    await invoke('set_wasapi_exclusive', { enabled: this.wasapiExclusive }).catch(() => {});
    if (this.discordEnabled) {
      invoke('discord_set_enabled', { enabled: true }).catch(() => {});
    }
    this.syncEqualizer();

    if (!pb) return;

    if (typeof pb.volume === 'number') this.volume = pb.volume;
    if (typeof pb.isMuted === 'boolean') this.isMuted = pb.isMuted;
    this.loopMode = pb.loopMode || 0;
    this.shuffleMode = !!pb.shuffleMode;
    this.autoplayMode = !!pb.autoplayMode;
    if (typeof pb.visualizerEnabled === 'boolean') this.visualizerEnabled = pb.visualizerEnabled;
    this.syncVisualizer();

    // Rehydrate the saved queue from the DB (order preserved by db_tracks_by_paths).
    if (Array.isArray(pb.queuePaths) && pb.queuePaths.length) {
      try {
        this.queue = await invoke('db_tracks_by_paths', { paths: pb.queuePaths });
      } catch {
        this.queue = [];
      }
    }

    // Re-load the last track but leave it paused at the saved position; the
    // PlayerControls watcher reads pendingSeek/pendingAutoplay when it loads.
    if (pb.songPath) {
      const song = await this.getTrackByPath(pb.songPath);
      if (song) {
        this.pendingSeek = pb.positionSecs || 0;
        this.pendingAutoplay = false;
        this.currentTime = pb.positionSecs || 0;
        this.isPlaying = false;
        this.currentSong = song;
      }
    }
  },

  async resetLibrary() {
    try {
      await invoke('db_reset');
    } catch (e) {
      console.error('Failed to reset database', e);
    }
    this.roots = [];
    this.favorites = [];
    this.playlists = [];
    this.recents = [];
    this.scanCount = 0;
    this.currentSong = null;
    this.currentTime = 0;
    this.duration = 0;
    this.queue = [];
    this.isPlaying = false;
    this.scanComplete = false;
    this.statusMessage = 'Library reset';
    this.bumpLibrary();
    try {
      await invoke('player_stop');
    } catch (e) {
      console.error('Failed to stop player during reset', e);
    }
    // Drop any lingering legacy IndexedDB blobs too.
    try {
      await idbDelete('library');
      await idbDelete('roots');
      await idbDelete('app_state');
    } catch (e) {
      console.error('Failed to clear legacy caches', e);
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

      // Persist scanned tracks straight into SQLite; returns how many were new.
      const added = await invoke('db_upsert_tracks', { tracks: result });

      if (!this.roots.includes(path)) {
        this.roots = [...this.roots, path];
        await invoke('db_set_roots', { roots: [...this.roots] });
        this.watchRoots();
      }

      const timeSeconds = ((endTime - startTime) / 1000).toFixed(2);
      this.statusMessage = `Added ${added} new tracks in ${timeSeconds}s`;

      this.scanDuration = timeSeconds;
      this.scanCount = await invoke('db_count');
      this.scanComplete = true;
      this.bumpLibrary();
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
      const added = await invoke('db_upsert_tracks', { tracks: result });

      // Register new roots so the watcher and streaming scope cover them. A
      // dropped path that contains scanned tracks is treated as a folder root;
      // otherwise fall back to each scanned track's containing folder.
      const newRoots = [];
      for (const p of list) {
        if (
          !this.roots.some((r) => isUnderRoot(p, r)) &&
          !newRoots.includes(p) &&
          result.some((s) => isUnderRoot(s.path, p))
        ) {
          newRoots.push(p);
        }
      }
      for (const d of new Set(result.map((s) => dirName(s.path)))) {
        if (!this.roots.some((r) => isUnderRoot(d, r)) && !newRoots.some((r) => isUnderRoot(d, r))) {
          newRoots.push(d);
        }
      }
      if (newRoots.length) {
        this.roots = [...this.roots, ...newRoots];
        await invoke('db_set_roots', { roots: [...this.roots] });
        this.watchRoots();
      }
      this.scanCount = await invoke('db_count');
      this.scanComplete = true;
      this.bumpLibrary();
      this.statusMessage = `Added ${added} new tracks`;
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
    await invoke('db_set_roots', { roots: [...this.roots] });

    let removed = [];
    try {
      removed = await invoke('db_remove_under_root', { root });
    } catch (e) {
      console.error('Failed to remove folder tracks', e);
    }
    if (removed.length) {
      const removedSet = new Set(removed);
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
      // Favorites/playlist items were cascaded in the DB; refresh the caches.
      await this.refreshFavorites();
      await this.refreshPlaylists();
    }
    this.scanCount = await invoke('db_count');
    this.statusMessage = `Removed folder: ${root}`;
    this.bumpLibrary();
    this.watchRoots();
  },

  // Re-scan all roots, merging any newly-added files and pruning files that no
  // longer exist on disk. Lightweight — does not touch unchanged tracks.
  async refreshLibrary() {
    if (this.roots.length === 0) return;
    this.loading = true;
    this.statusMessage = 'Refreshing library...';
    try {
      for (const root of this.roots) {
        const result = await invoke('scan_music_folder', {
          path: root,
          useParallelism: this.useParallelism,
        });
        await invoke('db_upsert_tracks', { tracks: result });
      }

      // Prune tracks whose files were deleted; drop them from the queue too.
      try {
        const gone = new Set(await invoke('db_prune_missing'));
        if (gone.size) this.queue = this.queue.filter((s) => !gone.has(s.path));
      } catch {
        /* prune unavailable — keep all */
      }

      this.scanCount = await invoke('db_count');
      this.scanComplete = true;
      this.bumpLibrary();
      this.statusMessage = `Library refreshed — ${this.scanCount} tracks`;
    } catch (e) {
      this.statusMessage = `Error: ${e}`;
    } finally {
      this.loading = false;
    }
  },

  // Rebuild the library by re-scanning every root (refreshes all metadata) and
  // pruning files that no longer exist.
  async reindexLibrary() {
    if (this.roots.length === 0) return;
    this.loading = true;
    this.statusMessage = 'Reindexing...';
    const startTime = performance.now();
    try {
      for (const root of this.roots) {
        const result = await invoke('scan_music_folder', {
          path: root,
          useParallelism: this.useParallelism,
        });
        await invoke('db_upsert_tracks', { tracks: result });
      }
      try {
        const gone = new Set(await invoke('db_prune_missing'));
        if (gone.size) this.queue = this.queue.filter((s) => !gone.has(s.path));
      } catch {
        /* ignore */
      }
      const secs = ((performance.now() - startTime) / 1000).toFixed(2);
      this.scanCount = await invoke('db_count');
      this.scanComplete = true;
      this.bumpLibrary();
      this.statusMessage = `Reindexed ${this.scanCount} tracks in ${secs}s`;
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

  // ---- WASAPI exclusive output -------------------------------------------

  async setWasapiExclusive(v) {
    this.wasapiExclusive = !!v;
    this.persistState();
    try {
      await invoke('set_wasapi_exclusive', { enabled: this.wasapiExclusive });
    } catch (err) {
      console.warn("Failed to set WASAPI exclusive:", err);
      // If enabling failed, revert so the UI stays in sync with the backend.
      if (this.wasapiExclusive) {
        this.wasapiExclusive = false;
        this.persistState();
      }
    }
    // Give the audio thread a moment to fully open/close the stream
    // before reloading the track, avoiding a race with CreateSink.
    await new Promise(r => setTimeout(r, 150));
    // Reload the current track on the newly-selected engine, preserving
    // position/play state (same approach as switching output device).
    if (this.currentSong) {
      this.pendingSeek = this.currentTime || 0;
      this.pendingAutoplay = this.isPlaying;
      this.currentSong = { ...this.currentSong };
    }
  },

  // ---- Discord Rich Presence ---------------------------------------------

  setDiscordEnabled(v) {
    this.discordEnabled = !!v;
    this.persistState();
    invoke('discord_set_enabled', { enabled: this.discordEnabled }).catch(() => {});
    if (this.discordEnabled) this.syncDiscord();
  },
  // Push the current track to Discord (no-op backend-side when disabled). Resolves
  // the album art URL first (cached per album) so Discord can show the cover.
  async syncDiscord() {
    if (!this.discordEnabled) return;
    if (!this.currentSong) {
      invoke('discord_clear').catch(() => {});
      return;
    }
    const song = this.currentSong;
    const title = song.title || '';
    const artist = song.artist || '';
    const album = song.album || '';

    const key = `${artist}␟${album || title}`.toLowerCase();
    let coverUrl = this.discordCoverCache[key];
    if (coverUrl === undefined) {
      try {
        coverUrl = (await invoke('discord_cover_art', { title, artist, album })) || '';
      } catch {
        coverUrl = '';
      }
      this.discordCoverCache[key] = coverUrl;
    }
    // The track may have changed while we awaited the lookup — bail if so.
    if (this.currentSong !== song) return;

    invoke('discord_update', {
      title,
      artist,
      album,
      coverUrl,
      isPlaying: this.isPlaying,
      position: this.currentTime || 0,
      duration: this.duration || 0,
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
  setLyricsOffset(ms) {
    // Clamp to ±3s and round to 50ms steps.
    let v = Math.round((Number(ms) || 0) / 50) * 50;
    v = Math.max(-3000, Math.min(3000, v));
    this.lyricsOffsetMs = v;
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
      // No explicit queue: start one with just this track (views normally pass
      // the visible list; autoplay/next then extends it from the DB).
      this.queue = song ? [{ ...song }] : [];
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

  // `favorites` is a cached array of paths (small) kept in sync with the DB so
  // isFavorite() stays a synchronous check for list rendering.
  isFavorite(path) {
    return this.favorites.includes(path);
  },

  async toggleFavorite(path) {
    if (!path) return;
    // Optimistic local update for instant UI, then reconcile with the DB.
    const idx = this.favorites.indexOf(path);
    if (idx >= 0) this.favorites.splice(idx, 1);
    else this.favorites.push(path);
    try {
      await invoke('db_toggle_favorite', { path });
    } catch (e) {
      console.error('Failed to toggle favorite', e);
    }
    this.bumpLibrary();
  },

  async moveInFavorites(from, to) {
    if (from === to) return;
    if (from < 0 || from >= this.favorites.length) return;
    if (to < 0 || to >= this.favorites.length) return;
    const [item] = this.favorites.splice(from, 1);
    this.favorites.splice(to, 0, item);
    try {
      await invoke('db_move_favorite', { from, to });
    } catch (e) {
      console.error('Failed to reorder favorites', e);
    }
    this.bumpLibrary();
  },

  // ---- Playlists --------------------------------------------------------

  // Write a playlist row (create or update) to the DB. Field names mirror the
  // db_playlists shape (is_smart/sort_by/limit_n…); Tauri maps them to the
  // command's snake_case params.
  async _savePlaylist(pl) {
    try {
      await invoke('db_upsert_playlist', {
        id: pl.id,
        name: pl.name || 'Playlist',
        description: pl.description || '',
        color: pl.color ?? null,
        cover: pl.cover ?? null,
        isSmart: !!pl.is_smart,
        rules: pl.rules ?? null,
        sortBy: pl.sort_by ?? null,
        sortOrder: pl.sort_order ?? null,
        limitN: pl.limit_n ?? null,
        liveUpdate: pl.live_update ?? null,
      });
    } catch (e) {
      console.error('Failed to save playlist', e);
    }
  },

  async createPlaylist(name, description = '', cover = null) {
    const id = 'pl_' + Date.now().toString(36) + Math.random().toString(36).slice(2, 7);
    await this._savePlaylist({
      id,
      name: (name || '').trim() || 'New Playlist',
      description: (description || '').trim(),
      color: null,
      cover: cover || null,
      is_smart: false,
      rules: null,
    });
    await this.refreshPlaylists();
    this.bumpLibrary();
    return this.getPlaylist(id);
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

  async deletePlaylist(id) {
    try {
      await invoke('db_delete_playlist', { id });
    } catch (e) {
      console.error('Failed to delete playlist', e);
    }
    await this.refreshPlaylists();
    this.bumpLibrary();
  },

  async renamePlaylist(id, name) {
    const pl = this.getPlaylist(id);
    if (pl && name && name.trim()) {
      pl.name = name.trim();
      await this._savePlaylist(pl);
      this.bumpLibrary();
    }
  },

  async updatePlaylist(id, name, description, cover) {
    const pl = this.getPlaylist(id);
    if (!pl) return;
    pl.name = (name || '').trim() || 'New Playlist';
    pl.description = (description || '').trim();
    if (cover !== undefined) pl.cover = cover;
    await this._savePlaylist(pl);
    this.bumpLibrary();
  },

  getPlaylist(id) {
    return this.playlists.find((p) => p.id === id);
  },

  async movePlaylistOrder(from, to) {
    if (from === to) return;
    if (from < 0 || from >= this.playlists.length) return;
    if (to < 0 || to >= this.playlists.length) return;
    const [item] = this.playlists.splice(from, 1);
    this.playlists.splice(to, 0, item);
    try {
      await invoke('db_move_playlist_order', { from, to });
    } catch (e) {
      console.error('Failed to reorder playlists', e);
    }
  },

  async addToPlaylist(id, paths) {
    const list = Array.isArray(paths) ? paths : [paths];
    try {
      await invoke('db_playlist_add', { id, paths: list });
    } catch (e) {
      console.error('Failed to add to playlist', e);
    }
    await this.refreshPlaylists();
    this.bumpLibrary();
  },

  async removeFromPlaylist(id, path) {
    try {
      await invoke('db_playlist_remove', { id, path });
    } catch (e) {
      console.error('Failed to remove from playlist', e);
    }
    await this.refreshPlaylists();
    this.bumpLibrary();
  },

  async moveInPlaylist(id, from, to) {
    if (from === to) return;
    try {
      await invoke('db_playlist_move_item', { id, from, to });
    } catch (e) {
      console.error('Failed to reorder playlist', e);
    }
    this.bumpLibrary();
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
    try {
      await invoke('db_remove_paths', { paths: [path] });
    } catch (e) {
      console.error('Failed to remove track from DB', e);
    }
    const favIdx = this.favorites.indexOf(path);
    if (favIdx >= 0) this.favorites.splice(favIdx, 1);
    await this.refreshPlaylists();

    this.scanCount = await invoke('db_count');
    this.bumpLibrary();
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
    try {
      await invoke('db_remove_paths', { paths: [path] });
    } catch (e) {
      console.error('Failed to remove track from DB', e);
    }
    const favIdx = this.favorites.indexOf(path);
    if (favIdx >= 0) this.favorites.splice(favIdx, 1);
    await this.refreshPlaylists();

    this.scanCount = await invoke('db_count');
    this.bumpLibrary();
    this.statusMessage = `Removed file from list: ${path}`;
  },

  // Fetch a normal playlist's tracks (in order) or a smart playlist's evaluated
  // matches from the DB. Async — views call it through useQuery.
  async playlistSongs(id) {
    try {
      return await invoke('db_playlist_tracks', { id });
    } catch (e) {
      console.error('Failed to load playlist tracks', e);
      return [];
    }
  },

  async playPlaylist(id) {
    const list = await this.playlistSongs(id);
    if (list.length > 0) {
      this.recordRecent('playlist', id);
      this.playSong(list[0], list);
    }
  },

  // ---- Play statistics --------------------------------------------------

  // Record accounting now lives in the SQLite `stats` table. These fire-and-
  // forget into the DB and bump `statsVersion` so Home insights refresh.
  recordPlayStart(path) {
    if (!path) return;
    invoke('db_record_play_start', { path }).catch(() => {});
    this.bumpStats();
  },

  // Count a completed/substantial listen toward the play count.
  recordPlay(path) {
    if (!path) return;
    invoke('db_record_play', { path }).catch(() => {});
    this.bumpStats();
  },

  // Count a skip (track abandoned early). Lightweight signal, not surfaced
  // prominently but kept for future "less interested" heuristics.
  recordSkip(path) {
    if (!path) return;
    invoke('db_record_skip', { path }).catch(() => {});
    this.bumpStats();
  },

  // Per-track stats for the info modal. Returns the frontend-friendly camelCase
  // shape. Async — callers await it when a detail view opens.
  async statFor(path) {
    try {
      const r = await invoke('db_stat', { path });
      return { playCount: r.play_count, lastPlayed: r.last_played, skipCount: r.skip_count };
    } catch {
      return { playCount: 0, lastPlayed: 0, skipCount: 0 };
    }
  },

  // Library-wide totals for the Home summary.
  async statsSummary() {
    try {
      const r = await invoke('db_stats_summary');
      return { totalPlays: r.total_plays, totalSeconds: r.total_seconds };
    } catch {
      return { totalPlays: 0, totalSeconds: 0 };
    }
  },

  // ---- Insight collections (fetched from the DB, newest/strongest first) ---
  // Each is an async loader; views drive them through useQuery so they refetch
  // when the library or play stats change.

  async recentlyPlayed(limit = 60) {
    try {
      return await invoke('db_recently_played', { limit });
    } catch {
      return [];
    }
  },
  async mostPlayed(limit = 60) {
    try {
      return await invoke('db_most_played', { limit });
    } catch {
      return [];
    }
  },
  async onRepeat(limit = 60) {
    try {
      return await invoke('db_on_repeat', { limit });
    } catch {
      return [];
    }
  },
  async recentlyAdded(limit = 60) {
    try {
      return await invoke('db_recently_added', { limit });
    } catch {
      return [];
    }
  },
  async rediscover(limit = 60) {
    try {
      return await invoke('db_rediscover', { limit });
    } catch {
      return [];
    }
  },

  // Top artists / genres for Stations. Mapped to { name, plays, tracks, coverPath }
  // so the Home station cards stay unchanged.
  async topArtists(limit = 14) {
    try {
      const rows = await invoke('db_top_artists', { limit });
      return rows.map((r) => ({
        name: r.artist,
        plays: r.plays,
        tracks: r.track_count,
        albums: r.album_count,
        coverPath: r.cover_path,
      }));
    } catch {
      return [];
    }
  },
  async topGenres(limit = 14) {
    try {
      const rows = await invoke('db_top_genres', { limit });
      return rows.map((r) => ({
        name: r.genre,
        plays: r.plays,
        tracks: r.track_count,
        coverPath: r.cover_path,
      }));
    } catch {
      return [];
    }
  },

  // ---- Recently-played containers ---------------------------------------

  // Record that a container (playlist/smart/station/album/collection) was
  // played, so the Home "Recently Played" shelf can surface it.
  recordRecent(type, key) {
    if (!type || !key) return;
    // Optimistic local update (Home shelf reads store.recents) then persist.
    this.recents = this.recents.filter((r) => !(r.type === type && r.key === key));
    this.recents.unshift({ type, key, ts: Date.now() });
    if (this.recents.length > 40) this.recents.length = 40;
    invoke('db_record_recent', { kind: type, key }).catch(() => {});
  },

  // ---- Stations ---------------------------------------------------------

  // Play a shuffled "station" built from every track by an artist or genre.
  async playStation(type, key) {
    let pool = [];
    try {
      pool = await invoke('db_station_tracks', { kind: type, key });
    } catch (e) {
      console.error('Failed to load station', e);
    }
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

  // Smart playlists are rows flagged `is_smart` in the shared playlists cache;
  // normal ones aren't. These getters split the unified array for views.
  get smartPlaylists() {
    return this.playlists.filter((p) => p && p.is_smart);
  },
  get normalPlaylists() {
    return this.playlists.filter((p) => p && !p.is_smart);
  },
  isSmart(pl) {
    return !!(pl && pl.is_smart);
  },

  // Return a smart playlist normalised to the camelCase shape the editor uses
  // (the cache stores snake_case as returned by db_playlists).
  getSmartPlaylist(id) {
    const p = this.playlists.find((x) => x.id === id && x.is_smart);
    if (!p) return null;
    return {
      id: p.id,
      name: p.name,
      description: p.description,
      color: p.color,
      cover: p.cover,
      rules: p.rules,
      sortBy: p.sort_by,
      sortOrder: p.sort_order,
      limit: p.limit_n,
      liveUpdate: p.live_update,
    };
  },

  // Evaluate a smart playlist's rules against the live library (native).
  async smartSongs(id) {
    try {
      return await invoke('db_playlist_tracks', { id });
    } catch (e) {
      console.error('Failed to evaluate smart playlist', e);
      return [];
    }
  },

  async playSmartPlaylist(id) {
    const list = await this.smartSongs(id);
    if (list.length > 0) {
      this.recordRecent('smart', id);
      this.playSong(list[0], list);
    }
  },

  async createSmartPlaylist(data) {
    const sp = newSmartPlaylist(data || {});
    await this._savePlaylist({
      id: sp.id,
      name: sp.name,
      description: sp.description,
      color: sp.color,
      cover: sp.cover,
      is_smart: true,
      rules: sp.rules,
      sort_by: sp.sortBy,
      sort_order: sp.sortOrder,
      limit_n: sp.limit,
      live_update: sp.liveUpdate,
    });
    await this.refreshPlaylists();
    this.bumpLibrary();
    return this.getPlaylist(sp.id);
  },

  async updateSmartPlaylist(id, data) {
    const pl = this.playlists.find((p) => p.id === id && p.is_smart);
    if (!pl) return;
    await this._savePlaylist({
      id,
      name: data.name ?? pl.name,
      description: data.description ?? pl.description,
      color: data.color ?? pl.color,
      cover: data.cover !== undefined ? data.cover : pl.cover,
      is_smart: true,
      rules: data.rules ?? pl.rules,
      sort_by: data.sortBy ?? pl.sort_by,
      sort_order: data.sortOrder ?? pl.sort_order,
      limit_n: data.limit ?? pl.limit_n,
      live_update: data.liveUpdate ?? pl.live_update,
    });
    await this.refreshPlaylists();
    this.bumpLibrary();
  },

  async deleteSmartPlaylist(id) {
    try {
      await invoke('db_delete_playlist', { id });
    } catch (e) {
      console.error('Failed to delete smart playlist', e);
    }
    await this.refreshPlaylists();
    this.bumpLibrary();
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

  async nextSong(userTriggered = false) {
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
        const song = await this.pickRandomSong();
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

  setWaveformEnabled(val) {
    this.waveformEnabled = !!val;
    this.persistState();
  },

  // Pick a random track from the library for unlimited-queue autoplay. Avoids
  // immediately repeating the current song when the library has alternatives.
  // A random track from the whole library (for unlimited-queue autoplay), chosen
  // natively so we never need the full list in the webview.
  async pickRandomSong() {
    try {
      return await invoke('db_random_track', {
        exclude: this.currentSong ? this.currentSong.path : null,
      });
    } catch {
      return null;
    }
  },

  // Path of the track that will play next under the current queue/loop settings,
  // or null when it's unpredictable (shuffle / autoplay-random). Used to
  // pre-decode the next track for gapless playback. For the autoplay-random case
  // the pick needs a DB round-trip, so it kicks off an async prefetch that sets
  // `preselectedNextSong` and returns null this tick; the caller's watcher re-runs
  // when that reactive field lands and then sees the resolved path.
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
        // Prefetch a random track; the watcher re-runs once it's set.
        if (!this._prefetchingRandom) {
          this._prefetchingRandom = true;
          this.pickRandomSong()
            .then((song) => {
              if (song) this.preselectedNextSong = { ...song };
            })
            .finally(() => {
              this._prefetchingRandom = false;
            });
        }
        return null;
      } else {
        return null;
      }
    }
    return this.queue[n] ? this.queue[n].path : null;
  },
});

store.loadLibrary();
