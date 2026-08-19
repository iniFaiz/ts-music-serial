import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ref, nextTick } from 'vue';

const { invoke } = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke }));

vi.mock('./store', async () => {
  const { reactive } = await import('vue');
  return {
    store: reactive({
      lyricsSource: 'netease',
      duration: 180,
    }),
  };
});

import { clearLyricsCache } from './lyricsCache';
import { useTrackLyrics } from './useTrackLyrics';

const flushPromises = () => new Promise((resolve) => setTimeout(resolve, 10));

describe('useTrackLyrics', () => {
  beforeEach(() => {
    clearLyricsCache();
    invoke.mockReset();
  });

  it('handles slow request followed by quick next track correctly (no race condition)', async () => {
    let resolveSong1;
    const song1Promise = new Promise((resolve) => {
      resolveSong1 = resolve;
    });

    let resolveSong2;
    const song2Promise = new Promise((resolve) => {
      resolveSong2 = resolve;
    });

    invoke.mockImplementation((cmd, args) => {
      if (args.path === 'song1.mp3') return song1Promise;
      if (args.path === 'song2.mp3') return song2Promise;
      return Promise.resolve(null);
    });

    const currentSong = ref({ path: 'song1.mp3', title: 'Song 1', artist: 'Artist 1' });
    const active = ref(true);
    const source = ref('netease');

    const { lyrics, lyricsLoading } = useTrackLyrics({
      song: () => currentSong.value,
      active: () => active.value,
      source: () => source.value,
    });

    await nextTick();
    expect(lyricsLoading.value).toBe(true);
    expect(lyrics.value).toBeUndefined();

    // Next song while Song 1 is still loading
    currentSong.value = { path: 'song2.mp3', title: 'Song 2', artist: 'Artist 2' };
    await nextTick();
    expect(lyricsLoading.value).toBe(true);
    expect(lyrics.value).toBeUndefined();

    // Song 2 resolves first
    const song2Lyrics = {
      synced: true,
      source: 'netease',
      lines: [{ time_ms: 1000, text: 'Song 2 line' }],
    };
    resolveSong2(song2Lyrics);
    await flushPromises();

    expect(lyricsLoading.value).toBe(false);
    expect(lyrics.value).toEqual(song2Lyrics);

    // Song 1 resolves late (slow internet)
    const song1Lyrics = {
      synced: true,
      source: 'netease',
      lines: [{ time_ms: 1000, text: 'Song 1 line' }],
    };
    resolveSong1(song1Lyrics);
    await flushPromises();

    // Lyrics should NOT be overwritten by Song 1
    expect(lyrics.value).toEqual(song2Lyrics);
    expect(lyricsLoading.value).toBe(false);
  });

  it('updates lyrics when switching to a song whose lyrics are already cached', async () => {
    const song1Lyrics = {
      synced: true,
      source: 'netease',
      lines: [{ time_ms: 1000, text: 'Song 1' }],
    };
    invoke.mockResolvedValueOnce(song1Lyrics);

    const currentSong = ref({ path: 'song1.mp3', title: 'Song 1', artist: 'Artist 1' });
    const active = ref(true);
    const source = ref('netease');

    const { lyrics, lyricsLoading } = useTrackLyrics({
      song: () => currentSong.value,
      active: () => active.value,
      source: () => source.value,
    });

    await flushPromises();
    expect(lyrics.value).toEqual(song1Lyrics);

    // Song 2 loads
    const song2Lyrics = {
      synced: true,
      source: 'netease',
      lines: [{ time_ms: 2000, text: 'Song 2' }],
    };
    invoke.mockResolvedValueOnce(song2Lyrics);
    currentSong.value = { path: 'song2.mp3', title: 'Song 2', artist: 'Artist 2' };
    await flushPromises();
    expect(lyrics.value).toEqual(song2Lyrics);

    // Switch back to Song 1 (which is cached)
    currentSong.value = { path: 'song1.mp3', title: 'Song 1', artist: 'Artist 1' };
    await flushPromises();
    expect(lyrics.value).toEqual(song1Lyrics);
    expect(lyricsLoading.value).toBe(false);
  });

  it('handles rapid skipping across multiple songs resolving out of order', async () => {
    const resolvers = {};
    invoke.mockImplementation((cmd, args) => {
      return new Promise((resolve) => {
        resolvers[args.path] = resolve;
      });
    });

    const currentSong = ref({ path: 'song1.mp3', title: 'Song 1', artist: 'Artist 1' });
    const active = ref(true);
    const source = ref('netease');

    const { lyrics, lyricsLoading } = useTrackLyrics({
      song: () => currentSong.value,
      active: () => active.value,
      source: () => source.value,
    });

    await nextTick();
    expect(lyricsLoading.value).toBe(true);

    // Skip to Song 2
    currentSong.value = { path: 'song2.mp3', title: 'Song 2', artist: 'Artist 2' };
    await nextTick();

    // Skip to Song 3
    currentSong.value = { path: 'song3.mp3', title: 'Song 3', artist: 'Artist 3' };
    await nextTick();

    // Song 2 resolves
    resolvers['song2.mp3']({ synced: true, source: 'netease', lines: [{ text: 'Song 2' }] });
    await flushPromises();
    // Song 2 should NOT be shown because current is Song 3
    expect(lyrics.value).toBeUndefined();
    expect(lyricsLoading.value).toBe(true);

    // Song 1 resolves
    resolvers['song1.mp3']({ synced: true, source: 'netease', lines: [{ text: 'Song 1' }] });
    await flushPromises();
    // Song 1 should NOT be shown because current is Song 3
    expect(lyrics.value).toBeUndefined();
    expect(lyricsLoading.value).toBe(true);

    // Song 3 resolves
    const song3Lyrics = { synced: true, source: 'netease', lines: [{ text: 'Song 3' }] };
    resolvers['song3.mp3'](song3Lyrics);
    await flushPromises();
    expect(lyrics.value).toEqual(song3Lyrics);
    expect(lyricsLoading.value).toBe(false);
  });

  it('handles active toggle and retry race conditions', async () => {
    let resolveFirst;
    let resolveSecond;
    let callCount = 0;

    invoke.mockImplementation(() => {
      callCount++;
      if (callCount === 1) {
        return new Promise((resolve) => {
          resolveFirst = resolve;
        });
      }
      return new Promise((resolve) => {
        resolveSecond = resolve;
      });
    });

    const currentSong = ref({ path: 'song1.mp3', title: 'Song 1', artist: 'Artist 1' });
    const active = ref(true);
    const source = ref('netease');

    const { lyrics, lyricsLoading, fetchLyrics } = useTrackLyrics({
      song: () => currentSong.value,
      active: () => active.value,
      source: () => source.value,
    });

    await nextTick();
    expect(lyricsLoading.value).toBe(true);

    // User triggers manual retry (force = true) while first is still loading
    fetchLyrics(true);
    await nextTick();

    // First (stale) request resolves
    resolveFirst({ synced: false, lines: [{ text: 'Stale' }] });
    await flushPromises();
    expect(lyrics.value).toBeUndefined();
    expect(lyricsLoading.value).toBe(true);

    // Second (forced) request resolves
    const freshLyrics = { synced: true, lines: [{ text: 'Fresh' }] };
    resolveSecond(freshLyrics);
    await flushPromises();
    expect(lyrics.value).toEqual(freshLyrics);
    expect(lyricsLoading.value).toBe(false);
  });
});
