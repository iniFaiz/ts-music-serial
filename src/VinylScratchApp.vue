<script setup>
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue';
import { invokeCommand as invoke } from './generated/ipc';
import { emit } from '@tauri-apps/api/event';
import CoverImage from './components/CoverImage.vue';
import {
  TONEARM_DROP_THRESHOLD,
  TONEARM_INNER_ANGLE,
  TONEARM_OUTER_ANGLE,
  TONEARM_REST_ANGLE,
  shortestAngleDelta,
  timeFromScratchRotation,
  tonearmAngleFromProgress,
  tonearmProgressFromAngle,
} from './vinylScratch';

const AUTO_ROTATION_DEG_PER_SEC = 200; // 33 1/3 RPM
const SCRATCH_SECONDS_PER_TURN = 5;
const STATUS_POLL_MS = 100;
const GRAIN_SCHEDULE_MS = 20;
const GRAIN_HOLD_MS = 48;

const discRef = ref(null);
const tonearmSvgRef = ref(null);
const scratchActive = ref(false);
const tonearmDragging = ref(false);
const needleDropped = ref(false);
const tonearmAngle = ref(TONEARM_REST_ANGLE);

const player = reactive({
  path: null,
  position: 0,
  duration: 0,
  playing: false,
  finished: false,
});

const hasTrack = computed(() => !!player.path && player.duration > 0);
const progress = computed(() => {
  if (!player.duration) return 0;
  return Math.min(1, Math.max(0, player.position / player.duration));
});
const tonearmStyle = computed(() => ({
  transform: `rotate(${tonearmAngle.value.toFixed(3)}deg)`,
}));

let statusTimer = null;
let statusInFlight = false;
let animationFrame = null;
let lastAnimationTime = 0;
let recordRotation = 0;
let recordVelocity = 0;
let motionQuery = null;
let reduceMotion = false;

let scratchPointerId = null;
let scratchLastAngle = 0;
let scratchAccumulatedAngle = 0;
let scratchStartTime = 0;
let scratchShouldResume = false;

let tonearmPointerId = null;
let tonearmStartPointerAngle = 0;
let tonearmStartAngle = TONEARM_REST_ANGLE;

// The scratch engine plays tiny grains from the real track. Requests are
// coalesced so rapid pointer movement never creates overlapping seek commands.
let pendingGrainPosition = null;
let grainScheduleTimer = null;
let grainPauseTimer = null;
let grainPromise = null;
let endingScratch = false;
let scratchPausePromise = null;
let tonearmPausePromise = null;

const clamp = (value, min, max) => Math.min(max, Math.max(min, value));

const syncMainWindow = (payload) => {
  emit('vinyl-playback-sync', payload).catch(() => {});
};

const setRecordTransform = () => {
  if (discRef.value) {
    discRef.value.style.transform = `rotate(${recordRotation.toFixed(3)}deg)`;
  }
};

const setPlaybackState = (playing, position = player.position) => {
  player.playing = playing;
  player.position = Math.max(0, Number(position) || 0);
  syncMainWindow({ playing, position: player.position });
};

const updateTonearmFromPlayback = () => {
  if (tonearmDragging.value || scratchActive.value) return;
  needleDropped.value = player.playing && hasTrack.value;
  tonearmAngle.value = needleDropped.value
    ? tonearmAngleFromProgress(progress.value)
    : TONEARM_REST_ANGLE;
};

const pollStatus = async () => {
  if (statusInFlight) return;
  statusInFlight = true;
  try {
    const status = await invoke('player_status');
    if (!status) return;

    const pathChanged = status.path !== player.path;
    player.path = status.path || null;
    player.duration = Math.max(0, Number(status.duration) || 0);
    player.finished = !!status.finished;

    if (!scratchActive.value && !tonearmDragging.value) {
      player.position = Math.max(0, Number(status.position) || 0);
      player.playing = !!status.playing;
      updateTonearmFromPlayback();
    }

    if (pathChanged) {
      recordRotation = 0;
      recordVelocity = 0;
      setRecordTransform();
    }
  } catch {
    // The main player may still be starting; the next poll will retry.
  } finally {
    statusInFlight = false;
  }
};

