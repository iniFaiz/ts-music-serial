import { invokeCommand as invoke } from '../../generated/ipc';
import { open, save, ask } from '@tauri-apps/plugin-dialog';
import { invalidateCover } from '../../coverCache';
import { requestDestructiveConsent } from '../../destructiveConsent';

export const createIntegrationsState = () => ({
  onlineMetadataEnabled: false,
  onlineMetadataRunning: false,
  onlineMetadataProgress: {
    processed: 0,
    total: 0,
    updated: 0,
    notFound: 0,
    failed: 0,
    done: false,
    cancelled: false,
  },
  onlineMetadataStatus: '',
  discordEnabled: false,
  discordCoverCache: {},
  musixmatchConfigured: false,
});

export function createIntegrationsActions() {
  return {
    setOnlineMetadataEnabled(value) {
      this.onlineMetadataEnabled = !!value;
      this.persistState();
      if (this.onlineMetadataEnabled) {
        // Enabling is an explicit user action, so immediately inspect the whole
        // library for missing fields/artwork.
        this.startOnlineMetadataImport();
      } else {
        invoke('cancel_online_metadata').catch(() => {});
        this.onlineMetadataStatus = this.onlineMetadataRunning
          ? 'Cancelling online metadata lookup...'
          : 'Online metadata is off';
      }
    },

    handleOnlineMetadataProgress(progress) {
      if (!progress || typeof progress !== 'object') return;
      this.onlineMetadataProgress = { ...this.onlineMetadataProgress, ...progress };
      if (!progress.done) {
        this.onlineMetadataStatus = progress.total
          ? `Checking ${progress.processed}/${progress.total} · ${progress.updated} updated`
          : 'No incomplete tracks found';
      }
    },

    async startOnlineMetadataImport(paths = null) {
      if (!this.onlineMetadataEnabled || this.onlineMetadataRunning) return;
      this.onlineMetadataRunning = true;
      this.onlineMetadataProgress = {
        processed: 0,
        total: 0,
        updated: 0,
        notFound: 0,
        failed: 0,
        done: false,
        cancelled: false,
      };
      this.onlineMetadataStatus = 'Finding missing metadata...';
      try {
        const consentToken = await requestDestructiveConsent('import_online_metadata');
        if (!consentToken) {
          this.onlineMetadataStatus = 'Online metadata scan cancelled';
          return;
        }
        const summary = await invoke('import_online_metadata', {
          paths: Array.isArray(paths) ? paths : null,
          consentToken,
        });
        // Keep every live clone (queue/current song) in sync with SQLite and
        // invalidate artwork misses so covers appear without restarting the app.
        for (const track of summary.tracks || []) {
          for (const queued of this.queue) {
            if (queued.path === track.path) Object.assign(queued, track);
          }
          if (this.currentSong && this.currentSong.path === track.path) {
            Object.assign(this.currentSong, track);
          }
          invalidateCover(track.path);
        }
        if ((summary.tracks || []).length) this.bumpLibrary();
        if (summary.cancelled) {
          this.onlineMetadataStatus = `Cancelled · ${summary.updated} updated`;
        } else if (summary.scanned === 0) {
          this.onlineMetadataStatus = 'All metadata and artwork are already complete';
        } else {
          this.onlineMetadataStatus = `${summary.updated} updated · ${summary.notFound} not found${
            summary.failed ? ` · ${summary.failed} failed` : ''
          }`;
        }
      } catch (error) {
        this.onlineMetadataStatus = `Online metadata error: ${error}`;
      } finally {
        this.onlineMetadataRunning = false;
      }
    },

    async consumePendingOpenFiles() {
      let files;
      try {
        files = await invoke('take_pending_open_files');
      } catch {
        return;
      }
      await this.openExternalFiles(files);
    },

    async openExternalFiles(paths) {
      const list = (Array.isArray(paths) ? paths : [paths]).filter(Boolean);
      if (list.length === 0) return;
      try {
        const inLib = await invoke('db_tracks_by_paths', { paths: list });
        const known = new Map(inLib.map((t) => [t.path, t]));
        const missing = list.filter((p) => !known.has(p));
        if (missing.length) {
          const probed = await invoke('probe_files', { paths: missing });
          for (const t of probed) known.set(t.path, t);
        }
        const songs = list.map((p) => known.get(p)).filter(Boolean);
        if (!songs.length) return;
        this.playSong(songs[0], songs);
        this.statusMessage =
          songs.length === 1 ? `Playing ${songs[0].title}` : `Playing ${songs.length} files`;
      } catch (e) {
        console.error('Failed to open files', e);
        this.statusMessage = `Error opening file: ${e}`;
      }
    },

    setDiscordEnabled(v) {
      this.discordEnabled = !!v;
      this.persistState();
      invoke('discord_set_enabled', { enabled: this.discordEnabled }).catch(() => {});
      if (this.discordEnabled) this.syncDiscord();
    },

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

    async setMusixmatchToken(v) {
      const token = String(v || '').trim();
      try {
        await invoke('set_musixmatch_token', { token });
        this.musixmatchConfigured = token.length > 0;
      } catch (e) {
        console.error('Failed to store Musixmatch token', e);
      }
    },

    async refreshMusixmatchStatus() {
      try {
        this.musixmatchConfigured = await invoke('musixmatch_token_status');
      } catch {
        this.musixmatchConfigured = false;
      }
    },

    async exportPlaylistM3u(id) {
      const pl = this.getPlaylist(id);
      if (!pl) return;
      try {
        const safeName = (pl.name || 'playlist').replace(/[\\/:*?"<>|]/g, '_');
        const dest = await save({
          defaultPath: `${safeName}.m3u8`,
          filters: [{ name: 'Playlist', extensions: ['m3u8', 'm3u'] }],
        });
        if (!dest) return;
        const count = await invoke('export_m3u', { dest, playlistId: id });
        this.statusMessage = `Exported ${count} tracks to ${dest}`;
      } catch (e) {
        console.error('Failed to export playlist', e);
        this.statusMessage = `Export failed: ${e}`;
      }
    },

    async importPlaylistM3u() {
      try {
        const src = await open({
          multiple: false,
          filters: [{ name: 'Playlist', extensions: ['m3u8', 'm3u'] }],
        });
        if (!src) return null;
        const paths = await invoke('import_m3u', { src });
        if (!Array.isArray(paths) || paths.length === 0) {
          this.statusMessage = 'No playable tracks found in that playlist';
          return null;
        }
        // Watch/authorize scope may have changed (new folders); refresh watcher.
        this.roots = await invoke('db_roots');
        this.watchRoots();

        const base = String(src).replace(/\\/g, '/').split('/').pop() || 'Imported Playlist';
        const name = base.replace(/\.(m3u8?|M3U8?)$/, '');
        const pl = await this.createPlaylist(name);
        if (pl) await this.addToPlaylist(pl.id, paths);
        this.scanCount = await invoke('db_count');
        this.statusMessage = `Imported ${paths.length} tracks into "${name}"`;
        return pl;
      } catch (e) {
        console.error('Failed to import playlist', e);
        this.statusMessage = `Import failed: ${e}`;
        return null;
      }
    },

    async exportBackup() {
      try {
        const dest = await save({
          defaultPath: 'ts-music-backup.db',
          filters: [{ name: 'TS Music Backup', extensions: ['db', 'tsmback'] }],
        });
        if (!dest) return;
        this.statusMessage = 'Exporting backup...';
        await invoke('db_export_backup', { dest });
        this.statusMessage = `Backup exported successfully to ${dest}`;
      } catch (e) {
        console.error('Failed to export backup', e);
        this.statusMessage = `Export backup failed: ${e}`;
      }
    },

    async importBackup() {
      try {
        const src = await open({
          multiple: false,
          filters: [{ name: 'TS Music Backup', extensions: ['db', 'tsmback'] }],
        });
        if (!src) return;

        this.showConfirm({
          title: 'Import Backup',
          message:
            'Importing this backup will overwrite your current library, settings, and playlists. Playback will stop. Proceed?',
          confirmText: 'Import',
          cancelText: 'Cancel',
          onConfirm: async () => {
            const consentToken = await requestDestructiveConsent('import_backup', [src]);
            if (!consentToken) {
              this.statusMessage = 'Backup import cancelled';
              return;
            }
            this.loading = true;
            this.statusMessage = 'Importing backup...';

            // Stop playback and reset player
            this.isPlaying = false;
            this.currentSong = null;
            this.currentTime = 0;
            this.duration = 0;
            try {
              await invoke('player_stop');
            } catch {
              /* ignore */
            }

            try {
              const res = await invoke('db_import_backup', { src, consentToken });

              // Backup paths are data, not filesystem authority. Every restored
              // root must be confirmed through the Rust-side native picker,
              // including a folder that still exists at the original location.
              for (const root of res.roots) {
                const action = root.exists ? 're-authorize' : 'locate';
                const confirmRoot = await ask(
                  root.exists
                    ? `The backup contains "${root.path}". Re-authorize this music folder before restoring access?`
                    : `The music folder "${root.path}" was not found. Select its new location to preserve statistics and playlists?`,
                  {
                    title: root.exists ? 'Authorize Music Folder' : 'Relocate Music Folder',
                    kind: 'warning',
                    okLabel: root.exists ? 'Authorize Folder' : 'Select Folder',
                    cancelLabel: 'Skip',
                  }
                );

                if (confirmRoot) {
                  this.statusMessage = `${action === 'locate' ? 'Relocating' : 'Authorizing'} ${root.path}...`;
                  await invoke('db_relocate_root', { oldRoot: root.path });
                }
              }

              // Prune remaining missing tracks and get details for the report
              this.statusMessage = 'Pruning missing tracks...';
              const missingTracks = await invoke('db_prune_and_get_missing');

              // Reload the library state from the new database file
              await this.loadLibrary();

              if (missingTracks.length > 0) {
                this.missingTracksReport = missingTracks;
                this.showBackupReportModal = true;
                this.statusMessage = `Backup imported. ${missingTracks.length} missing tracks were removed from library.`;
              } else {
                this.statusMessage = 'Backup imported successfully';
              }
            } catch (e) {
              console.error('Failed to import backup', e);
              this.statusMessage = `Import backup failed: ${e}`;
            } finally {
              this.loading = false;
            }
          },
        });
      } catch (err) {
        console.error(err);
        this.statusMessage = 'Error selecting backup file';
      }
    },
  };
}
