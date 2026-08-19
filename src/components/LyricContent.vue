<script setup>
// Renders the inner content of a single lyric line, shared by the fullscreen
// player, mini player, and lyrics sidebar:
//   • Apple Music Sing style smooth word-by-word karaoke wipe,
//   • Organic swell & gentle ambient glow exclusively on sustained notes (held >= 1000ms),
//   • a smaller romanization (romaji) sub-line beneath, when enabled + present.
const props = defineProps({
  line: { type: Object, required: true },
  active: { type: Boolean, default: false },
  isPast: { type: Boolean, default: false },
  // Playhead position (ms). Parents pass the live value only for the active
  // line (0 otherwise) so non-active lines don't re-render every poll tick.
  currentMs: { type: Number, default: 0 },
  showRomaji: { type: Boolean, default: false },
});

// Threshold for a sustained vocal hold (1 second or longer)
const HELD_THRESHOLD_MS = 1000;

// State of one karaoke word relative to the playhead
function wordClass(w) {
  if (props.isPast && props.currentMs === 0) return 'lc-word lc-sung';
  const now = props.currentMs;
  if (now >= w.time_ms + w.duration_ms) return 'lc-word lc-sung';
  if (now < w.time_ms) return 'lc-word lc-unsung';
  const isHeld = (w.duration_ms || 0) >= HELD_THRESHOLD_MS;
  return isHeld ? 'lc-word lc-active lc-held' : 'lc-word lc-active';
}

// Drives the left→right gradient wipe.
// On sustained / held notes (duration >= 1000ms), applies Apple Music Sing
// subtle organic 3D scale swell and soft ambient glow that smoothly settles back.
function wordStyle(w) {
  if (props.isPast && props.currentMs === 0) return { '--p': '100%' };
  const now = props.currentMs;
  if (now >= w.time_ms + w.duration_ms) return { '--p': '100%' };
  if (now < w.time_ms) return { '--p': '0%' };

  const duration = Math.max(1, w.duration_ms || 0);
  const p = Math.max(0, Math.min(1, (now - w.time_ms) / duration));

  let scaleStyle = {};
  if (duration >= HELD_THRESHOLD_MS) {
    // Dynamic swell for held vocals: gently scales from 1.0 up to ~1.08 - 1.12
    const maxBonus = Math.min(0.12, 0.05 + ((duration - HELD_THRESHOLD_MS) / 15000) * 0.07);
    const scale = 1.0 + maxBonus * Math.sin(p * Math.PI * 0.5);
    scaleStyle = {
      transform: `scale(${scale.toFixed(3)})`,
    };
  }

  return {
    '--p': `${(p * 100).toFixed(2)}%`,
    ...scaleStyle,
  };
}
</script>

<template>
  <span class="lc">
    <span v-if="(active || isPast) && line.words && line.words.length" class="lc-karaoke">
      <span v-for="(w, wi) in line.words" :key="wi" :class="wordClass(w)" :style="wordStyle(w)">{{
        w.text
      }}</span>
    </span>
    <span v-else class="lc-plain">{{ line.text }}</span>

    <!-- Background/harmony vocals: a smaller, dimmer secondary tier (Apple-Music
         style). Word-timed wipe on the active/past line, plain text otherwise. -->
    <span v-if="(active || isPast) && line.bg && line.bg.length" class="lc-bg lc-karaoke">
      <span v-for="(w, wi) in line.bg" :key="wi" :class="wordClass(w)" :style="wordStyle(w)">{{
        w.text
      }}</span>
    </span>
    <span v-else-if="line.bg_text" class="lc-bg lc-bg-plain">{{ line.bg_text }}</span>

    <span v-if="line.romaji" class="lc-romaji-wrap" :class="{ 'lc-romaji-show': showRomaji }"
      ><span class="lc-romaji" :class="{ 'lc-romaji-active': active }">{{
        line.romaji
      }}</span></span
    >
  </span>
</template>

<style scoped>
@property --p {
  syntax: '<percentage>';
  inherits: false;
  initial-value: 0%;
}

.lc {
  display: block;
}
.lc-karaoke {
  display: inline;
  white-space: pre-wrap;
}

/* Each word is an inline-block clipped gradient driven by --p so all words maintain identical
   font antialiasing, weight, and baseline brightness across states. */
.lc-word {
  --p: 0%;
  display: inline-block;
  vertical-align: baseline;
  white-space: pre-wrap;
  position: relative;
  transform-origin: center 80%;
  transform: scale(1);
  transition:
    transform 0.4s cubic-bezier(0.25, 1, 0.5, 1),
    filter 0.4s ease;
  background-image: linear-gradient(
    90deg,
    rgba(255, 255, 255, 0.98) 0%,
    rgba(255, 255, 255, 0.98) var(--p, 0%),
    rgba(255, 255, 255, 0.34) var(--p, 0%),
    rgba(255, 255, 255, 0.34) 100%
  );
  background-size: 100% 100%;
  background-repeat: no-repeat;
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
  -webkit-text-fill-color: transparent;
  text-shadow: none;
}

.lc-word.lc-unsung {
  --p: 0%;
}

.lc-word.lc-sung {
  --p: 100%;
}

.lc-word.lc-active {
  /* No CSS transition on --p: JS interpolates --p smoothly on every animation frame via RAF,
     preventing trailing lag and premature snap-jumps on fast words/syllables. */
}

/* Only when a note is sustained (held >= 1000ms) */
.lc-word.lc-active.lc-held {
  filter: drop-shadow(0 0 10px rgba(255, 255, 255, 0.45));
  z-index: 2;
  transition:
    transform 0.15s ease-out,
    filter 0.3s ease;
}

/* Background/harmony vocal tier — smaller and dimmer than the main line, on its
   own row beneath it. The karaoke variant reuses .lc-word (same gradient wipe);
   the wrapper opacity keeps the whole tier subordinate to the lead vocal. */
.lc-bg {
  display: block;
  font-size: 0.68em;
  font-weight: 600;
  line-height: 1.35;
  letter-spacing: 0;
  margin-top: 0.1em;
  opacity: 0.72;
}
.lc-bg-plain {
  color: rgba(255, 255, 255, 0.55);
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
