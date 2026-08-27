import { describe, expect, it } from 'vitest';
import { windowBounds } from './useVirtualWindow';

describe('windowBounds', () => {
  const base = { viewportHeight: 600, pitch: 50, total: 10000, bufferRows: 8 };

  it('renders the visible window plus buffers at the top of the list', () => {
    const { start, end } = windowBounds({ ...base, abovePx: 0 });
    expect(start).toBe(0);
    // ceil(600/50) visible + 2*8 buffer
    expect(end).toBe(12 + 16);
  });

  it('keeps the start non-negative when scrolling near the top', () => {
    const { start } = windowBounds({ ...base, abovePx: 100 });
    expect(start).toBe(0);
  });

  it('follows the scroll position with a leading buffer', () => {
    const { start, end } = windowBounds({ ...base, abovePx: 5000 });
    expect(start).toBe(Math.floor(5000 / 50) - 8);
    expect(end).toBe(start + 12 + 16);
  });

  it('clamps end to the total row count near the bottom', () => {
    const { end } = windowBounds({ ...base, abovePx: 499000 });
    expect(end).toBeLessThanOrEqual(base.total);
    expect(end).toBe(base.total);
  });

  it('handles fractional pitches from measured rows', () => {
    const { start, end } = windowBounds({ ...base, abovePx: 1000, pitch: 56.5 });
    expect(start).toBe(Math.floor(1000 / 56.5) - 8);
    expect(end).toBe(start + Math.ceil(600 / 56.5) + 16);
  });

  it('degrades to a safe window instead of dividing by zero', () => {
    const { start, end } = windowBounds({ ...base, abovePx: 0, pitch: 0 });
    // A zero/degenerate pitch falls back to 1px per row and the start stays
    // clamped non-negative.
    expect(start).toBe(0);
    expect(end).toBeLessThanOrEqual(base.total);
    expect(end).toBeGreaterThan(start);
  });

  it('returns an empty window for an empty list', () => {
    const { start, end } = windowBounds({ ...base, abovePx: 200, total: 0 });
    expect(end).toBe(0);
    expect(end).toBeGreaterThanOrEqual(start);
  });
});
