import { computed, nextTick, ref } from 'vue';

// Pure slice math for the windowing engine, split out so unit tests can pin
// the boundary behavior without a DOM.
export function windowBounds({ abovePx, viewportHeight, pitch, total, bufferRows }) {
  const safePitch = pitch > 0 ? pitch : 1;
  const start = Math.max(0, Math.floor(abovePx / safePitch) - bufferRows);
  const visible = Math.ceil(viewportHeight / safePitch) + bufferRows * 2;
  return {
    start,
    end: Math.min(total, start + visible),
  };
}

// Walk up from an element to its nearest scrollable ancestor (overflow-y
// auto/scroll), as used by list views that render inside the app's shared
// scroll container.
export function resolveScrollParent(el) {
  let node = el ? el.parentElement : null;
  while (node) {
    const oy = getComputedStyle(node).overflowY;
    if (oy === 'auto' || oy === 'scroll') return node;
    node = node.parentElement;
  }
  return null;
}

// Shared windowing engine for long lists (SongList rows, QueuePanel rows).
//
// Renders only the visible slice of rows (+buffer) inside a padded wrapper
// whose paddingTop/Bottom preserve scroll geometry, so the scrollbar reflects
// the full list. Row pitch is remeasured from real rows; scroll/resize
// updates are rAF-coalesced. Each row carries its real index in the full list
// (computed by the caller) so targets and highlights stay correct.
export function useVirtualWindow({
  rowsWrapper,
  getScrollContainer,
  rowSelector,
  itemCount,
  enabled,
  initialPitch = 56,
  initialEnd = 60,
  bufferRows = 8,
  fallbackGapPx = 4,
}) {
  const rowPitch = ref(initialPitch);
  const viewStart = ref(0);
  const viewEnd = ref(initialEnd);
  let rafPending = false;
  let scrollEl = null;

  // Pad the wrapper so the rendered slice sits at the correct scroll offset
  // and the scrollbar reflects the full list height (null when not windowing).
  const virtualPadStyle = computed(() => {
    if (!enabled()) return null;
    const total = itemCount();
    const start = Math.max(0, Math.min(viewStart.value, total));
    const end = Math.min(viewEnd.value, total);
    return {
      paddingTop: `${start * rowPitch.value}px`,
      paddingBottom: `${(total - end) * rowPitch.value}px`,
    };
  });

  const measureRowPitch = () => {
    if (!enabled()) return;
    const wrap = rowsWrapper.value;
    if (!wrap || typeof wrap.querySelectorAll !== 'function') return;
    const rows = wrap.querySelectorAll(rowSelector);
    if (rows.length >= 2) {
      const d = rows[1].getBoundingClientRect().top - rows[0].getBoundingClientRect().top;
      if (d > 10) rowPitch.value = d;
    } else if (rows.length === 1) {
      const h = rows[0].getBoundingClientRect().height;
      if (h > 10) rowPitch.value = h + fallbackGapPx; // + list gap between rows
    }
  };

  const updateWindow = () => {
    if (!enabled()) return;
    const wrap = rowsWrapper.value;
    if (!wrap || typeof wrap.getBoundingClientRect !== 'function') return;
    const container = scrollEl || getScrollContainer();
    if (!container) {
      // No scrollable ancestor found — render everything (safe fallback).
      viewStart.value = 0;
      viewEnd.value = itemCount();
      return;
    }
    // When the padded wrapper starts at the container's content origin this
    // difference is simply container.scrollTop; the rect form also works when
    // other content sits above the wrapper.
    const abovePx = Math.max(0, container.getBoundingClientRect().top - wrap.getBoundingClientRect().top);
    const { start, end } = windowBounds({
      abovePx,
      viewportHeight: container.clientHeight,
      pitch: rowPitch.value || initialPitch,
      total: itemCount(),
      bufferRows,
    });
    viewStart.value = start;
    viewEnd.value = end;
  };

  const scheduleWindowUpdate = () => {
    if (rafPending) return;
    rafPending = true;
    requestAnimationFrame(() => {
      rafPending = false;
      measureRowPitch();
      updateWindow();
    });
  };

  // Re-window after content/enabled changes settle into the DOM.
  const refresh = () =>
    nextTick(() => {
      measureRowPitch();
      updateWindow();
    });

  const attach = () => {
    scrollEl = getScrollContainer();
    if (scrollEl) scrollEl.addEventListener('scroll', scheduleWindowUpdate, { passive: true });
    window.addEventListener('resize', scheduleWindowUpdate);
  };

  const detach = () => {
    if (scrollEl) scrollEl.removeEventListener('scroll', scheduleWindowUpdate);
    scrollEl = null;
    window.removeEventListener('resize', scheduleWindowUpdate);
    if (rafPending) {
      cancelAnimationFrame(rafPending);
      rafPending = false;
    }
  };

  return {
    rowPitch,
    viewStart,
    viewEnd,
    virtualPadStyle,
    measureRowPitch,
    updateWindow,
    scheduleWindowUpdate,
    refresh,
    attach,
    detach,
  };
}