const animateRecord = (time) => {
  const dt = lastAnimationTime ? Math.min(0.05, (time - lastAnimationTime) / 1000) : 0;
  lastAnimationTime = time;

  if (!scratchActive.value) {
    const target = player.playing && !reduceMotion ? AUTO_ROTATION_DEG_PER_SEC : 0;
    const ease = 1 - Math.exp(-dt * (target ? 7 : 9));
    recordVelocity += (target - recordVelocity) * ease;
    recordRotation += recordVelocity * dt;
    if (Math.abs(recordRotation) > 36000) recordRotation %= 360;
    setRecordTransform();
  }

  animationFrame = requestAnimationFrame(animateRecord);
};

const pointOnDisc = (event) => {
  const rect = discRef.value?.getBoundingClientRect();
  if (!rect) return null;
  const x = event.clientX - (rect.left + rect.width / 2);
  const y = event.clientY - (rect.top + rect.height / 2);
  return {
    angle: (Math.atan2(y, x) * 180) / Math.PI,
    radius: Math.hypot(x, y),
    maxRadius: rect.width / 2,
  };
};

const stopGrainTimer = () => {
  if (grainPauseTimer) clearTimeout(grainPauseTimer);
  grainPauseTimer = null;
};

const flushGrain = async () => {
  if (grainScheduleTimer) clearTimeout(grainScheduleTimer);
  grainScheduleTimer = null;
  if (grainPromise || endingScratch || !scratchActive.value || pendingGrainPosition === null) {
    return;
  }

  const target = pendingGrainPosition;
  pendingGrainPosition = null;
  grainPromise = (async () => {
    if (scratchPausePromise) await scratchPausePromise;
    await invoke('player_seek', { position: target });
    await invoke('player_resume');
    stopGrainTimer();
    grainPauseTimer = setTimeout(() => {
      grainPauseTimer = null;
      invoke('player_pause').catch(() => {});
    }, GRAIN_HOLD_MS);
    syncMainWindow({ playing: false, position: target });
  })();

  try {
    await grainPromise;
  } catch {
    // A failed grain should not break the remaining scratch gesture.
  } finally {
    grainPromise = null;
    if (pendingGrainPosition !== null && scratchActive.value && !endingScratch) {
      grainScheduleTimer = setTimeout(flushGrain, GRAIN_SCHEDULE_MS);
    }
  }
};

const queueGrain = (position) => {
  pendingGrainPosition = position;
  if (grainPromise || grainScheduleTimer) return;
  grainScheduleTimer = setTimeout(flushGrain, GRAIN_SCHEDULE_MS);
};

const beginScratch = (event) => {
  if (event.button !== 0 || !hasTrack.value || !needleDropped.value || tonearmDragging.value) {
    return;
  }

  const point = pointOnDisc(event);
  if (!point || point.radius < point.maxRadius * 0.12) return;
  event.preventDefault();
  discRef.value?.setPointerCapture(event.pointerId);

  scratchPointerId = event.pointerId;
  scratchLastAngle = point.angle;
  scratchAccumulatedAngle = 0;
  scratchStartTime = player.position;
  scratchShouldResume = needleDropped.value;
  scratchActive.value = true;
  endingScratch = false;
  recordVelocity = 0;

  scratchPausePromise = invoke('player_pause').catch(() => {});
  setPlaybackState(false);
};

const moveScratch = (event) => {
  if (!scratchActive.value || event.pointerId !== scratchPointerId) return;
  const point = pointOnDisc(event);
  if (!point) return;
  event.preventDefault();

  const delta = shortestAngleDelta(scratchLastAngle, point.angle);
  scratchLastAngle = point.angle;
  scratchAccumulatedAngle += delta;
  recordRotation += delta;
  setRecordTransform();

  player.position = timeFromScratchRotation(
    scratchStartTime,
    scratchAccumulatedAngle,
    player.duration,
    SCRATCH_SECONDS_PER_TURN
  );
  tonearmAngle.value = tonearmAngleFromProgress(progress.value);
  queueGrain(player.position);
};

