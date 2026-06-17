<script setup>
import { computed } from 'vue';

const props = defineProps({
  name: { type: String, default: '' },
  cover: { type: String, default: null },
  // Box styling (size/rounding/shadow). The initials font scales from `size`.
  className: { type: String, default: 'h-10 w-10 rounded' },
  size: { type: Number, default: 40 },
});

// Deterministic hue from the name so each playlist keeps a consistent colour.
const hue = computed(() => {
  const s = props.name || '';
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) % 360;
  return h;
});

const gradient = computed(
  () => `linear-gradient(135deg, hsl(${hue.value} 60% 48%), hsl(${(hue.value + 45) % 360} 62% 26%))`
);

const initials = computed(() => {
  const words = (props.name || '').trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return '♪'; // ♪
  const first = words[0][0] || '';
  const last = words.length > 1 ? words[words.length - 1][0] : '';
  return (first + last).toUpperCase();
});
</script>

<template>
  <div :class="[className, 'overflow-hidden flex items-center justify-center shrink-0']">
    <img v-if="cover" :src="cover" class="w-full h-full object-cover" alt="" draggable="false" />
    <div
      v-else
      class="w-full h-full flex items-center justify-center"
      :style="{ background: gradient }"
    >
      <span
        class="font-bold text-white/90 leading-none tracking-tight select-none"
        :style="{ fontSize: Math.round(size * 0.4) + 'px' }"
        >{{ initials }}</span
      >
    </div>
  </div>
</template>
