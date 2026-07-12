<script setup>
import { ref, watch, onMounted, onUnmounted, nextTick } from 'vue';

const props = defineProps({
  text: { type: String, required: true },
  speed: { type: Number, default: 30 }, // smooth readable speed (px/sec)
  delay: { type: Number, default: 2500 }, // pause duration at start/end (ms)
  gap: { type: Number, default: 50 }, // spacing between original and duplicate text
  center: { type: Boolean, default: false }, // center text if not overflowing
});

const containerRef = ref(null);
const textRef = ref(null);

const isOverflowing = ref(false);
const isFadedLeft = ref(false);

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

  // 1. Wait at the start (Idle phase)
  timeoutId = setTimeout(() => {
    // 2. Start moving: apply smooth ease-in-out transition and left fade-in
    isFadedLeft.value = true;
    transitionStyle.value = `transform ${duration}s cubic-bezier(0.42, 0, 0.58, 1)`;
    transformStyle.value = `translate3d(-${distance}px, 0, 0)`;

    // 3. Just before reaching the end (0.8s prior), fade out the left edge
    const fadeOutDelay = Math.max(0, duration - 0.8);
    timeoutId = setTimeout(() => {
      isFadedLeft.value = false;

      // 4. Once fully stopped at the end, reset instantly to translation 0
      const remainingTime = duration - fadeOutDelay;
      timeoutId2 = setTimeout(() => {
        runScrollCycle(textWidth, containerWidth);
      }, remainingTime * 1000);

    }, fadeOutDelay * 1000);

  }, props.delay);
};

const measureAndStart = () => {
  cleanupTimeouts();
  isOverflowing.value = false;
  isFadedLeft.value = false;
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
@property --mq-left-mask {
  syntax: '<length>';
  inherits: false;
  initial-value: 0px;
}

.marquee-container {
  position: relative;
  width: 100%;
  --mq-left-mask: 0px;

  /* The right edge is always faded if text overflows, left edge transitions in/out */
  mask-image: linear-gradient(
    to right,
    transparent 0px,
    #000 var(--mq-left-mask),
    #000 calc(100% - 16px),
    transparent 100%
  );
  -webkit-mask-image: linear-gradient(
    to right,
    transparent 0px,
    #000 var(--mq-left-mask),
    #000 calc(100% - 16px),
    transparent 100%
  );

  /* Animates the left fade mask stop smoothly */
  transition: --mq-left-mask 0.8s cubic-bezier(0.25, 1, 0.5, 1);
}

.marquee-container.fade-left-active {
  --mq-left-mask: 16px;
}

.marquee-text-item {
  display: inline-block;
}
</style>
