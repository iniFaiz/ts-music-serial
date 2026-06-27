<script setup>
import { computed } from 'vue';

const props = defineProps({
  // Two comma-separated hex colours, e.g. "#fa2d48,#7a1020". Falls back to a
  // deterministic hue derived from the title when omitted.
  color: { type: String, default: '' },
  title: { type: String, default: '' },
  // Small upper-case eyebrow shown top-left (e.g. "Made for You").
  topLabel: { type: String, default: '' },
  // Optional one-liner shown along the bottom.
  subtitle: { type: String, default: '' },
  // Watermark glyph: one of the keys handled in the template below.
  icon: { type: String, default: '' },
  className: { type: String, default: 'w-full h-full rounded-xl' },
  // Render the big centred title (Apple "big card" look). When false, only the
  // eyebrow/subtitle chrome shows — handy for compact art.
  showTitle: { type: Boolean, default: true },
  // Optional custom cover image (data URL). When set it replaces the gradient;
  // a dark scrim keeps any overlaid text legible.
  cover: { type: String, default: null },
});

const colors = computed(() => {
  const parts = (props.color || '').split(',').map((c) => c.trim()).filter(Boolean);
  if (parts.length >= 2) return parts;
  // Deterministic hue from the title so untinted cards stay stable.
  const s = props.title || '';
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) % 360;
  return [`hsl(${h} 65% 50%)`, `hsl(${(h + 40) % 360} 60% 28%)`];
});

const gradient = computed(
  () => `linear-gradient(145deg, ${colors.value[0]} 0%, ${colors.value[1]} 100%)`
);
</script>

<template>
  <div
    :class="[className, 'relative overflow-hidden flex flex-col justify-between select-none']"
    :style="cover ? null : { background: gradient }"
  >
    <!-- Custom cover image + legibility scrim -->
    <template v-if="cover">
      <img :src="cover" class="absolute inset-0 w-full h-full object-cover" alt="" draggable="false" />
      <div
        v-if="topLabel || (showTitle && title) || subtitle"
        class="absolute inset-0"
        style="background: linear-gradient(to top, rgba(0,0,0,0.65), rgba(0,0,0,0.05) 55%, rgba(0,0,0,0.25))"
      ></div>
    </template>

    <!-- Soft sheen + watermark icon (gradient mode only) -->
    <div
      v-if="!cover"
      class="absolute inset-0 opacity-40"
      style="background: radial-gradient(120% 80% at 100% 0%, rgba(255,255,255,0.35), transparent 60%)"
    ></div>
    <svg
      v-if="icon && !cover"
      class="absolute -right-5 -bottom-6 w-2/3 h-2/3 text-white/15 rotate-[-8deg]"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.6"
      stroke-linecap="round"
      stroke-linejoin="round"
    >
      <template v-if="icon === 'clock'">
        <circle cx="12" cy="12" r="9" />
        <path d="M12 7v5l3 2" />
      </template>
      <template v-else-if="icon === 'repeat'">
        <path d="M17 1l4 4-4 4" />
        <path d="M3 11V9a4 4 0 0 1 4-4h14" />
        <path d="M7 23l-4-4 4-4" />
        <path d="M21 13v2a4 4 0 0 1-4 4H3" />
      </template>
      <template v-else-if="icon === 'fire'">
        <path
          d="M12 2c1 3-1 4-2 6s0 4 2 4 3-2 2-5c2 1 4 4 4 7a6 6 0 1 1-12 0c0-4 3-6 4-9 1 .5 2 .5 2-3z"
          fill="currentColor"
          stroke="none"
        />
      </template>
      <template v-else-if="icon === 'star'">
        <polygon
          points="12 2 15 9 22 9.3 16.5 14 18.5 21 12 17 5.5 21 7.5 14 2 9.3 9 9"
          fill="currentColor"
          stroke="none"
        />
      </template>
      <template v-else-if="icon === 'heart'">
        <path
          d="M20.8 4.6a5.5 5.5 0 0 0-7.8 0L12 5.7l-1-1.1a5.5 5.5 0 1 0-7.8 7.8l1 1L12 21.2l7.8-7.8 1-1a5.5 5.5 0 0 0 0-7.8z"
          fill="currentColor"
          stroke="none"
        />
      </template>
      <template v-else-if="icon === 'radio'">
        <circle cx="12" cy="12" r="2" />
        <path d="M4.9 19.1a10 10 0 0 1 0-14.2M7.8 16.2a6 6 0 0 1 0-8.4M16.2 7.8a6 6 0 0 1 0 8.4M19.1 4.9a10 10 0 0 1 0 14.2" />
      </template>
      <template v-else-if="icon === 'bolt'">
        <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" fill="currentColor" stroke="none" />
      </template>
      <template v-else>
        <!-- sparkles (default) -->
        <path d="M12 3l1.8 4.7L18.5 9l-4.7 1.8L12 15l-1.8-4.2L5.5 9l4.7-1.3L12 3z" fill="currentColor" stroke="none" />
        <path d="M19 14l.9 2.3L22 17l-2.1.7L19 20l-.9-2.3L16 17l2.1-.7L19 14z" fill="currentColor" stroke="none" />
      </template>
    </svg>

    <!-- Eyebrow -->
    <div v-if="topLabel" class="relative px-4 pt-3.5">
      <span class="text-[10px] font-bold uppercase tracking-[0.14em] text-white/80">{{ topLabel }}</span>
    </div>
    <div v-else class="relative"></div>

    <!-- Big title -->
    <div v-if="showTitle" class="relative px-4 flex-1 flex items-center">
      <span class="text-white font-extrabold leading-[1.02] tracking-tight drop-shadow text-2xl xl:text-3xl break-words">
        {{ title }}
      </span>
    </div>

    <!-- Subtitle -->
    <div class="relative px-4 pb-3.5">
      <span v-if="subtitle" class="text-[11px] font-medium text-white/85 line-clamp-2 leading-snug">
        {{ subtitle }}
      </span>
    </div>
  </div>
</template>
