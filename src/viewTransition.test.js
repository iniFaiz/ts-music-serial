import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import {
  setMorphCollectionKey,
  getMorphCollectionKey,
  navigateWithTransition,
} from './viewTransition';

const originalDocument = globalThis.document;
const originalGetComputedStyle = globalThis.getComputedStyle;

const createStyle = (initial = {}) => {
  const values = new Map(Object.entries(initial));
  return {
    getPropertyValue: (name) => values.get(name) || '',
    setProperty: (name, value) => values.set(name, value),
    removeProperty: (name) => values.delete(name),
  };
};

describe('viewTransition', () => {
  beforeEach(() => {
    setMorphCollectionKey(null);
  });

  afterEach(() => {
    if (originalDocument === undefined) {
      delete globalThis.document;
    } else {
      globalThis.document = originalDocument;
    }
    if (originalGetComputedStyle === undefined) {
      delete globalThis.getComputedStyle;
    } else {
      globalThis.getComputedStyle = originalGetComputedStyle;
    }
  });

  it('stores and retrieves morphCollectionKey', () => {
    setMorphCollectionKey('favorites');
    expect(getMorphCollectionKey()).toBe('favorites');
    setMorphCollectionKey(null);
    expect(getMorphCollectionKey()).toBe(null);
  });

  it('tracks the exact source and destination corner radii during a morph', async () => {
    const rootStyle = createStyle();
    const sourceStyle = createStyle();
    const targetStyle = createStyle({ 'view-transition-name': 'shared-cover' });
    const source = {
      style: sourceStyle,
      parentElement: null,
      setAttribute: () => {},
      closest: () => null,
    };
    const target = { style: targetStyle };
    let navigated = false;
    let capturedRadii = null;

    globalThis.getComputedStyle = (el) => ({
      borderTopLeftRadius: el === source ? '4px' : '16px',
      borderRadius: el === source ? '4px' : '16px',
    });
    globalThis.document = {
      documentElement: {
        style: rootStyle,
        classList: { add: () => {}, remove: () => {} },
      },
      querySelectorAll: (selector) => {
        if (selector === '[data-last-clicked]') return [];
        if (selector === '[style*="view-transition-name"]') {
          return navigated ? [source, target] : [source];
        }
        return [];
      },
      startViewTransition: (update) => ({
        finished: (async () => {
          await update();
          capturedRadii = {
            from: rootStyle.getPropertyValue('--shared-cover-radius-from'),
            to: rootStyle.getPropertyValue('--shared-cover-radius-to'),
          };
        })(),
      }),
    };

    await navigateWithTransition(
      async () => {
        navigated = true;
      },
      source,
      'shared-cover',
      'to-album-transition'
    );

    expect(capturedRadii).toEqual({ from: '4px', to: '16px' });
    expect(rootStyle.getPropertyValue('--shared-cover-radius-from')).toBe('');
    expect(rootStyle.getPropertyValue('--shared-cover-radius-to')).toBe('');
  });
});
