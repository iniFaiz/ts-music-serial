import { watch } from 'vue';

import { invokeCommand as invoke } from './generated/ipc';
import { createKeySequenceMatcher } from './nyancatEasterEgg';
import { KONAMI_CODE, createCodeSequenceMatcher } from './vinylScratch';

const SEEK_STEP = 5;
const SEEK_STEP_BIG = 10;
const VOLUME_STEP = 0.05;
const VOLUME_STEP_BIG = 0.1;

export function createGlobalShortcuts(store) {
  const nyancatSequence = createKeySequenceMatcher('nyancat');
  const konamiSequence = createCodeSequenceMatcher(KONAMI_CODE);
  let blendFrame = null;
  let phaseFrame = null;
  let lastPhaseTime = null;
  let motionQuery = null;

  const stopPhase = () => {
    if (phaseFrame) cancelAnimationFrame(phaseFrame);
    phaseFrame = null;
    lastPhaseTime = null;
  };

  const startPhase = () => {
    if (phaseFrame || motionQuery?.matches) return;
    const step = (time) => {
      if (lastPhaseTime !== null) {
        const elapsed = Math.min(50, time - lastPhaseTime);
        store.nyancatPhase = (store.nyancatPhase + elapsed * (360 / 5500)) % 360;
      }
      lastPhaseTime = time;
      if (store.nyancatMode || store.nyancatBlend > 0) {
        phaseFrame = requestAnimationFrame(step);
      } else {
        phaseFrame = null;
        lastPhaseTime = null;
      }
    };
    phaseFrame = requestAnimationFrame(step);
  };

  const easeInOutCubic = (value) =>
    value < 0.5 ? 4 * value * value * value : 1 - Math.pow(-2 * value + 2, 3) / 2;

  const animateBlend = (enabled) => {
    if (blendFrame) cancelAnimationFrame(blendFrame);
    blendFrame = null;
    if (enabled) startPhase();
    const from = Math.min(1, Math.max(0, Number(store.nyancatBlend) || 0));
    const to = enabled ? 1 : 0;
    if (from === to) {
      if (!enabled) stopPhase();
      return;
    }
    if (motionQuery?.matches) {
      store.nyancatBlend = to;
      stopPhase();
      return;
    }
    const startedAt = performance.now();
    const duration = 850 * Math.abs(to - from);
    const step = (time) => {
      const progress = Math.min(1, (time - startedAt) / duration);
      store.nyancatBlend = from + (to - from) * easeInOutCubic(progress);
      if (progress < 1) {
        blendFrame = requestAnimationFrame(step);
      } else {
        store.nyancatBlend = to;
        blendFrame = null;
        if (!enabled) stopPhase();
      }
    };
    blendFrame = requestAnimationFrame(step);
  };

  const stopModeWatch = watch(
    () => store.nyancatMode,
    (enabled) => animateBlend(enabled),
    { flush: 'sync' }
  );

  const onMotionChange = () => {
    if (motionQuery?.matches) stopPhase();
    animateBlend(store.nyancatMode);
  };

  const isTypingTarget = (event) => {
    const element = event.target;
    if (!element) return false;
    return (
      ['INPUT', 'TEXTAREA', 'SELECT'].includes(element.tagName) || element.isContentEditable
    );
  };

  const seekBy = (delta) => {
    if (!store.currentSong) return;
    const unclamped = (store.currentTime || 0) + delta;
    store.seek(Math.min(store.duration || Infinity, Math.max(0, unclamped)));
  };

  const bumpVolume = (delta) => {
    const value = Math.round(((store.volume || 0) + delta) * 100) / 100;
    store.setVolume(Math.min(1, Math.max(0, value)));
  };

  const openVinylScratchWindow = async () => {
    try {
      await invoke('open_vinyl_scratch_window');
    } catch (error) {
      console.error('Failed to open vinyl scratch window', error);
      store.statusMessage = `Could not open Vinyl Scratch: ${error}`;
    }
  };

  const handleKeydown = (event) => {
    if (event.ctrlKey || event.metaKey || event.altKey || event.isComposing) {
      konamiSequence.reset();
    } else if (!event.repeat && konamiSequence.push(event.code)) {
      event.preventDefault();
      openVinylScratchWindow();
      return;
    }

    if (event.ctrlKey || event.metaKey || event.altKey || event.isComposing) {
      nyancatSequence.reset();
    } else if (!event.repeat && nyancatSequence.push(event.key)) {
      store.nyancatMode = !store.nyancatMode;
    }

    if (event.ctrlKey && event.shiftKey && /^f$/i.test(event.key)) {
      event.preventDefault();
      if (!store.miniPlayerOpen) store.toggleFullscreen();
      return;
    }
    if (event.ctrlKey && event.shiftKey && /^m$/i.test(event.key)) {
      event.preventDefault();
      store.toggleMiniPlayer();
      return;
    }
    if (
      (event.ctrlKey || event.metaKey) &&
      !event.shiftKey &&
      !event.altKey &&
      /^k$/i.test(event.key)
    ) {
      event.preventDefault();
      store.toggleCommandPalette();
      return;
    }
    if (event.key === 'Escape' && store.commandPaletteOpen) {
      store.closeCommandPalette();
      return;
    }
    if (event.key === 'Escape' && store.miniPlayerOpen) {
      store.exitMiniPlayer();
      return;
    }
    if (event.key === 'Escape' && store.fullscreenOpen) {
      store.exitFullscreenWithTransition();
      return;
    }
    if (isTypingTarget(event)) return;

    if ((event.ctrlKey || event.metaKey) && !event.altKey && event.key === 'ArrowRight') {
      event.preventDefault();
      store.nextSong(true);
      return;
    }
    if ((event.ctrlKey || event.metaKey) && !event.altKey && event.key === 'ArrowLeft') {
      event.preventDefault();
      store.prevSong();
      return;
    }
    if (event.ctrlKey || event.metaKey || event.altKey) return;

    if (event.code.length === 6 && event.code.startsWith('Digit')) {
      if (store.currentSong && store.duration > 0) {
        event.preventDefault();
        store.seek((store.duration * Number(event.code.slice(5))) / 10);
      }
      return;
    }

    switch (event.code) {
      case 'Space':
      case 'KeyK':
        event.preventDefault();
        store.togglePlay();
        break;
      case 'ArrowRight':
        event.preventDefault();
        seekBy(event.shiftKey ? SEEK_STEP_BIG : SEEK_STEP);
        break;
      case 'ArrowLeft':
        event.preventDefault();
        seekBy(event.shiftKey ? -SEEK_STEP_BIG : -SEEK_STEP);
        break;
      case 'ArrowUp':
        event.preventDefault();
        bumpVolume(event.shiftKey ? VOLUME_STEP_BIG : VOLUME_STEP);
        break;
      case 'ArrowDown':
        event.preventDefault();
        bumpVolume(event.shiftKey ? -VOLUME_STEP_BIG : -VOLUME_STEP);
        break;
      case 'Home':
        if (store.currentSong) {
          event.preventDefault();
          store.seek(0);
        }
        break;
      case 'KeyM':
        event.preventDefault();
        store.toggleMute();
        break;
      case 'KeyS':
        event.preventDefault();
        store.toggleShuffle();
        break;
      case 'KeyR':
        event.preventDefault();
        store.toggleLoop();
        break;
      case 'KeyL':
        if (store.currentSong) {
          event.preventDefault();
          store.toggleFavorite(store.currentSong.path);
        }
        break;
    }
  };

  const mount = () => {
    motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
    motionQuery.addEventListener('change', onMotionChange);
    window.addEventListener('keydown', handleKeydown);
  };

  const unmount = () => {
    if (blendFrame) cancelAnimationFrame(blendFrame);
    stopPhase();
    motionQuery?.removeEventListener('change', onMotionChange);
    window.removeEventListener('keydown', handleKeydown);
    stopModeWatch();
  };

  return { mount, unmount };
}
