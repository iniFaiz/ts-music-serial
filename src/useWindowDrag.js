import { onUnmounted } from 'vue';

const INTERACTIVE_SELECTOR = 'button, a, input, select, textarea, [role="button"]';
const DRAG_THRESHOLD_PX = 3;

// Calling Tauri's startDragging for every mousedown can race a quick mouseup on
// Windows. If the native command lands after the button is released, Windows can
// keep the move loop alive until the next click. Begin the native move only once
// the user has actually moved the pointer with the primary button held down.
export const useWindowDrag = (appWindow, { onDoubleClick } = {}) => {
  let removePendingListeners = () => {};

  const cancelPendingDrag = () => {
    removePendingListeners();
  };

  const beginWindowDrag = (event) => {
    if (event.button !== 0 || event.buttons !== 1) return;
    if (event.target.closest(INTERACTIVE_SELECTOR)) return;

    // `mousedown.detail` is already 2 on the second press. Handling it here
    // avoids beginning a drag before the separate dblclick event is delivered.
    if (event.detail === 2) {
      cancelPendingDrag();
      onDoubleClick?.();
      return;
    }

    cancelPendingDrag();

    const startX = event.clientX;
    const startY = event.clientY;

    const removeListeners = () => {
      document.removeEventListener('mousemove', onMouseMove, true);
      document.removeEventListener('mouseup', cancelPendingDrag, true);
      window.removeEventListener('blur', cancelPendingDrag);
      if (removePendingListeners === removeListeners) removePendingListeners = () => {};
    };

    const onMouseMove = (moveEvent) => {
      // A mouseup that occurred between events must never be allowed to start a
      // native drag operation.
      if (moveEvent.buttons !== 1) {
        removeListeners();
        return;
      }

      const deltaX = moveEvent.clientX - startX;
      const deltaY = moveEvent.clientY - startY;
      if (deltaX * deltaX + deltaY * deltaY < DRAG_THRESHOLD_PX * DRAG_THRESHOLD_PX) {
        return;
      }

      removeListeners();
      appWindow.startDragging().catch(() => {});
    };

    removePendingListeners = removeListeners;
    document.addEventListener('mousemove', onMouseMove, true);
    document.addEventListener('mouseup', cancelPendingDrag, true);
    window.addEventListener('blur', cancelPendingDrag);
  };

  onUnmounted(cancelPendingDrag);

  return { beginWindowDrag, cancelPendingDrag };
};
