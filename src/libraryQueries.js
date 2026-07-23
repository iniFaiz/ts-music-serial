import { invokeCommand as invoke } from './generated/ipc';
import { prewarmCovers } from './coverCache';

export const TRACK_PAGE_SIZE = 300;

export function tracksPageCacheKey({
  sortBy = 'title',
  order = 'asc',
  search = '',
  offset = 0,
  limit = TRACK_PAGE_SIZE,
} = {}) {
  return `tracks:${sortBy}:${order}:${search || ''}:${offset}:${limit}`;
}

export async function fetchTracksPage({
  sortBy = 'title',
  order = 'asc',
  search = '',
  offset = 0,
  limit = TRACK_PAGE_SIZE,
} = {}) {
  const result = await invoke('db_tracks_page', {
    sortBy,
    order,
    search: search || null,
    offset,
    limit,
  });
  if (result && Array.isArray(result.tracks)) {
    await prewarmCovers(result.tracks.map((t) => t.path), 50);
  }
  return result;
}

export async function fetchAlbums() {
  const rows = await invoke('db_albums', { search: null });
  const result = rows.map((row) => ({
    name: row.album,
    artist: row.artist,
    count: row.track_count,
    coverPath: row.cover_path,
    lastPlayed: row.last_played,
    allArtists: row.all_artists,
  }));
  await prewarmCovers(result.map((r) => r.coverPath), 50);
  return result;
}

export async function fetchArtists() {
  const rows = await invoke('db_artists', { search: null });
  const result = rows.map((row) => ({
    name: row.artist,
    count: row.track_count,
    albums: row.album_count,
    coverPath: row.cover_path,
    lastPlayed: row.last_played,
  }));
  await prewarmCovers(result.map((r) => r.coverPath), 50);
  return result;
}

export async function fetchFavorites() {
  const result = await invoke('db_favorites');
  if (Array.isArray(result)) prewarmCovers(result.map((t) => t.path));
  return result;
}

export async function fetchAlbumTracks(album) {
  const result = await invoke('db_album_tracks', { album });
  if (Array.isArray(result)) prewarmCovers(result.map((t) => t.path));
  return result;
}

export async function fetchArtistTracks(artist) {
  const result = await invoke('db_artist_tracks', { artist });
  if (Array.isArray(result)) prewarmCovers(result.map((t) => t.path));
  return result;
}

export async function fetchPlaylistTracks(id) {
  const result = await invoke('db_playlist_tracks', { id });
  if (Array.isArray(result)) prewarmCovers(result.map((t) => t.path));
  return result;
}
