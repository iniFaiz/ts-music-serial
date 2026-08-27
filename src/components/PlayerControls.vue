<script setup>
import { ref, watch, onMounted, onUnmounted, computed } from 'vue';
import { store } from '../store';
import { invokeCommand as invoke } from '../generated/ipc';
import { listen } from '@tauri-apps/api/event';
import { useRouter } from 'vue-router';
import CoverImage from './CoverImage.vue';
import Visualizer from './Visualizer.vue';
import WaveformSeekbar from './WaveformSeekbar.vue';
import { navigateWithTransition } from '../viewTransition';
import { loadWaveform, getCachedWaveform } from '../waveformCache';
import MarqueeText from './MarqueeText.vue';
import { createNyanCatSeekStyle } from '../nyancatTheme';
import LosslessBadge from './LosslessBadge.vue';
import { formatTime } from '../timeFormat';
import { useSeekControl } from '../useSeekControl';

const router = useRouter();
const playerCoverRef = ref(null);

// Playback is handled natively in Rust (rodio + symphonia). This component
// issues commands and consumes the throttled telemetry stream. `seekValue` is
// driven by the rAF interpolation loop and slider v-model below, so the shared
// scrub engine runs without its own clock-sync watcher.
const seek = useSeekControl({ syncWithClock: false });
const { seekValue } = seek;
const playbackError = ref(null);

// Precomputed waveform (peaks) for the current track, when the waveform seek bar
// is enabled. Null while loading / unavailable, so we fall back to the slider.
const waveformPeaks = ref(null);

async function resolveWaveform(path) {
  if (!store.waveformEnabled || !path) {
    waveformPeaks.value = null;
    return;
  }
  const cached = getCachedWaveform(path);
  if (cached !== undefined) {
    waveformPeaks.value = cached;
    return;
  }
  waveformPeaks.value = null; // show the slider until peaks arrive
  const result = await loadWaveform(path);
  // Ignore a stale result if the track changed while decoding.
  if (store.currentSong && store.currentSong.path === path) {
    waveformPeaks.value = result;
  }
}

// Re-resolve when the track changes or the setting is toggled on.
watch(
  () => [store.currentSong && store.currentSong.path, store.waveformEnabled],
  () => resolveWaveform(store.currentSong && store.currentSong.path),
  { immediate: true }
);

const navigateToArtist = (artistName) => {
  if (!artistName || artistName === 'Unknown Artist') return;
  const navigate = () => router.push({ name: 'ArtistDetail', params: { name: artistName } });
  navigateWithTransition(navigate, null);
};

const openFullScreen = () => {
  store.toggleFullscreen();
};

const progressPercentage = computed(() => {
  if (!store.currentSong) return 0;
  const max = store.duration || 100;
  const val = Number(seekValue.value) || 0;
  return Math.min(Math.max((val / max) * 100, 0), 100);
});

const mainSeekStyle = computed(() => {
  return createNyanCatSeekStyle({
    percentage: progressPercentage.value,
    thumbSize: 12,
    playedColor: 'var(--accent-color)',
    unplayedColor: '#4b5563',
    mix: store.waveformEnabled ? 0 : store.nyancatBlend,
    phase: store.nyancatPhase,
  });
});

const volumePercentage = computed(() => {
  return (store.isMuted ? 0 : store.volume) * 100;
});

let stateTimer = null;
// Seek suppression timestamp lives on the store (store.lastSeekAt) so the
// fullscreen player and lyric-click seeks suppress this poll too.
let endedHandledFor = null; // latch so a finished track only advances once
// Track selection is a native session effect. This watcher updates presentation
// and integrations only; it never issues a second load command.
watch(
  () => store.currentSong,
  async (song) => {
    if (!song) {
      playbackError.value = null;
      endedHandledFor = null;
      store.isPlaying = false;
      store.duration = 0;
      store.currentTime = 0;
      seekValue.value = 0;
      store.syncDiscord();
      return;
    }
    playbackError.value = null;
    endedHandledFor = null;

    store.duration = song.duration_secs || store.duration || 0;
    store.currentSampleRate = song.sample_rate;
    store.currentBitDepth = song.bit_depth;
    seekValue.value = store.currentTime || 0;
    await applyNormalization(song);
    pushMediaMetadata(song);
    pushMediaPlayback();
    store.syncDiscord();
  },
  { immediate: true }
);

