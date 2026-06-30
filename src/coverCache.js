import { invoke } from '@tauri-apps/api/core';

// Shared, module-level cover cache.
//
// Cover art is extracted by the Rust `get_track_cover` command, which opens the
// audio file, parses its tags and base64-encodes the embedded picture. That work
// must NOT be repeated every time a CoverImage component mounts (e.g. on every
// page navigation), otherwise covers visibly flash/reload. This cache keeps each
// resolved cover for the lifetime of the app session so subsequent renders are
// instant.
//
//   cache:    path -> data URL string, or null when the file has no cover art.
//   inflight: path -> Promise, so concurrent requests for the same path share a
//             single backend call instead of firing N identical invokes.
//
// The cache is LRU-bounded: each cover is a base64 data URL (~20-40KB), so an
// unbounded Map would grow to tens of MB on a large library scrolled through in
// one session. The Rust disk cache (cover_cache_dir) stays the source of truth,
// so an evicted entry just re-decodes quickly on next view.
const cache = new Map();
const inflight = new Map();
const MAX_COVERS = 500;

// Promote a key to most-recently-used (Map keeps insertion order, so re-inserting
// moves it to the end where it survives eviction longest).
function touch(path) {
  if (cache.has(path)) {
    const v = cache.get(path);
    cache.delete(path);
    cache.set(path, v);
  }
}

// Insert/overwrite, then evict the coldest entries (front of the Map) past the cap.
function cacheSet(path, value) {
  cache.set(path, value);
  while (cache.size > MAX_COVERS) {
    const oldest = cache.keys().next().value;
    cache.delete(oldest);
  }
}

export function hasCachedCover(path) {
  return cache.has(path);
}

export function getCachedCover(path) {
  if (!cache.has(path)) return null;
  touch(path);
  return cache.get(path) ?? null;
}

export async function loadCover(path) {
  if (!path) return null;
  if (cache.has(path)) {
    touch(path);
    return cache.get(path);
  }
  if (inflight.has(path)) return inflight.get(path);

  const request = invoke('get_track_cover', { path })
    .then((result) => {
      const value = result ?? null;
      cacheSet(path, value);
      return value;
    })
    .catch(() => {
      // Cache the miss as "no cover" so a failing file isn't retried on every
      // navigation. The placeholder UI is shown for null.
      cacheSet(path, null);
      return null;
    })
    .finally(() => {
      inflight.delete(path);
    });

  inflight.set(path, request);
  return request;
}

export function clearCoverCache() {
  cache.clear();
  inflight.clear();
}
