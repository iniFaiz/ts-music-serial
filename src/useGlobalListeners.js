import { onMounted, onUnmounted } from 'vue';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { store } from './store';

// Global Tauri event wiring for the main window, extracted from App.vue:
// drag & drop import grants, watcher-driven library refreshes, second-launch
// file forwarding, WASAPI fallback surfacing, online-metadata progress,
// audio device hotplug, and vinyl-scratch cross-window playback sync.
//
// Registers its own lifecycle hooks — must be called during component setup.
// Every subscription is best-effort: a failed listener degrades that single
// feature, never the app.
export function useGlobalListeners() {
  const unlisteners = new Set();

  onMounted(async () => {
    // Track a subscribe promise: remember its unsubscribe fn on success.
    const track = (promise) =>
      promise
        .then((off) => {
          if (typeof off === 'function') unlisteners.add(off);
        })
        .catch(() => {});

    await Promise.all([
      // Only the native window event can mint an indexing grant. The webview
      // event is presentation-only and never forwards filesystem paths into IPC.
      track(
        listen('library-drop-grant', (event) => {
          const grantId = event.payload && event.payload.grantId;
          if (grantId) store.addPaths(grantId);
        })
      ),

      // Drag & drop: highlight while hovering; Rust handles the actual drop.
      track(
        getCurrentWebview().onDragDropEvent((event) => {
          const t = event.payload.type;
          if (t === 'enter' || t === 'over') {
            store.dragActive = true;
          } else if (t === 'leave') {
            store.dragActive = false;
          } else if (t === 'drop') {
            store.dragActive = false;
          }
        })
      ),

      // Rust has already indexed the exact changed paths before this event arrives.
      track(
        listen('library-changed', (event) => {
          store.handleLibraryChanged(event.payload);
        })
      ),

      // A second app launch (double-clicked audio file) forwarded its files here.
      track(
        listen('open-files-pending', () => {
          store.consumePendingOpenFiles();
        })
      ),

      // Surface WASAPI-exclusive fallback so the user knows it dropped to shared mode.
      track(
        listen('wasapi-exclusive-error', (e) => {
          const msg = e && e.payload ? `: ${e.payload}` : '';
          store.statusMessage = `WASAPI exclusive unavailable — using shared mode${msg}`;
          // The backend has already disabled exclusive mode; sync the frontend.
          if (store.wasapiExclusive) {
            store.wasapiExclusive = false;
            store.persistState();
          }
        })
      ),

      track(
        listen('online-metadata-progress', (e) => {
          store.handleOnlineMetadataProgress(e.payload);
        })
      ),

      track(
        listen('audio-devices-changed', () => {
          store.handleAudioDevicesChanged();
        })
      ),

      // The native vinyl window controls the same Rust audio engine directly.
      // Keep this window's reactive UI aligned after tonearm or scratch gestures there.
      track(
        listen('vinyl-playback-sync', (event) => {
          const payload = event.payload || {};
          if (typeof payload.position === 'number' && Number.isFinite(payload.position)) {
            store.currentTime = Math.max(0, payload.position);
            store.lastSeekAt = Date.now();
          }
          if (payload.playing === true && store.currentSong && store.playbackFinished) {
            store.playSong(store.currentSong, null, {
              autoplay: true,
              startAt: typeof payload.position === 'number' ? payload.position : 0,
            });
          }
        })
      ),
    ]);
  });

  onUnmounted(() => {
    for (const off of unlisteners) {
      try {
        off();
      } catch {
        // teardown is best-effort
      }
    }
    unlisteners.clear();
  });
}
