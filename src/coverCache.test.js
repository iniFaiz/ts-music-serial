import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke, convertFileSrc } = vi.hoisted(() => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((path) => `asset:${path}`),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke, convertFileSrc }));

import {
  clearCoverCache,
  getCachedCover,
  hasCachedCover,
  loadCover,
  loadCoverDataUrl,
} from './coverCache';

describe('coverCache', () => {
  beforeEach(() => {
    clearCoverCache();
    invoke.mockReset();
    convertFileSrc.mockClear();
  });

  it('retries one transient backend failure and caches the successful URL', async () => {
    invoke
      .mockRejectedValueOnce(new Error('roots are restoring'))
      .mockResolvedValueOnce('C:\\cache\\cover.jpg');

    await expect(loadCover('D:\\Music\\song.flac')).resolves.toBe('asset:C:\\cache\\cover.jpg');
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(hasCachedCover('D:\\Music\\song.flac')).toBe(true);
  });

  it('does not permanently cache a genuine miss', async () => {
    invoke.mockResolvedValueOnce(null).mockResolvedValueOnce('C:\\cache\\late.jpg');

    await expect(loadCover('D:\\Music\\late.flac')).resolves.toBeNull();
    expect(hasCachedCover('D:\\Music\\late.flac')).toBe(false);
    await expect(loadCover('D:\\Music\\late.flac')).resolves.toBe('asset:C:\\cache\\late.jpg');
  });

  it('does not permanently cache command failures', async () => {
    invoke
      .mockRejectedValueOnce(new Error('first attempt'))
      .mockRejectedValueOnce(new Error('retry'))
      .mockResolvedValueOnce('C:\\cache\\recovered.jpg');

    await expect(loadCover('D:\\Music\\recover.flac')).resolves.toBeNull();
    expect(hasCachedCover('D:\\Music\\recover.flac')).toBe(false);
    await expect(loadCover('D:\\Music\\recover.flac')).resolves.toBe(
      'asset:C:\\cache\\recovered.jpg'
    );
  });

  it('caches the data URL fallback after an asset-protocol image error', async () => {
    invoke.mockResolvedValue('data:image/jpeg;base64,abc');

    await expect(loadCoverDataUrl('D:\\Music\\song.flac')).resolves.toBe(
      'data:image/jpeg;base64,abc'
    );
    expect(getCachedCover('D:\\Music\\song.flac')).toBe('data:image/jpeg;base64,abc');
  });
});
