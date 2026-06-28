<script setup>
// 10-band graphic equalizer UI. The DSP itself runs in Rust (an EqualizerSource
// filter spliced into the decoder→sink chain); this panel only edits the shared
// gain values via the store, which forwards them with player_set_equalizer.
import { computed } from 'vue';
import { store } from '../store';
import {
  EQ_FREQ_LABELS,
  EQ_PRESET_LIST,
  EQ_MIN_DB,
  EQ_MAX_DB,
} from '../equalizer';
import ToggleInt from './settings/ToggleInt.vue';
import SliderInt from './settings/SliderInt.vue';
import SelectInt from './settings/SelectInt.vue';

const freqLabels = EQ_FREQ_LABELS;

// Built-in presets, plus a transient "Custom" entry so the dropdown can show the
// current state once a band has been hand-adjusted (selecting it is a no-op).
const presetOptions = computed(() => {
  const opts = EQ_PRESET_LIST.map((p) => ({ value: p.id, label: p.label }));
  if (store.eqPreset === 'custom') opts.push({ value: 'custom', label: 'Custom' });
  return opts;
});

function onPreset(id) {
  if (id === 'custom') return;
  store.applyEqPreset(id);
}

// Filled-track gradient for a band, mirroring the accent-fill look of SliderInt.
// The slider is rotated -90deg in CSS, so a left→right gradient reads bottom→top.
function bandStyle(v) {
  const pct = ((v - EQ_MIN_DB) / (EQ_MAX_DB - EQ_MIN_DB)) * 100;
  return {
    background: `linear-gradient(to right, var(--accent-color) ${pct}%, #4b5563 ${pct}%)`,
  };
}

function gainLabel(v) {
  if (v > 0) return `+${v}`;
  return `${v}`;
}

function onBand(i, e) {
  store.setEqBand(i, Number(e.target.value));
}
</script>

<template>
  <div>
    <ToggleInt
      :modelValue="store.eqEnabled"
      @update:modelValue="(v) => store.setEqEnabled(v)"
      label="Enable equalizer"
    />

    <!-- Everything below is dimmed/inert while the EQ is off. -->
    <div :class="store.eqEnabled ? '' : 'opacity-40 pointer-events-none'">
      <!-- Preset dropdown -->
      <SelectInt
        label="Preset"
        :modelValue="store.eqPreset"
        :options="presetOptions"
        @update:modelValue="onPreset"
      />

      <!-- Band sliders -->
      <div class="flex items-end justify-between gap-1 mt-3">
        <div
          v-for="(label, i) in freqLabels"
          :key="i"
          class="flex flex-col items-center gap-2 flex-1 min-w-0"
        >
          <span class="text-[10px] text-gray-400 tabular-nums leading-none">
            {{ gainLabel(store.eqBands[i]) }}
          </span>
          <div class="eq-slider-wrap">
            <input
              type="range"
              class="eq-slider"
              :min="EQ_MIN_DB"
              :max="EQ_MAX_DB"
              step="1"
              :value="store.eqBands[i]"
              :style="bandStyle(store.eqBands[i])"
              @input="(e) => onBand(i, e)"
            />
          </div>
          <span class="text-[10px] text-gray-500 leading-none">{{ label }}</span>
        </div>
      </div>

      <!-- Pre-amp + reset -->
      <div class="mt-5 border-t border-white/5">
        <SliderInt
          label="Pre-amp"
          :modelValue="store.eqPreampDb"
          :min="EQ_MIN_DB"
          :max="EQ_MAX_DB"
          :step="1"
          suffix=" dB"
          @update:modelValue="(v) => store.setEqPreamp(v)"
        />
        <p class="text-xs text-gray-500 -mt-1">
          Lower the pre-amp if boosting bands causes clipping/distortion.
        </p>
      </div>

      <div class="flex justify-end mt-2">
        <button
          @click="store.resetEq()"
          class="text-xs font-medium text-gray-400 hover:text-white"
        >
          Reset to flat
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* Fixed box that hosts the rotated horizontal range, turning it into a vertical
   fader. Rotating a real <input type=range> is the most reliable cross-engine
   way to get a vertical slider in WebView2. */
.eq-slider-wrap {
  height: 130px;
  width: 26px;
  display: flex;
  align-items: center;
  justify-content: center;
}
.eq-slider {
  /* Width here becomes the visual height after the -90deg rotation. */
  width: 130px;
  height: 4px;
  border-radius: 4px;
  transform: rotate(-90deg);
  -webkit-appearance: none;
  appearance: none;
  cursor: pointer;
  outline: none;
}
.eq-slider::-webkit-slider-thumb {
  -webkit-appearance: none;
  height: 14px;
  width: 14px;
  border-radius: 50%;
  background: #fff;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.45);
  cursor: pointer;
}
</style>