const finishScratch = async (event = null) => {
  if (!scratchActive.value) return;
  if (event && event.pointerId !== scratchPointerId) return;

  const pointerId = scratchPointerId;
  scratchPointerId = null;
  endingScratch = true;
  pendingGrainPosition = null;
  if (grainScheduleTimer) clearTimeout(grainScheduleTimer);
  grainScheduleTimer = null;

  if (pointerId !== null && discRef.value?.hasPointerCapture(pointerId)) {
    discRef.value.releasePointerCapture(pointerId);
  }

  try {
    if (scratchPausePromise) await scratchPausePromise;
    scratchPausePromise = null;
    if (grainPromise) await grainPromise.catch(() => {});
    stopGrainTimer();
    await invoke('player_pause').catch(() => {});
    await invoke('player_seek', { position: player.position }).catch(() => {});

    if (scratchShouldResume && needleDropped.value) {
      await invoke('player_resume').catch(() => {});
      setPlaybackState(true, player.position);
    } else {
      setPlaybackState(false, player.position);
    }
  } finally {
    scratchActive.value = false;
    scratchShouldResume = false;
    endingScratch = false;
  }
};

const tonearmPointerAngle = (event) => {
  const rect = tonearmSvgRef.value?.getBoundingClientRect();
  if (!rect) return null;
  const pivotX = rect.left + rect.width * (142 / 180);
  const pivotY = rect.top + rect.height * (42 / 290);
  return (Math.atan2(event.clientY - pivotY, event.clientX - pivotX) * 180) / Math.PI;
};

const beginTonearmDrag = (event) => {
  if (event.button !== 0 || scratchActive.value) return;
  const pointerAngle = tonearmPointerAngle(event);
  if (pointerAngle === null) return;
  event.preventDefault();
  event.currentTarget.setPointerCapture(event.pointerId);

  tonearmPointerId = event.pointerId;
  tonearmStartPointerAngle = pointerAngle;
  tonearmStartAngle = tonearmAngle.value;
  tonearmDragging.value = true;

  if (player.playing) {
    tonearmPausePromise = invoke('player_pause').catch(() => {});
    setPlaybackState(false);
  } else {
    tonearmPausePromise = null;
  }
};

const moveTonearm = (event) => {
  if (!tonearmDragging.value || event.pointerId !== tonearmPointerId) return;
  const pointerAngle = tonearmPointerAngle(event);
  if (pointerAngle === null) return;
  event.preventDefault();

  const delta = shortestAngleDelta(tonearmStartPointerAngle, pointerAngle);
  tonearmAngle.value = clamp(tonearmStartAngle + delta, TONEARM_REST_ANGLE, TONEARM_INNER_ANGLE);
};

const finishTonearmDrag = async (event) => {
  if (!tonearmDragging.value || event.pointerId !== tonearmPointerId) return;
  const pointerId = tonearmPointerId;
  tonearmPointerId = null;

  if (event.currentTarget.hasPointerCapture(pointerId)) {
    event.currentTarget.releasePointerCapture(pointerId);
  }

  try {
    if (tonearmPausePromise) await tonearmPausePromise;
    tonearmPausePromise = null;

    if (hasTrack.value && tonearmAngle.value >= TONEARM_DROP_THRESHOLD) {
      const landedAngle = clamp(tonearmAngle.value, TONEARM_OUTER_ANGLE, TONEARM_INNER_ANGLE);
      player.position = tonearmProgressFromAngle(landedAngle) * player.duration;
      needleDropped.value = true;
      tonearmAngle.value = landedAngle;
      await invoke('player_seek', { position: player.position }).catch(() => {});
      await invoke('player_resume').catch(() => {});
      setPlaybackState(true, player.position);
    } else {
      needleDropped.value = false;
      tonearmAngle.value = TONEARM_REST_ANGLE;
      await invoke('player_pause').catch(() => {});
      setPlaybackState(false, 0);
    }
  } finally {
    tonearmDragging.value = false;
  }
};

