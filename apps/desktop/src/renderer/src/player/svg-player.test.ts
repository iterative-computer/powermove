// @vitest-environment happy-dom
import { expect, it, vi } from 'vitest';
import { createEngine } from './player';
import { createSvgPlayer } from './svg-player';
import type { WebScene } from './scene';

/* The scene the SVG player reads: the engine's serialized project and no packaged assets. */
function sceneOf(PM: any): WebScene {
  const serialized = PM.serialize();
  const snapshot = typeof serialized === 'string' ? JSON.parse(serialized) : structuredClone(serialized);
  return { format: 'powermove-web', version: 1, project: snapshot.proj || snapshot, assets: {}, fonts: [], effects: [], transitions: [], layerTypes: [], warnings: [] };
}

it('preserves a corner anchor through SVG playback and animated resizing', async () => {
  const PM = createEngine(null);
  PM.proj = PM.mkProject({ w: 128, h: 128, dur: 3 });
  const layer = PM.mkLayer('shape', { d: { shape: 'rect', w: 20, h: 20, radius: 0, stroke: 0,
    sizeAnchorBounds: { x0: -10, x1: 10, y0: -10, y1: 10 } },
    p: { 'anchor.x': -10, 'anchor.y': -10 } }, PM.proj);
  layer.d.w = PM.P(20, { kf: [PM.KF(0, 20, 'linear'), PM.KF(2, 60, 'linear')] });
  PM.proj.layers = [layer];
  const scene = sceneOf(PM);
  expect((scene.project.layers[0] as any).d.sizeAnchorBounds).toEqual(layer.d.sizeAnchorBounds);
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  const player = await createSvgPlayer({ svg, scene });
  try {
    for (const time of [0, 1, 2]) {
      await player.seek(time);
      const rect = svg.querySelector('[data-layer-id] rect')!;
      expect(rect.getAttribute('transform')).toBe('translate(-10 -10)');
      expect(Number(rect.getAttribute('width'))).toBeCloseTo(20 + time * 20);
    }
  } finally { player.destroy(); }
});

/* happy-dom has no fonts or canvas: measure every grapheme as 20px wide. */
function stubTextMeasurement() {
  const fonts = Object.getOwnPropertyDescriptor(document, 'fonts');
  Object.defineProperty(document, 'fonts', { configurable: true, value: { load: async () => [], check: () => true, ready: Promise.resolve(), add() {}, delete() {} } });
  const context = { font: '', textAlign: 'left', textBaseline: 'alphabetic', letterSpacing: '0px',
    measureText: (text: string) => ({ width: [...text].length * 20, actualBoundingBoxLeft: 0, actualBoundingBoxRight: [...text].length * 20, actualBoundingBoxAscent: 30, actualBoundingBoxDescent: 8 }) };
  const spy = vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(context as any);
  return () => { spy.mockRestore(); if (fonts) Object.defineProperty(document, 'fonts', fonts); else delete (document as any).fonts; };
}

it('draws stagger-animated text glyph by glyph and settles to rest', async () => {
  const restore = stubTextMeasurement();
  const PM = createEngine(null);
  PM.proj = PM.mkProject({ w: 320, h: 120, dur: 3 });
  const layer = PM.mkLayer('text', { d: { text: 'Hi yo', size: 40, color: '#ffffff', align: 'left' } }, PM.proj);
  layer.d.animators = [{ id: 'a', name: 'Rise', enabled: true, mode: 'stagger', unit: 'characters', order: 'forward', direction: 'in', easing: 'linear',
    p: Object.fromEntries(Object.entries({ amount: 100, delay: 0, duration: 1, stagger: 0.5, opacity: 0, y: 20, blur: 6 }).map(([key, value]) => [key, PM.P(value)])) }];
  PM.proj.layers = [layer];
  const scene = sceneOf(PM);
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  const player = await createSvgPlayer({ svg, scene });
  try {
    const glyphs = () => [...svg.querySelectorAll('[data-layer-id] g > text')];
    await player.seek(0.5);
    // Only the first glyph has started: half faded, half risen, still blurred.
    expect(glyphs().map((glyph) => glyph.textContent)).toEqual(['H']);
    expect(Number(glyphs()[0]!.getAttribute('opacity'))).toBeCloseTo(0.5);
    expect(glyphs()[0]!.getAttribute('transform')).toBe(`matrix(1 0 0 1 0 ${40 * .82 + 10})`);
    expect(svg.querySelectorAll('feGaussianBlur')).toHaveLength(1);
    await player.seek(2.9);
    expect(glyphs().map((glyph) => glyph.textContent)).toEqual(['H', 'i', 'y', 'o']);
    expect(glyphs().every((glyph) => !glyph.hasAttribute('opacity') && !glyph.hasAttribute('filter'))).toBe(true);
    // At rest each glyph sits at its measured advance on the baseline.
    expect(glyphs().map((glyph) => glyph.getAttribute('transform'))).toEqual([0, 20, 60, 80].map((x) => `matrix(1 0 0 1 ${x} ${40 * .82})`));
  } finally { player.destroy(); restore(); }
});
