// @vitest-environment happy-dom
import { readFileSync } from 'node:fs';
import { create, type Font } from 'fontkit';
import { describe, it, expect } from 'vitest';
import { fontOutlines } from './extrusion-outlines';
import { buildExtrusionGeometry } from './extrusion-geometry';
import { EXTRUSION_DEFAULTS } from './extrusion-data';

describe('extrusion with packaged variable fonts', () => {
  it.each(['Inter', 'Geist', 'GeistMono'])('%s has finite geometry across weights and bevel profiles', name => {
    const font = create(readFileSync(`assets/fonts/${name}-Variable.ttf`)) as Font;
    for (const weight of [100, 400, 900]) for (const bevelStyle of ['none', 'convex', 'concave'] as const) {
      const outlines = fontOutlines(font, { text: 'Powermove B8%@ ffi', size: 100, weight, 'fontAxis.opsz': 32 });
      const geometry = buildExtrusionGeometry(outlines, { ...EXTRUSION_DEFAULTS, depth: 40, bevelDepth: 4, bevelStyle });
      for (const name of ['position', 'normal', 'uv']) expect([...geometry.getAttribute(name).array].every(Number.isFinite)).toBe(true);
      expect(geometry.getAttribute('position').count).toBeGreaterThan(100);
      geometry.dispose();
    }
  });
  it.each(['Inter', 'Geist', 'GeistMono'])('%s keeps a High-quality title within the artwork budget', name => {
    const font = create(readFileSync(`assets/fonts/${name}-Variable.ttf`)) as Font;
    const outlines = fontOutlines(font, { text: 'Studio Motion', size: 100, weight: 700 }, 20);
    const geometry = buildExtrusionGeometry(outlines, { ...EXTRUSION_DEFAULTS, quality: 'high', depth: 40, bevelDepth: 3, bevelStyle: 'convex' });
    expect(geometry.getAttribute('position').count).toBeLessThan(250_000);
    expect(geometry.groups.map(group => group.materialIndex)).toEqual([0, 1, 2, 3]);
    geometry.dispose();
  });
  it('also extrudes the static font fixture', () => {
    const font = create(readFileSync('e2e/fixtures/font-registry.ttf')) as Font;
    expect(Object.keys(font.variationAxes)).toHaveLength(0);
    const geometry = buildExtrusionGeometry(fontOutlines(font, { text: 'B8% Static', size: 80 }), { ...EXTRUSION_DEFAULTS, depth: 20 });
    expect(geometry.getAttribute('position').count).toBeGreaterThan(0);
    geometry.dispose();
  });
});
