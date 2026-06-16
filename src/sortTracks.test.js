import { describe, it, expect } from 'vitest';
import { compareTracks, sortTracks } from './sortTracks';

describe('sortTracks', () => {
  it('orders by artist, then album, then track number, then title', () => {
    const input = [
      { artist: 'B', album: 'X', track_number: 2, title: 'b2' },
      { artist: 'A', album: 'Y', track_number: 1, title: 'a-y1' },
      { artist: 'A', album: 'X', track_number: 2, title: 'a-x2' },
      { artist: 'A', album: 'X', track_number: 1, title: 'a-x1' },
    ];
    const out = sortTracks(input).map((t) => t.title);
    expect(out).toEqual(['a-x1', 'a-x2', 'a-y1', 'b2']);
  });

  it('treats missing fields as Unknown/0 without throwing', () => {
    const a = { title: 'song' }; // artist defaults to "Unknown Artist"
    const b = { artist: 'Zeta' };
    expect(compareTracks(a, b)).toBeLessThan(0);
  });

  it('does not mutate the input array', () => {
    const input = [{ artist: 'B' }, { artist: 'A' }];
    const copy = [...input];
    sortTracks(input);
    expect(input).toEqual(copy);
  });
});
