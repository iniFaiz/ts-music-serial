import { invokeCommand as invoke } from '../../generated/ipc';

const queueMetadata = new Map();
let activeStationSession = null;
const STATION_BATCH_SIZE = 24;

const nativeQueueEntry = (song) => {
  if (song.queueId) queueMetadata.set(song.queueId, { ...song });
  return {
    id: song.queueId || '',
    path: song.path,
    durationHint: song.duration_secs || 0,
    trackGainDb: typeof song.track_gain_db === 'number' ? song.track_gain_db : null,
    trackPeak: typeof song.track_peak === 'number' ? song.track_peak : null,
  };
};

export const createPlaybackState = () => ({
  currentSong: null,
  currentSampleRate: null,
  currentBitDepth: null,
  isPlaying: false,
  isBuffering: false,
  volume: 1,
  isMuted: false,
  currentTime: 0,
  duration: 0,
  nyancatMode: false,
  nyancatBlend: 0,
  nyancatPhase: 0,
  lastSeekAt: 0,
  queue: [],
  playbackSessionSnapshot: null,
  loopMode: 0,
  shuffleMode: false,
  autoplayMode: false,
  visualizerEnabled: true,
  playbackFinished: false,
  sleepTimerMode: 'off',
  sleepTimerDeadline: 0,
});

