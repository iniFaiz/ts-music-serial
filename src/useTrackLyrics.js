import { ref, watch } from 'vue';

import { loadLyrics } from './lyricsCache';

export function useTrackLyrics({ song, active, source }) {
  const lyrics = ref(undefined);
  const loading = ref(false);
  let requestVersion = 0;

  const fetchLyrics = async (force = false) => {
    const current = song();
    if (!current) {
      requestVersion += 1;
      lyrics.value = null;
      loading.value = false;
      return;
    }
    const request = ++requestVersion;
    const path = current.path;
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
