import { describe, expect, it } from 'vitest';
import {
  KONAMI_CODE,
  TONEARM_INNER_ANGLE,
  TONEARM_OUTER_ANGLE,
  createCodeSequenceMatcher,
  shortestAngleDelta,
  timeFromScratchRotation,
  tonearmAngleFromProgress,
  tonearmProgressFromAngle,
} from './vinylScratch';

describe('vinyl scratch easter egg helpers', () => {
  it('recognizes the complete Konami code and resets after a match', () => {
    const matcher = createCodeSequenceMatcher(KONAMI_CODE);
    const matches = KONAMI_CODE.map((code) => matcher.push(code));

    expect(matches.slice(0, -1).every((matched) => !matched)).toBe(true);
    expect(matches.at(-1)).toBe(true);
    expect(matcher.push('KeyA')).toBe(false);
  });

  it('recovers when a mismatch is also the first key of the sequence', () => {
    const matcher = createCodeSequenceMatcher(['ArrowUp', 'ArrowDown']);

    matcher.push('ArrowUp');
    expect(matcher.push('ArrowUp')).toBe(false);
    expect(matcher.push('ArrowDown')).toBe(true);
  });

  it('keeps angular movement continuous across the angle boundary', () => {
    expect(shortestAngleDelta(179, -179)).toBe(2);
    expect(shortestAngleDelta(-179, 179)).toBe(-2);
  });

  it('maps clockwise and counter-clockwise turns to clamped track time', () => {
    expect(timeFromScratchRotation(30, 180, 120, 6)).toBe(33);
    expect(timeFromScratchRotation(2, -360, 120, 6)).toBe(0);
    expect(timeFromScratchRotation(119, 360, 120, 6)).toBe(120);
  });

  it('maps tonearm placement to track progress in both directions', () => {
    expect(tonearmProgressFromAngle(TONEARM_OUTER_ANGLE)).toBe(0);
    expect(tonearmProgressFromAngle(TONEARM_INNER_ANGLE)).toBe(1);
    expect(tonearmProgressFromAngle(100)).toBe(1);
    expect(tonearmAngleFromProgress(0.5)).toBe((TONEARM_OUTER_ANGLE + TONEARM_INNER_ANGLE) / 2);
  });
});
