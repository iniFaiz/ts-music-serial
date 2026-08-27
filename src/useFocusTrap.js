import { watch, nextTick, onUnmounted } from 'vue';

const FOCUSABLE_SELECTOR = [
  'button:not([disabled]):not([aria-hidden="true"])',
  'a[href]',
  'input:not([disabled]):not([type="hidden"])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"]):not([disabled])',
].join(', ');

export function getFocusableElements(container) {
  if (!container) return [];
  return Array.from(container.querySelectorAll(FOCUSABLE_SELECTOR)).filter(
    (el) => el.offsetParent !== null || el.getClientRects().length > 0
  );
}

/**
 * useFocusTrap composable
 * Traps keyboard focus within a modal container when active.
 *
 * @param {import('vue').Ref<HTMLElement|null>} containerRef
 * @param {import('vue').Ref<boolean>|(() => boolean)} active
 * @param {Object} [options]
 * @param {() => void} [options.onEscape]
 * @param {import('vue').Ref<HTMLElement|null>|string} [options.initialFocus]
 * @param {boolean} [options.returnFocus=true]
 */
export function useFocusTrap(containerRef, active, options = {}) {
  const { onEscape, initialFocus, returnFocus = true } = options;
  let previousActiveElement = null;

  const handleKeydown = (event) => {
    const container = containerRef.value;
    if (!container) return;

    if (event.key === 'Escape') {
      if (typeof onEscape === 'function') {
        event.preventDefault();
        event.stopPropagation();
        onEscape();
      }
      return;
    }

    if (event.key !== 'Tab') return;

    const focusables = getFocusableElements(container);
    if (focusables.length === 0) {
      event.preventDefault();
      return;
    }

    const first = focusables[0];
    const last = focusables[focusables.length - 1];

    if (event.shiftKey) {
      if (document.activeElement === first || !container.contains(document.activeElement)) {
        event.preventDefault();
        last.focus();
      }
    } else {
      if (document.activeElement === last || !container.contains(document.activeElement)) {
        event.preventDefault();
        first.focus();
      }
    }
  };

  const activate = async () => {
    previousActiveElement = document.activeElement;
    document.addEventListener('keydown', handleKeydown, true);

    await nextTick();
    const container = containerRef.value;
    if (!container) return;

    let targetEl = null;
    if (initialFocus) {
      if (typeof initialFocus === 'string') {
        targetEl = container.querySelector(initialFocus);
      } else if (initialFocus?.value) {
        targetEl = initialFocus.value;
      }
    }

    if (!targetEl) {
      const focusables = getFocusableElements(container);
      targetEl = focusables[0] || container;
    }

    if (targetEl && typeof targetEl.focus === 'function') {
      targetEl.focus();
    }
  };

  const deactivate = () => {
    document.removeEventListener('keydown', handleKeydown, true);
    if (returnFocus && previousActiveElement && typeof previousActiveElement.focus === 'function') {
      try {
        previousActiveElement.focus();
      } catch {
        // Ignored if element is no longer attached
      }
      previousActiveElement = null;
    }
  };

  watch(
    typeof active === 'function' ? active : () => active.value,
    (isActive) => {
      if (isActive) {
        activate();
      } else {
        deactivate();
      }
    },
    { immediate: true }
  );

  onUnmounted(() => {
    deactivate();
  });

  return {
    activate,
    deactivate,
  };
}