// Reactively prepare the next track whenever the transition settings, queue, or next track path changes
watch(
  () => {
    if (store.transitionMode === 'off' || store.wasapiExclusive) return null;
    const song = store.nextUpEntry();
    return song
      ? { id: song.queueId || null, path: song.path, duration: song.duration_secs || 0 }
      : null;
  },
  (next) => {
    if (next && store.currentSong) {
      invoke('player_prepare_next', {
        path: next.path,
        durationHint: next.duration,
        queueEntryId: next.id,
      }).catch(() => {});
    }
  },
  { immediate: true, deep: true }
);

watch(
  () => store.isPlaying,
  () => {
    pushMediaPlayback();
    store.syncDiscord();
  }
);

// ---- System Media Transport Controls (Windows media overlay + media keys) ---

function pushMediaMetadata(song) {
  if (!song) return;
  invoke('smtc_set_metadata', {
    title: song.title || '',
    artist: song.artist || '',
    album: song.album || '',
    duration: store.duration || 0,
    path: song.path,
  }).catch(() => {});
}

function pushMediaPlayback() {
  invoke('smtc_set_playback', {
    playing: store.isPlaying,
    position: store.currentTime || 0,
  }).catch(() => {});
}

// ---- Volume normalization (Sound Check) -------------------------------------
// Push the per-track gain to the backend. Uses the ReplayGain tag when present,
// otherwise kicks off a one-time background loudness analysis and re-applies.
async function applyNormalization(song) {
  if (!song) return;
  const enabled = store.normalizationEnabled;
  let gain = null;
  let peak = null;
  if (enabled && typeof song.track_gain_db === 'number') {
    gain = song.track_gain_db;
    peak = typeof song.track_peak === 'number' ? song.track_peak : null;
  }
  try {
    // Enabled/pre-amp live backend-side (player_set_normalization_settings);
    // per-track calls carry only the track's ReplayGain data.
    await invoke('player_set_normalization', { gainDb: gain, peak });
  } catch {
    // ignore — normalization is best-effort
  }
  // No tag gain: compute loudness in the background, then re-apply if still current.
  if (enabled && gain == null) {
    invoke('compute_track_gain', { path: song.path })
      .then((g) => {
        if (
          store.currentSong &&
          store.currentSong.path === song.path &&
          store.normalizationEnabled
        ) {
          invoke('player_set_normalization', { gainDb: g, peak: null }).catch(() => {});
        }
      })
      .catch(() => {});
  }
}

// Re-apply when the normalization settings change mid-playback.
watch(
  () => [store.normalizationEnabled, store.normalizationPreampDb],
  () => {
    if (store.currentSong) applyNormalization(store.currentSong);
  }
);

let unlistenMedia = null;
let unlistenPlaybackSession = null;
let unlistenPlayerTelemetry = null;

const handleMediaControl = (payload) => {
  const action = payload && payload.action;
  switch (action) {
    case 'play':
      if (!store.isPlaying) store.togglePlay();
      break;
    case 'pause':
      if (store.isPlaying) store.togglePlay();
      break;
    case 'toggle':
      store.togglePlay();
      break;
    case 'next':
      store.nextSong(true);
      break;
    case 'previous':
      store.prevSong();
      break;
    case 'stop':
      invoke('player_stop').catch(() => {});
      break;
    case 'seek':
      if (typeof payload.position === 'number') {
        seekValue.value = payload.position;
        onSeekCommit();
      }
      break;
    case 'seek_forward': {
      const t = Math.min((store.currentTime || 0) + 10, store.duration || 0);
      seekValue.value = t;
      onSeekCommit();
      break;
    }
    case 'seek_backward': {
      const t = Math.max((store.currentTime || 0) - 10, 0);
      seekValue.value = t;
      onSeekCommit();
      break;
    }
  }
};

watch(
  () => [store.volume, store.isMuted],
  async ([vol, muted]) => {
    try {
      await invoke('player_set_volume', { volume: muted ? 0 : vol });
    } catch {
      // ignore
    }
  }
);

