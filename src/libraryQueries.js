import { invoke } from '@tauri-apps/api/core';

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

export function fetchTracksPage({
  sortBy = 'title',
  order = 'asc',
  search = '',
  offset = 0,
  limit = TRACK_PAGE_SIZE,
} = {}) {
  return invoke('db_tracks_page', {
    sortBy,
    order,
    search: search || null,
    offset,
    limit,
  });
}

export async function fetchAlbums() {
  const rows = await invoke('db_albums', { search: null });
  return rows.map((row) => ({
    name: row.album,
    artist: row.artist,
    count: row.track_count,
    coverPath: row.cover_path,
    lastPlayed: row.last_played,
    allArtists: row.all_artists,
  }));
}

export async function fetchArtists() {
  const rows = await invoke('db_artists', { search: null });
  return rows.map((row) => ({
    name: row.artist,
    count: row.track_count,
    albums: row.album_count,
    coverPath: row.cover_path,
    lastPlayed: row.last_played,
  }));
}

export const fetchFavorites = () => invoke('db_favorites');
export const fetchAlbumTracks = (album) => invoke('db_album_tracks', { album });
export const fetchArtistTracks = (artist) => invoke('db_artist_tracks', { artist });
export const fetchPlaylistTracks = (id) => invoke('db_playlist_tracks', { id });