const onMotionPreferenceChange = (event) => {
  reduceMotion = event.matches;
};

onMounted(() => {
  motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
  reduceMotion = motionQuery.matches;
  motionQuery.addEventListener('change', onMotionPreferenceChange);
  pollStatus();
  statusTimer = setInterval(pollStatus, STATUS_POLL_MS);
  animationFrame = requestAnimationFrame(animateRecord);
});

onUnmounted(() => {
  if (statusTimer) clearInterval(statusTimer);
  if (animationFrame) cancelAnimationFrame(animationFrame);
  if (grainScheduleTimer) clearTimeout(grainScheduleTimer);
  stopGrainTimer();
  if (motionQuery) motionQuery.removeEventListener('change', onMotionPreferenceChange);
  if (scratchActive.value) {
    const shouldResume = scratchShouldResume && needleDropped.value;
    const finalPosition = player.position;
    (async () => {
      if (scratchPausePromise) await scratchPausePromise;
      if (grainPromise) await grainPromise.catch(() => {});
      stopGrainTimer();
      await invoke('player_pause').catch(() => {});
      await invoke('player_seek', { position: finalPosition }).catch(() => {});
      if (shouldResume) await invoke('player_resume').catch(() => {});
      syncMainWindow({ playing: shouldResume, position: finalPosition });
    })();
  }
});
</script>

<template>
  <main class="native-turntable" :class="{ 'is-scratching': scratchActive }">
    <div class="platter-shell">
      <div class="platter-rim"></div>
      <div
        ref="discRef"
        class="vinyl-record"
        :class="{
          disabled: !hasTrack || !needleDropped,
          scratching: scratchActive,
        }"
        role="slider"
        tabindex="0"
        aria-label="Vinyl record. Drag to scratch while the tonearm is on the record."
        aria-valuemin="0"
        :aria-valuemax="player.duration"
        :aria-valuenow="Math.round(player.position)"
        @pointerdown="beginScratch"
        @pointermove="moveScratch"
        @pointerup="finishScratch"
        @pointercancel="finishScratch"
      >
        <div class="vinyl-grooves"></div>
        <div class="vinyl-reflection"></div>
        <div class="record-label">
          <CoverImage v-if="player.path" :path="player.path" className="record-cover" />
          <div v-else class="record-cover record-cover-empty"></div>
          <span class="record-spindle"></span>
        </div>
      </div>
    </div>

    <svg
      ref="tonearmSvgRef"
      class="tonearm-svg"
      viewBox="0 0 180 290"
      aria-label="Tonearm. Drag it onto the record to play or back to its rest to pause."
      role="slider"
      :aria-valuemin="TONEARM_REST_ANGLE"
      :aria-valuemax="TONEARM_INNER_ANGLE"
      :aria-valuenow="Math.round(tonearmAngle)"
    >
      <defs>
        <linearGradient id="native-tonearm-metal" x1="0" x2="1">
          <stop offset="0" stop-color="#777a80" />
          <stop offset="0.42" stop-color="#f5f5f4" />
          <stop offset="0.65" stop-color="#c9cbd0" />
          <stop offset="1" stop-color="#686b72" />
        </linearGradient>
        <radialGradient id="native-pivot-metal" cx="42%" cy="35%">
          <stop offset="0" stop-color="#fafafa" />
          <stop offset="0.34" stop-color="#bfc1c5" />
          <stop offset="1" stop-color="#686b72" />
        </radialGradient>
      </defs>

      <circle cx="142" cy="42" r="32" fill="#111216" stroke="#3f424a" stroke-width="6" />
      <circle
        cx="142"
        cy="42"
        r="14"
        fill="url(#native-pivot-metal)"
        stroke="#777a82"
        stroke-width="3"
      />
      <circle cx="142" cy="42" r="5" fill="#e4e4e7" opacity="0.8" />

      <g
        class="tonearm-moving"
        :class="{
          dragging: tonearmDragging,
          dropped: needleDropped,
        }"
        :style="tonearmStyle"
        @pointerdown="beginTonearmDrag"
        @pointermove="moveTonearm"
        @pointerup="finishTonearmDrag"
        @pointercancel="finishTonearmDrag"
      >
        <path
          class="tonearm-hit-area"
          d="M142 42 C141 96 124 150 78 219 L58 247"
          fill="none"
          stroke="transparent"
          stroke-width="34"
          stroke-linecap="round"
        />
        <path
          d="M142 42 C141 96 124 150 78 219"
          fill="none"
          stroke="url(#native-tonearm-metal)"
          stroke-width="9"
          stroke-linecap="round"
        />
        <path
          d="M78 219l-20 28"
          fill="none"
          stroke="#d6d7da"
          stroke-width="8"
          stroke-linecap="round"
        />
        <path d="M48 238l23 16-13 19-23-16z" fill="#24262b" stroke="#686b74" stroke-width="3" />
        <path
          class="stylus"
          d="M46 264l-4 12"
          stroke="var(--accent-color)"
          stroke-width="3"
          stroke-linecap="round"
        />
      </g>
    </svg>
  </main>