// While dragging: update the visible time only, and keep the poll from snapping
// the thumb back to the old position. The slider v-models `seekValue` (also
// driven by the rAF interpolation loop above), so the handlers read it without
// event arguments.
const onSeekInput = () => seek.onSeekInput();

// On release: issue a single seek command via the shared store action (handles
// the finished-track reload case and the seek-suppression timestamp), then
// refresh the Discord presence for the new position.
const onSeekCommit = () => {
  seek.onSeekCommit();
  store.syncDiscord();
};

const handleTrackEnded = async () => {
  const current = store.currentSong;
  if (!current || endedHandledFor === current.path) return;
  endedHandledFor = current.path;

  await store.nextSong(false);
};

let finishedSince = 0; // wall-clock ms when 'finished' first latched (transition fallback)
let lastMediaPush = 0; // throttle OS media-overlay timeline pushes

const applyPlayerTelemetry = async (status) => {
  if (!store.currentSong || store.isBuffering) return;
  if (status.duration > 0) store.duration = status.duration;
  if (Date.now() - store.lastSeekAt > 500) {
    if (status.finished) {
      store.currentTime = store.duration;
      seekValue.value = store.duration;
    } else {
      store.currentTime = status.position;
      seekValue.value = status.position;
    }
  }
  if (status.finished) {
    // 'off'/exclusive advance immediately; crossfade/gapless give the backend a
    // short window to drive the transition itself before we fall back to it.
    if (!finishedSince) finishedSince = Date.now();
    if (
      store.transitionMode === 'off' ||
      store.wasapiExclusive ||
      Date.now() - finishedSince > 600
    ) {
      await handleTrackEnded();
      finishedSince = 0;
    }
  } else {
    finishedSince = 0;
  }
  // Keep the native session's upcoming slot filled in autoplay mode so the
  // crossfade/gapless boundary has a prepared track to transition into.
  // (Without this, autoplay only appends AFTER a track ends, which is too
  // late for any transition.) crossfadeSecs may be undefined until the first
  // session snapshot arrives, hence the Number()|| fallback.
  if (
    store.autoplayMode &&
    store.loopMode !== 2 &&
    status.duration > 0 &&
    status.duration - status.position <= Math.max(30, (Number(store.crossfadeSecs) || 6) + 15)
  ) {
    store.ensureAutoplayUpcoming();
  }
  // Keep the OS media overlay's timeline roughly in sync (~every 2s).
  if (Date.now() - lastMediaPush > 2000 && !store.isBuffering) {
    pushMediaPlayback();
    lastMediaPush = Date.now();
  }
};

// Smoothly advance the visible position between 8 Hz telemetry snapshots. Each
// backend event snaps currentTime to native truth and corrects any drift. The
// loop only reschedules itself while a track is actively playing â€” once
// paused/stopped/buffering it stops instead of spinning at ~60fps forever, and
// the watcher below restarts it when playback resumes.
let rafId = null;
let lastFrameTs = 0;
const interpolate = (ts) => {
  if (!store.currentSong || !store.isPlaying || store.isBuffering) {
    rafId = null;
    lastFrameTs = 0; // force a fresh baseline on restart so dt cannot jump
    return;
  }
  // Don't fight an in-progress seek, and skip until we have a baseline frame.
  if (!lastFrameTs || Date.now() - store.lastSeekAt < 500) {
    lastFrameTs = ts;
    rafId = requestAnimationFrame(interpolate);
    return;
  }
  const dt = (ts - lastFrameTs) / 1000;
  lastFrameTs = ts;
  if (dt <= 0 || dt > 1) {
    rafId = requestAnimationFrame(interpolate); // ignore huge gaps (e.g. backgrounded window)
    return;
  }
  const dur = store.duration || 0;
  let t = (store.currentTime || 0) + dt;
  if (dur > 0 && t > dur) t = dur;
  store.currentTime = t;
  seekValue.value = t;
  rafId = requestAnimationFrame(interpolate);
};

// Restart the interpolation loop when playback resumes or buffering completes.
watch(
  () => [store.isPlaying, store.isBuffering],
  ([playing, buffering]) => {
    if (playing && !buffering && !rafId && store.currentSong) {
      rafId = requestAnimationFrame(interpolate);
    }
  }
);

