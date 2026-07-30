import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock('./generated/ipc', () => ({
  invokeCommand: invokeMock,
}));

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({}),
  LogicalSize: class LogicalSize {},
}));

const roots = ['D:\\Apple Music', 'D:\\Lagu'];

function successfulStartup(command) {
  switch (command) {
    case 'db_count':
      return 294;
    case 'db_roots':
    case 'restore_roots':
      return roots;
    case 'db_favorite_paths':
    case 'db_playlists':
    case 'db_recents':
    case 'take_pending_open_files':
      return [];
    case 'db_kv_get':
      return null;
    case 'musixmatch_token_status':
      return false;
    default:
      return null;
  }
}

describe('store library startup', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command) => successfulStartup(command));
  });

  it('loads the native library automatically when the store is imported', async () => {
    vi.resetModules();
    const { store } = await import('./store');

    await vi.waitFor(() => expect(store.libraryReady).toBe(true));

    expect(store.scanCount).toBe(294);
    expect(store.roots).toEqual(roots);
    expect(invokeMock).toHaveBeenCalledWith('db_count');
    expect(invokeMock).toHaveBeenCalledWith('restore_roots');
  });

  it('ends the loading state when native startup fails', async () => {
    vi.resetModules();
    invokeMock.mockImplementation(async (command) => {
      if (command === 'db_roots') throw new Error('database unavailable');
      return successfulStartup(command);
    });

    const { store } = await import('./store');
    await vi.waitFor(() => expect(store.libraryReady).toBe(true));

    expect(store.statusMessage).toContain('Failed to load library');
  });
});
