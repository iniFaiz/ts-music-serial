import { ref, watch } from 'vue';

import { getCachedLyrics, loadLyrics } from './lyricsCache';

export function useTrackLyrics({ song, active, source }) {
  const lyrics = ref(undefined);
  const loading = ref(false);
  let requestVersion = 0;

  const fetchLyrics = async (force = false) => {
    const current = song();
    if (!current || !current.path) {
      requestVersion += 1;
      lyrics.value = null;
      loading.value = false;
      return;
    }
    const request = ++requestVersion;
    const path = current.path;

    if (!force) {
      const cached = getCachedLyrics(path);
      if (cached !== undefined) {
        lyrics.value = cached;
        loading.value = false;
        return;
      }
    }

    loading.value = true;
    lyrics.value = undefined;
    const result = await loadLyrics(current, { force });
    if (request === requestVersion && song()?.path === path) {
      lyrics.value = result;
      loading.value = false;
    }
  };

  watch(
    [active, () => song()?.path],
    ([isActive, path]) => {
      if (!path) {
        requestVersion += 1;
        lyrics.value = null;
        loading.value = false;
      } else if (isActive) {
        fetchLyrics();
      } else {
        requestVersion += 1;
        loading.value = false;
      }
    },
    { immediate: true }
  );

  watch(source, () => {
    requestVersion += 1;
    lyrics.value = undefined;
    loading.value = false;
    if (active()) fetchLyrics(true);
  });

  return { lyrics, lyricsLoading: loading, fetchLyrics };
}
