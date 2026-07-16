import { effectScope, nextTick } from 'vue';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('./store', async () => {
  const { reactive } = await import('vue');
  return {
    store: reactive({
      libraryReady: false,
      libraryVersion: 0,
      statsVersion: 0,
    }),
  };
});

import { store } from './store';
import { useQuery } from './useLibraryData';

async function flushQuery() {
  await nextTick();
  await Promise.resolve();
  await nextTick();
}

describe('useQuery', () => {
  beforeEach(() => {
    store.libraryReady = false;
    store.libraryVersion = 0;
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
    expect(query.loading.value).toBe(true);
    expect(query.data.value).toEqual(['first']);

    resolveRefresh(['second']);
    await flushQuery();
    expect(query.loading.value).toBe(false);
    expect(query.data.value).toEqual(['second']);
    scope.stop();
  });
});
