// Pull a small, vibrant 3-color palette out of an image (album art) for the
// Apple-Music-style animated gradient backdrop. Shared by the fullscreen player
// and the mini player so both derive identical colors and transition the same way.
//
// The palette is computed natively in Rust (`get_track_palette`) from the cached
// cover thumbnail — no second decode in a webview canvas, and the result is
// cached on disk keyed by mtime+size. `extractColorsFromImage` remains as a
// canvas fallback for the rare case the native call fails (or when only a URL,
// not a track path, is available).

import { invoke } from '@tauri-apps/api/core';

const DEFAULT_PALETTE = ['#ff2d55', '#5856d6', '#007aff'];

export function defaultPalette() {
  return [...DEFAULT_PALETTE];
}

// Preferred path: derive the palette natively from a track's file path. Falls
// back to the canvas extractor (using `fallbackUrl`, if given) when the native
// command errors, and to the default palette when the track has no cover art.
export async function extractColorsForPath(path, fallbackUrl = null) {
  if (!path) {
    return fallbackUrl ? extractColorsFromImage(fallbackUrl) : defaultPalette();
  }
  try {
    const colors = await invoke('get_track_palette', { path });
    if (Array.isArray(colors) && colors.length === 3) return colors;
    // null = no cover art on this track.
    return defaultPalette();
  } catch (e) {
    console.error('Native palette failed, falling back to canvas', e);
    return fallbackUrl ? extractColorsFromImage(fallbackUrl) : defaultPalette();
  }
}

export function extractColorsFromImage(url) {
  if (!url) {
    return Promise.resolve([...DEFAULT_PALETTE]);
  }
  return new Promise((resolve) => {
    const img = new Image();
    img.crossOrigin = 'anonymous';
    img.onload = () => {
      try {
        const canvas = document.createElement('canvas');
        canvas.width = 12;
        canvas.height = 12;
        const ctx = canvas.getContext('2d');
        if (!ctx) {
          resolve([...DEFAULT_PALETTE]);
          return;
        }
        ctx.drawImage(img, 0, 0, 12, 12);
        const imgData = ctx.getImageData(0, 0, 12, 12).data;

        const pxs = [];
        for (let i = 0; i < imgData.length; i += 4) {
          const r = imgData[i];
          const g = imgData[i + 1];
          const b = imgData[i + 2];
          const a = imgData[i + 3];
          if (a < 150) continue;

          const max = Math.max(r, g, b);
          const min = Math.min(r, g, b);
          const saturation = max - min;
          const brightness = (r + g + b) / 3;

          // Ignore extreme blacks/whites/greys for vibrancy
          if (brightness > 240 && saturation < 20) continue;
          if (brightness < 15 && saturation < 10) continue;

          pxs.push({ r, g, b, saturation, brightness });
        }

        if (pxs.length === 0) {
          for (let i = 0; i < imgData.length; i += 4) {
            const r = imgData[i];
            const g = imgData[i + 1];
            const b = imgData[i + 2];
            pxs.push({ r, g, b, saturation: Math.max(r, g, b) - Math.min(r, g, b), brightness: (r + g + b) / 3 });
          }
        }

        pxs.sort((a, b) => b.saturation - a.saturation);

        const chosen = [];
        for (const p of pxs) {
          const isSimilar = chosen.some((c) => {
            const dr = c.r - p.r;
            const dg = c.g - p.g;
            const db = c.b - p.b;
            return Math.sqrt(dr * dr + dg * dg + db * db) < 65;
          });
          if (!isSimilar) {
            chosen.push(p);
            if (chosen.length >= 3) break;
          }
        }

        if (chosen.length < 3) {
          for (const p of pxs) {
            if (!chosen.includes(p)) {
              chosen.push(p);
              if (chosen.length >= 3) break;
            }
          }
        }

        while (chosen.length < 3) {
          chosen.push({ r: 60, g: 60, b: 60, saturation: 0, brightness: 60 });
        }

        resolve(chosen.map((c) => `rgb(${c.r}, ${c.g}, ${c.b})`));
      } catch (e) {
        console.error('Color extraction failed', e);
        resolve([...DEFAULT_PALETTE]);
      }
    };
    img.onerror = () => {
      resolve([...DEFAULT_PALETTE]);
    };
    img.src = url;
  });
}