onMounted(async () => {
  rafId = requestAnimationFrame(interpolate);

  // Checkpoint playback position periodically so resume-on-launch is accurate.
  stateTimer = setInterval(() => {
    if (store.currentSong) store.persistState();
  }, 5000);
  window.addEventListener('beforeunload', flushState);

  // Forward OS media-key / overlay button presses into the player.
  try {
    unlistenMedia = await listen('media-control', (e) => handleMediaControl(e.payload));
  } catch {
    // ignore â€” media controls are best-effort
  }

  try {
    unlistenPlaybackSession = await listen('playback-session-event', (e) => {
      const update = e.payload;
      if (update?.events?.some((event) => event.type === 'accounting')) {
        store.bumpStats();
      }
      store.applyPlaybackSessionUpdate(update);
    });
  } catch {
    // Typed playback events are best-effort during development upgrades.
  }

  try {
    unlistenPlayerTelemetry = await listen('player-telemetry', (e) => {
      applyPlayerTelemetry(e.payload).catch(() => {});
    });
    // Cover the small race between window creation and listener registration.
    const initialStatus = await invoke('player_status');
    await applyPlayerTelemetry(initialStatus);
  } catch {
    // Telemetry is best-effort during development upgrades.
  }
});

onUnmounted(() => {
  if (rafId) cancelAnimationFrame(rafId);
  if (stateTimer) clearInterval(stateTimer);
  if (unlistenMedia) unlistenMedia();
  if (unlistenPlaybackSession) unlistenPlaybackSession();
  if (unlistenPlayerTelemetry) unlistenPlayerTelemetry();
  window.removeEventListener('beforeunload', flushState);
});

const flushState = () => {
  if (store.currentSong) store.flushState();
};
</script>

