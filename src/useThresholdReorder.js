import { onUnmounted, ref } from 'vue';

// Shared click-vs-drag row reordering for list-like UIs (the song list's
// playlist/favorites mode and the sidebar playlist rail).
//
// A press only becomes a drag after the pointer moves `threshold` px
// vertically, so plain clicks still play/navigate. Once a drag commits (or, if
// `markOnActivate`, as soon as it activates), `dragDidReorder` latches briefly
// so the trailing click event can be swallowed instead of double-firing the
// row action.
//
//   container      () -> element that holds the rows
//   rowAttribute   dataset key holding the row index ('plDragIdx' maps to
//                  the `data-pl-drag-idx` DOM attribute)
//   move           (fromIndex, toIndex) => void — called once per completed drag
//   ignoredSelector presses starting inside these elements never begin a drag
export function useThresholdReorder({
  container,
  rowAttribute,
  move,
  threshold = 5,
  ignoredSelector = 'button',
  markOnActivate = false,
}) {
  const dragIndex = ref(-1);
  const overIndex = ref(-1);
  const dragActive = ref(false); // true once threshold exceeded
  const dragDidReorder = ref(false);

  let startY = 0;
  let pendingIndex = -1;
  let clearReorderTimer = null;

  const selector = `[data-${rowAttribute.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}]`;

  const rowIndexAt = (clientY) => {
    const el = container();
    if (!el || typeof el.querySelectorAll !== 'function') return -1;
    const rows = el.querySelectorAll(selector);
    for (const row of rows) {
      const rect = row.getBoundingClientRect();
      if (clientY >= rect.top && clientY <= rect.bottom) {
        return parseInt(row.dataset[rowAttribute], 10);
      }
    }
    if (rows.length > 0) {
      const firstRect = rows[0].getBoundingClientRect();
      if (clientY < firstRect.top) return 0;
      const lastRect = rows[rows.length - 1].getBoundingClientRect();
      if (clientY > lastRect.bottom) return rows.length - 1;
    }
    return -1;
  };

  const detach = () => {
    document.removeEventListener('mousemove', onMouseMove);
    document.removeEventListener('mouseup', onMouseUp);
  };

  const onMouseMove = (e) => {
    if (pendingIndex === -1) return;
    const dy = Math.abs(e.clientY - startY);
    // Activate the drag once the threshold is exceeded.
    if (!dragActive.value && dy >= threshold) {
      dragActive.value = true;
      dragIndex.value = pendingIndex;
      overIndex.value = pendingIndex;
      if (markOnActivate) dragDidReorder.value = true;
      document.body.style.userSelect = 'none';
      document.body.style.cursor = 'grabbing';
    }
    if (dragActive.value) {
      e.preventDefault();
      const idx = rowIndexAt(e.clientY);
      if (idx !== -1) overIndex.value = idx;
    }
  };

  const onMouseUp = () => {
    try {
      if (
        dragActive.value &&
        dragIndex.value !== -1 &&
        overIndex.value !== -1 &&
        dragIndex.value !== overIndex.value
      ) {
        move(dragIndex.value, overIndex.value);
        dragDidReorder.value = true;
      }
    } finally {
      dragIndex.value = -1;
      overIndex.value = -1;
      dragActive.value = false;
      pendingIndex = -1;
      detach();
      document.body.style.userSelect = '';
      document.body.style.cursor = '';
      if (clearReorderTimer) clearTimeout(clearReorderTimer);
      clearReorderTimer = setTimeout(() => {
        dragDidReorder.value = false;
      }, 50);
    }
  };

  const onRowMouseDown = (index, e) => {
    // Don't initiate a drag from interactive elements.
    if (e && e.target && typeof e.target.closest === 'function' && e.target.closest(ignoredSelector)) {
      return;
    }
    pendingIndex = index;
    startY = e ? e.clientY : 0;
    dragDidReorder.value = false;
    document.addEventListener('mousemove', onMouseMove);
    document.addEventListener('mouseup', onMouseUp);
  };

  // Consume the click directly following a completed drag. Returns true when
  // the click must be swallowed.
  const consumeIfDragged = () => {
    if (!dragDidReorder.value) return false;
    dragDidReorder.value = false;
    return true;
  };

  onUnmounted(() => {
    pendingIndex = -1;
    detach();
    document.body.style.userSelect = '';
    document.body.style.cursor = '';
    if (clearReorderTimer) clearTimeout(clearReorderTimer);
  });

  return { dragIndex, overIndex, dragActive, dragDidReorder, onRowMouseDown, consumeIfDragged };
}
