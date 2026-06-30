<template>
  <div class="py-2.5" :class="{ 'opacity-40 pointer-events-none select-none': disabled }">
    <div class="flex items-center justify-between mb-2">
      <span class="text-gray-300 font-medium text-sm">{{ label }}</span>
      <span class="text-sm text-gray-400 tabular-nums">{{ display }}</span>
    </div>
    <input
      type="range"
      :min="min"
      :max="max"
      :step="step"
      :value="modelValue"
      :disabled="disabled"
      @input="$emit('update:modelValue', Number($event.target.value))"
      class="w-full h-1 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
      :style="{
        background: `linear-gradient(to right, var(--accent-color) ${pct}%, #4b5563 ${pct}%)`,
      }"
    />
  </div>
</template>

<script setup>
import { computed } from 'vue';

const props = defineProps({
  modelValue: { type: Number, required: true },
  label: { type: String, required: true },
  min: { type: Number, default: 0 },
  max: { type: Number, default: 100 },
  step: { type: Number, default: 1 },
  suffix: { type: String, default: '' },
  disabled: { type: Boolean, default: false },
});
defineEmits(['update:modelValue']);

const pct = computed(() =>
  Math.min(100, Math.max(0, ((props.modelValue - props.min) / (props.max - props.min)) * 100))
);
const display = computed(() => {
  const sign = props.modelValue > 0 && props.suffix.includes('dB') ? '+' : '';
  return `${sign}${props.modelValue}${props.suffix}`;
});
</script>

<style scoped>
input[type='range']::-webkit-slider-thumb {
  -webkit-appearance: none;
  height: 13px;
  width: 13px;
  border-radius: 50%;
  background: var(--accent-color);
  margin-top: -6px;
}
</style>
