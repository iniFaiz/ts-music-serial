// Reactive data-access helper for the query-driven library.
//
// The library now lives in SQLite (Rust); the webview no longer holds the full
// songs array. Views fetch what they show via `db_*` commands. This composable
// re-runs a fetcher whenever the library changes (`store.libraryVersion`) or any
// of the view's own reactive `deps` change, and exposes the latest result as a
// ref — so a component just declares *what* it wants, not *when* to reload.

import { ref, watch } from 'vue';
import { store } from './store';

// Query results live outside component instances. This matters for detail pages
// (and for views that are not currently mounted): navigating away must not throw
// away data that was already read from SQLite. The cache is deliberately small
// and LRU-bounded so opening many albums/playlists cannot grow webview memory
// forever.
const queryCache = new Map();
const MAX_CACHE_ENTRIES = 96;

function resolveValue(source) {
  if (typeof source === 'function') return source();
  if (source && typeof source === 'object' && 'value' in source) return source.value;
  return source;
}

function resolveCacheKey(cacheKey) {
  const key = resolveValue(cacheKey);
  return key === null || key === undefined || key === '' ? null : String(key);
}

function versionFor(watchStats, deps) {
  return JSON.stringify([
    store.libraryVersion,
    watchStats ? store.statsVersion : null,
    ...deps.map(resolveValue),
  ]);
}

function touchEntry(key, entry) {
  queryCache.delete(key);
  queryCache.set(key, entry);
}

function trimCache() {
  if (queryCache.size <= MAX_CACHE_ENTRIES) return;
  for (const [key, entry] of queryCache) {
    // Never evict a request another view may currently be awaiting.
    if (!entry.promise) queryCache.delete(key);
    if (queryCache.size <= MAX_CACHE_ENTRIES) break;
  }
}

function cacheEntry(cacheKey) {
  const key = resolveCacheKey(cacheKey);
  if (!key) return { key: null, entry: null };
  let entry = queryCache.get(key);
  if (!entry) {
    entry = {
      data: undefined,
      error: null,
      hasResolved: false,
      version: null,
      promise: null,
      promiseVersion: null,
    };
    queryCache.set(key, entry);
  } else {
    touchEntry(key, entry);
  }
  return { key, entry };
}

// Return the last successful value even if a newer library version exists. A
// stale value is useful as an instant placeholder while it refreshes silently.
export function getCachedQuery(cacheKey) {
  const key = resolveCacheKey(cacheKey);
  const entry = key ? queryCache.get(key) : null;
  if (!entry || !entry.hasResolved) return undefined;
  touchEntry(key, entry);
  return entry.data;
}

// Warm or refresh a cache entry. Concurrent callers for the same key/version
// share one promise, preventing several cached playlist components from issuing
// the same SQLite query at once.
export function prefetchQuery(
  fetcher,
  { cacheKey, watchStats = false, deps = [], version = null } = {}
) {
  const { key, entry } = cacheEntry(cacheKey);
  if (!key) return Promise.resolve().then(fetcher);

  const wantedVersion = version ?? versionFor(watchStats, deps);
  if (entry.hasResolved && entry.version === wantedVersion) {
    return Promise.resolve(entry.data);
  }
  if (entry.promise && entry.promiseVersion === wantedVersion) return entry.promise;

  const promise = Promise.resolve()
    .then(fetcher)
    .then((result) => {
      // A newer refresh may have started while this request was running. Only
      // the newest request is allowed to replace the cache entry.
      if (entry.promise === promise) {
        entry.data = result;
        entry.error = null;
        entry.hasResolved = true;
        entry.version = wantedVersion;
      }
      return result;
    })
    .catch((error) => {
      if (entry.promise === promise) entry.error = error;
      throw error;
    })
    .finally(() => {
      if (entry.promise === promise) {
        entry.promise = null;
        entry.promiseVersion = null;
        trimCache();
      }
    });

  entry.promise = promise;
  entry.promiseVersion = wantedVersion;
  trimCache();
  return promise;
}

// `fetcher` is an async function returning the data. Options:
//   deps     – extra reactive getters to watch (e.g. () => route.params.name)
//   initial  – value held until the first fetch resolves
//   watchStats – also reload when play stats change (Home insights)
//   waitForReady – defer the first query until the SQLite library is initialized
//   cacheKey – stable key used to reuse the result across component instances
export function useQuery(
  fetcher,
  { deps = [], initial = null, watchStats = false, waitForReady = true, cacheKey = null } = {}
) {
  let activeKey = resolveCacheKey(cacheKey);
  const initialCached = activeKey ? getCachedQuery(activeKey) : undefined;
  const data = ref(initialCached === undefined ? initial : initialCached);
  const loading = ref(initialCached === undefined);
  const error = ref(null);
  let token = 0;
  let hasResolved = initialCached !== undefined;

  async function run() {
    // Views are mounted while store.loadLibrary() is still restoring/migrating
    // the SQLite library. Querying during that window can legitimately return an
    // empty result, which used to be rendered as a real empty library before the
    // libraryVersion bump triggered a second query.
    if (waitForReady && !store.libraryReady) {
      token++;
      loading.value = !hasResolved;
      return;
    }

    const mine = ++token;
    const nextKey = resolveCacheKey(cacheKey);
    const cached = nextKey ? getCachedQuery(nextKey) : undefined;

    // A reactive cache key (for example playlist:<id>) represents different
    // data. Switch to that entry immediately, or to the declared initial value
    // if this is genuinely its first load.
    if (nextKey !== activeKey) {
      activeKey = nextKey;
      hasResolved = cached !== undefined;
      data.value = hasResolved ? cached : initial;
    } else if (cached !== undefined) {
      data.value = cached;
      hasResolved = true;
    }

    // Once a query has resolved, later invalidations are stale-while-revalidate:
    // keep the existing rows/cards visible and do not flash a loading screen.
    loading.value = !hasResolved;
    error.value = null;
    try {
      const result = activeKey
        ? await prefetchQuery(fetcher, {
            cacheKey: activeKey,
            watchStats,
            deps,
          })
        : await fetcher();
      if (mine === token) {
        data.value = result;
        hasResolved = true;
      }
    } catch (e) {
      console.error('Library query failed', e);
      if (mine === token) {
        error.value = e;
        // Keep a previously successful result visible when a background refresh
        // fails instead of briefly replacing the page with an empty state.
        if (!hasResolved) data.value = initial;
      }
    } finally {
      if (mine === token) loading.value = false;
    }
  }

  const sources = [() => store.libraryVersion, ...deps];
  if (waitForReady) sources.unshift(() => store.libraryReady);
  if (watchStats) sources.push(() => store.statsVersion);
  watch(sources, run, { immediate: true });

  return { data, loading, error, refresh: run };
}
