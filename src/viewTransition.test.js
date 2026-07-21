import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import {
  setMorphCollectionKey,
  getMorphCollectionKey,
} from './viewTransition';

describe('viewTransition', () => {
  beforeEach(() => {
    setMorphCollectionKey(null);
  });

  it('stores and retrieves morphCollectionKey', () => {
    setMorphCollectionKey('favorites');
    expect(getMorphCollectionKey()).toBe('favorites');
    setMorphCollectionKey(null);
    expect(getMorphCollectionKey()).toBe(null);
  });
});
