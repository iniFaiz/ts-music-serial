import { ref, watch } from 'vue';
import { store } from './store';

// Shared scrub logic for the seek sliders in PlayerControls, MiniPlayer and
// FullScreenPlayer.
//
// While dragging (input events) the visible time is previewed locally and
// `store.lastSeekAt` is stamped so telemetry/polling never snaps the thumb
// back to the old position mid-drag. On release (change/commit) a single
// `store.seek()` is issued via the shared store action, which also handles the
// finished-track reload case.
//
// Components either bind the returned `seekValue` ref to the slider
// (v-model — then call the handlers without arguments) or let the handlers
// read `event.target.value` directly from raw @input/@change bindings.
export function useSeekControl({ syncWithClock = true } = {}) {
  const seekValue = ref(0);
  let held = false;

  if (syncWithClock) {
    // Keep the slider in sync with playback unless the user is dragging it.
    watch(
      () => store.currentTime,
      (t) => {
        if (!held) seekValue.value = t || 0;
      },
      { immediate: true }
    );
  }

  const readValue = (eventOrValue) => {
    if (typeof eventOrValue === 'number') return eventOrValue;
    const raw = eventOrValue?.target?.value ?? seekValue.value;
    return Number(raw);
  };

  const onSeekInput = (eventOrValue) => {
    held = true;
    store.lastSeekAt = Date.now();
    const t = readValue(eventOrValue);
    if (!Number.isFinite(t)) return;
    seekValue.value = t;
    store.currentTime = t;
  };

  const onSeekCommit = (eventOrValue) => {
    held = false;
    const t = readValue(eventOrValue);
    if (!Number.isFinite(t)) return Promise.resolve();
    return store.seek(t);
  };

  // Programmatic jump (media keys): move the thumb, then issue one seek.
  const seekTo = (seconds) => {
    seekValue.value = seconds;
    return store.seek(seconds);
  };

  return { seekValue, onSeekInput, onSeekCommit, seekTo };
}
