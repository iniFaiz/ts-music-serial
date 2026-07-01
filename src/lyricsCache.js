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
//
// LRU-bounded: a full synced lyric set is a sizeable array of objects, so cap how
// many live on the JS heap at once. The disk cache stays the source of truth.
const cache = new Map();
const inflight = new Map();
const MAX_LYRICS = 300;

function touch(path) {
  if (cache.has(path)) {
    const v = cache.get(path);
    cache.delete(path);
    cache.set(path, v);
  }
}

function cacheSet(path, value) {
  cache.set(path, value);
  while (cache.size > MAX_LYRICS) {
    const oldest = cache.keys().next().value;
    cache.delete(oldest);
  }
}

export function getCachedLyrics(path) {
  if (!cache.has(path)) return undefined;
  touch(path);
  return cache.get(path);
}

// Resolve lyrics for a song through the backend pipeline. `force` bypasses both
// this cache and the backend disk cache (manual retry).
export async function loadLyrics(song, { force = false } = {}) {
  if (!song || !song.path) return null;
  const path = song.path;
  if (!force && cache.has(path)) {
    touch(path);
    return cache.get(path);
  }
  if (!force && inflight.has(path)) return inflight.get(path);

  const req = invoke('get_lyrics', {
    path,
    title: song.title || '',
    artist: song.artist || '',
    album: song.album || '',
    durationSecs: Math.round(song.duration_secs || store.duration || 0),
    lyricsSource: store.lyricsSource || 'netease',
    force,
  })
    .then((res) => {
      const value = res || null;
      cacheSet(path, value);
      return value;
    })
    .catch(() => {
      cacheSet(path, null);
      return null;
    })
    .finally(() => {
      inflight.delete(path);
    });

  inflight.set(path, req);
  return req;
}

// Index of the lyric line that should be highlighted at `timeMs` (the last line
// whose timestamp has passed). Returns -1 before the first line or after the
// last line's estimated end. Assumes lines are sorted by time_ms.
// `songDurationMs` is the total song duration — used to cap the last line.
export function activeLineIndex(lines, timeMs, songDurationMs = 0) {
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

  // If the last line is active, check if time has exceeded its estimated end
  if (ans >= 0 && ans === lines.length - 1 && lines.length >= 2) {
    const lastLine = lines[ans];

    // For gap lines, use their built-in endTimeMs
    if (lastLine.endTimeMs && timeMs > lastLine.endTimeMs) return -1;

    // Estimate end time from average duration of all preceding vocal lines
    let totalDur = 0;
    let count = 0;
    for (let i = 0; i < lines.length - 1; i++) {
      if (lines[i].isGap) continue;
      // Duration of line i = start of next line - start of this line
      let nextStart = null;
      for (let j = i + 1; j < lines.length; j++) {
        if (lines[j].time_ms != null) { nextStart = lines[j].time_ms; break; }
      }
      if (nextStart != null) {
        totalDur += nextStart - lines[i].time_ms;
        count++;
      }
    }

    // Decide when the final line stops being "active". The old average-duration
    // estimate cut it off early — the line went dim while the vocal was still
    // being sung. Prefer real timing:
    //   • word-synced: the last (main or background) word's end + a short tail so
    //     the fully-lit line lingers briefly, like Apple Music.
    //   • otherwise: hold until the track ends rather than guessing short.
    const lastWordEnd = (arr) =>
      arr && arr.length ? arr[arr.length - 1].time_ms + arr[arr.length - 1].duration_ms : 0;
    const wordEnd = Math.max(lastWordEnd(lastLine.words), lastWordEnd(lastLine.bg));

    let lineEnd;
    if (wordEnd > 0) {
      lineEnd = wordEnd + 1500;
    } else if (songDurationMs > 0) {
      lineEnd = songDurationMs;
    } else if (count > 0) {
      lineEnd = lastLine.time_ms + totalDur / count;
    } else {
      lineEnd = Infinity;
    }
    if (songDurationMs > 0) lineEnd = Math.min(lineEnd, songDurationMs + 500);
    if (timeMs > lineEnd) return -1;
  }

  return ans;
}

export function clearLyricsCache() {
  cache.clear();
  inflight.clear();
}
