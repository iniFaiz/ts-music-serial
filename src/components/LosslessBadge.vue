<script setup>
// Shared "Lossless" badge + details popover used by the mini player, fullscreen
// player and player bar. Click toggles a popover showing codec/bit-depth/rate.
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { store } from '../store';
import { LOSSLESS_LOGO_PATH } from '../losslessLogo';

defineProps({
  // Where the popover opens relative to the badge: 'up' (above) or 'down' (below).
  placement: { type: String, default: 'up' },
  // Compact icon-only variant used in the player bar.
  iconOnly: { type: Boolean, default: false },
});

const open = ref(false);

const isLossless = computed(() => {
  if (!store.currentSong || !store.currentSong.path) return false;
  const ext = store.currentSong.path.split('.').pop().toLowerCase();
  return ['flac', 'wav', 'alac', 'm4a'].includes(ext);
});

const specs = computed(() => {
  const song = store.currentSong;
  if (!song || !song.path) return '24-bit 48kHz ALAC';
  const ext = song.path.split('.').pop().toLowerCase();
  const bits = store.currentBitDepth || song.bit_depth;
  const hz = store.currentSampleRate || song.sample_rate;
  if (bits && hz) {
    const bitStr = `${bits}-bit`;
    const rateStr = hz >= 1000 ? `${(hz / 1000).toFixed(1).replace('.0', '')}kHz` : `${hz}Hz`;
    const codecStr = ext === 'm4a' ? 'ALAC' : ext.toUpperCase();
    return `${bitStr} ${rateStr} ${codecStr}`;
  }
  if (ext === 'flac') return '24-bit 48kHz FLAC';
  if (ext === 'wav') return '16-bit 44.1kHz WAV';
  return '24-bit 48kHz ALAC';
});

const close = () => {
  open.value = false;
};
onMounted(() => document.addEventListener('click', close));
onUnmounted(() => document.removeEventListener('click', close));
</script>

<template>
  <div v-if="isLossless" :class="iconOnly ? 'relative shrink-0' : 'relative inline-flex'">
    <button
      @click.stop="open = !open"
      :class="
        iconOnly
          ? 'flex shrink-0 items-center justify-center text-gray-500 hover:text-gray-300 transition-colors focus:outline-none'
          : 'flex items-center gap-1 px-1.5 py-0.5 rounded bg-white/10 hover:bg-white/15 transition-colors border border-white/10 text-white/70 hover:text-white text-[9px] font-bold uppercase tracking-wider select-none focus:outline-none leading-none'
      "
      title="Lossless Audio"
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 15 9"
        :class="iconOnly ? 'h-2.5 w-[17px] fill-current' : 'block h-2 w-[13px] fill-current shrink-0'"
      >
        <path :d="LOSSLESS_LOGO_PATH" />
      </svg>
      <span v-if="!iconOnly" class="leading-none">Lossless</span>
    </button>

    <div
      v-if="open"
      @click.stop
      class="lossless-popover absolute left-1/2 -translate-x-1/2 z-[120] bg-[#1c1c1e] border border-[#323236] rounded-xl shadow-2xl p-4 w-[230px] text-center select-none animate-lossless-pop"
      :class="placement === 'down' ? 'top-full mt-3' : 'bottom-full mb-3'"
    >
      <!-- Arrow -->
      <div
        v-if="placement === 'down'"
        class="absolute bottom-full left-1/2 -translate-x-1/2 translate-y-1/2 w-2 h-2 bg-[#1c1c1e] border-l border-t border-[#323236] rotate-45"
      ></div>
      <div
        v-else
        class="absolute top-full left-1/2 -translate-x-1/2 -translate-y-1/2 w-2 h-2 bg-[#1c1c1e] border-r border-b border-[#323236] rotate-45"
      ></div>

      <div class="flex justify-center mb-2">
        <svg
          xmlns="http://www.w3.org/2000/svg"
          viewBox="0 0 15 9"
          class="h-5 w-[35px] text-white fill-current"
        >
          <path
            :d="LOSSLESS_LOGO_PATH"
          />
        </svg>
      </div>
      <h4 class="text-sm font-bold text-white mb-0.5">Lossless</h4>
      <p class="mb-3 text-xs leading-normal text-gray-400">
        This audio is playing with lossless compression.
      </p>
      <div
        class="bg-[#2c2c2e]/60 rounded-lg py-1 px-3 text-xs font-semibold text-[var(--accent-color)] font-variant-numeric tracking-wide border border-white/5"
      >
        {{ specs }}
      </div>
    </div>
  </div>
</template>

<style scoped>
.animate-lossless-pop {
  animation: losslessPop 0.15s cubic-bezier(0.16, 1, 0.3, 1) forwards;
}
@keyframes losslessPop {
  from {
    opacity: 0;
    transform: translate(-50%, 4px) scale(0.95);
  }
  to {
    opacity: 1;
    transform: translate(-50%, 0) scale(1);
  }
}
</style>
