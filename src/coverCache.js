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
const cache = new Map();
const inflight = new Map();

export function hasCachedCover(path) {
  return cache.has(path);
}

export function getCachedCover(path) {
  return cache.get(path) ?? null;
}

export async function loadCover(path) {
  if (!path) return null;
  if (cache.has(path)) return cache.get(path);
  if (inflight.has(path)) return inflight.get(path);

  const request = invoke('get_track_cover', { path })
    .then((result) => {
      const value = result ?? null;
      cache.set(path, value);
      return value;
    })
    .catch(() => {
      // Cache the miss as "no cover" so a failing file isn't retried on every
      // navigation. The placeholder UI is shown for null.
      cache.set(path, null);
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
