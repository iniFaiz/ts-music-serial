export const KONAMI_CODE = [
  'ArrowUp',
  'ArrowUp',
  'ArrowDown',
  'ArrowDown',
  'ArrowLeft',
  'ArrowRight',
  'ArrowLeft',
  'ArrowRight',
  'KeyB',
  'KeyA',
];

export const TONEARM_REST_ANGLE = -22;
export const TONEARM_OUTER_ANGLE = -5;
export const TONEARM_INNER_ANGLE = 13;
export const TONEARM_DROP_THRESHOLD = -12;

// Small stateful matcher for KeyboardEvent.code sequences. A mismatch can still
// begin a new sequence, which makes repeated ArrowUp presses feel forgiving.
export function createCodeSequenceMatcher(sequence) {
  const target = Array.isArray(sequence) ? sequence.filter(Boolean) : [];
  let index = 0;

  return {
    push(code) {
      if (!target.length || typeof code !== 'string') return false;

      if (code === target[index]) {
        index += 1;
      } else {
        index = code === target[0] ? 1 : 0;
      }

      if (index === target.length) {
        index = 0;
        return true;
      }
      return false;
    },

    reset() {
      index = 0;
    },
  };
}

// Return the shortest signed movement between two angles, including when the
// pointer crosses the -180/180 degree boundary.
export function shortestAngleDelta(previous, next) {
  const delta = Number(next) - Number(previous);
  if (!Number.isFinite(delta)) return 0;
  return ((((delta + 180) % 360) + 360) % 360) - 180;
}

export function timeFromScratchRotation(startTime, rotationDelta, duration, secondsPerTurn = 5) {
  const safeStart = Math.max(0, Number(startTime) || 0);
  const safeTurn = Math.max(0.1, Number(secondsPerTurn) || 5);
  const next = safeStart + ((Number(rotationDelta) || 0) / 360) * safeTurn;
  const max = Math.max(0, Number(duration) || 0);
  return Math.min(max || Infinity, Math.max(0, next));
}

export function tonearmProgressFromAngle(
  angle,
  outerAngle = TONEARM_OUTER_ANGLE,
  innerAngle = TONEARM_INNER_ANGLE
) {
  const span = Number(innerAngle) - Number(outerAngle);
  if (!Number.isFinite(span) || Math.abs(span) < 0.001) return 0;
  return Math.min(1, Math.max(0, (Number(angle) - Number(outerAngle)) / span));
}

export function tonearmAngleFromProgress(
  progress,
  outerAngle = TONEARM_OUTER_ANGLE,
  innerAngle = TONEARM_INNER_ANGLE
) {
  const safeProgress = Math.min(1, Math.max(0, Number(progress) || 0));
  return Number(outerAngle) + (Number(innerAngle) - Number(outerAngle)) * safeProgress;
}
