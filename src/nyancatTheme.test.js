import { describe, expect, it } from 'vitest';
import { createNyanCatSeekStyle } from './nyancatTheme';

const baseOptions = {
  percentage: 45,
  thumbSize: 12,
  playedColor: '#fff',
  unplayedColor: 'rgba(255,255,255,0.28)',
};

describe('createNyanCatSeekStyle', () => {
  it('uses one clipped gradient instead of a full-width stacked layer', () => {
    const style = createNyanCatSeekStyle({ ...baseOptions, active: true });

    expect(style.background).toContain('#ff2d55 0%');
    expect(style.background).toContain('#ff2d55 calc(45% + 0.6px)');
    expect(style.background).toContain(
      `${baseOptions.unplayedColor} calc(45% + 0.6px), ${baseOptions.unplayedColor} 100%`
    );
    expect(style).not.toHaveProperty('backgroundImage');
    expect(style).not.toHaveProperty('backgroundSize');
  });

  it('formats negative thumb correction as valid calc subtraction', () => {
    const style = createNyanCatSeekStyle({ ...baseOptions, percentage: 75, active: true });

    expect(style.background).toContain('#ff2d55 calc(75% - 3px)');
    expect(style.background).toContain(`${baseOptions.unplayedColor} calc(75% - 3px)`);
  });

  it('uses the background shorthand and the same unplayed color in both modes', () => {
    const normal = createNyanCatSeekStyle({ ...baseOptions, active: false });
    const nyancat = createNyanCatSeekStyle({ ...baseOptions, active: true });

    expect(normal).toEqual({
      background: `linear-gradient(to right, #fff calc(45% + 0.6px), rgba(255,255,255,0.28) calc(45% + 0.6px))`,
    });
    expect(nyancat.background).toContain(baseOptions.unplayedColor);
  });

  it('crossfades only played colors while keeping unplayed unchanged', () => {
    const style = createNyanCatSeekStyle({ ...baseOptions, mix: 0.5 });

    expect(style.background).toContain('color-mix(in srgb, #fff 50%, #ff2d55 50%)');
    expect(style.background).toContain(
      `${baseOptions.unplayedColor} calc(45% + 0.6px), ${baseOptions.unplayedColor} 100%`
    );
    expect(style.boxShadow).toContain('rgba(49,210,255,0.29)');
  });

  it('rotates rainbow stops without hue-shifting the normal played color', () => {
    const style = createNyanCatSeekStyle({ ...baseOptions, mix: 0.01, phase: 180 });

    expect(style.background).toContain('color-mix(in srgb, #fff 99%, rgb(');
    expect(style.background).toContain(') 1%)');
    expect(style).not.toHaveProperty('filter');
  });
});
