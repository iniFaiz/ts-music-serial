// Opacity ramp for the three instrumental-gap dots shown between lyric lines:
// each dot fills over its third of the gap as playback advances.
export function gapDotColor(line, dotIdx, currentMs) {
  if (!line.isGap) return 'rgba(255, 255, 255, 0.2)';
  const duration = line.endTimeMs - line.time_ms;
  const elapsed = Math.max(0, Math.min(duration, currentMs - line.time_ms));
  const p = elapsed / duration;

  const startRange = dotIdx * 0.33;
  const dotProgress = Math.max(0, Math.min(1, (p - startRange) / 0.33));
  const opacity = 0.2 + (0.95 - 0.2) * dotProgress;

  return `rgba(255, 255, 255, ${opacity.toFixed(3)})`;
}
