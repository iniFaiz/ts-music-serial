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
export function useQuery(fetcher, { deps = [], initial = null, watchStats = false } = {}) {
  const data = ref(initial);
  const loading = ref(true);
  let token = 0;

  async function run() {
    const mine = ++token;
    loading.value = true;
    try {
      const result = await fetcher();
      if (mine === token) data.value = result;
    } catch (e) {
      console.error('Library query failed', e);
      if (mine === token) data.value = initial;
    } finally {
      if (mine === token) loading.value = false;
    }
  }

  const sources = [() => store.libraryVersion, ...deps];
  if (watchStats) sources.push(() => store.statsVersion);
  watch(sources, run, { immediate: true });

  return { data, loading, refresh: run };
}