<template>
  <div
    class="bg-[#181818] border-t border-[#282828] z-50 select-none flex flex-col"
    style="view-transition-name: player-bar"
  >
    <div v-if="playbackError" class="bg-red-900/50 text-[10px] text-red-200 p-1 px-4 text-center">
      {{ playbackError }}
    </div>

    <div class="h-24 flex items-center justify-between px-4">
      <!-- Controls -->
      <div
        class="flex items-center justify-start gap-1.5 sm:gap-3 md:gap-4.5 flex-1 min-w-[95px] sm:min-w-[150px] md:min-w-[180px] lg:min-w-[200px] pl-1 sm:pl-4"
      >
        <!-- Shuffle -->
        <button
          @click="store.toggleShuffle()"
          class="transition hidden sm:block disabled:opacity-30 disabled:cursor-not-allowed disabled:pointer-events-none"
          :class="
            store.shuffleMode ? 'text-[var(--accent-color)]' : 'text-gray-400 hover:text-white'
          "
          :disabled="!store.currentSong"
          type="button"
          :aria-label="$t('player.toggleShuffle')"
          :title="$t('player.toggleShuffle')"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <path d="M16 3h5v5M4 20L21 3M21 16v5h-5M15 15l6 6M4 4l5 5" />
          </svg>
        </button>

        <!-- Prev -->
        <button
          type="button"
          @click="store.prevSong()"
          class="text-gray-300 hover:text-white transition disabled:opacity-30 disabled:cursor-not-allowed disabled:pointer-events-none"
          :disabled="!store.currentSong"
          :aria-label="$t('player.prevTrack')"
          :title="$t('player.prevTrack')"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="24"
            height="24"
            viewBox="0 0 24 24"
            fill="currentColor"
            stroke="none"
            aria-hidden="true"
          >
            <polygon points="19 20 9 12 19 4 19 20"></polygon>
            <line x1="5" y1="19" x2="5" y2="5" stroke="currentColor" stroke-width="2"></line>
          </svg>
        </button>

        <!-- Play/Pause -->
        <button
          type="button"
          @click="store.togglePlay()"
          class="bg-white text-black rounded-full p-2 hover:scale-105 transition flex items-center justify-center disabled:opacity-30 disabled:cursor-not-allowed disabled:pointer-events-none"
          :disabled="!store.currentSong"
          :aria-label="$t('player.togglePlay')"
          :title="$t('player.togglePlay')"
        >
          <svg
            v-if="store.isBuffering"
            class="animate-spin"
            xmlns="http://www.w3.org/2000/svg"
            width="24"
            height="24"
            viewBox="0 0 24 24"
            fill="none"
            aria-hidden="true"
          >
            <circle
              class="opacity-25"
              cx="12"
              cy="12"
              r="9"
              stroke="currentColor"
              stroke-width="3"
            ></circle>
            <path
              class="opacity-90"
              fill="currentColor"
              d="M12 3a9 9 0 0 1 9 9h-3a6 6 0 0 0-6-6V3z"
            ></path>
          </svg>
          <svg
            v-else-if="store.isPlaying"
            xmlns="http://www.w3.org/2000/svg"
            width="24"
            height="24"
            viewBox="0 0 24 24"
            fill="currentColor"
            stroke="none"
            aria-hidden="true"
          >
            <rect x="6" y="4" width="4" height="16"></rect>
            <rect x="14" y="4" width="4" height="16"></rect>
          </svg>
          <svg
            v-else
            xmlns="http://www.w3.org/2000/svg"
            width="24"
            height="24"
            viewBox="0 0 24 24"
            fill="currentColor"
            stroke="none"
            aria-hidden="true"
          >
            <polygon points="5 3 19 12 5 21 5 3"></polygon>
          </svg>
        </button>

        <!-- Next -->
        <button
          type="button"
          @click="store.nextSong(true)"
          class="text-gray-300 hover:text-white transition disabled:opacity-30 disabled:cursor-not-allowed disabled:pointer-events-none"
          :disabled="!store.currentSong"
          :aria-label="$t('player.nextTrack')"
          :title="$t('player.nextTrack')"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="24"
            height="24"
            viewBox="0 0 24 24"
            fill="currentColor"
            stroke="none"
            aria-hidden="true"
          >
            <polygon points="5 4 15 12 5 20 5 4"></polygon>
            <line x1="19" y1="5" x2="19" y2="19" stroke="currentColor" stroke-width="2"></line>
          </svg>
        </button>

        <!-- Loop -->
        <button
          type="button"
          @click="store.toggleLoop()"
          class="transition relative hidden sm:block disabled:opacity-30 disabled:cursor-not-allowed disabled:pointer-events-none"
          :class="
            store.loopMode > 0 ? 'text-[var(--accent-color)]' : 'text-gray-400 hover:text-white'
          "
          :disabled="!store.currentSong"
          :aria-label="$t('player.toggleRepeat')"
          :title="$t('player.toggleRepeat')"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <path d="M17 1l4 4-4 4"></path>
            <path d="M3 11V9a4 4 0 0 1 4-4h14"></path>
            <path d="M7 23l-4-4 4-4"></path>
            <path d="M21 13v2a4 4 0 0 1-4 4H3"></path>
          </svg>
          <span v-if="store.loopMode === 2" class="absolute -top-1 -right-2 text-[8px] font-bold"
            >1</span
          >
        </button>
      </div>

      <!-- Progress bar -->
      <div
        class="flex flex-col items-center flex-1 min-w-[110px] sm:min-w-[180px] md:min-w-[220px] lg:min-w-[300px] px-1 sm:px-4"
      >
        <div
          v-if="store.currentSong"
          class="flex items-center gap-2 md:gap-4 mb-1.5 md:mb-2 w-full justify-center"
        >
          <!-- Left spacer container: ensures the title/artist text is centered regardless of cover image size -->
          <div
            class="w-[30px] sm:w-[60px] md:w-[75px] lg:w-[80px] flex items-center justify-start shrink-0"
          >
            <!-- Group container: CoverImage on left, Lossless Badge on right, both aligned to top -->
            <div class="hidden sm:flex items-start shrink-0 gap-1.5 relative">
              <button
                ref="playerCoverRef"
                type="button"
                @click="openFullScreen()"
                class="shrink-0 rounded-md overflow-hidden relative group focus:outline-none transition-all duration-300 ease-out hover:scale-105 active:scale-95 hover:shadow-lg hover:shadow-black/60 hover:ring-1 hover:ring-white/30 cursor-pointer"
                :title="$t('player.fullScreen')"
                :aria-label="$t('player.fullScreen')"
              >
                <CoverImage
                  :path="store.currentSong.path"
                  className="h-8 w-8 md:h-10 md:w-10 rounded-md shadow-sm bg-[#333] transition-transform duration-300 ease-out group-hover:scale-110"
                />
                <div
                  class="absolute inset-0 bg-black/50 backdrop-blur-[1px] flex items-center justify-center opacity-0 group-hover:opacity-100 transition-all duration-300 ease-out pointer-events-none"
                >
                  <svg
                    xmlns="http://www.w3.org/2000/svg"
                    width="14"
                    height="14"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2.5"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    aria-hidden="true"
                    class="text-white transform scale-75 group-hover:scale-100 transition-all duration-300 ease-out drop-shadow"
                  >
                    <polyline points="15 3 21 3 21 9" />
                    <polyline points="9 21 3 21 3 15" />
                    <line x1="21" y1="3" x2="14" y2="10" />
                    <line x1="3" y1="21" x2="10" y2="14" />
                  </svg>
                </div>
              </button>

              <!-- Lossless Badge (shared component, icon-only variant) -->
              <LosslessBadge placement="down" icon-only class="relative mt-0.5 shrink-0" />
            </div>
          </div>

          <!-- Song Title & Artist text container: set flex-1, text-center and min-w-0 -->
          <div class="flex flex-col overflow-hidden text-center min-w-0 flex-1">
            <MarqueeText
              :text="store.currentSong.title"
              :center="true"
              class="text-xs md:text-sm font-medium text-white max-w-[80px] sm:max-w-[180px] md:max-w-[260px] lg:max-w-[360px] xl:max-w-[450px] mx-auto"
            />
            <button
              type="button"
              @click="navigateToArtist(store.currentSong.artist)"
              class="text-[10px] md:text-xs text-gray-400 hover:text-[var(--accent-color)] hover:underline cursor-pointer truncate max-w-[80px] sm:max-w-[180px] md:max-w-[260px] lg:max-w-[360px] xl:max-w-[450px] transition-colors mx-auto bg-transparent border-0 p-0 block"
            >
              {{ store.currentSong.artist }}
            </button>
          </div>

          <!-- Right spacer container: matches the width of the left container to center text perfectly -->
          <div
            class="w-[30px] sm:w-[60px] md:w-[75px] lg:w-[80px] flex items-center justify-end shrink-0"
          >
            <button
              type="button"
              @click="store.runMutation(() => store.toggleFavorite(store.currentSong.path))"
              class="transition hover:scale-110 shrink-0"
              :class="
                store.isFavorite(store.currentSong.path)
                  ? 'text-[var(--accent-color)]'
                  : 'text-gray-400 hover:text-white'
              "
              :title="
                store.isFavorite(store.currentSong.path)
                  ? $t('player.removeFromFavorites')
                  : $t('player.addToFavorites')
              "
              :aria-label="
                store.isFavorite(store.currentSong.path)
                  ? $t('player.removeFromFavorites')
                  : $t('player.addToFavorites')
              "
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="18"
                height="18"
                viewBox="0 0 24 24"
                :fill="store.isFavorite(store.currentSong.path) ? 'currentColor' : 'none'"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path
                  d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"
                ></path>
              </svg>
            </button>
          </div>
        </div>
        <div v-else class="h-10 mb-2 flex items-center text-gray-500 text-sm">Select a song</div>

        <div
          class="w-full flex items-center gap-1.5 sm:gap-3 text-[10px] sm:text-xs text-gray-400 font-variant-numeric tabular-nums"
        >
          <span>{{ formatTime(store.currentTime) }}</span>
          <!-- Seek track. The plain slider and the waveform occupy the same space
               and cross-fade when toggled. It stays thin when the waveform is off
               (so the player bar keeps its compact height) and gains room only
               when the waveform is on. -->
          <div
            class="relative flex-1 flex items-center"
            :class="store.waveformEnabled ? 'h-8' : 'h-5'"
          >
            <input
              type="range"
              min="0"
              :max="Math.max(store.duration || 100, seekValue)"
              v-model.number="seekValue"
              @input="onSeekInput"
              @change="onSeekCommit"
              :aria-label="$t('player.seekLabel')"
              class="seeker-input absolute inset-x-0 top-1/2 -translate-y-1/2 w-full rounded-lg appearance-none cursor-pointer accent-[var(--accent-color)] transition-opacity duration-300 disabled:cursor-not-allowed"
              :class="store.waveformEnabled ? 'opacity-0 pointer-events-none' : 'opacity-100'"
              :style="mainSeekStyle"
              :disabled="!store.currentSong"
            />
            <Transition name="wf-fade">
              <WaveformSeekbar
                v-if="store.waveformEnabled"
                class="absolute inset-0"
                :peaks="waveformPeaks"
                :current="seekValue"
                :duration="Math.max(store.duration || 100, seekValue)"
                :disabled="!store.currentSong"
                :nyancat="store.nyancatMode"
                @input="
                  (v) => {
                    seekValue = v;
                    onSeekInput();
                  }
                "
                @commit="
                  (v) => {
                    seekValue = v;
                    onSeekCommit();
                  }
                "
              />
            </Transition>
          </div>
          <span>{{ formatTime(store.duration) }}</span>
        </div>
      </div>

      <!-- Volume -->
      <div
        class="flex items-center justify-end gap-1.5 sm:gap-2.5 md:gap-3 flex-1 min-w-[70px] sm:min-w-[140px] md:min-w-[180px] lg:min-w-[220px] pr-1 sm:pr-4"
      >
        <!-- Real-time audio visualizer (reacts to the playing track) -->
        <Visualizer v-if="store.visualizerEnabled && store.currentSong" />

        <!-- Lyrics panel toggle -->
        <button
          v-if="store.currentSong"
          type="button"
          @click="
            store.queuePanelOpen = false;
            store.lyricsPanelOpen = !store.lyricsPanelOpen;
          "
          class="transition hover:text-white"
          :class="store.lyricsPanelOpen ? 'text-[var(--accent-color)]' : 'text-gray-400'"
          :title="$t('player.lyrics')"
          :aria-label="$t('player.lyrics')"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <path
              d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"
            />
            <line x1="8.5" y1="10" x2="13.5" y2="10" />
            <line x1="8.5" y1="13.5" x2="11.5" y2="13.5" />
          </svg>
        </button>

        <!-- Queue toggle (with an ∞ badge when unlimited autoplay is on) -->
        <button
          type="button"
          @click="
            store.lyricsPanelOpen = false;
            store.queuePanelOpen = !store.queuePanelOpen;
          "
          class="transition hover:text-white relative"
          :class="store.queuePanelOpen ? 'text-[var(--accent-color)]' : 'text-gray-400'"
          :title="$t('player.queue')"
          :aria-label="$t('player.queue')"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <line x1="3" y1="6" x2="16" y2="6"></line>
            <line x1="3" y1="12" x2="13" y2="12"></line>
            <line x1="3" y1="18" x2="13" y2="18"></line>
            <polygon points="18 14 22 16.5 18 19" fill="currentColor" stroke="none"></polygon>
            <line x1="18" y1="9" x2="18" y2="13"></line>
          </svg>
          <span
            v-if="store.autoplayMode"
            class="absolute -top-2 -right-2 h-3.5 w-3.5 rounded-full bg-[var(--accent-color)] flex items-center justify-center ring-2 ring-[#181818] shadow"
            title="Autoplay on"
            aria-hidden="true"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="9"
              height="9"
              viewBox="0 0 24 24"
              fill="none"
              stroke="white"
              stroke-width="2.5"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path
                d="M12 12c-2-2.67-4-4-6-4a4 4 0 1 0 0 8c2 0 4-1.33 6-4Zm0 0c2 2.67 4 4 6 4a4 4 0 0 0 0-8c-2 0-4 1.33-6 4Z"
              />
            </svg>
          </span>
        </button>

        <!-- Fullscreen toggle -->
        <button
          v-if="store.currentSong"
          type="button"
          @click="openFullScreen()"
          class="transition text-gray-400 hover:text-white hover:scale-110 active:scale-95 shrink-0 cursor-pointer hidden xs:flex items-center justify-center"
          :title="$t('player.fullScreen')"
          :aria-label="$t('player.fullScreen')"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="17"
            height="17"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <polyline points="15 3 21 3 21 9"></polyline>
            <polyline points="9 21 3 21 3 15"></polyline>
            <line x1="21" y1="3" x2="14" y2="10"></line>
            <line x1="3" y1="21" x2="10" y2="14"></line>
          </svg>
        </button>
        <button
          type="button"
          @click="store.toggleMute()"
          class="text-gray-400 hover:text-white transition cursor-pointer flex items-center justify-center shrink-0"
          :title="store.isMuted ? $t('common.unmute') : $t('common.mute')"
          :aria-label="store.isMuted ? $t('common.unmute') : $t('common.mute')"
        >
          <!-- Mute Icon -->
          <svg
            v-if="store.isMuted"
            xmlns="http://www.w3.org/2000/svg"
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"></polygon>
            <line x1="23" y1="9" x2="17" y2="15"></line>
            <line x1="17" y1="9" x2="23" y2="15"></line>
          </svg>
          <!-- Low Volume Icon -->
          <svg
            v-else-if="store.volume <= 0.5"
            xmlns="http://www.w3.org/2000/svg"
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"></polygon>
            <path d="M15.54 8.46a5 5 0 0 1 0 7.07"></path>
          </svg>
          <!-- High Volume Icon -->
          <svg
            v-else
            xmlns="http://www.w3.org/2000/svg"
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"></polygon>
            <path d="M19.07 4.93a10 10 0 0 1 0 14.14"></path>
            <path d="M15.54 8.46a5 5 0 0 1 0 7.07"></path>
          </svg>
        </button>
        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          :aria-label="$t('player.volumeLabel')"
          :value="store.isMuted ? 0 : store.volume"
          @input="store.setVolume($event.target.value)"
          class="hidden sm:block w-16 md:w-24 h-1 rounded-lg appearance-none cursor-pointer accent-[var(--accent-color)] hover:accent-white transition-opacity duration-200"
          :class="store.isMuted ? 'opacity-40' : 'opacity-100'"
          :style="{
            background: `linear-gradient(to right, var(--accent-color) calc(${volumePercentage} * (100% - 12px) / 100 + 6px), #4b5563 calc(${volumePercentage} * (100% - 12px) / 100 + 6px))`,
          }"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
