import { computed, nextTick, ref, watch } from 'vue';
import { store } from './store';

// Mini player window sizing choreography, extracted from MiniPlayer.
//
// Lyrics/artwork views use fixed window sizes; the compact bar is measured
// from its real chrome elements so it fits its content exactly (no stray gap
// or clipping regardless of fonts/scale). The queue forces the tall lyrics
// size. The compact bar's height depends on the lossless badge (per-track),
// so it refits when the song changes while compact.
//
// Options are getters so the caller keeps owning view state:
//   getView         () => 'compact' | 'lyrics' | 'artwork'
//   getQueuePresent () => boolean — queue sheet mounted/animating
//   getSongPath     () => string | null
export function useMiniWindowSize({ getView, getQueuePresent, getSongPath }) {
  const MINI_WIDTH = 360;
  const topChromeEl = ref(null);
  const bottomChromeEl = ref(null);

  function fitCompact() {
    if (!store.miniPlayerOpen) return;
    nextTick(() => {
      const top = topChromeEl.value ? topChromeEl.value.offsetHeight : 0;
      const bottom = bottomChromeEl.value ? bottomChromeEl.value.offsetHeight : 0;
      if (top && bottom) store.applyMiniSize(MINI_WIDTH, top + bottom);
    });
  }

  const windowView = computed(() => (getQueuePresent() ? 'lyrics' : getView()));
  watch(
    windowView,
    (v) => {
      if (!store.miniPlayerOpen) return;
      if (v === 'compact') fitCompact();
      else store.applyMiniViewSize(v);
    },
    { immediate: true }
  );
  watch(
    () => getSongPath(),
    () => {
      if (store.miniPlayerOpen && windowView.value === 'compact') fitCompact();
    }
  );

  return { topChromeEl, bottomChromeEl, fitCompact };
}
