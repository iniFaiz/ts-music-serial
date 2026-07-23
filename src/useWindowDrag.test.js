import { describe, expect, it, vi } from 'vitest';

vi.mock('vue', () => ({
  onUnmounted: vi.fn(),
}));

import { useWindowDrag } from './useWindowDrag';

const createEventTarget = () => {
  const listeners = new Map();
  return {
    addEventListener: vi.fn((type, listener) => listeners.set(type, listener)),
    removeEventListener: vi.fn((type, listener) => {
      if (listeners.get(type) === listener) listeners.delete(type);
    }),
    emit(type, event) {
      listeners.get(type)?.(event);
    },
  };
};

const mouseDown = (overrides = {}) => ({
  button: 0,
  buttons: 1,
  clientX: 10,
  clientY: 10,
  detail: 1,
  target: { closest: () => null },
  ...overrides,
});

const mouseMove = (overrides = {}) => ({
  buttons: 1,
  clientX: 14,
  clientY: 10,
  ...overrides,
});

describe('useWindowDrag', () => {
  it('starts dragging only after primary-button movement crosses the threshold', async () => {
    const oldDocument = globalThis.document;
    const oldWindow = globalThis.window;
    const documentTarget = createEventTarget();
    const windowTarget = createEventTarget();
    globalThis.document = documentTarget;
    globalThis.window = windowTarget;

    try {
      const startDragging = vi.fn(() => Promise.resolve(true));
      const { beginWindowDrag } = useWindowDrag({ startDragging });

      beginWindowDrag(mouseDown());
      documentTarget.emit('mousemove', mouseMove({ clientX: 10 }));
      expect(startDragging).not.toHaveBeenCalled();

      documentTarget.emit('mousemove', mouseMove({ clientX: 11 }));
      expect(startDragging).toHaveBeenCalledOnce();
      await Promise.resolve();
    } finally {
      globalThis.document = oldDocument;
      globalThis.window = oldWindow;
    }
  });

  it('cancels a pending drag when mouseup wins the race', () => {
    const oldDocument = globalThis.document;
    const oldWindow = globalThis.window;
    const documentTarget = createEventTarget();
    globalThis.document = documentTarget;
    globalThis.window = createEventTarget();

    try {
      const startDragging = vi.fn(() => Promise.resolve(true));
      const { beginWindowDrag } = useWindowDrag({ startDragging });

      beginWindowDrag(mouseDown());
      documentTarget.emit('mouseup', {});
      documentTarget.emit('mousemove', mouseMove());

      expect(startDragging).not.toHaveBeenCalled();
    } finally {
      globalThis.document = oldDocument;
      globalThis.window = oldWindow;
    }
  });

  it('handles double-click once and never drags later rapid clicks', () => {
    const oldDocument = globalThis.document;
    const oldWindow = globalThis.window;
    const documentTarget = createEventTarget();
    globalThis.document = documentTarget;
    globalThis.window = createEventTarget();

    try {
      const startDragging = vi.fn(() => Promise.resolve(true));
      const onDoubleClick = vi.fn();
      const { beginWindowDrag } = useWindowDrag({ onDoubleClick, startDragging });

      beginWindowDrag(mouseDown({ detail: 2 }));
      beginWindowDrag(mouseDown({ detail: 3 }));
      documentTarget.emit('mousemove', mouseMove());

      expect(onDoubleClick).toHaveBeenCalledOnce();
      expect(startDragging).not.toHaveBeenCalled();
    } finally {
      globalThis.document = oldDocument;
      globalThis.window = oldWindow;
    }
  });
});
