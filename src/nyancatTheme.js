const NYANCAT_RAINBOW_STOPS = [
  ['#ff2d55', 0],
  ['#ff9500', 0.125],
  ['#ffcc00', 0.25],
  ['#34c759', 0.375],
  ['#32ade6', 0.5],
  ['#007aff', 0.625],
  ['#5856d6', 0.75],
  ['#af52de', 0.875],
  ['#ff2d55', 1],
].map(([color, fraction]) => ({ color, fraction, hsl: hexToHsl(color) }));

const roundPosition = (value) => Math.round(value * 10000) / 10000;

function createStopPosition(progress, pixelCorrection, fraction) {
  if (fraction === 0) return '0%';

  const percentage = roundPosition(progress * fraction);
  const pixels = roundPosition(pixelCorrection * fraction);
  if (pixels === 0) return `${percentage}%`;

  const operator = pixels > 0 ? '+' : '-';
  return `calc(${percentage}% ${operator} ${Math.abs(pixels)}px)`;
}

function blendColor(baseColor, rainbowColor, mix) {
  if (mix >= 1) return rainbowColor;

  const rainbowWeight = roundPosition(mix * 100);
  const baseWeight = roundPosition(100 - rainbowWeight);
  return `color-mix(in srgb, ${baseColor} ${baseWeight}%, ${rainbowColor} ${rainbowWeight}%)`;
}

function hexToHsl(hex) {
  const value = Number.parseInt(hex.slice(1), 16);
  const red = ((value >> 16) & 255) / 255;
  const green = ((value >> 8) & 255) / 255;
  const blue = (value & 255) / 255;
  const max = Math.max(red, green, blue);
  const min = Math.min(red, green, blue);
  const delta = max - min;
  const lightness = (max + min) / 2;

  if (delta === 0) return [0, 0, lightness];

  let hue;
  if (max === red) hue = ((green - blue) / delta) % 6;
  else if (max === green) hue = (blue - red) / delta + 2;
  else hue = (red - green) / delta + 4;

  hue = (hue * 60 + 360) % 360;
  const saturation = delta / (1 - Math.abs(2 * lightness - 1));
  return [hue, saturation, lightness];
}

function rotateRainbowColor(stop, phase) {
  const normalizedPhase = (((Number(phase) || 0) % 360) + 360) % 360;
  if (normalizedPhase === 0) return stop.color;

  const [hue, saturation, lightness] = stop.hsl;
  const [red, green, blue] = hslToRgb(hue + normalizedPhase, saturation, lightness);
  return `rgb(${red},${green},${blue})`;
}

export function hslToRgb(hue, saturation, lightness) {
  const h = (((hue % 360) + 360) % 360) / 360;
  const s = Math.min(1, Math.max(0, saturation));
  const l = Math.min(1, Math.max(0, lightness));

  if (s === 0) {
    const channel = Math.round(l * 255);
    return [channel, channel, channel];
  }

  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const hueToChannel = (offset) => {
    let t = offset;
    if (t < 0) t += 1;
    if (t > 1) t -= 1;
    if (t < 1 / 6) return p + (q - p) * 6 * t;
    if (t < 1 / 2) return q;
    if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
    return p;
  };

  return [
    Math.round(hueToChannel(h + 1 / 3) * 255),
    Math.round(hueToChannel(h) * 255),
    Math.round(hueToChannel(h - 1 / 3) * 255),
  ];
}

export function nyancatRainbowRgb(time, index, count, saturation = 0.96, lightness = 0.62) {
  const position = index / Math.max(1, count);
  const hue = (time / 18 + position * 360) % 360;
  return hslToRgb(hue, saturation, lightness);
}

export function createNyanCatSeekStyle({
  percentage,
  thumbSize,
  playedColor,
  unplayedColor,
  active,
  mix,
  phase = 0,
}) {
  const progress = Math.min(100, Math.max(0, Number(percentage) || 0));
  const rainbowMix = Math.min(
    1,
    Math.max(0, mix === undefined ? (active ? 1 : 0) : Number(mix) || 0)
  );
  const halfThumb = thumbSize / 2;
  // Equivalent to P * (width - thumb) + halfThumb. Keeping the thumb
  // correction makes the painted edge line up with the native thumb centre.
  const pixelCorrection = Math.round((halfThumb - (progress * thumbSize) / 100) * 10000) / 10000;
  const fillStop = createStopPosition(progress, pixelCorrection, 1);

  if (rainbowMix > 0) {
    // Use one gradient instead of stacked background images. On some WebViews
    // an invalid background-size causes the top rainbow layer to fall back to
    // full width. A hard colour stop guarantees the rainbow ends at progress
    // and the rest is exactly the player's original unplayed colour.
    const rainbowStops = NYANCAT_RAINBOW_STOPS.map(
      (stop) =>
        `${blendColor(playedColor, rotateRainbowColor(stop, phase), rainbowMix)} ${createStopPosition(
          progress,
          pixelCorrection,
          stop.fraction
        )}`
    ).join(', ');
    const glowMix = roundPosition(rainbowMix);

    return {
      background: `linear-gradient(to right, ${rainbowStops}, ${unplayedColor} ${fillStop}, ${unplayedColor} 100%)`,
      boxShadow: `0 0 7px rgba(49,210,255,${roundPosition(
        0.58 * glowMix
      )}), 0 0 13px rgba(210,82,255,${roundPosition(0.32 * glowMix)})`,
    };
  }

  return {
    // Deliberately use the background shorthand, matching the original player
    // implementation. This clears the native white background underneath the
    // translucent unplayed colour instead of compositing on top of it.
    background: `linear-gradient(to right, ${playedColor} ${fillStop}, ${unplayedColor} ${fillStop})`,
  };
}
