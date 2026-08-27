<script setup>
import { ref, onUnmounted } from 'vue';
import TsLogo from '../TsLogo.vue';

defineProps({
  version: {
    type: String,
    default: '1.0.0',
  },
});

// Fidget spinner physics easter egg
const rotation = ref(0);
const currentVelocity = ref(0);
const isSpinning = ref(false);
let velocity = 0;
let lastTime = null;
let animId = null;
let resetTimer = null;

const FRICTION = 0.991; // Smooth ball-bearing deceleration
const IMPULSE = 550; // Velocity boost per click (degrees/sec)
const MAX_VELOCITY = 7200; // Top speed cap

const onLogoClick = () => {
  if (resetTimer) {
    clearTimeout(resetTimer);
    resetTimer = null;
  }

  // Normalize angle before spinning
  rotation.value = rotation.value % 360;

  // Add momentum on each click/tap (stacks when spammed)
  velocity = Math.min(MAX_VELOCITY, velocity + IMPULSE);
  currentVelocity.value = velocity;
  isSpinning.value = true;

  if (!animId) {
    lastTime = performance.now();
    animId = requestAnimationFrame(updatePhysics);
  }
};

const updatePhysics = (now) => {
  if (!lastTime) lastTime = now;
  const dt = Math.min((now - lastTime) / 1000, 0.1);
  lastTime = now;

  if (Math.abs(velocity) > 0.4) {
    rotation.value = (rotation.value + velocity * dt) % 360;
    // Exponential decay proportional to elapsed time
    velocity *= Math.pow(FRICTION, dt * 60);
    currentVelocity.value = velocity;
    animId = requestAnimationFrame(updatePhysics);
  } else {
    // Total stop reached -> snap back to upright original position
    velocity = 0;
    currentVelocity.value = 0;
    animId = null;
    lastTime = null;

    isSpinning.value = false;
    // Take shortest path back to 0° / upright
    if (rotation.value > 180) {
      rotation.value = 360;
    } else {
      rotation.value = 0;
    }

    resetTimer = setTimeout(() => {
      if (!isSpinning.value) {
        rotation.value = 0;
      }
    }, 550);
  }
};

onUnmounted(() => {
  if (animId) {
    cancelAnimationFrame(animId);
    animId = null;
  }
  if (resetTimer) {
    clearTimeout(resetTimer);
    resetTimer = null;
  }
});
</script>

<template>
  <div class="mt-12 pt-8 pb-4 flex flex-col items-center justify-center text-center select-none">
    <!-- App Logo (Interactive Fidget Spinner Easter Egg) -->
    <button
      type="button"
      class="mb-4 cursor-pointer active:scale-95 transition-transform duration-75 select-none rounded-full flex items-center justify-center bg-transparent border-0 p-0 focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent-color)]"
      @click="onLogoClick"
      aria-label="ts-music logo spinner"
      title="ts-music"
    >
      <div
        class="transition-shadow duration-300 rounded-full"
        :class="{
          'drop-shadow-[0_0_16px_rgba(250,45,72,0.45)]': currentVelocity > 1500,
        }"
        :style="{
          transform: `rotate(${rotation}deg)`,
          transition: isSpinning
            ? 'none'
            : 'transform 0.5s cubic-bezier(0.34, 1.4, 0.64, 1)',
          willChange: currentVelocity > 0 ? 'transform' : 'auto',
        }"
      >
        <TsLogo :size="104" />
      </div>
    </button>

    <!-- App Name -->
    <h2 class="text-2xl font-bold text-white tracking-tight">ts-music</h2>

    <!-- Version & Copyright -->
    <div class="mt-2.5 space-y-1 text-xs text-gray-400">
      <div>{{ $t('settings.appCredit.version', { version }) }}</div>
      <div>{{ $t('settings.appCredit.copyright') }}</div>
    </div>
  </div>
</template>
