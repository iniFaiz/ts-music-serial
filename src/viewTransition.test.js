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

  it('morphs back to the exact clicked song row when an album has multiple artists', async () => {
    const rootStyle = createStyle();

    // Create DOM structure representing AlbumDetail with multiple artists:
    // Header (Artist A)
    // Song 1 (Artist A)
    // Song 2 (Artist A)
    // Song 3 (Artist A)
    // Song 4 (Artist B - 1st song)
    // Song 5 (Artist B - 2nd song)
    // Song 6 (Artist B - 3rd song)
    const headerCoverImg = { style: createStyle(), dataset: {} };
    const headerEl = {
      style: createStyle(),
      dataset: { coverKey: 'Multi Album', albumKey: 'Multi Album', artistKey: 'Artist A' },
      querySelector: (sel) => (sel === '.cover-image' ? headerCoverImg : null),
    };

    let taggedElement = null;
    const createSongRow = (path, artist) => {
      const coverImg = { style: createStyle(), dataset: {} };
      const origSetProperty = coverImg.style.setProperty;
      coverImg.style.setProperty = (name, val) => {
        if (name === 'view-transition-name' && val === 'shared-cover') {
          taggedElement = coverImg;
        }
        origSetProperty(name, val);
      };
      const row = {
        style: createStyle(),
        dataset: { songPath: path, artistKey: artist, albumKey: 'Multi Album' },
        classList: { contains: (c) => c === 'song-row' },
        querySelector: (sel) => (sel === '.cover-image' ? coverImg : null),
      };
      row.closest = (sel) => {
        if (sel.includes('song-row') || sel.includes('data-song-path')) return row;
        return null;
      };
      coverImg.closest = row.closest;
      coverImg.setAttribute = (attr, val) => {
        coverImg.dataset[attr] = val;
      };
      return { row, coverImg };
    };

    const song1 = createSongRow('/music/song1.mp3', 'Artist A');
    const song2 = createSongRow('/music/song2.mp3', 'Artist A');
    const song3 = createSongRow('/music/song3.mp3', 'Artist A');
    const song4 = createSongRow('/music/song4.mp3', 'Artist B');
    const song5 = createSongRow('/music/song5.mp3', 'Artist B');
    const song6 = createSongRow('/music/song6.mp3', 'Artist B');

    const allRows = [song1.row, song2.row, song3.row, song4.row, song5.row, song6.row];
    const allCovers = [
      headerEl,
      song1.row,
      song2.row,
      song3.row,
      song4.row,
      song5.row,
      song6.row,
    ];

    globalThis.getComputedStyle = () => ({
      borderTopLeftRadius: '4px',
      borderRadius: '4px',
    });

    globalThis.document = {
      documentElement: {
        style: rootStyle,
        classList: { add: () => {}, remove: () => {} },
      },
      querySelector: (selector) => {
        if (selector === '[data-last-clicked="true"]') return null; // Component remounted after route change
        return null;
      },
      querySelectorAll: (selector) => {
        if (selector === '[data-last-clicked]') return [];
        if (selector.includes('data-song-path')) return allRows;
        if (selector.includes('data-cover-key') || selector.includes('data-artist-key')) {
          return allCovers;
        }
        if (selector.includes('view-transition-name')) return [];
        return [];
      },
      startViewTransition: (update) => ({
        finished: (async () => {
          await update();
        })(),
      }),
    };

    globalThis.window = {
      history: {
        state: { back: '/albums/Multi%20Album' },
      },
    };

    // Step 1: User clicks on Song 6 (Artist B, song 3) in AlbumDetail
    let currentRouteValue = { name: 'AlbumDetail', params: { name: 'Multi Album' } };
    await navigateWithTransition(
      async () => {
        currentRouteValue = { name: 'ArtistDetail', params: { name: 'Artist B' } };
      },
      song6.coverImg,
      'shared-cover',
      'to-artist-transition'
    );

    // Step 2: User clicks Back from ArtistDetail to AlbumDetail
    let afterEachCallback = null;
    const mockRouter = {
      currentRoute: {
        get value() {
          return currentRouteValue;
        },
      },
      resolve: (_path) => ({ name: 'AlbumDetail', params: { name: 'Multi Album' } }),
      back: () => {
        currentRouteValue = { name: 'AlbumDetail', params: { name: 'Multi Album' } };
        if (afterEachCallback) afterEachCallback();
      },
      afterEach: (cb) => {
        afterEachCallback = cb;
        return () => {};
      },
    };

    const { goBackWithTransition } = await import('./viewTransition');
    await goBackWithTransition(mockRouter);

    // Verify that the morph transition tagged Song 6 (Artist B, Song 3)
    // and NOT Song 4 (Artist B, Song 1)
    expect(taggedElement).toBe(song6.coverImg);
    expect(song6.coverImg.style.getPropertyValue('view-transition-name')).toBe(''); // Restored after finish
    expect(song4.coverImg.style.getPropertyValue('view-transition-name')).toBe('');
  });
});
