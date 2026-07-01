import { invoke, convertFileSrc } from '@tauri-apps/api/core';

// Shared, module-level cover cache.
//
// The Rust `get_track_cover_path` command ensures a downscaled JPEG thumbnail
// exists on disk (in cover_cache_dir) and returns its filesystem path. We then
// wrap that path with `convertFileSrc` so the webview loads the image directly
// through the asset protocol — no base64, no image bytes crossing IPC on every
// render. The webview caches the decoded image itself, so re-mounting a
// CoverImage across page navigation is essentially free.
//
//   cache:    path -> asset URL string, or null when the file has no cover art.
//   inflight: path -> Promise, so concurrent requests for the same path share a
//             single backend call instead of firing N identical invokes.
//
// Entries are now just short URL strings (not base64 blobs), so the cache is
// cheap; the LRU cap only guards against pathological unbounded growth. An
// evicted entry re-resolves with a single cheap disk-existence check.
const cache = new Map();
const inflight = new Map();
const MAX_COVERS = 2000;

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

  const request = invoke('get_track_cover_path', { path })
    .then((result) => {
      // Backend returns the on-disk thumbnail path (or null for no art). Convert
      // it to an asset-protocol URL the <img> can load without base64/IPC.
      const value = result ? convertFileSrc(result) : null;
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
