import { invokeCommand as invoke } from './generated/ipc';

// Shared, module-level waveform cache.
//
// The Rust `get_waveform` command decodes a track once, reduces it to a fixed
// number of peak amplitudes (one byte each, 0..255) and caches that on disk. We
// keep an in-memory LRU of the resulting arrays so re-selecting a track (or the
// seek bar re-mounting) is free, and de-dupe concurrent requests for the same
// path so we never kick off two decodes at once.
//
//   cache:    path -> Uint8Array of bar heights, or null when the track is
//             genuinely undecodable (Rust answered Ok(None) after trying).
//   inflight: path -> Promise, shared by concurrent callers.
//
// A rejected invoke (transient backend/IPC hiccup) is deliberately NOT cached:
// callers get null to fall back to the slider, but the next attempt retries
// instead of staying poisoned until the user runs "Clear Cache".
const cache = new Map();
const inflight = new Map();
const MAX_WAVEFORMS = 300;

function touch(path) {
  if (cache.has(path)) {
    const v = cache.get(path);
    cache.delete(path);
    cache.set(path, v);
  }
}

function cacheSet(path, value) {
  cache.set(path, value);
  while (cache.size > MAX_WAVEFORMS) {
    const oldest = cache.keys().next().value;
    cache.delete(oldest);
  }
}

export function getCachedWaveform(path) {
  if (!cache.has(path)) return undefined;
  touch(path);
  return cache.get(path);
}

export async function loadWaveform(path) {
  if (!path) return null;
  if (cache.has(path)) {
    touch(path);
    return cache.get(path);
  }
  if (inflight.has(path)) return inflight.get(path);

  const request = invoke('get_waveform', { path })
    .then((result) => {
      const value = result ? Uint8Array.from(result) : null;
      cacheSet(path, value);
      return value;
    })
    .catch(() => {
      // Transient failure — degrade to the slider without caching a negative,
      // so a later attempt can succeed.
      return null;
    })
    .finally(() => {
      inflight.delete(path);
    });

  inflight.set(path, request);
  return request;
}

export function clearWaveformCache() {
  cache.clear();
  inflight.clear();
}
