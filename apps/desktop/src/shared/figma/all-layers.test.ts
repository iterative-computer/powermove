import { it, expect } from 'vitest';
import { allFeatures } from './__fixtures__/all-features';
import { convertFigma, fallbackScopes, unsupportedReasons, blendModes } from './convert';
import { validateFigmaDoc, inverse, mul } from './doc';
const fixture = allFeatures();
it.each(fixture.nodes.map((n) => [n.name, n] as const))(
  'maps %s consistently with the plugin capability check',
  (name, node) => {
    const d = validateFigmaDoc({ ...fixture, nodes: [node] }),
      plan = convertFigma(d, { w: 930, h: 680 });
    if (['Glass effect', 'Texture effect'].includes(name)) {
      expect(plan.issues).toHaveLength(1);
      expect(plan.commands[0]!.layerType).toBe('image');
      expect(fallbackScopes(d)).toHaveLength(1);
    } else {
      expect(plan.issues).toEqual([]);
      expect(fallbackScopes(d)).toEqual([]);
      expect(plan.commands.every((c) => c.layerType !== 'image' || !!c.imageHash)).toBe(true);
    }
  },
);
it('preserves arbitrary stop counts, alpha, affine coordinates and per-paint blend modes', () => {
  const node = structuredClone(fixture.nodes[0]!);
  node.fills[0]!.gradientStops = Array.from({ length: 100 }, (_, i) => ({
    position: i / 99,
    color: { r: i / 99, g: 0, b: 1, a: i / 99 },
  }));
  const c = convertFigma({ ...fixture, nodes: [node] }, { w: 100, h: 100 }).commands[0]!;
  expect(c.effects).toEqual([]);
  expect(c.content.fillGradient.stops).toHaveLength(100);
  expect(c.content.fillGradient.stops[0].color).toBe('#0000FF00');
  const m = node.fills[0]!.gradientTransform!;
  expect(mul(m, inverse(m))).toEqual([1, 0, 0, 1, 0, 0]);
});
it('keeps real assets separate from last-resort PNGs and requires missing bytes explicitly', () => {
  const n = fixture.nodes.find((n) => n.name === 'Image CROP')!,
    p = convertFigma({ ...fixture, nodes: [n] }, { w: 100, h: 100 });
  expect(p.commands.find((c) => c.layerType === 'image')).toMatchObject({
    imageHash: 'checker',
    content: { imageMapping: { mode: 'crop', transform: [1, 0, 0, 1, 0.1, 0] } },
  });
  expect(
    convertFigma({ ...fixture, images: {}, nodes: [n] }, { w: 100, h: 100 }).issues[0]!.reason,
  ).toContain('missing image bytes');
  expect(fallbackScopes({ ...fixture, images: {}, nodes: [n] })[0]!.reasons).toContain('missing image bytes');
});
it('preserves mixed text metrics in one source layer', () => {
  const p = convertFigma(
    { ...fixture, nodes: [fixture.nodes.find((n) => n.name === 'Styled text')!] },
    { w: 100, h: 100 },
  );
  expect(p.commands).toHaveLength(1);
  expect(p.commands[0]!.content.runs[1]).toMatchObject({
    size: 24,
    font: 'Arial',
    weight: 700,
    tracking: 1,
    leading: 30,
    decoration: 'UNDERLINE',
    textCase: 'UPPER',
  });
});
it('maps every official Figma blend mode', () => {
  for (const mode of [
    'NORMAL',
    'PASS_THROUGH',
    'DARKEN',
    'MULTIPLY',
    'LINEAR_BURN',
    'COLOR_BURN',
    'LIGHTEN',
    'SCREEN',
    'LINEAR_DODGE',
    'COLOR_DODGE',
    'OVERLAY',
    'SOFT_LIGHT',
    'HARD_LIGHT',
    'DIFFERENCE',
    'EXCLUSION',
    'HUE',
    'SATURATION',
    'COLOR',
    'LUMINOSITY',
  ])
    expect(blendModes[mode]).toBeTruthy();
});

it('uses computed mixed-width stroke geometry as editable shapes with original provenance', () => {
  const node = structuredClone(fixture.nodes[0]!);
  node.fills = [{ type: 'SOLID', color: { r: 1, g: 0, b: 0 } }];
  node.strokes = [{ type: 'SOLID', color: { r: 0, g: 0, b: 1 } }];
  node.blendMode='MULTIPLY';
  node.strokePaths = [{ data: 'M-2-2H92V62H-2Z M2 2V58H88V2Z', windingRule: 'NONZERO' }];
  const p = convertFigma({ ...fixture, nodes: [node] }, { w: 100, h: 100 });
  expect(p.issues).toEqual([]);
  expect(p.commands.map((c) => c.layerType)).toEqual(['group', 'shape', 'shape']);
  expect(p.commands.every((c) => c.source.nodeId === node.id)).toBe(true);
  expect(p.commands.map(c=>c.blend)).toEqual(['multiply','normal','normal']);
  expect(p.commands.find((c) => c.source.role === 'outline:node')!.content.paths).toHaveLength(2);
});
it('maps progressive blur into persisted radius and endpoint channels', () => {
  const node = structuredClone(fixture.nodes[0]!);
  node.effects = [
    {
      type: 'LAYER_BLUR',
      blurType: 'PROGRESSIVE',
      radius: 12,
      startRadius: 1,
      startOffset: { x: 0, y: 0 },
      endOffset: { x: 1, y: 0 },
    },
  ];
  const p = convertFigma({ ...fixture, nodes: [node] }, { w: 100, h: 100 });
  expect(p.issues).toEqual([]);
  expect(p.commands[0]!.effects[0]).toMatchObject({
    type: 'figmaLayerBlur',
    params: { radius: 12, startRadius: 1, progressive: true, endX: 1 },
  });
});
