// Small rolling matcher used by the keyboard easter egg. Keeping it independent
// from the DOM makes the sequence behavior deterministic and easy to test.
export function createKeySequenceMatcher(sequence) {
  const target = String(sequence || '').toLowerCase();
  let buffer = '';

  return {
    push(key) {
      if (!target || typeof key !== 'string') return false;

      // Navigation/editing keys break a partially typed sequence. Modifier keys
      // are handled by App.vue so pressing Shift for an uppercase letter works.
      if (key.length !== 1) {
        buffer = '';
        return false;
      }

      buffer = (buffer + key.toLowerCase()).slice(-target.length);
      if (buffer === target) {
        buffer = '';
        return true;
      }
      return false;
    },

    reset() {
      buffer = '';
    },
  };
}
