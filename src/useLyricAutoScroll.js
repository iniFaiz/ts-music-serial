import { nextTick, onUnmounted, watch } from 'vue';

// Shared engine behind the three synced-lyrics surfaces (fullscreen player,
// mini player and the side lyrics panel): an eased RAF auto-scroll that keeps
// the active line centred, optionally pausing for ~3s while the user scrolls.
//
// Surfaces differ only in tuning, passed via options:
//   scrollDuration — ms of easing per jump (650 fullscreen / 600 others)
//   gapTargetRem   — rem height used to centre a gap-dots line; must match the
//                    surface's `.lp-line-gap.lp-active` CSS height
//   trackUserScroll— whether manual scrolling suspends auto-follow
export function useLyricAutoScroll({
  container, // () -> scrollable element | null
  lines, // computed: processed lyric lines (see processLyricLines)
  activeIdx, // computed: active line index (-1 when none/unsynced)
  scrollDuration = 600,
  gapTargetRem = 2.2,
  trackUserScroll = true,
}) {
  let rafId = null;
  let isAutoScrolling = false;
  let userPausedUntil = 0;
  let userScrollTimer = null;
  let lastScrolledIdx = -1;

  // easeInOutQuart — slow start, fast middle, slow end
  const easeInOutQuart = (t) =>
    t < 0.5 ? 8 * t * t * t * t : 1 - Math.pow(-2 * t + 2, 4) / 2;

  const smoothScrollTo = (el, targetTop, duration) => {
    if (rafId) cancelAnimationFrame(rafId);
    const start = el.scrollTop;
    const delta = targetTop - start;
    if (Math.abs(delta) < 2) return;

    const t0 = performance.now();
    isAutoScrolling = true;
    const step = (now) => {
      const progress = Math.min((now - t0) / duration, 1);
      el.scrollTop = start + delta * easeInOutQuart(progress);
      if (progress < 1) {
        rafId = requestAnimationFrame(step);
      } else {
        rafId = null;
        // Short grace period so the scroll-end event doesn't flip the flag yet
        setTimeout(() => {
          isAutoScrolling = false;
        }, 80);
      }
    };
    rafId = requestAnimationFrame(step);
  };

  const scrollToLine = async (idx) => {
    const el = container();
    if (!el) return;
    const node = el.querySelector(`[data-line="${idx}"]`);
    if (!node) return;

    const currentLine = lines.value[idx];

    let targetTop = node.offsetTop;
    let targetH = node.offsetHeight;

    if (currentLine && currentLine.isGap) {
      const rem = parseFloat(getComputedStyle(document.documentElement).fontSize) || 16;
      targetH = gapTargetRem * rem;
    } else if (idx > 0) {
      // Collapsed gap rows above the active line carry stale offset heights —
      // subtract them so the target lands where the layout will settle.
      for (let k = 0; k < idx; k++) {
        if (lines.value[k] && lines.value[k].isGap) {
          const gapEl = el.querySelector(`[data-line="${k}"]`);
          if (gapEl) {
            const gapH = gapEl.offsetHeight;
            if (gapH > 0) {
              const mb = parseFloat(window.getComputedStyle(gapEl).marginBottom) || 0;
              targetTop -= gapH + mb;
            }
          }
        }
      }
    }

    // Center the active line vertically inside the scroll area
    const target = Math.max(0, targetTop - el.clientHeight / 2 + targetH / 2);
    smoothScrollTo(el, target, scrollDuration);
  };

  const followActive = async (idx) => {
    if (idx < 0 || idx === lastScrolledIdx) return;
    if (Date.now() < userPausedUntil) return; // user is in control
    lastScrolledIdx = idx;

    await nextTick();
    scrollToLine(idx);
  };

  const resetScrollState = () => {
    lastScrolledIdx = -1;
    userPausedUntil = 0;
  };

  const onUserScroll = trackUserScroll
    ? () => {
        if (isAutoScrolling) return;
        userPausedUntil = Date.now() + 3000;
        if (userScrollTimer) clearTimeout(userScrollTimer);
        userScrollTimer = setTimeout(() => {
          userPausedUntil = 0;
        }, 3100);
      }
    : undefined;

  // Follow the active line whenever playback advances it.
  watch(activeIdx, followActive);

  onUnmounted(() => {
    if (rafId) cancelAnimationFrame(rafId);
    if (userScrollTimer) clearTimeout(userScrollTimer);
  });

  return { scrollToLine, resetScrollState, onUserScroll };
}
