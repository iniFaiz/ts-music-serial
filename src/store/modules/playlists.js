import { invokeCommand as invoke } from '../../generated/ipc';
import { newSmartPlaylist } from '../../smartPlaylists';
import { requestDestructiveConsent } from '../../destructiveConsent';

export const createPlaylistsState = () => ({
  favoritesVersion: 0,
  playlistsVersion: 0,
  favorites: [],
  playlists: [],
  smartModal: { open: false, mode: 'create', smartId: null },
  recents: [],
  playlistModal: { open: false, pendingSongPath: null, mode: 'create', playlistId: null },
  confirmModal: {
    open: false,
    title: '',
    message: '',
    confirmText: 'Confirm',
    cancelText: 'Cancel',
    onConfirm: null,
  },
});

export function createPlaylistsActions() {
  return {
bumpFavorites() {
    this.favoritesVersion++;
  },

bumpPlaylists() {
    this.playlistsVersion++;
  },

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
    this.bumpFavorites();
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
    this.bumpFavorites();
  },

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
    this.bumpPlaylists();
    return this.getPlaylist(id);
  },

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

showConfirm(options) {
    this.confirmModal.title = options.title || 'Confirm';
    this.confirmModal.message = options.message || '';
    this.confirmModal.confirmText = options.confirmText || 'Confirm';
    this.confirmModal.cancelText = options.cancelText || 'Cancel';
    this.confirmModal.onConfirm = options.onConfirm || null;
    this.confirmModal.open = true;
  },

closeConfirm() {
    this.confirmModal.open = false;
    this.confirmModal.title = '';
    this.confirmModal.message = '';
    this.confirmModal.confirmText = 'Confirm';
    this.confirmModal.cancelText = 'Cancel';
    this.confirmModal.onConfirm = null;
  },

async deletePlaylist(id) {
    const consentToken = await requestDestructiveConsent('delete_playlist', [id]);
    if (!consentToken) return false;
    try {
      await invoke('db_delete_playlist', { id, consentToken });
    } catch (e) {
      console.error('Failed to delete playlist', e);
      throw e;
    }
    await this.refreshPlaylists();
    this.bumpPlaylists();
    return true;
  },

async renamePlaylist(id, name) {
    const pl = this.getPlaylist(id);
    if (pl && name && name.trim()) {
      pl.name = name.trim();
      await this._savePlaylist(pl);
      this.bumpPlaylists();
    }
  },

async updatePlaylist(id, name, description, cover) {
    const pl = this.getPlaylist(id);
    if (!pl) return;
    pl.name = (name || '').trim() || 'New Playlist';
    pl.description = (description || '').trim();
    if (cover !== undefined) pl.cover = cover;
    await this._savePlaylist(pl);
    this.bumpPlaylists();
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
    this.bumpPlaylists();
  },

async removeFromPlaylist(id, path) {
    try {
      await invoke('db_playlist_remove', { id, path });
    } catch (e) {
      console.error('Failed to remove from playlist', e);
    }
    await this.refreshPlaylists();
    this.bumpPlaylists();
  },

async moveInPlaylist(id, from, to) {
    if (from === to) return;
    try {
      await invoke('db_playlist_move_item', { id, from, to });
    } catch (e) {
      console.error('Failed to reorder playlist', e);
    }
    this.bumpPlaylists();
  },

async requestDeleteConsent(paths) {
    return requestDestructiveConsent('delete_audio', paths);
  },

async deleteSong(path, existingConsentToken = null) {
    const consentToken =
      existingConsentToken || (await requestDestructiveConsent('delete_audio', [path]));
    if (!consentToken) return false;
    try {
      await invoke('player_delete_file', { path, consentToken });
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
    const favIdx = this.favorites.indexOf(path);
    if (favIdx >= 0) this.favorites.splice(favIdx, 1);
    await this.refreshPlaylists();

    this.scanCount = await invoke('db_count');
    this.bumpLibrary();
    this.statusMessage = `Moved file to Trash: ${path}`;
    return true;
  },

async removeSongFromLibrary(path) {
    const consentToken = await requestDestructiveConsent('remove_library_tracks', [path]);
    if (!consentToken) return false;
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
      await invoke('db_remove_paths', { paths: [path], consentToken });
    } catch (e) {
      console.error('Failed to remove track from DB', e);
      throw e;
    }
    const favIdx = this.favorites.indexOf(path);
    if (favIdx >= 0) this.favorites.splice(favIdx, 1);
    await this.refreshPlaylists();

    this.scanCount = await invoke('db_count');
    this.bumpLibrary();
    this.statusMessage = `Removed file from list: ${path}`;
    return true;
  },

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

recordPlayStart(path) {
    if (!path) return;
    invoke('db_record_play_start', { path }).catch(() => {});
    this.bumpStatsImmediate();
  },

recordPlay(path) {
    if (!path) return;
    invoke('db_record_play', { path }).catch(() => {});
    this.bumpStats();
  },

recordSkip(path) {
    if (!path) return;
    invoke('db_record_skip', { path }).catch(() => {});
    this.bumpStats();
  },

async statFor(path) {
    try {
      const r = await invoke('db_stat', { path });
      return { playCount: r.play_count, lastPlayed: r.last_played, skipCount: r.skip_count };
    } catch {
      return { playCount: 0, lastPlayed: 0, skipCount: 0 };
    }
  },

async statsSummary() {
    try {
      const r = await invoke('db_stats_summary');
      return { totalPlays: r.total_plays, totalSeconds: r.total_seconds };
    } catch {
      return { totalPlays: 0, totalSeconds: 0 };
    }
  },

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

recordRecent(type, key) {
    if (!type || !key) return;
    // Optimistic local update (Home shelf reads store.recents) then persist.
    this.recents = this.recents.filter((r) => !(r.type === type && r.key === key));
    this.recents.unshift({ type, key, ts: Date.now() });
    if (this.recents.length > 40) this.recents.length = 40;
    invoke('db_record_recent', { kind: type, key }).catch(() => {});
  },

get smartPlaylists() {
    return this.playlists.filter((p) => p && p.is_smart);
  },

get normalPlaylists() {
    return this.playlists.filter((p) => p && !p.is_smart);
  },

isSmart(pl) {
    return !!(pl && pl.is_smart);
  },

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
    this.bumpPlaylists();
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
    this.bumpPlaylists();
  },

async deleteSmartPlaylist(id) {
    const consentToken = await requestDestructiveConsent('delete_playlist', [id]);
    if (!consentToken) return false;
    try {
      await invoke('db_delete_playlist', { id, consentToken });
    } catch (e) {
      console.error('Failed to delete smart playlist', e);
      throw e;
    }
    await this.refreshPlaylists();
    this.bumpPlaylists();
    return true;
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
  }
  };
}