export function createPlaybackActions() {
  let playbackIntentTail = Promise.resolve();
  return {
async playSong(song, newQueue = null, options = {}) {
    if (!options.preserveStation) activeStationSession = null;
    if (newQueue && newQueue.length > 0) {
      this.queue = newQueue.map((s) => ({
        ...s,
        queueId: s.queueId || Math.random().toString(36).substring(2, 9),
      }));
      let idx = -1;
      if (song) {
        idx = newQueue.indexOf(song);
        if (idx === -1) {
          idx = this.queue.findIndex((s) => s.path === song.path);
        }
      }
      if (idx === -1) idx = 0;
      this.currentSong = { ...this.queue[idx] };
    } else if (this.queue.length === 0) {
      // No explicit queue: start one with just this track
      if (song) {
        const entry = {
          ...song,
          queueId: song.queueId || Math.random().toString(36).substring(2, 9),
        };
        this.queue = [entry];
        this.currentSong = { ...entry };
      } else {
        this.queue = [];
        this.currentSong = null;
      }
    } else {
      // Keep existing queue, but play this song. Try to find/match it.
      let idx = -1;
      if (song) {
        if (song.queueId) {
          idx = this.queue.findIndex((s) => s.queueId === song.queueId);
        }
        if (idx === -1) {
          idx = this.queue.findIndex((s) => s.path === song.path);
        }
      }
      if (idx !== -1) {
        if (!this.queue[idx].queueId) {
          this.queue[idx].queueId = Math.random().toString(36).substring(2, 9);
        }
        this.currentSong = { ...this.queue[idx] };
      } else if (song) {
        const entry = {
          ...song,
          queueId: song.queueId || Math.random().toString(36).substring(2, 9),
        };
        this.queue.push(entry);
        this.currentSong = { ...entry };
      } else {
        this.currentSong = null;
      }
    }
    if (!this.currentSong) return;
    this.recordPlayStart(this.currentSong.path);
    const autoplay = options.autoplay !== false;
    this.isPlaying = autoplay;
    this.playbackFinished = false;
    this.isBuffering = true;
    this.persistState();
    try {
      await this.sendPlaybackIntent({
        type: 'play_queue',
        entries: this.queue.map(nativeQueueEntry),
        startEntryId: this.currentSong.queueId,
        autoplay,
        startAt: options.startAt ?? null,
        shuffle: !!this.shuffleMode,
        repeat: ['off', 'all', 'one'][this.loopMode] || 'off',
        autoplayMode: !!this.autoplayMode,
        sleep: this.nativeSleepMode(),
      });
    } catch (error) {
      this.isPlaying = false;
      this.statusMessage = `Playback failed: ${error}`;
    } finally {
      this.isBuffering = false;
    }
  },

currentQueueIndex() {
    if (!this.currentSong) return -1;
    if (this.currentSong.queueId) {
      const idx = this.queue.findIndex((s) => s.queueId === this.currentSong.queueId);
      if (idx !== -1) return idx;
    }
    return this.queue.findIndex((s) => s.path === this.currentSong.path);
  },

async playRandom(songs, options = {}) {
    const list = Array.isArray(songs) ? songs : [];
    if (list.length === 0) return false;
    try {
      const selected = await invoke('db_random_track_from_paths', {
        paths: list.map((song) => song.path),
        exclude: null,
      });
      if (!selected) return false;
      this.shuffleMode = true;
      await this.playSong(selected, list, options);
      return true;
    } catch (error) {
      console.error('Failed to select a random track', error);
      return false;
    }
  },

playNext(song) {
    const queueId =
      song.queueId && !this.queue.some((e) => e.queueId === song.queueId)
        ? song.queueId
        : Math.random().toString(36).substring(2, 9);
    const entry = { ...song, queueId };
    if (this.queue.length === 0) {
      if (this.currentSong) {
        if (!this.currentSong.queueId) {
          this.currentSong.queueId = Math.random().toString(36).substring(2, 9);
        }
        const curr = { ...this.currentSong };
        this.queue = [curr, entry];
      } else {
        this.queue = [entry];
      }
    } else {
      const idx = this.currentQueueIndex();
      this.queue.splice(idx + 1, 0, entry);
    }
    this.sendPlaybackIntent({
      type: 'enqueue',
      entries: [nativeQueueEntry(entry)],
      afterCurrent: true,
    }).catch((error) => console.error('Failed to insert queue entry', error));
    this.persistState();
  },

playNextSongs(songs) {
    const list = songs.map((s) => {
      const queueId =
        s.queueId && !this.queue.some((e) => e.queueId === s.queueId)
          ? s.queueId
          : Math.random().toString(36).substring(2, 9);
      return { ...s, queueId };
    });
    if (this.queue.length === 0) {
      if (this.currentSong) {
        if (!this.currentSong.queueId) {
          this.currentSong.queueId = Math.random().toString(36).substring(2, 9);
        }
        const curr = { ...this.currentSong };
        this.queue = [curr, ...list];
      } else {
        this.queue = [...list];
      }
    } else {
      const idx = this.currentQueueIndex();
      this.queue.splice(idx + 1, 0, ...list);
    }
    this.sendPlaybackIntent({
      type: 'enqueue',
      entries: list.map(nativeQueueEntry),
      afterCurrent: true,
    }).catch((error) => console.error('Failed to insert queue entries', error));
    this.persistState();
  },

addToQueue(songs) {
    const list = (Array.isArray(songs) ? songs : [songs]).map((s) => {
      const queueId =
        s.queueId && !this.queue.some((e) => e.queueId === s.queueId)
          ? s.queueId
          : Math.random().toString(36).substring(2, 9);
      return { ...s, queueId };
    });
    if (this.queue.length === 0 && this.currentSong) {
      if (!this.currentSong.queueId) {
        this.currentSong.queueId = Math.random().toString(36).substring(2, 9);
      }
      this.queue = [{ ...this.currentSong }];
    }
    this.queue.push(...list);
    this.sendPlaybackIntent({
      type: 'enqueue',
      entries: list.map(nativeQueueEntry),
      afterCurrent: false,
    }).catch((error) => console.error('Failed to append queue entries', error));
    this.persistState();
  },

removeFromQueue(index) {
    if (index < 0 || index >= this.queue.length) return;
    const [removed] = this.queue.splice(index, 1);
    this.sendPlaybackIntent({
      type: 'remove_queue_item',
      entryId: removed.queueId,
    }).catch((error) => console.error('Failed to remove queue entry', error));
    this.persistState();
  },

async removeQueuePaths(paths) {
    const removed = new Set(Array.isArray(paths) ? paths : [paths]);
    const entryIds = this.queue
      .filter((entry) => removed.has(entry.path))
      .map((entry) => entry.queueId)
      .filter(Boolean);
    const currentId = this.currentSong?.queueId;
    entryIds.sort((left, right) => Number(left === currentId) - Number(right === currentId));
    for (const entryId of entryIds) {
      await this.sendPlaybackIntent({ type: 'remove_queue_item', entryId });
    }
  },

moveInQueue(from, to) {
    if (from === to) return;
    if (from < 0 || from >= this.queue.length) return;
    if (to < 0 || to >= this.queue.length) return;
    const [item] = this.queue.splice(from, 1);
    this.queue.splice(to, 0, item);
    this.sendPlaybackIntent({
      type: 'move_queue_item',
      entryId: item.queueId,
      toIndex: to,
    }).catch((error) => console.error('Failed to move queue entry', error));
    this.persistState();
  },

async playQueueIndex(index) {
    if (index < 0 || index >= this.queue.length) return;
    if (!this.queue[index].queueId) {
      this.queue[index].queueId = Math.random().toString(36).substring(2, 9);
    }
    this.currentSong = { ...this.queue[index] };
    this.isPlaying = true;
    this.playbackFinished = false;
    this.isBuffering = true;
    try {
      await this.sendPlaybackIntent({
        type: 'select_entry',
        entryId: this.queue[index].queueId,
        autoplay: true,
        startAt: null,
      });
    } finally {
      this.isBuffering = false;
    }
    this.persistState();
  },

clearQueue() {
    if (this.currentSong) {
      if (!this.currentSong.queueId) {
        this.currentSong.queueId = Math.random().toString(36).substring(2, 9);
      }
      this.queue = [{ ...this.currentSong }];
    } else {
      this.queue = [];
    }
    this.sendPlaybackIntent({ type: 'clear_upcoming' }).catch((error) =>
      console.error('Failed to clear upcoming queue', error)
    );
    this.persistState();
  },

async playStation(type, key) {
    let batch = null;
    try {
      batch = await invoke('db_station_start', {
        kind: type,
        key,
        limit: STATION_BATCH_SIZE,
      });
    } catch (e) {
      console.error('Failed to load station', e);
    }
    if (!batch?.tracks?.length) return;
    activeStationSession = {
      id: batch.session_id,
      hasMore: !!batch.has_more,
    };
    this.shuffleMode = false; // native station order is already shuffled
    this.autoplayMode = true; // requests the next lazy native window at the end
    this.recordRecent('station', `${type}:${key}`);
    this.playSong(batch.tracks[0], batch.tracks, { preserveStation: true });
  },

async togglePlay() {
    if (!this.currentSong) return;
    if (this.playbackFinished && this.currentSong) {
      await this.playSong(this.currentSong, null, { autoplay: true, startAt: 0 });
    } else {
      await this.sendPlaybackIntent({ type: 'set_playing', playing: !this.isPlaying });
    }
  },

toggleLoop() {
    if (!this.currentSong) return;
    this.loopMode = (this.loopMode + 1) % 3;
    this.pushPlaybackModes();
    this.persistState();
  },

nativeSleepMode() {
    if (this.sleepTimerMode === 'end') return { mode: 'end_track' };
    if (this.sleepTimerMode === 'end-queue') return { mode: 'end_queue' };
    if (typeof this.sleepTimerMode === 'number' && this.sleepTimerDeadline > 0) {
      return { mode: 'deadline', deadlineMs: this.sleepTimerDeadline };
    }
    return { mode: 'off' };
  },

sendPlaybackIntent(intent) {
    const run = playbackIntentTail
      .catch(() => {})
      .then(() => invoke('playback_session_intent', { intent }))
      .then((update) => {
        this.applyPlaybackSessionUpdate(update);
        return update;
      });
    playbackIntentTail = run.catch(() => {});
    return run;
  },

pushPlaybackModes() {
    return this.sendPlaybackIntent({
      type: 'set_modes',
      shuffle: !!this.shuffleMode,
      repeat: ['off', 'all', 'one'][this.loopMode] || 'off',
      autoplay: !!this.autoplayMode,
    }).catch((error) => console.error('Failed to update playback modes', error));
  },

applyPlaybackSessionUpdate(update) {
    if (!update) return;
    if (update.snapshot) {
      const snapshot = update.snapshot;
      const available = [...this.queue];
      const queue = snapshot.queue.map((entry) => {
        let index = available.findIndex((song) => song.queueId === entry.id);
        if (index === -1) index = available.findIndex((song) => song.path === entry.path);
        const song =
          index === -1
            ? queueMetadata.get(entry.id) ||
              [...queueMetadata.values()].find((candidate) => candidate.path === entry.path)
            : available.splice(index, 1)[0];
        const hydrated = song
          ? { ...song, queueId: entry.id }
          : {
              queueId: entry.id,
              path: entry.path,
              duration_secs: entry.durationHint || 0,
              title: entry.path.split(/[\\/]/).pop() || entry.path,
              artist: 'Unknown Artist',
              album: 'Unknown Album',
            };
        queueMetadata.set(entry.id, hydrated);
        return hydrated;
      });
      this.queue = queue;
      this.currentSong = snapshot.currentEntryId
        ? queue.find((entry) => entry.queueId === snapshot.currentEntryId) || null
        : null;
      this.shuffleMode = !!snapshot.shuffle;
      this.loopMode = { off: 0, all: 1, one: 2 }[snapshot.repeat] ?? 0;
      this.autoplayMode = !!snapshot.autoplay;
      this.isPlaying = !!snapshot.playing;
      this.transitionMode = snapshot.transition || this.transitionMode;
      this.crossfadeSecs = snapshot.crossfadeSecs || this.crossfadeSecs;
      const sleep = snapshot.sleep || { mode: 'off' };
      if (sleep.mode === 'end_track') {
        this.sleepTimerMode = 'end';
        this.sleepTimerDeadline = 0;
      } else if (sleep.mode === 'end_queue') {
        this.sleepTimerMode = 'end-queue';
        this.sleepTimerDeadline = 0;
      } else if (sleep.mode === 'deadline') {
        this.sleepTimerDeadline = sleep.deadlineMs || 0;
        this.sleepTimerMode = Math.max(
          1,
          Math.ceil((this.sleepTimerDeadline - Date.now()) / 60000)
        );
      } else {
        this.sleepTimerMode = 'off';
        this.sleepTimerDeadline = 0;
      }
      this.playbackSessionSnapshot = snapshot;
    }
    const effect = update.effect;
    if (effect?.type === 'load') {
      this.playbackFinished = false;
      this.currentTime = effect.startAt || 0;
    } else if (effect?.type === 'set_playing') {
      this.isPlaying = !!effect.playing;
    } else if (effect?.type === 'stop') {
      this.isPlaying = false;
      this.playbackFinished = true;
      if (String(effect.reason || '').startsWith('sleep_')) {
        this.sleepTimerMode = 'off';
        this.sleepTimerDeadline = 0;
      }
    }
    this.persistState();
  },

async nextSong(userTriggered = false) {
    if (!this.currentSong || this.queue.length === 0) return;
    try {
      this.isBuffering = true;
      let update = await this.sendPlaybackIntent({
        type: 'next',
        userTriggered: !!userTriggered,
      });

      if (update.effect?.type === 'request_autoplay') {
        if (activeStationSession) {
          if (!activeStationSession.hasMore) {
            activeStationSession = null;
            await this.sendPlaybackIntent({ type: 'set_playing', playing: false });
            this.playbackFinished = true;
            return;
          }
          const batch = await invoke('db_station_next', {
            sessionId: activeStationSession.id,
            limit: STATION_BATCH_SIZE,
          });
          activeStationSession.hasMore = !!batch.has_more;
          if (!batch.tracks?.length) {
            activeStationSession = null;
            await this.sendPlaybackIntent({ type: 'set_playing', playing: false });
            this.playbackFinished = true;
            return;
          }
          await this.playSong(batch.tracks[0], batch.tracks, {
            autoplay: true,
            preserveStation: true,
          });
          return;
        }
        const song = await this.pickRandomSong();
        if (!song) {
          await this.sendPlaybackIntent({ type: 'set_playing', playing: false });
          this.playbackFinished = true;
          return;
        }
        const entry = { ...song, queueId: Math.random().toString(36).substring(2, 9) };
        this.queue.push(entry);
        update = await this.sendPlaybackIntent({
          type: 'append_autoplay',
          entry: nativeQueueEntry(entry),
          playNow: true,
        });
      }
    } catch (error) {
      console.error('Failed to advance native playback session', error);
    } finally {
      this.isBuffering = false;
    }
  },

async prevSong() {
    if (!this.currentSong || this.queue.length === 0) return;
    try {
      this.isBuffering = true;
      await this.sendPlaybackIntent({ type: 'previous', position: this.currentTime || 0 });
    } catch (error) {
      console.error('Failed to rewind native playback session', error);
    } finally {
      this.isBuffering = false;
    }
  },

async seek(time) {
    const t = Math.max(0, Number(time) || 0);
    this.currentTime = t;
    this.lastSeekAt = Date.now();
    if (this.playbackFinished && this.currentSong) {
      this.playSong(this.currentSong, null, { autoplay: true, startAt: t });
    } else {
      try {
        await invoke('player_seek', { position: t });
      } catch (e) {
        console.error('Seek failed', e);
      }
    }
  },

setVolume(val) {
    const num = parseFloat(val);
    this.volume = num;
    if (num > 0 && this.isMuted) {
      this.isMuted = false;
    } else if (num === 0 && !this.isMuted) {
      this.isMuted = true;
    }
  },

toggleMute() {
    this.isMuted = !this.isMuted;
    if (!this.isMuted && this.volume === 0) {
      this.volume = 0.25; // Default volume when unmuting from 0
    }
    this.persistState();
  },

toggleShuffle() {
    if (!this.currentSong) return;
    this.shuffleMode = !this.shuffleMode;
    this.pushPlaybackModes();
    this.persistState();
  },

toggleAutoplay() {
    this.autoplayMode = !this.autoplayMode;
    this.pushPlaybackModes();
    this.persistState();
  },

syncVisualizer() {
    invoke('player_set_spectrum_enabled', { enabled: this.visualizerEnabled }).catch(() => {});
  },

setVisualizerEnabled(val) {
    this.visualizerEnabled = !!val;
    this.syncVisualizer();
    this.persistState();
  },

setWaveformEnabled(val) {
    this.waveformEnabled = !!val;
    this.persistState();
  },

setSleepTimer(mode) {
    if (mode === 'off' || mode === null || mode === undefined) {
      this.sleepTimerMode = 'off';
      this.sleepTimerDeadline = 0;
      this.sendPlaybackIntent({ type: 'set_sleep', sleep: this.nativeSleepMode() }).catch(() => {});
      return;
    }
    if (mode === 'end' || mode === 'end-queue') {
      this.sleepTimerMode = mode;
      this.sleepTimerDeadline = 0;
      this.sendPlaybackIntent({ type: 'set_sleep', sleep: this.nativeSleepMode() }).catch(() => {});
      return;
    }
    const raw = Number(mode);
    if (!isFinite(raw) || raw <= 0) {
      this.sleepTimerMode = 'off';
      this.sleepTimerDeadline = 0;
      this.sendPlaybackIntent({ type: 'set_sleep', sleep: this.nativeSleepMode() }).catch(() => {});
      return;
    }
    const minutes = Math.min(1440, Math.max(1, Math.round(raw))); // clamp 1min–24h
    this.sleepTimerMode = minutes;
    this.sleepTimerDeadline = Date.now() + minutes * 60000;
    this.sendPlaybackIntent({ type: 'set_sleep', sleep: this.nativeSleepMode() }).catch(() => {});
  },

async pickRandomSong() {
    try {
      return await invoke('db_auto_dj_next', {
        currentPath: this.currentSong ? this.currentSong.path : null,
        recentPaths: this.queue.slice(-100).map((entry) => entry.path),
      });
    } catch {
      return null;
    }
  },

nextUpEntry() {
    const nativeNextId = this.playbackSessionSnapshot?.nextEntryId;
    if (!nativeNextId) return null;
    return this.queue.find((entry) => entry.queueId === nativeNextId) || null;
  },

nextUpPath() {
    const entry = this.nextUpEntry();
    return entry ? entry.path : null;
  }
  };
}
