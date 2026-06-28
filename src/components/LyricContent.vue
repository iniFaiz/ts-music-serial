<script setup>
// Renders the inner content of a single lyric line, shared by the fullscreen
// player and the lyrics sidebar:
//   • word-by-word "karaoke" wipe for the active line when the provider gives
//     per-word timing (line.words),
//   • a smaller romanization (romaji) sub-line beneath, when enabled + present.
// Non-active lines render as plain text so per-frame work stays on one line.
const props = defineProps({
  line: { type: Object, required: true },
  active: { type: Boolean, default: false },
  isPast: { type: Boolean, default: false },
  // Playhead position (ms). Parents pass the live value only for the active
  // line (0 otherwise) so non-active lines don't re-render every poll tick.
  currentMs: { type: Number, default: 0 },
  showRomaji: { type: Boolean, default: false },
});

// State of one karaoke word relative to the playhead.
function wordClass(w) {
  if (props.isPast && props.currentMs === 0) return 'lc-word lc-sung';
  const now = props.currentMs;
  if (now >= w.time_ms + w.duration_ms) return 'lc-word lc-sung';
  if (now < w.time_ms) return 'lc-word';
  return 'lc-word lc-active';
}

// Drives the left→right gradient wipe. background-size is 200%, so position
// 100% = fully dim, 0% = fully lit; a CSS transition smooths between ticks.
function wordStyle(w) {
  if (props.isPast && props.currentMs === 0) return { backgroundPositionX: '0%' };
  const now = props.currentMs;
  if (now >= w.time_ms + w.duration_ms) return { backgroundPositionX: '0%' };
  if (now < w.time_ms) return { backgroundPositionX: '100%' };
  const p = Math.max(0, Math.min(1, (now - w.time_ms) / Math.max(1, w.duration_ms)));
  return { backgroundPositionX: ((1 - p) * 100).toFixed(2) + '%' };
}
</script>

<template>
  <span class="lc">
    <span v-if="(active || isPast) && line.words && line.words.length" class="lc-karaoke"><span
        v-for="(w, wi) in line.words"
        :key="wi"
        :class="wordClass(w)"
        :style="wordStyle(w)"
      >{{ w.text }}</span></span>
    <span v-else class="lc-plain">{{ line.text }}</span>

    <span
      v-if="line.romaji"
      class="lc-romaji-wrap"
      :class="{ 'lc-romaji-show': showRomaji }"
    ><span
        class="lc-romaji"
        :class="{ 'lc-romaji-active': active }"
      >{{ line.romaji }}</span></span>
  </span>
</template>

<style scoped>
.lc {
  display: block;
}
.lc-karaoke {
  display: inline;
  white-space: pre-wrap;
}

/* Each word is its own clipped gradient so the active word fills left→right. */
.lc-word {
  background-image: linear-gradient(
    90deg,
    rgba(255, 255, 255, 0.98) 0%,
    rgba(255, 255, 255, 0.98) 50%,
    rgba(255, 255, 255, 0.34) 50%,
    rgba(255, 255, 255, 0.34) 100%
  );
  background-size: 200% 100%;
  background-position-x: 100%;
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
  -webkit-text-fill-color: transparent;
  white-space: pre-wrap;
  transition: background-position 0.12s linear;
  /* Karaoke glyphs are filled by a semi-transparent gradient (unsung = 34%
     opacity). An inherited text-shadow (used by the fullscreen player) would
     bleed through those low-opacity glyphs and darken them, so the fullscreen
     karaoke looked muddier than the shadow-less sidebar. Drop the shadow here so
     both views render the wipe identically. Plain (non-karaoke) lines keep it. */
  text-shadow: none;
}
.lc-word.lc-sung {
  background-position-x: 0%;
}

/* Romanization sub-line. The wrapper is a 1-row grid whose track animates
   between 0fr and 1fr, giving a smooth auto-height expand/collapse (plus fade)
   when the romaji toggle is flipped. */
.lc-romaji-wrap {
  display: grid;
  grid-template-rows: 0fr;
  opacity: 0;
  margin-top: 0;
  transition:
    grid-template-rows 0.34s cubic-bezier(0.4, 0, 0.2, 1),
    opacity 0.26s ease,
    margin-top 0.34s cubic-bezier(0.4, 0, 0.2, 1);
}
.lc-romaji-wrap.lc-romaji-show {
  grid-template-rows: 1fr;
  opacity: 1;
  margin-top: 0.14em;
}

/* The grid item must clip to 0 height while collapsed. */
.lc-romaji {
  overflow: hidden;
  min-height: 0;
  font-size: 0.6em;
  font-weight: 600;
  line-height: 1.3;
  letter-spacing: 0;
  color: rgba(255, 255, 255, 0.3);
  transition: color 0.45s cubic-bezier(0.25, 1, 0.5, 1);
}
.lc-romaji-active {
  color: rgba(255, 255, 255, 0.72);
}
</style>
