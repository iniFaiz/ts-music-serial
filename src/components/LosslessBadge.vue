<script setup>
// Shared "Lossless" badge + details popover, matching the one in PlayerControls
// and FullScreenPlayer. Click toggles a popover showing the codec/bit-depth/rate.
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { store } from '../store';

defineProps({
  // Where the popover opens relative to the badge: 'up' (above) or 'down' (below).
  placement: { type: String, default: 'up' },
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
  <div v-if="isLossless" class="relative inline-flex">
    <button
      @click.stop="open = !open"
      class="flex items-center gap-1 px-1.5 py-0.5 rounded bg-white/10 hover:bg-white/15 transition-colors border border-white/10 text-white/70 hover:text-white text-[9px] font-bold uppercase tracking-wider select-none focus:outline-none leading-none"
      title="Lossless Audio"
    >
      <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 15 9" class="block h-2 w-[13px] fill-current shrink-0">
        <path d="M8.184,0.35C9.944,0.35 10.703,3.296 11.338,5.238C11.673,3.842 11.497,3.542 11.857,3.542C11.99,3.542 12.126,3.633 12.126,3.798C12.126,3.809 12.123,3.839 12.117,3.883L12.091,4.058C12.02,4.522 11.845,5.494 11.654,6.144C13.198,10.191 14.345,4.861 14.474,3.772C14.493,3.615 14.612,3.542 14.731,3.542C14.891,3.542 15.022,3.662 14.997,3.843C14.72,5.605 14.295,8.35 12.547,8.35C11.582,8.35 11.04,7.595 10.611,6.73C9.54,4.626 9.047,1.093 7.997,1.093C7.66,1.093 7.411,1.444 7.394,1.444C7.362,1.444 7.337,1.301 7.023,0.909C7.322,0.567 7.734,0.35 8.184,0.35ZM2.458,0.354C5.211,0.354 5.456,7.618 7.014,7.618C7.197,7.618 7.394,7.507 7.61,7.256C7.729,7.458 7.851,7.638 7.978,7.796C7.667,8.151 7.28,8.35 6.795,8.35C5.054,8.349 4.306,5.434 3.663,3.466C3.511,4.097 3.432,4.669 3.402,4.925C3.382,5.088 3.263,5.163 3.143,5.163C3.009,5.163 2.874,5.071 2.874,4.908L2.874,4.908L2.877,4.87C2.966,4.223 3.146,3.243 3.347,2.56C3.079,1.858 2.745,1.091 2.252,1.091C1.257,1.091 0.687,3.591 0.527,4.925C0.508,5.088 0.388,5.163 0.268,5.163C0.135,5.163 0,5.071 0,4.908C0,4.896 0.001,4.883 0.002,4.87C0.283,2.836 0.808,0.354 2.458,0.354ZM5.315,0.35C5.809,0.35 6.339,0.608 6.797,1.211C6.822,1.241 7.078,1.639 7.159,1.777C8.277,3.802 8.818,7.627 9.881,7.627C10.065,7.627 10.264,7.513 10.484,7.256C10.604,7.458 10.726,7.638 10.852,7.796C10.542,8.15 10.155,8.35 9.67,8.35C6.933,8.349 6.636,1.09 5.128,1.09C4.788,1.09 4.536,1.444 4.519,1.444C4.487,1.444 4.462,1.301 4.148,0.909C4.455,0.558 4.87,0.35 5.315,0.35Z" />
      </svg>
      <span class="leading-none">Lossless</span>
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
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 15 9" class="h-5 w-[35px] text-white fill-current">
          <path d="M8.184,0.35C9.944,0.35 10.703,3.296 11.338,5.238C11.673,3.842 11.497,3.542 11.857,3.542C11.99,3.542 12.126,3.633 12.126,3.798C12.126,3.809 12.123,3.839 12.117,3.883L12.091,4.058C12.02,4.522 11.845,5.494 11.654,6.144C13.198,10.191 14.345,4.861 14.474,3.772C14.493,3.615 14.612,3.542 14.731,3.542C14.891,3.542 15.022,3.662 14.997,3.843C14.72,5.605 14.295,8.35 12.547,8.35C11.582,8.35 11.04,7.595 10.611,6.73C9.54,4.626 9.047,1.093 7.997,1.093C7.66,1.093 7.411,1.444 7.394,1.444C7.362,1.444 7.337,1.301 7.023,0.909C7.322,0.567 7.734,0.35 8.184,0.35ZM2.458,0.354C5.211,0.354 5.456,7.618 7.014,7.618C7.197,7.618 7.394,7.507 7.61,7.256C7.729,7.458 7.851,7.638 7.978,7.796C7.667,8.151 7.28,8.35 6.795,8.35C5.054,8.349 4.306,5.434 3.663,3.466C3.511,4.097 3.432,4.669 3.402,4.925C3.382,5.088 3.263,5.163 3.143,5.163C3.009,5.163 2.874,5.071 2.874,4.908L2.874,4.908L2.877,4.87C2.966,4.223 3.146,3.243 3.347,2.56C3.079,1.858 2.745,1.091 2.252,1.091C1.257,1.091 0.687,3.591 0.527,4.925C0.508,5.088 0.388,5.163 0.268,5.163C0.135,5.163 0,5.071 0,4.908C0,4.896 0.001,4.883 0.002,4.87C0.283,2.836 0.808,0.354 2.458,0.354ZM5.315,0.35C5.809,0.35 6.339,0.608 6.797,1.211C6.822,1.241 7.078,1.639 7.159,1.777C8.277,3.802 8.818,7.627 9.881,7.627C10.065,7.627 10.264,7.513 10.484,7.256C10.604,7.458 10.726,7.638 10.852,7.796C10.542,8.15 10.155,8.35 9.67,8.35C6.933,8.349 6.636,1.09 5.128,1.09C4.788,1.09 4.536,1.444 4.519,1.444C4.487,1.444 4.462,1.301 4.148,0.909C4.455,0.558 4.87,0.35 5.315,0.35Z" />
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
