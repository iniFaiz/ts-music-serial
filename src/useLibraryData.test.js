import { effectScope, nextTick } from 'vue';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('./store', async () => {
  const { reactive } = await import('vue');
  return {
    store: reactive({
      libraryReady: false,
      libraryVersion: 0,
      favoritesVersion: 0,
      playlistsVersion: 0,
      statsVersion: 0,
    }),
  };
});

import { store } from './store';
import { useQuery } from './useLibraryData';

async function flushQuery() {
  await nextTick();
  for (let i = 0; i < 5; i++) await Promise.resolve();
  await nextTick();
}

describe('useQuery', () => {
  beforeEach(() => {
    store.libraryReady = false;
    store.libraryVersion = 0;
    store.favoritesVersion = 0;
    store.playlistsVersion = 0;
    store.statsVersion = 0;
  });

  it('does not expose a false empty result before the library is ready', async () => {
    const fetcher = vi.fn().mockResolvedValue(['track']);
    const scope = effectScope();
    const query = scope.run(() => useQuery(fetcher, { initial: [] }));

    await flushQuery();
    expect(fetcher).not.toHaveBeenCalled();
    expect(query.loading.value).toBe(true);
    expect(query.data.value).toEqual([]);

    store.libraryReady = true;
    store.libraryVersion++;
    await flushQuery();

    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(query.loading.value).toBe(false);
    expect(query.data.value).toEqual(['track']);
    scope.stop();
  });

  it('keeps the last successful data visible while refreshing', async () => {
    store.libraryReady = true;
    let resolveRefresh;
    const fetcher = vi
      .fn()
      .mockResolvedValueOnce(['first'])
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveRefresh = resolve;
          })
      );
    const scope = effectScope();
    const query = scope.run(() => useQuery(fetcher, { initial: [] }));

    await flushQuery();
    expect(query.data.value).toEqual(['first']);

    store.libraryVersion++;
    await nextTick();
    expect(query.loading.value).toBe(false);
    expect(query.data.value).toEqual(['first']);

    resolveRefresh(['second']);
    await flushQuery();
    expect(query.loading.value).toBe(false);
    expect(query.data.value).toEqual(['second']);
    scope.stop();
  });

  it('reuses a successful cached result across component instances', async () => {
    store.libraryReady = true;
    const firstFetcher = vi.fn().mockResolvedValue(['cached']);
    const firstScope = effectScope();
    const first = firstScope.run(() =>
      useQuery(firstFetcher, { initial: [], cacheKey: 'test:shared' })
    );

    await vi.waitFor(() => expect(first.data.value).toEqual(['cached']));
    expect(firstFetcher).toHaveBeenCalledTimes(1);
    firstScope.stop();

    const secondFetcher = vi.fn().mockResolvedValue(['unexpected']);
    const secondScope = effectScope();
    const second = secondScope.run(() =>
      useQuery(secondFetcher, { initial: [], cacheKey: 'test:shared' })
    );

    expect(second.data.value).toEqual(['cached']);
    expect(second.loading.value).toBe(false);
    await flushQuery();
    expect(secondFetcher).not.toHaveBeenCalled();
    secondScope.stop();
  });

  it('reloads only when its scoped dependency changes', async () => {
    store.libraryReady = true;
    const fetcher = vi.fn().mockResolvedValue(['favorite']);
    const scope = effectScope();
    const query = scope.run(() =>
      useQuery(fetcher, {
        initial: [],
        cacheKey: 'test:favorites-scope',
        deps: [() => store.favoritesVersion],
      })
    );

    await vi.waitFor(() => expect(fetcher).toHaveBeenCalledTimes(1));
    store.playlistsVersion++;
    await nextTick();
    expect(fetcher).toHaveBeenCalledTimes(1);

    store.favoritesVersion++;
    await nextTick();
    expect(query.loading.value).toBe(false);
    await vi.waitFor(() => expect(fetcher).toHaveBeenCalledTimes(2));
    scope.stop();
  });
});
