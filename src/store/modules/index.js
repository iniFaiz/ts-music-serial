import { createIntegrationsState, createIntegrationsActions } from './integrations';
import { createLibraryState, createLibraryActions } from './library';
import { createPlaybackState, createPlaybackActions } from './playback';
import { createPlaylistsState, createPlaylistsActions } from './playlists';
import { createSettingsState, createSettingsActions } from './settings';
import { createWindowState, createWindowActions } from './window';

export const createStoreState = () => ({
  ...createLibraryState(),
  ...createPlaybackState(),
  ...createPlaylistsState(),
  ...createSettingsState(),
  ...createIntegrationsState(),
  ...createWindowState(),
});

export const createStoreActions = () => {
  const actions = {};
  for (const domainActions of [
    createLibraryActions(),
    createPlaybackActions(),
    createPlaylistsActions(),
    createSettingsActions(),
    createIntegrationsActions(),
    createWindowActions(),
  ]) {
    Object.defineProperties(actions, Object.getOwnPropertyDescriptors(domainActions));
  }
  return actions;
};
