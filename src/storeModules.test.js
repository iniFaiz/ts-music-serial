import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({}),
  LogicalSize: class LogicalSize {},
}));

import { createStoreActions, createStoreState } from './store/modules';

describe('modular store composition', () => {
  it('preserves playlist getters when domain actions are composed', () => {
    const target = createStoreState();
    Object.defineProperties(target, Object.getOwnPropertyDescriptors(createStoreActions()));
    target.playlists = [
      { id: 'regular', is_smart: false },
      { id: 'smart', is_smart: true },
    ];

    expect(Object.getOwnPropertyDescriptor(target, 'smartPlaylists')?.get).toBeTypeOf('function');
    expect(target.smartPlaylists.map((playlist) => playlist.id)).toEqual(['smart']);
    expect(target.normalPlaylists.map((playlist) => playlist.id)).toEqual(['regular']);
  });
});
