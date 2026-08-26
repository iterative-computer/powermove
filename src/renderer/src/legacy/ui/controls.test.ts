import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './controls';

function controlsRegistry(): PMRegistry {
  const PM: PMRegistry = {
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
  };
  install(PM);
  return PM;
}

describe('legacy controls install', () => {
  it('round-trips hex, RGB, and HSB channels', () => {
    const color = controlsRegistry().Color;

    for (const hex of ['#FF6B1A', '#3366CC', '#09090A']) {
      const rgb = color.hexToRgb(hex);
      const roundTrip = color.hexToRgb(color.rgbToHex(color.hsvToRgb(color.rgbToHsv(rgb))));
      expect({
        direct: color.rgbToHex(rgb),
        withinTolerance: Math.max(...['r', 'g', 'b'].map(key => Math.abs(roundTrip[key] - rgb[key]))) <= 2,
      }).toEqual({ direct: hex, withinTolerance: true });
    }

    expect(color.normalizeHex('#f60')).toBe('#FF6600');
    expect(color.normalizeHex('not-a-color')).toBeNull();
  });

  it('clamps channel edits to their valid ranges', () => {
    const color = controlsRegistry().Color;

    expect(color.rgbToHex({ r: 999, g: -10, b: 127.6 })).toBe('#FF0080');
    expect(color.hsvToRgb({ h: 120, s: 100, v: 100 })).toEqual({ r: 0, g: 255, b: 0 });
  });
});
