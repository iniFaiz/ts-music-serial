import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { invokeMock, consentMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  consentMock: vi.fn(),
}));

vi.mock('./generated/ipc', () => ({
  invokeCommand: invokeMock,
}));

vi.mock('./destructiveConsent', () => ({
  requestDestructiveConsent: consentMock,
}));

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({}),
  LogicalSize: class LogicalSize {},
}));

import { createStoreActions, createStoreState } from './store/modules';

const createTestStore = () => {
  const target = createStoreState();
  Object.defineProperties(target, Object.getOwnPropertyDescriptors(createStoreActions()));
  return target;
};

describe('database-backed optimistic mutations', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    invokeMock.mockReset();
    consentMock.mockReset();
    consentMock.mockResolvedValue('consent-token');
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('rolls back and reloads favorites when toggle persistence fails', async () => {
    const store = createTestStore();
    store.favorites = ['D:\\Music\\one.flac'];
    invokeMock.mockImplementation(async (command) => {
      if (command === 'db_toggle_favorite') throw new Error('database locked');
      if (command === 'db_favorite_paths') return ['D:\\Music\\one.flac'];
      return null;
    });

    await expect(store.toggleFavorite('D:\\Music\\one.flac')).rejects.toThrow('database locked');

    expect(store.favorites).toEqual(['D:\\Music\\one.flac']);
    expect(invokeMock).toHaveBeenCalledWith('db_favorite_paths');
    expect(store.toasts.at(-1)).toMatchObject({
      type: 'error',
      message: expect.stringContaining('Updating favorite failed'),
    });
  });

  it('restores playlist ordering and refreshes native state after a reorder failure', async () => {
    const store = createTestStore();
    const original = [
      { id: 'one', name: 'One', is_smart: false },
      { id: 'two', name: 'Two', is_smart: false },
    ];
    store.playlists = original.map((playlist) => ({ ...playlist }));
    invokeMock.mockImplementation(async (command) => {
      if (command === 'db_move_playlist_order') throw new Error('write failed');
      if (command === 'db_playlists') return original.map((playlist) => ({ ...playlist }));
      return null;
    });

    await expect(store.movePlaylistOrder(0, 1)).rejects.toThrow('write failed');

    expect(store.playlists.map((playlist) => playlist.id)).toEqual(['one', 'two']);
    expect(invokeMock).toHaveBeenCalledWith('db_playlists');
    expect(store.toasts.at(-1)?.message).toContain('Reordering playlists failed');
  });

  it('propagates _savePlaylist errors and rolls back an edited playlist', async () => {
    const store = createTestStore();
    const original = { id: 'one', name: 'Original', description: '', is_smart: false };
    store.playlists = [{ ...original }];
    invokeMock.mockImplementation(async (command) => {
      if (command === 'db_upsert_playlist') throw new Error('disk full');
      if (command === 'db_playlists') return [{ ...original }];
      return null;
    });

    await expect(store.updatePlaylist('one', 'Changed', '', null)).rejects.toThrow('disk full');

    expect(store.getPlaylist('one')?.name).toBe('Original');
    expect(store.toasts.filter((toast) => toast.type === 'error')).toHaveLength(1);
    expect(store.toasts[0].message).toContain('Saving playlist failed');
  });
});