/* Waveform â†” slider cross-fade when the seek bar is toggled. The seek track has
   a fixed height, so only opacity changes â€” the bars also rise from the baseline
   in JS (see WaveformSeekbar) for a lively toggle-on. */
.wf-fade-enter-active,
.wf-fade-leave-active {
  transition: opacity 0.3s ease;
}
.wf-fade-enter-from,
.wf-fade-leave-to {
  opacity: 0;
}

/* Custom styled range slider thumb for generic sliders (e.g., volume) */
input[type='range']::-webkit-slider-thumb {
  -webkit-appearance: none;
  height: 12px;
  width: 12px;
  border-radius: 50%;
  background: #ffffff;
  margin-top: -4px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4);
  transition:
    transform 0.15s ease-in-out,
    background-color 0.15s ease-in-out;
}
input[type='range']::-moz-range-thumb {
  height: 12px;
  width: 12px;
  border: 0;
  border-radius: 50%;
  background: #ffffff;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4);
  transition: transform 0.15s ease-in-out;
}

/* Seeker progress bar specific hover animations */
.seeker-input {
  height: 4px;
  transition: height 0.15s cubic-bezier(0.4, 0, 0.2, 1);
}

.seeker-input::-webkit-slider-thumb {
  transform: scale(0);
  transition:
    transform 0.15s cubic-bezier(0.4, 0, 0.2, 1),
    margin-top 0.15s cubic-bezier(0.4, 0, 0.2, 1);
}
.seeker-input::-moz-range-thumb {
  transform: scale(0);
  transition: transform 0.15s cubic-bezier(0.4, 0, 0.2, 1);
}

.seeker-input:hover,
.seeker-input:active {
  height: 6px;
}
.seeker-input:hover::-webkit-slider-thumb,
.seeker-input:active::-webkit-slider-thumb {
  transform: scale(1);
  margin-top: -3px;
}
.seeker-input:hover::-moz-range-thumb,
.seeker-input:active::-moz-range-thumb {
  transform: scale(1);
}

.animate-fade-in {
  animation: fadeIn 0.15s cubic-bezier(0.16, 1, 0.3, 1) forwards;
  transform-origin: top center;
}

@keyframes fadeIn {
  from {
    opacity: 0;
    transform: translate(-50%, -4px) scale(0.95);
  }
  to {
    opacity: 1;
    transform: translate(-50%, 0) scale(1);
  }
}
</style>
