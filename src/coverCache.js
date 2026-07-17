import { ref } from 'vue';
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
const RETRY_DELAY_MS = 100;

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

  const request = invokeCoverPath(path)
    .then((result) => {
      // Backend returns the on-disk thumbnail path (or null for no art). Convert
      // it to an asset-protocol URL the <img> can load without base64/IPC.
      const value = result ? convertFileSrc(result) : null;
      // A real miss is deliberately not cached forever: indexing or online
      // metadata may attach artwork to this same path moments later.
      if (value) cacheSet(path, value);
      return value;
    })
    .catch((error) => {
      // An IPC/authorization failure can be transient while roots are being
      // restored or an index transaction is finishing. Never turn that into a
      // permanent negative cache entry.
      console.warn(`Failed to load cover for ${path}`, error);
      return null;
    })
    .finally(() => {
      inflight.delete(path);
    });

  inflight.set(path, request);
  return request;
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function invokeCoverPath(path) {
  try {
    return await invoke('get_track_cover_path', { path });
  } catch (firstError) {
    // One short retry absorbs the startup/indexing race without creating an
    // unbounded request loop for genuinely inaccessible files.
    await delay(RETRY_DELAY_MS);
    try {
      return await invoke('get_track_cover_path', { path });
    } catch {
      throw firstError;
    }
  }
}

// Rare fallback for a valid thumbnail that WebView2 cannot fetch through the
// asset protocol. Normal cards stay on the zero-copy filesystem URL path.
export async function loadCoverDataUrl(path) {
  if (!path) return null;
  try {
    const value = (await invoke('get_track_cover', { path })) || null;
    if (value) cacheSet(path, value);
    return value;
  } catch (error) {
    console.warn(`Failed to load fallback cover for ${path}`, error);
    return null;
  }
}

// Remove a successful URL without notifying every CoverImage. Used by a single
// component when its <img> reports that the asset URL itself could not load.
export function evictCover(path) {
  cache.delete(path);
  inflight.delete(path);
}

export function clearCoverCache() {
  cache.clear();
  inflight.clear();
}

// Bumped whenever a cover is invalidated; CoverImage watches it so already-
// mounted instances re-resolve (their `path` prop doesn't change on tag edits,
// so the normal path watcher wouldn't refire).
export const coverVersion = ref(0);

// Successful URLs remain cached. Components whose previous request returned
// null get another chance after indexing, metadata import, or root restoration.
export function retryMissingCovers() {
  coverVersion.value++;
}

// Drop one track's cached cover (after the tag editor rewrote the file). The
// next resolve re-runs get_track_cover_path, whose on-disk key includes
// mtime+size — so the thumbnail regenerates from the new embedded art.
export function invalidateCover(path) {
  cache.delete(path);
  inflight.delete(path);
  coverVersion.value++;
}
