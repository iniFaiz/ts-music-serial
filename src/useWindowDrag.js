import { onUnmounted } from 'vue';
import { invokeCommand as invoke } from './generated/ipc';

const INTERACTIVE_SELECTOR = 'button, a, input, select, textarea, [role="button"]';
// One pixel filters a stationary click without adding a noticeable drag dead-zone.
const DRAG_THRESHOLD_PX = 1;

// Starting a native drag is asynchronous from the webview's point of view. The
// backend command performs the final physical-button check on Windows so a
// delayed IPC request cannot enter the native move loop after mouseup.
const startNativeWindowDrag = () => invoke('start_window_drag');

export const useWindowDrag = ({ onDoubleClick, startDragging = startNativeWindowDrag } = {}) => {
  let removePendingListeners = () => {};

  const cancelPendingDrag = () => {
    removePendingListeners();
  };

  const beginWindowDrag = (event) => {
    if (event.button !== 0 || event.buttons !== 1) return;
    if (event.target.closest(INTERACTIVE_SELECTOR)) return;

    // `mousedown.detail` is already 2 on the second press and keeps increasing
    // for rapid subsequent clicks. None of those presses should begin a drag.
    if (event.detail > 1) {
      cancelPendingDrag();
      if (event.detail === 2) onDoubleClick?.();
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
      startDragging().catch(() => {});
    };

    removePendingListeners = removeListeners;
    document.addEventListener('mousemove', onMouseMove, true);
    document.addEventListener('mouseup', cancelPendingDrag, true);
    window.addEventListener('blur', cancelPendingDrag);
  };

  onUnmounted(cancelPendingDrag);

  return { beginWindowDrag, cancelPendingDrag };
};
