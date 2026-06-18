import { watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { store } from './store';

watch(() => store.lyricsSource, () => {
  clearLyricsCache();
});

// Session cache of resolved lyrics, keyed by track path. Value is a Lyrics
// object ({ synced, source, lines: [{ time_ms, text }] }) or null ("not found").
// The Rust backend additionally caches to disk, so this just avoids re-invoking
// across re-opens of the fullscreen player within a session.
const cache = new Map();
const inflight = new Map();

export function getCachedLyrics(path) {
  return cache.has(path) ? cache.get(path) : undefined;
}

// Resolve lyrics for a song through the backend pipeline. `force` bypasses both
// this cache and the backend disk cache (manual retry).
export async function loadLyrics(song, { force = false } = {}) {
  if (!song || !song.path) return null;
  const path = song.path;
  if (!force && cache.has(path)) return cache.get(path);
  if (!force && inflight.has(path)) return inflight.get(path);

  const req = invoke('get_lyrics', {
    path,
    title: song.title || '',
    artist: song.artist || '',
    album: song.album || '',
    durationSecs: Math.round(song.duration_secs || store.duration || 0),
    musixmatchToken: store.musixmatchToken || null,
    lyricsSource: store.lyricsSource || 'lrclib',
    force,
  })
    .then((res) => {
      const value = res || null;
      cache.set(path, value);
      return value;
    })
    .catch(() => {
      cache.set(path, null);
      return null;
    })
    .finally(() => {
      inflight.delete(path);
    });

  inflight.set(path, req);
  return req;
}

// Index of the lyric line that should be highlighted at `timeMs` (the last line
// whose timestamp has passed). Returns -1 before the first line. Assumes lines
// are sorted by time_ms (the backend guarantees this).
export function activeLineIndex(lines, timeMs) {
  if (!lines || lines.length === 0) return -1;
  let lo = 0;
  let hi = lines.length - 1;
  let ans = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const t = lines[mid].time_ms;
    if (t == null || t <= timeMs) {
      ans = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return ans;
}

export function clearLyricsCache() {
  cache.clear();
  inflight.clear();
}