</template>

<style scoped>
:global(html.vinyl-native-window),
:global(html.vinyl-native-window body),
:global(html.vinyl-native-window #app) {
  width: 100%;
  height: 100%;
  overflow: hidden;
  margin: 0;
  background: #111113;
}

.native-turntable {
  position: relative;
  width: 100vw;
  height: 100vh;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  user-select: none;
  background:
    linear-gradient(122deg, rgba(255, 255, 255, 0.025), transparent 32%),
    repeating-linear-gradient(0deg, rgba(255, 255, 255, 0.009) 0 1px, transparent 1px 4px),
    radial-gradient(circle at 34% 42%, #29292d 0, #202024 46%, #17171a 100%);
  box-shadow:
    inset 0 0 0 1px rgba(255, 255, 255, 0.07),
    inset 0 -45px 90px rgba(0, 0, 0, 0.2);
}

.native-turntable::after {
  position: absolute;
  inset: 0;
  z-index: 20;
  content: '';
  border: 1px solid rgba(255, 255, 255, 0.05);
  pointer-events: none;
}

.platter-shell {
  position: absolute;
  top: 50%;
  left: 3.2vw;
  width: min(80vw, 92vh);
  aspect-ratio: 1;
  border-radius: 50%;
  transform: translateY(-50%);
  background: #08080a;
  box-shadow:
    0 23px 37px rgba(0, 0, 0, 0.62),
    0 0 0 3px #17171a,
    0 0 0 5px rgba(255, 255, 255, 0.055),
    inset 0 0 0 8px #0b0b0d;
}

.platter-rim {
  position: absolute;
  inset: 0;
  border-radius: 50%;
  background: repeating-conic-gradient(from 1deg, #6b6b70 0 1deg, transparent 1deg 4.5deg);
  opacity: 0.44;
  mask: radial-gradient(circle, transparent 0 94%, #000 94.3% 97%, transparent 97.3%);
}

.vinyl-record {
  position: absolute;
  inset: 2.4%;
  overflow: hidden;
  border: 1px solid #020203;
  border-radius: 50%;
  outline: none;
  cursor: grab;
  touch-action: none;
  background:
    radial-gradient(
      circle at 50% 50%,
      transparent 0 16.6%,
      rgba(255, 255, 255, 0.03) 17% 17.8%,
      transparent 18.2%
    ),
    radial-gradient(
      circle at 31% 23%,
      #333338 0,
      #17171a 28%,
      #08080a 56%,
      #151518 80%,
      #050506 100%
    );
  box-shadow:
    inset 0 0 38px rgba(255, 255, 255, 0.055),
    0 13px 27px rgba(0, 0, 0, 0.68);
  will-change: transform;
}

.vinyl-record:focus-visible {
  box-shadow:
    0 0 0 3px color-mix(in srgb, var(--accent-color) 68%, transparent),
    0 13px 27px rgba(0, 0, 0, 0.68);
}

.vinyl-record.scratching {
  cursor: grabbing;
  box-shadow:
    0 0 0 2px color-mix(in srgb, var(--accent-color) 64%, transparent),
    0 0 34px color-mix(in srgb, var(--accent-color) 18%, transparent),
    0 18px 34px rgba(0, 0, 0, 0.72);
}

.vinyl-record.disabled {
  cursor: default;
}

.vinyl-grooves {
  position: absolute;
  inset: 1.2%;
  border-radius: 50%;
  background: repeating-radial-gradient(
    circle,
    transparent 0 2px,
    rgba(255, 255, 255, 0.047) 2.3px 2.85px,
    rgba(0, 0, 0, 0.12) 3.15px 4.7px
  );
  opacity: 0.76;
  pointer-events: none;
}

.vinyl-reflection {
  position: absolute;
  inset: -9%;
  border-radius: 50%;
  background: conic-gradient(
    from 205deg,
    transparent 0 19%,
    rgba(255, 255, 255, 0.105) 27%,
    transparent 35% 63%,
    rgba(255, 255, 255, 0.046) 72%,
    transparent 81%
  );
  opacity: 0.58;
  pointer-events: none;
}

.record-label {
  position: absolute;
  top: 50%;
  left: 50%;
  width: 33%;
  aspect-ratio: 1;
  overflow: hidden;
  border: 4px solid #0d0d0f;
  border-radius: 50%;
  transform: translate(-50%, -50%);
  background: #232631;
  box-shadow:
    0 0 0 1px rgba(255, 255, 255, 0.12),
    0 2px 10px rgba(0, 0, 0, 0.62);
  pointer-events: none;
}

.record-cover {
  width: 100%;
  height: 100%;
  border: 0;
  border-radius: 50%;
}

.record-cover-empty {
  background: radial-gradient(circle at 35% 28%, #3c4250, #20232b 72%);
}

.record-spindle {
  position: absolute;
  top: 50%;
  left: 50%;
  width: 12px;
  height: 12px;
  border: 2px solid rgba(255, 255, 255, 0.7);
  border-radius: 50%;
  transform: translate(-50%, -50%);
  background: #09090b;
  box-shadow: 0 1px 4px #000;
}

.tonearm-svg {
  position: absolute;
  top: 1.5vh;
  right: 0.7vw;
  z-index: 10;
  width: min(35vw, 42vh);
  height: auto;
  overflow: visible;
  filter: drop-shadow(0 9px 6px rgba(0, 0, 0, 0.5));
}

.tonearm-moving {
  cursor: grab;
  touch-action: none;
  transform-box: view-box;
  transform-origin: 142px 42px;
  transition: transform 0.32s cubic-bezier(0.2, 0.82, 0.2, 1);
}

.tonearm-moving.dragging {
  cursor: grabbing;
  transition: none;
}

.tonearm-moving.dropped .stylus {
  filter: drop-shadow(0 0 4px var(--accent-color));
}

.stylus {
  transition: filter 0.2s ease;
}

@media (max-aspect-ratio: 1/1) {
  .platter-shell {
    top: 53%;
    left: 2.5vw;
    width: min(84vw, 76vh);
  }

  .tonearm-svg {
    width: min(38vw, 36vh);
  }
}

@media (prefers-reduced-motion: reduce) {
  .tonearm-moving,
  .stylus {
    transition-duration: 0.01ms;
  }
}
</style>
