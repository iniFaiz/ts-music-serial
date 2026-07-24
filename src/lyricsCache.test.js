import { describe, expect, it, vi } from 'vitest';

vi.mock('./store', async () => {
  const { reactive } = await import('vue');
  return {
    store: reactive({
      lyricsSource: 'netease',
    }),
  };
});

import { processLyricLines, activeLineIndex } from './lyricsCache';

describe('processLyricLines', () => {
  it('returns raw lines as-is when not synced or empty', () => {
    expect(processLyricLines([], true)).toEqual([]);
    const unsynced = [{ text: 'Line 1' }];
    expect(processLyricLines(unsynced, false)).toEqual(unsynced);
  });

  it('adds an intro gap if first line starts after 6000ms', () => {
    const raw = [{ time_ms: 12000, text: 'First line' }];
    const res = processLyricLines(raw, true);
    expect(res).toHaveLength(2);
    expect(res[0]).toEqual({
      isGap: true,
      time_ms: 2000,
      endTimeMs: 11000,
      text: '• • •',
    });
    expect(res[1].text).toBe('First line');
  });

  it('automatically inserts a gap line when distance between consecutive lines is > 10 seconds (10000ms)', () => {
    const raw = [
      { time_ms: 3000, text: 'First line' },
      { time_ms: 21000, text: 'Second line after 18s interlude' },
    ];
    const res = processLyricLines(raw, true);
    expect(res).toHaveLength(3);
    expect(res[0].text).toBe('First line');
    expect(res[1]).toEqual({
      isGap: true,
      time_ms: 6500,
      endTimeMs: 20000,
      text: '• • •',
    });
    expect(res[2].text).toBe('Second line after 18s interlude');
  });

  it('does NOT insert a gap line when distance between consecutive lines is <= 10 seconds', () => {
    const raw = [
      { time_ms: 3000, text: 'First line' },
      { time_ms: 11000, text: 'Second line 8s later' },
    ];
    const res = processLyricLines(raw, true);
    expect(res).toHaveLength(2);
    expect(res[0].text).toBe('First line');
    expect(res[1].text).toBe('Second line 8s later');
  });

  it('does NOT insert an outro gap line at the end of the song', () => {
    const raw = [{ time_ms: 3000, text: 'Last line' }];
    const res = processLyricLines(raw, true, 30000);
    expect(res).toHaveLength(1);
    expect(res[0].text).toBe('Last line');
  });
});

describe('activeLineIndex', () => {
  it('turns off (-1) after the last plain lyric line finishes', () => {
    const lines = [
      { time_ms: 10000, text: 'Line 1' },
      { time_ms: 15000, text: 'Line 2' },
      { time_ms: 20000, text: 'Last line' },
    ];
    // avg duration = 5000ms. Last line starts at 20000ms -> ends around 20000 + 5000 + 1500 = 26500ms
    expect(activeLineIndex(lines, 21000, 60000)).toBe(2);
    expect(activeLineIndex(lines, 27000, 60000)).toBe(-1);
  });
});
