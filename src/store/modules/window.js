import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';

const appWindow = getCurrentWindow();
const MINI_SIZES = {
  lyrics: { w: 360, h: 620 },
  compact: { w: 360, h: 150 },
  artwork: { w: 360, h: 360 },
};
const MINI_MIN_WIDTH = 360;
const MINI_MIN_HEIGHT = 150;
const MAIN_MIN_WIDTH = 1000;
const MAIN_MIN_HEIGHT = 480;
const MAIN_DEFAULT_WIDTH = 1200;
const MAIN_DEFAULT_HEIGHT = 700;

export const createWindowState = () => ({
  commandPaletteOpen: false,
  fullscreenOpen: false,
  fullscreenOverlayVisible: false,
  miniPlayerOpen: false,
  selectedAlbum: null,
  dragActive: false,
  devicesVersion: 0,
  queuePanelOpen: false,
  lyricsPanelOpen: false,
  toasts: [],
});

export function createWindowActions() {
  let savedWindowSize = null;
  let savedWindowMaximized = false;
  let toastSequence = 0;
  return {
    showToast(message, options = {}) {
      const text = String(message || '').trim();
      if (!text) return null;
      const id = ++toastSequence;
      const type = ['success', 'error', 'warning', 'info'].includes(options.type)
        ? options.type
        : 'info';
      const duration = Number.isFinite(options.duration)
        ? Math.max(0, Number(options.duration))
        : type === 'error'
          ? 6500
          : 3500;
      this.toasts.push({ id, message: text, type });
      if (duration > 0) {
        setTimeout(() => this.dismissToast(id), duration);
      }
      return id;
    },

    dismissToast(id) {
      const index = this.toasts.findIndex((toast) => toast.id === id);
      if (index >= 0) this.toasts.splice(index, 1);
    },

    runMutation(task) {
      try {
        const result = typeof task === 'function' ? task() : task;
        return Promise.resolve(result).catch((error) => {
          if (!error?.__tsMusicToastShown) {
            this.showToast(error?.message || String(error || 'Operation failed'), {
              type: 'error',
            });
          }
          return false;
        });
      } catch (error) {
        if (!error?.__tsMusicToastShown) {
          this.showToast(error?.message || String(error || 'Operation failed'), {
            type: 'error',
          });
        }
        return Promise.resolve(false);
      }
    },

    async enterFullscreenWithTransition() {
      if (!this.currentSong || this.fullscreenOpen) return;
      this.fullscreenOverlayVisible = true;

      // Wait for the fade-in transition (300ms)
      await new Promise((r) => setTimeout(r, 300));

      this.fullscreenOpen = true;
      try {
        await appWindow.setFullscreen(true);
      } catch (err) {
        console.warn('Tauri fullscreen error:', err);
      }

      // Wait slightly for OS window sizing transition to settle
      await new Promise((r) => setTimeout(r, 150));

      this.fullscreenOverlayVisible = false;
    },

    async exitFullscreenWithTransition() {
      if (!this.fullscreenOpen) return;
      this.fullscreenOverlayVisible = true;

      // Wait for the fade-in transition (300ms)
      await new Promise((r) => setTimeout(r, 300));

      this.fullscreenOpen = false;
      try {
        await appWindow.setFullscreen(false);
      } catch (err) {
        console.warn('Tauri fullscreen restore error:', err);
      }

      // Wait slightly for OS window sizing transition to settle
      await new Promise((r) => setTimeout(r, 150));

      this.fullscreenOverlayVisible = false;
    },

    openFullscreen() {
      this.enterFullscreenWithTransition();
    },

    closeFullscreen() {
      this.exitFullscreenWithTransition();
    },

    toggleFullscreen() {
      if (this.fullscreenOpen) {
        this.exitFullscreenWithTransition();
      } else {
        this.enterFullscreenWithTransition();
      }
    },

    async enterMiniPlayer() {
      if (this.miniPlayerOpen) return;
      // The mini player and native fullscreen are mutually exclusive.
      if (this.fullscreenOpen) await this.exitFullscreenWithTransition();
      this.miniPlayerOpen = true;
      try {
        savedWindowMaximized = await appWindow.isMaximized();
        if (savedWindowMaximized) await appWindow.unmaximize();
        savedWindowSize = await appWindow.outerSize();
        // Lower the min size before shrinking, or the OS clamps the new size.
        await appWindow.setMinSize(new LogicalSize(MINI_MIN_WIDTH, MINI_MIN_HEIGHT));
        await this.applyMiniViewSize('lyrics');
        await appWindow.setResizable(false);
        await appWindow.setAlwaysOnTop(this.miniAlwaysOnTop);
      } catch (e) {
        console.warn('Failed to enter mini player', e);
      }
    },

    async applyMiniViewSize(view) {
      if (!this.miniPlayerOpen) return;
      const s = MINI_SIZES[view] || MINI_SIZES.lyrics;
      try {
        await appWindow.setSize(new LogicalSize(s.w, s.h));
      } catch (e) {
        console.warn('Failed to resize mini player', e);
      }
    },

    async applyMiniSize(width, height) {
      if (!this.miniPlayerOpen) return;
      try {
        await appWindow.setSize(new LogicalSize(Math.round(width), Math.round(height)));
      } catch (e) {
        console.warn('Failed to resize mini player', e);
      }
    },

    async exitMiniPlayer() {
      if (!this.miniPlayerOpen) return;
      this.miniPlayerOpen = false;
      try {
        await appWindow.setAlwaysOnTop(false);
        await appWindow.setResizable(true);
        await appWindow.setMinSize(new LogicalSize(MAIN_MIN_WIDTH, MAIN_MIN_HEIGHT));
        // Fall back to the default size if the saved size is somehow missing, so the
        // window is never left stuck at the mini size.
        const target = savedWindowSize || new LogicalSize(MAIN_DEFAULT_WIDTH, MAIN_DEFAULT_HEIGHT);
        await appWindow.setSize(target);
        if (savedWindowMaximized) await appWindow.maximize();
      } catch (e) {
        console.warn('Failed to exit mini player', e);
      }
      savedWindowSize = null;
      savedWindowMaximized = false;
    },

    toggleMiniPlayer() {
      if (this.miniPlayerOpen) this.exitMiniPlayer();
      else this.enterMiniPlayer();
    },

    setMiniAlwaysOnTop(v) {
      this.miniAlwaysOnTop = !!v;
      this.persistState();
      if (this.miniPlayerOpen) {
        appWindow.setAlwaysOnTop(this.miniAlwaysOnTop).catch(() => {});
      }
    },

    closePopup() {
      this.scanComplete = false;
    },

    openCommandPalette() {
      this.commandPaletteOpen = true;
    },

    closeCommandPalette() {
      this.commandPaletteOpen = false;
    },

    toggleCommandPalette() {
      this.commandPaletteOpen = !this.commandPaletteOpen;
    },
  };
}
