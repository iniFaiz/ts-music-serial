<script setup>
import { ref, watch, onMounted, onUnmounted, nextTick } from 'vue';

const props = defineProps({
  text: { type: String, required: true },
  speed: { type: Number, default: 30 }, // smooth readable speed (px/sec)
  gap: { type: Number, default: 50 }, // spacing between original and duplicate text
  center: { type: Boolean, default: false }, // center text if not overflowing
});

const containerRef = ref(null);
const textRef = ref(null);

const isOverflowing = ref(false);
const isFadedLeft = ref(false);
const isFirstScroll = ref(true); // track new song plays to use 2.5s initial delay

const transformStyle = ref('translate3d(0, 0, 0)');
const transitionStyle = ref('none');

let timeoutId = null;
let timeoutId2 = null;

const cleanupTimeouts = () => {
  if (timeoutId) clearTimeout(timeoutId);
  if (timeoutId2) clearTimeout(timeoutId2);
};

const runScrollCycle = (textWidth, containerWidth) => {
  cleanupTimeouts();

  // Reset to start state (Idle at 0 translation)
  isFadedLeft.value = false;
  transformStyle.value = 'translate3d(0, 0, 0)';
  transitionStyle.value = 'none';

  const distance = textWidth + props.gap;
  const duration = distance / props.speed;

  // Initial scroll delay is 2.5s to let user read title, subsequent loops are 7s to avoid spam
  const currentDelay = isFirstScroll.value ? 2500 : 7000;

  // 1. Wait at the start (Idle phase)
  timeoutId = setTimeout(() => {
    isFirstScroll.value = false; // mark initial scroll done

    // 2. Start moving: apply smooth ease-in-out transition and left fade-in
    isFadedLeft.value = true;
    transitionStyle.value = `transform ${duration}s cubic-bezier(0.42, 0, 0.58, 1)`;
    transformStyle.value = `translate3d(-${distance}px, 0, 0)`;

    // 3. Fade out the left edge *before* the start of the duplicate text reaches it.
    // The duplicate text enters the left edge during the last gap-travel duration.
    const gapTime = props.gap / props.speed;
    const fadeOutDelay = Math.max(0, duration - Math.max(0.8, gapTime));
    
    timeoutId = setTimeout(() => {
      isFadedLeft.value = false;

      // 4. Once fully stopped at the end, reset instantly to translation 0
      const remainingTime = duration - fadeOutDelay;
      timeoutId2 = setTimeout(() => {
        runScrollCycle(textWidth, containerWidth);
      }, remainingTime * 1000);

    }, fadeOutDelay * 1000);

  }, currentDelay);
};

const measureAndStart = () => {
  cleanupTimeouts();
  isOverflowing.value = false;
  isFadedLeft.value = false;
  isFirstScroll.value = true; // reset on song change
  transformStyle.value = 'translate3d(0, 0, 0)';
  transitionStyle.value = 'none';

  nextTick(() => {
    if (!containerRef.value || !textRef.value) return;

    const containerWidth = containerRef.value.clientWidth;
    const textWidth = textRef.value.scrollWidth;

    if (textWidth > containerWidth) {
      isOverflowing.value = true;
      runScrollCycle(textWidth, containerWidth);
    }
  });
};

watch(
  () => props.text,
  () => {
    measureAndStart();
  }
);

let resizeObserver = null;
onMounted(() => {
  measureAndStart();
  if (typeof window !== 'undefined' && containerRef.value) {
    resizeObserver = new ResizeObserver(() => {
      measureAndStart();
    });
    resizeObserver.observe(containerRef.value);
  }
});

onUnmounted(() => {
  cleanupTimeouts();
  if (resizeObserver) resizeObserver.disconnect();
});
</script>

<template>
  <div
    ref="containerRef"
    class="marquee-container overflow-hidden w-full relative"
    :class="{ 'fade-left-active': isFadedLeft }"
  >
    <div
      class="marquee-content flex whitespace-nowrap"
      :class="{
        'mx-auto justify-center': center && !isOverflowing,
        'justify-start': !center && !isOverflowing,
        'w-max': isOverflowing,
      }"
      :style="{ transform: transformStyle, transition: transitionStyle }"
    >
      <span ref="textRef" class="marquee-text-item select-none shrink-0">{{ text }}</span>
      <template v-if="isOverflowing">
        <span :style="{ width: gap + 'px' }" class="shrink-0"></span>
        <span class="marquee-text-item select-none shrink-0">{{ text }}</span>
      </template>
    </div>
  </div>
</template>

<style scoped>
@property --mq-left-start {
  syntax: '<length>';
  inherits: false;
  initial-value: -16px;
}

@property --mq-left-mask {
  syntax: '<length>';
  inherits: false;
  initial-value: -16px;
}

.marquee-container {
  position: relative;
  width: 100%;
  --mq-left-start: -16px;
  --mq-left-mask: -16px;

  /* Shift the left mask off-screen to negative coordinates (-16px) when inactive.
     This mathematically guarantees absolutely zero subpixel anti-aliasing transparency 
     leak at the 0px edge when the text is stationary. */
  mask-image: linear-gradient(
    to right,
    transparent var(--mq-left-start),
    #000 var(--mq-left-mask),
    #000 calc(100% - 16px),
    transparent 100%
  );
  -webkit-mask-image: linear-gradient(
    to right,
    transparent var(--mq-left-start),
    #000 var(--mq-left-mask),
    #000 calc(100% - 16px),
    transparent 100%
  );

  /* Animates the left fade mask stop smoothly */
  transition:
    --mq-left-start 0.8s cubic-bezier(0.25, 1, 0.5, 1),
    --mq-left-mask 0.8s cubic-bezier(0.25, 1, 0.5, 1);
}

.marquee-container.fade-left-active {
  --mq-left-start: 0px;
  --mq-left-mask: 16px;
}

.marquee-text-item {
  display: inline-block;
}
</style>
