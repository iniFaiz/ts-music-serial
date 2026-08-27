import { invokeCommand as invoke } from '../../generated/ipc';
import { EQ_PRESETS, EQ_MIN_DB, EQ_MAX_DB, matchPreset } from '../../equalizer';
import { getInitialLocale, setLanguage as setI18nLanguage } from '../../i18n';

export const createSettingsState = () => ({
  language: getInitialLocale(),
  outputDevice: null,
  normalizationEnabled: false,
  normalizationPreampDb: 0,
  transitionMode: 'off',
  crossfadeSecs: 6,
  wasapiExclusive: false,
  closeToTray: false,
  eqEnabled: false,
  eqPreampDb: 0,
  eqBands: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  eqPreset: 'flat',
  lyricsSource: 'netease',
  showRomaji: false,
  lyricsOffsetMs: 0,
  waveformEnabled: false,
  miniAlwaysOnTop: false,
});

export function createSettingsActions() {
  return {
    async setOutputDevice(name) {
      this.outputDevice = name || null;
      try {
        await invoke('set_output_device', { name: this.outputDevice });
      } catch (e) {
        console.error('Failed to set output device', e);
      }
      this.persistState();
      // Reload the current track on the new device, preserving position/play state.
      if (this.currentSong) {
        await this.sendPlaybackIntent({
          type: 'select_entry',
          entryId: this.currentSong.queueId,
          autoplay: this.isPlaying,
          startAt: this.currentTime || 0,
        });
      }
    },

    async handleAudioDevicesChanged() {
      console.log('Audio devices changed detected by backend. Bumping version...');
      this.devicesVersion++;
      // If we are currently set to System Default, we need to force re-open the stream
      // to pick up the new physical default device, and reload the track.
      if (this.outputDevice === null) {
        try {
          await invoke('set_output_device', { name: null });
        } catch (e) {
          console.error('Failed to reset output device on device change', e);
        }
        if (this.currentSong) {
          await this.sendPlaybackIntent({
            type: 'select_entry',
            entryId: this.currentSong.queueId,
            autoplay: this.isPlaying,
            startAt: this.currentTime || 0,
          });
        }
      }
    },

    setNormalizationEnabled(v) {
      this.normalizationEnabled = !!v;
      this.persistState();
      invoke('player_set_normalization_settings', {
        enabled: this.normalizationEnabled,
        preampDb: this.normalizationPreampDb,
      }).catch(() => {});
    },

    setNormalizationPreamp(v) {
      this.normalizationPreampDb = Number(v) || 0;
      this.persistState();
      invoke('player_set_normalization_settings', {
        enabled: this.normalizationEnabled,
        preampDb: this.normalizationPreampDb,
      }).catch(() => {});
    },

    setTransitionMode(v) {
      this.transitionMode = v;
      this.persistState();
      invoke('player_set_transition', {
        mode: this.transitionMode,
        crossfadeSecs: this.crossfadeSecs,
      }).catch(() => {});
    },

    setCrossfadeSecs(v) {
      this.crossfadeSecs = Math.max(1, Math.min(12, Number(v) || 6));
      this.persistState();
      invoke('player_set_transition', {
        mode: this.transitionMode,
        crossfadeSecs: this.crossfadeSecs,
      }).catch(() => {});
    },

    async setWasapiExclusive(v) {
      this.wasapiExclusive = !!v;
      this.persistState();
      try {
        await invoke('set_wasapi_exclusive', { enabled: this.wasapiExclusive });
      } catch (err) {
        console.warn('Failed to set WASAPI exclusive:', err);
        // If enabling failed, revert so the UI stays in sync with the backend.
        if (this.wasapiExclusive) {
          this.wasapiExclusive = false;
          this.persistState();
        }
      }
      // Give the audio thread a moment to fully open/close the stream
      // before reloading the track, avoiding a race with CreateSink.
      await new Promise((r) => setTimeout(r, 150));
      // Reload the current track on the newly-selected engine, preserving
      // position/play state (same approach as switching output device).
      if (this.currentSong) {
        await this.sendPlaybackIntent({
          type: 'select_entry',
          entryId: this.currentSong.queueId,
          autoplay: this.isPlaying,
          startAt: this.currentTime || 0,
        });
      }
    },

    setCloseToTray(v) {
      this.closeToTray = !!v;
      this.persistState();
      invoke('set_close_to_tray', { enabled: this.closeToTray }).catch(() => {});
    },

    syncEqualizer() {
      invoke('player_set_equalizer', {
        enabled: this.eqEnabled,
        gains: [...this.eqBands],
        preampDb: this.eqPreampDb,
      }).catch(() => {});
    },

    setEqEnabled(v) {
      this.eqEnabled = !!v;
      this.persistState();
      this.syncEqualizer();
    },

    setEqBand(i, v) {
      if (i < 0 || i >= this.eqBands.length) return;
      const bands = [...this.eqBands];
      bands[i] = Math.max(EQ_MIN_DB, Math.min(EQ_MAX_DB, Number(v) || 0));
      this.eqBands = bands;
      // Hand-editing a band switches the selection to the matching preset (if the
      // curve happens to equal one) or 'custom'.
      this.eqPreset = matchPreset(bands);
      this.persistState();
      this.syncEqualizer();
    },

    setEqPreamp(v) {
      this.eqPreampDb = Math.max(EQ_MIN_DB, Math.min(EQ_MAX_DB, Number(v) || 0));
      this.persistState();
      this.syncEqualizer();
    },

    applyEqPreset(id) {
      const preset = EQ_PRESETS[id];
      if (!preset) return;
      this.eqBands = [...preset.gains];
      this.eqPreset = id;
      this.persistState();
      this.syncEqualizer();
    },

    resetEq() {
      this.eqBands = [...EQ_PRESETS.flat.gains];
      this.eqPreampDb = 0;
      this.eqPreset = 'flat';
      this.persistState();
      this.syncEqualizer();
    },

    setLyricsSource(v) {
      this.lyricsSource = String(v || 'netease');
      this.persistState();
    },

    toggleRomaji() {
      this.showRomaji = !this.showRomaji;
      this.persistState();
    },

    setLyricsOffset(ms) {
      // Clamp to ±3s and round to 50ms steps.
      let v = Math.round((Number(ms) || 0) / 50) * 50;
      v = Math.max(-3000, Math.min(3000, v));
      this.lyricsOffsetMs = v;
      this.persistState();
    },

    setLanguage(lang) {
      if (!['en', 'id'].includes(lang)) return;
      this.language = lang;
      setI18nLanguage(lang);
      this.persistState();
    },
  };
}
