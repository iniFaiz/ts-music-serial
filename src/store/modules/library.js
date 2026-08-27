import { invokeCommand as invoke } from '../../generated/ipc';
import { idbGet, idbDelete } from '../../libraryStore';
import { retryMissingCovers } from '../../coverCache';
import { EQ_BAND_COUNT } from '../../equalizer';
import { requestDestructiveConsent } from '../../destructiveConsent';
import { setLanguage as setI18nLanguage } from '../../i18n';

function dirName(path) {
  const idx = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  return idx > 0 ? path.slice(0, idx) : path;
}

export const createLibraryState = () => ({
  libraryVersion: 0,
  statsVersion: 0,
  libraryReady: false,
  roots: [],
  loading: false,
  resettingLibrary: false,
  statusMessage: 'Ready to scan',
  selectedPath: '',
  searchQuery: '',
  useParallelism: true,
  scanComplete: false,
  scanDuration: '0',
  scanCount: 0,
  missingTracksReport: [],
  showBackupReportModal: false,
});

export function createLibraryActions() {
  let persistStateTimer = null;
  let statsVersionTimer = null;
  return {
    bumpLibrary() {
      this.libraryVersion++;
      retryMissingCovers();
    },

    bumpStats() {
      if (statsVersionTimer) clearTimeout(statsVersionTimer);
      statsVersionTimer = setTimeout(() => {
        statsVersionTimer = null;
        this.statsVersion++;
      }, 1500);
    },

    bumpStatsImmediate() {
      if (statsVersionTimer) clearTimeout(statsVersionTimer);
      statsVersionTimer = null;
      this.statsVersion++;
    },

    async getTrackByPath(path) {
      if (!path) return null;
      try {
        return await invoke('db_track', { path });
      } catch {
        return null;
      }
    },

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
            closeToTray: this.closeToTray,
            eqEnabled: this.eqEnabled,
            eqPreampDb: this.eqPreampDb,
            eqBands: [...this.eqBands],
            eqPreset: this.eqPreset,
            lyricsSource: this.lyricsSource,
            showRomaji: this.showRomaji,
            lyricsOffsetMs: this.lyricsOffsetMs,
            miniAlwaysOnTop: this.miniAlwaysOnTop,
            waveformEnabled: this.waveformEnabled,
            onlineMetadataEnabled: this.onlineMetadataEnabled,
            language: this.language,
          },
        });
        await invoke('db_kv_set', {
          key: 'playback',
          value: {
            // Queue identity/order/modes are persisted by the native session on
            // every mutation. This row only stores presentation/checkpoint state.
            positionSecs: this.currentTime || 0,
            volume: this.volume,
            isMuted: this.isMuted,
            visualizerEnabled: this.visualizerEnabled,
          },
        });
      } catch (e) {
        console.error('Failed to persist app state', e);
      }
    },

    async loadLibrary() {
      this.libraryReady = false;
      try {
        // One-time migration of the legacy IndexedDB blob into SQLite, the first
        // time the DB is empty. After this the DB is the sole source of truth.
        if ((await invoke('db_count')) === 0) {
          await this.migrateFromIndexedDb();
        }

        const [roots, count] = await Promise.all([invoke('db_roots'), invoke('db_count')]);
        this.roots = roots;
        this.scanCount = count;

        if (this.roots.length > 0) {
          try {
            this.roots = await invoke('restore_roots');
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
        await Promise.all([
          this.refreshFavorites(),
          this.refreshPlaylists(),
          this.refreshRecents(),
        ]);
        this.refreshMusixmatchStatus();
        await this.restoreState();
        // If the app was launched by double-clicking an audio file, play it now —
        // after restoreState so it overrides the restored (paused) session.
        await this.consumePendingOpenFiles();

        // Background scan on startup: automatically detect new/removed files
        // since the last run. Run asynchronously without blocking startup.
        if (this.roots.length > 0) {
          this.refreshLibrary();
        }
      } catch (e) {
        console.error('Failed to load library', e);
        this.statusMessage = `Failed to load library: ${e}`;
      } finally {
        // `libraryReady` means the startup attempt has completed. Keeping it false
        // after an IPC/database error traps every library view behind a permanent
        // loading screen and hides the actionable error message.
        this.libraryReady = true;
      }
    },

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
        if (typeof s.closeToTray === 'boolean') this.closeToTray = s.closeToTray;
        if (typeof s.eqEnabled === 'boolean') this.eqEnabled = s.eqEnabled;
        if (typeof s.eqPreampDb === 'number') this.eqPreampDb = s.eqPreampDb;
        if (Array.isArray(s.eqBands) && s.eqBands.length === EQ_BAND_COUNT)
          this.eqBands = s.eqBands.map((n) => Number(n) || 0);
        if (typeof s.eqPreset === 'string') this.eqPreset = s.eqPreset;
        if (typeof s.lyricsSource === 'string') this.lyricsSource = s.lyricsSource;
        if (typeof s.showRomaji === 'boolean') this.showRomaji = s.showRomaji;
        if (typeof s.lyricsOffsetMs === 'number') this.lyricsOffsetMs = s.lyricsOffsetMs;
        if (typeof s.miniAlwaysOnTop === 'boolean') this.miniAlwaysOnTop = s.miniAlwaysOnTop;
        if (typeof s.waveformEnabled === 'boolean') this.waveformEnabled = s.waveformEnabled;
        if (typeof s.onlineMetadataEnabled === 'boolean')
          this.onlineMetadataEnabled = s.onlineMetadataEnabled;
        if (typeof s.language === 'string') {
          this.language = s.language;
          setI18nLanguage(s.language);
        }

        // Re-select the saved output device (the audio thread starts on default).
        if (this.outputDevice) {
          invoke('set_output_device', { name: this.outputDevice }).catch(() => {});
        }
      }

      // Sync restored/default transition and normalization settings with backend
      invoke('player_set_transition', {
        mode: this.transitionMode,
        crossfadeSecs: this.crossfadeSecs,
      }).catch(() => {});
      invoke('player_set_normalization_settings', {
        enabled: this.normalizationEnabled,
        preampDb: this.normalizationPreampDb,
      }).catch(() => {});
      await invoke('set_wasapi_exclusive', { enabled: this.wasapiExclusive }).catch(() => {});
      if (this.discordEnabled) {
        invoke('discord_set_enabled', { enabled: true }).catch(() => {});
      }
      // Mirror the persisted close-to-tray setting into the backend (creates the
      // tray icon when enabled; the backend default is off).
      if (this.closeToTray) {
        invoke('set_close_to_tray', { enabled: true }).catch(() => {});
      }
      this.syncEqualizer();

      if (pb && typeof pb.volume === 'number') this.volume = pb.volume;
      if (pb && typeof pb.isMuted === 'boolean') this.isMuted = pb.isMuted;
      if (pb && typeof pb.visualizerEnabled === 'boolean')
        this.visualizerEnabled = pb.visualizerEnabled;
      this.syncVisualizer();

      // The native session is the durable source of queue identity/order/modes.
      // Full track metadata is hydrated from SQLite in bounded pages.
      let nativeSnapshot;
      try {
        nativeSnapshot = await invoke('playback_session_snapshot');
        const metadata = new Map();
        for (let offset = 0; offset < nativeSnapshot.queue.length; offset += 250) {
          const page = await invoke('playback_queue_page', { offset, limit: 250 });
          if (page.revision !== nativeSnapshot.revision) {
            throw new Error('Playback queue changed while it was being restored');
          }
          for (const entry of page.entries) metadata.set(entry.entryId, entry.track);
        }
        this.queue = nativeSnapshot.queue.map((entry) => ({
          ...(metadata.get(entry.id) || {
            path: entry.path,
            title: entry.path.split(/[\\/]/).pop() || entry.path,
            artist: 'Unknown Artist',
            album: 'Unknown Album',
            duration_secs: entry.durationHint || 0,
          }),
          queueId: entry.id,
        }));
        this.applyPlaybackSessionUpdate({ snapshot: nativeSnapshot });
      } catch (error) {
        console.error('Failed to restore native playback queue', error);
        nativeSnapshot = null;
        this.queue = [];
      }

      // One-release compatibility path for sessions saved before native session
      // persistence existed.
      const savedEntries =
        !nativeSnapshot?.queue?.length && pb
          ? Array.isArray(pb.queueEntries)
            ? pb.queueEntries.filter((entry) => entry && typeof entry.path === 'string')
            : Array.isArray(pb.queuePaths)
              ? pb.queuePaths.map((path) => ({ id: null, path }))
              : []
          : [];
      if (savedEntries.length > 0) {
        try {
          const tracks = await invoke('db_tracks_by_paths', {
            paths: savedEntries.map((entry) => entry.path),
          });
          let trackIndex = 0;
          this.queue = savedEntries.flatMap((entry) => {
            const track = tracks[trackIndex];
            if (!track || track.path !== entry.path) return [];
            trackIndex++;
            return [
              {
                ...track,
                queueId: entry.id || '',
              },
            ];
          });
        } catch {
          this.queue = [];
        }
      }

      // Load the native current entry paused at the last presentation checkpoint.
      const nativeCurrentId = nativeSnapshot?.currentEntryId;
      if (nativeCurrentId) {
        this.currentTime = pb?.positionSecs || 0;
        this.isPlaying = false;
        await this.sendPlaybackIntent({
          type: 'select_entry',
          entryId: nativeCurrentId,
          autoplay: false,
          startAt: pb?.positionSecs || 0,
        }).catch((error) => console.error('Failed to load restored native queue entry', error));
      } else if (pb?.songPath) {
        const song = await this.getTrackByPath(pb.songPath);
        if (song) {
          this.currentTime = pb.positionSecs || 0;
          this.isPlaying = false;
          let qIdx = pb.currentEntryId
            ? this.queue.findIndex((entry) => entry.queueId === pb.currentEntryId)
            : -1;
          if (qIdx === -1) qIdx = this.queue.findIndex((s) => s.path === song.path);
          if (qIdx !== -1) {
            this.currentSong = { ...this.queue[qIdx] };
          } else {
            const restored = { ...song, queueId: '' };
            this.queue.push(restored);
            this.currentSong = restored;
          }
          await this.playSong(this.currentSong, this.queue, {
            autoplay: false,
            startAt: pb.positionSecs || 0,
          });
        }
      }
    },

    async resetLibrary() {
      const consentToken = await requestDestructiveConsent('reset_library');
      if (!consentToken) {
        this.statusMessage = 'Library reset cancelled';
        return;
      }
      this.resettingLibrary = true;
      this.loading = true;
      this.statusMessage = 'Resetting library...';
      try {
        await invoke('db_reset', { consentToken });
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
      this.statusMessage = 'Clearing caches...';
      this.bumpLibrary();
      try {
        await this.sendPlaybackIntent({ type: 'clear' });
        await invoke('player_stop');
        await invoke('clear_cache', { kind: null });
      } catch (e) {
        console.error('Failed to stop player or clear native caches during reset', e);
      }
      // Drop any lingering legacy IndexedDB blobs too.
      try {
        await idbDelete('library');
        await idbDelete('roots');
        await idbDelete('app_state');
      } catch (e) {
        console.error('Failed to clear legacy caches', e);
      }
      this.statusMessage = 'Library reset';
      this.resettingLibrary = false;
      this.loading = false;
    },

    async clearNativeCache() {
      if (this.loading) return;
      this.loading = true;
      this.statusMessage = 'Clearing cover, waveform, lyrics and loudness caches...';
      try {
        const result = await invoke('clear_cache', { kind: null });
        const freed = Number(result?.bytesFreed || 0);
        const amount =
          freed >= 1024 * 1024
            ? `${(freed / (1024 * 1024)).toFixed(1)} MB`
            : `${Math.round(freed / 1024)} KB`;
        this.statusMessage = `Cache cleared — ${result?.removedFiles || 0} files, ${amount} freed`;
      } catch (error) {
        this.statusMessage = `Failed to clear cache: ${error}`;
      } finally {
        this.loading = false;
      }
    },

    async selectAndScan() {
      this.loading = true;
      this.scanComplete = false;
      this.statusMessage = 'Choose a music folder...';
      try {
        const result = await invoke('add_library_root', {
          useParallelism: this.useParallelism,
        });
        if (!result) return;
        this.roots = await invoke('db_roots');
        const timeSeconds = (result.durationMs / 1000).toFixed(2);
        this.statusMessage = `Added ${result.added} new tracks in ${timeSeconds}s`;
        this.scanDuration = timeSeconds;
        this.scanCount = result.total;
        this.scanComplete = true;
        this.bumpLibrary();
        if (this.onlineMetadataEnabled && result.added > 0) {
          this.startOnlineMetadataImport();
        }
      } catch (err) {
        console.error(err);
        this.statusMessage = `Error: ${err}`;
      } finally {
        this.loading = false;
      }
    },

    async addPaths(dndGrant) {
      if (!dndGrant) return;
      this.loading = true;
      this.statusMessage = 'Adding dropped items...';
      try {
        const result = await invoke('index_library', {
          useParallelism: this.useParallelism,
          pruneMissing: false,
          dndGrant,
        });
        this.roots = await invoke('db_roots');
        this.scanCount = result.total;
        this.scanComplete = true;
        this.bumpLibrary();
        this.statusMessage = `Added ${result.added} new tracks`;
        if (this.onlineMetadataEnabled && result.added > 0) {
          this.startOnlineMetadataImport();
        }
      } catch (e) {
        this.statusMessage = `Error: ${e}`;
      } finally {
        this.loading = false;
      }
    },

    async removeRoot(root) {
      let removed;
      try {
        const consentToken = await requestDestructiveConsent('remove_library_root', [root]);
        if (!consentToken) {
          this.statusMessage = 'Folder removal cancelled';
          return;
        }
        removed = await invoke('remove_library_root', { root, consentToken });
        this.roots = await invoke('db_roots');
      } catch (e) {
        console.error('Failed to remove folder tracks', e);
        this.statusMessage = `Failed to remove folder: ${e}`;
        return;
      }
      if (removed.length) {
        await this.removeQueuePaths(removed);
        // Favorites/playlist items were cascaded in the DB; refresh the caches.
        await this.refreshFavorites();
        await this.refreshPlaylists();
      }
      this.scanCount = await invoke('db_count');
      this.statusMessage = `Removed folder: ${root}`;
      this.bumpLibrary();
      this.watchRoots();
    },

    async refreshLibrary() {
      if (this.roots.length === 0) return;
      this.loading = true;
      this.statusMessage = 'Refreshing library...';
      try {
        const result = await invoke('index_library', {
          useParallelism: this.useParallelism,
          pruneMissing: true,
          dndGrant: null,
        });
        this.scanCount = result.total;
        this.scanComplete = true;
        this.bumpLibrary();
        if (result.removed > 0) {
          await this.reconcileQueueWithLibrary();
          await this.refreshFavorites();
          await this.refreshPlaylists();
        }
        if (this.onlineMetadataEnabled && result.added > 0) {
          this.startOnlineMetadataImport();
        }
        this.statusMessage = `Library refreshed — ${this.scanCount} tracks`;
      } catch (e) {
        this.statusMessage = `Error: ${e}`;
      } finally {
        this.loading = false;
      }
    },

    async reindexLibrary() {
      if (this.roots.length === 0) return;
      this.loading = true;
      this.statusMessage = 'Reindexing...';
      try {
        const result = await invoke('index_library', {
          useParallelism: this.useParallelism,
          pruneMissing: true,
          dndGrant: null,
        });
        const secs = (result.durationMs / 1000).toFixed(2);
        this.scanCount = result.total;
        this.scanComplete = true;
        this.bumpLibrary();
        if (result.removed > 0) {
          await this.reconcileQueueWithLibrary();
          await this.refreshFavorites();
          await this.refreshPlaylists();
        }
        this.statusMessage = `Reindexed ${this.scanCount} tracks in ${secs}s`;
      } catch (e) {
        this.statusMessage = `Error: ${e}`;
      } finally {
        this.loading = false;
      }
    },

    async reconcileQueueWithLibrary() {
      try {
        const original = [...this.queue];
        const metadata = new Map();
        const missingIds = [];
        for (let offset = 0; offset < original.length; offset += 400) {
          const entries = original.slice(offset, offset + 400);
          const tracks = await invoke('db_tracks_by_paths', {
            paths: entries.map((entry) => entry.path),
          });
          let trackIndex = 0;
          for (const entry of entries) {
            const track = tracks[trackIndex];
            if (track?.path === entry.path) {
              metadata.set(entry.queueId, track);
              trackIndex++;
            } else if (entry.queueId) {
              missingIds.push(entry.queueId);
            }
          }
        }

        // Removing the current entry advances the native state machine. Process it
        // last so missing neighbours cannot accidentally select it in between.
        const currentId = this.currentSong?.queueId;
        missingIds.sort((left, right) => Number(left === currentId) - Number(right === currentId));
        for (const entryId of missingIds) {
          await this.sendPlaybackIntent({ type: 'remove_queue_item', entryId });
        }

        this.queue = this.queue.map((entry) => ({
          ...(metadata.get(entry.queueId) || entry),
          queueId: entry.queueId,
        }));
        if (this.currentSong) {
          const current = this.queue.find((entry) => entry.queueId === this.currentSong.queueId);
          if (current) this.currentSong = current;
        }
      } catch (error) {
        console.error('Failed to reconcile playback queue after indexing', error);
      }
    },

    async handleLibraryChanged(summary = {}) {
      this.scanCount = Number.isFinite(summary.total) ? summary.total : await invoke('db_count');
      this.scanComplete = true;
      await this.reconcileQueueWithLibrary();
      if (summary.removed > 0) {
        await this.refreshFavorites();
        await this.refreshPlaylists();
      }
      this.bumpLibrary();
      if (!this.loading && !this.onlineMetadataRunning) {
        this.statusMessage = `Library updated — ${this.scanCount} tracks`;
      }
    },

    watchRoots() {
      invoke('watch_roots').catch(() => {});
    },

    applyTrackUpdate(track) {
      for (const s of this.queue) {
        if (s.path === track.path) Object.assign(s, track);
      }
      if (this.currentSong && this.currentSong.path === track.path) {
        Object.assign(this.currentSong, track);
      }
      this.bumpLibrary();
    },

    setParallelism(val) {
      this.useParallelism = val;
    },
  };
}
