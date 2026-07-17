import { describe, expect, it } from 'vitest';
import { createKeySequenceMatcher } from './nyancatEasterEgg';

describe('createKeySequenceMatcher', () => {
  it('matches nyancat after it is typed consecutively', () => {
    const matcher = createKeySequenceMatcher('nyancat');
    const matches = [...'nyancat'].map((key) => matcher.push(key));

    expect(matches).toEqual([false, false, false, false, false, false, true]);
  });

  it('is case-insensitive and can find the sequence after unrelated text', () => {
    const matcher = createKeySequenceMatcher('nyancat');
    const matches = [...'helloNYANCAT'].map((key) => matcher.push(key));

    expect(matches.at(-1)).toBe(true);
  });

  it('resets a partial sequence after a non-printable key', () => {
    const matcher = createKeySequenceMatcher('nyancat');
    [...'nyan'].forEach((key) => matcher.push(key));
    matcher.push('ArrowLeft');

    expect([...'cat'].some((key) => matcher.push(key))).toBe(false);
  });

  it('can match again so the easter egg can be toggled off', () => {
    const matcher = createKeySequenceMatcher('nyancat');
    const typeSequence = () => [...'nyancat'].map((key) => matcher.push(key)).at(-1);

    expect(typeSequence()).toBe(true);
    expect(typeSequence()).toBe(true);
  });
});
