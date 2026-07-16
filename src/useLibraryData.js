// Reactive data-access helper for the query-driven library.
//
// The library now lives in SQLite (Rust); the webview no longer holds the full
// songs array. Views fetch what they show via `db_*` commands. This composable
// re-runs a fetcher whenever the library changes (`store.libraryVersion`) or any
// of the view's own reactive `deps` change, and exposes the latest result as a
// ref — so a component just declares *what* it wants, not *when* to reload.

import { ref, watch } from 'vue';
import { store } from './store';

// `fetcher` is an async function returning the data. Options:
//   deps     – extra reactive getters to watch (e.g. () => route.params.name)
//   initial  – value held until the first fetch resolves
//   watchStats – also reload when play stats change (Home insights)
//   waitForReady – defer the first query until the SQLite library is initialized
export function useQuery(
  fetcher,
  { deps = [], initial = null, watchStats = false, waitForReady = true } = {}
) {
  const data = ref(initial);
  const loading = ref(true);
  const error = ref(null);
  let token = 0;
  let hasResolved = false;

  async function run() {
    // Views are mounted while store.loadLibrary() is still restoring/migrating
    // the SQLite library. Querying during that window can legitimately return an
    // empty result, which used to be rendered as a real empty library before the
    // libraryVersion bump triggered a second query.
    if (waitForReady && !store.libraryReady) {
      token++;
      loading.value = true;
      return;
    }

    const mine = ++token;
    loading.value = true;
    error.value = null;
    try {
      const result = await fetcher();
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
