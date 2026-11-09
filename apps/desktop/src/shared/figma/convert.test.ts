import { describe, it, expect } from 'vitest';
import { convertFigma, unsupportedReasons, clipboardFallbacks } from './convert';
import { validateFigmaDoc, type FigmaDoc, type FigmaNode } from './doc';
export const node = (patch: Partial<FigmaNode> = {}): FigmaNode => ({
  id: '1',
  fileKey: 'synthetic-file',
  type: 'RECTANGLE',
  name: 'Card',
  w: 100,
  h: 100,
  absoluteTransform: [1, 0, 0, 1, 0, 0],
  fills: [{ type: 'SOLID', color: { r: 1, g: 0, b: 0 } }],
  strokes: [],
  effects: [],
  children: [],
  ...patch,
});
export const doc = (nodes = [node()]): FigmaDoc => ({
  format: 'powermove-figma',
  version: 1,
  source: 'plugin',
  nodes,
  images: {},
});
const plan = (n: FigmaNode) => convertFigma(doc([n]), { w: 640, h: 360 });
describe('host-independent Figma commands', () => {
  it('validates and retains provenance without UI or registry state', () => {
    expect(validateFigmaDoc(doc())).toEqual(doc());
    expect(plan(node()).commands[0]).toMatchObject({
      type: 'add_layer',
      source: { nodeId: '1', fileKey: 'synthetic-file' },
      content: { color: '#FF0000FF' },
    });
  });
  it('keeps equal Figma side widths as a uniform editable stroke', () => {
    const stroke = { type: 'SOLID', color: { r: 0, g: 0, b: 1 } };
    const uniform = plan(node({
      strokes: [stroke],
      strokeWeight: 10,
      strokeWeights: [10, 10, 10, 10],
      strokeAlign: 'INSIDE',
      dashes: [18, 10],
    })).commands[0]!;
    expect(uniform.content.stroke).toBe(10);
    expect(uniform.content.strokeStyle).toMatchObject({ align: 'inside', dashes: [18, 10] });
    expect(uniform.content.strokeStyle).not.toHaveProperty('weights');
    const independent = plan(node({
      strokes: [stroke],
      strokeWeights: [2, 4, 6, 8],
      strokeAlign: 'INSIDE',
    })).commands[0]!;
    expect(independent.content.stroke).toBe(8);
    expect(independent.content.strokeStyle).toMatchObject({ weights: [2, 4, 6, 8] });
  });
  it('maps opacity and every engine blend mode', () => {
    for (const mode of [
      'NORMAL',
      'MULTIPLY',
      'SCREEN',
      'OVERLAY',
      'SOFT_LIGHT',
      'DIFFERENCE',
      'LIGHTEN',
      'DARKEN',
      'LINEAR_DODGE',
    ]) {
      const c = plan(node({ opacity: 0.4, blendMode: mode })).commands[0]!;
      expect(c.properties.opacity).toBe(40);
      expect(c.layerType).toBe('shape');
      expect(c.blend).not.toBeUndefined();
    }
  });
  it.each(['GRADIENT_LINEAR', 'GRADIENT_RADIAL', 'GRADIENT_ANGULAR', 'GRADIENT_DIAMOND'])('maps a %s as shape paint data', (type) => {
    const p = plan(
      node({
        fills: [
          {
            type,
            gradientTransform: [1, 0, 0, 1, 0, 0],
            gradientStops: [
              { position: 0, color: { r: 1, g: 0, b: 0 } },
              { position: 1, color: { r: 0, g: 0, b: 1 } },
            ],
          },
        ],
      }),
    );
    expect(p.issues).toEqual([]);
    expect(p.commands[0]!.effects).toEqual([]);
    expect(p.commands[0]!.content.fillGradient).toMatchObject({type:type.slice(9).toLowerCase(),transform:[1,0,0,1,0,0],stops:[{position:0,color:'#FF0000FF'},{position:1,color:'#0000FFFF'}]});
  });
  it('maps shadow and layer blur in authored order', () => {
    const p = plan(
      node({
        effects: [
          { type: 'DROP_SHADOW', radius: 8, color: { r: 0, g: 0, b: 0, a: 0.5 }, offset: { x: 3, y: 4 } },
          { type: 'LAYER_BLUR', radius: 12 },
        ],
      }),
    );
    expect(p.issues).toEqual([]);
    expect(p.commands[0]!.effects.map((e) => e.type)).toEqual(['figmaShadow', 'blur']);
    expect(p.commands[0]!.effects[0]!.params).toMatchObject({ offsetX: 3, offsetY: 4, radius: 8, opacity: 50 });
  });
  it('creates rounded clipping mattes for overflowing children and keeps instances editable', () => {
    const p = plan(
      node({
        type: 'INSTANCE',
        radii: [20, 20, 20, 20],
        clipsContent: true,
        children: [node({ id: 'child', absoluteTransform: [1, 0, 0, 1, 80, 0] })],
      }),
    );
    expect(p.issues).toEqual([]);
    expect(p.commands[0]!.layerType).toBe('group');
    expect(p.commands.find((c) => c.ref === 'child:node')!.matte).toBe('1:clip');
    expect(p.commands.find((c) => c.ref === '1:clip')!.content.radius).toBe(20);
  });
  it('uses the preceding alpha mask across siblings in the correct stack order', () => {
    const p = convertFigma(doc([node({ id: 'mask', isMask: true, maskType: 'ALPHA' }), node({ id: 'paint' })]), {
      w: 640,
      h: 360,
    });
    expect(p.commands.map((c) => c.ref)).toEqual(['paint:node', 'mask:node']);
    expect(p.commands[0]!.matte).toBe('mask:node');
  });
  it('splits multiple primitive paints into editable grouped layers', () => {
    const p = plan(
      node({
        fills: [
          { type: 'SOLID', color: { r: 1, g: 0, b: 0 } },
          { type: 'SOLID', color: { r: 0, g: 0, b: 1 } },
        ],
        strokes: [{ type: 'SOLID', color: { r: 0, g: 1, b: 0 } }],
        strokeAlign: 'CENTER',
        join: 'ROUND',
      }),
    );
    expect(p.issues).toEqual([]);
    expect(p.commands.map((c) => c.ref)).toEqual(['1:node', '1:stroke-0', '1:fill-1', '1:fill-0']);
  });
  it('maps editable vector points', () => {
    const c = plan(node({ type: 'VECTOR', paths: [{ data: 'M0 0 L100 0 L0 100 Z', windingRule: 'NONZERO' }] }))
      .commands[0]!;
    expect(c.content.paths[0].vertices).toHaveLength(3);
  });
  it('maps mixed text colors to editable text style ranges', () => {
    const p = plan(
      node({
        type: 'TEXT',
        text: {
          characters: 'Red Blue',
          runs: [
            {
              start: 0,
              end: 4,
              font: 'Arial',
              style: 'Regular',
              size: 20,
              fills: [{ type: 'SOLID', color: { r: 1, g: 0, b: 0 } }],
            },
            {
              start: 4,
              end: 8,
              font: 'Arial',
              style: 'Regular',
              size: 20,
              fills: [{ type: 'SOLID', color: { r: 0, g: 0, b: 1 } }],
            },
          ],
        },
      }),
    );
    expect(p.issues).toEqual([]);
    expect(p.commands[0]!.content.runs[1]).toMatchObject({ start: 4, end: 8, color: '#0000FFFF' });
  });
  it.each([
    {effects:[{type:'GLASS'}]}, {effects:[{type:'TEXTURE'}]},
    { fills: [{ type: 'GRADIENT_ANGULAR' }] },
    { type: 'VECTOR', paths: [] },
    { type:'UNKNOWN' }, {blendMode:'FUTURE_BLEND'},
  ] as Partial<FigmaNode>[])('flattens just the unsupported node: %j', (patch) => {
    const n = node({
      strokes: [{ type: 'SOLID', color: { r: 0, g: 0, b: 0 } }],
      ...patch,
      raster: { png: 'iVBORw0KGgo=', bounds: { x: 0, y: 0, w: 100, h: 100 } },
    });
    expect(unsupportedReasons(n).length).toBeGreaterThan(0);
    const p = convertFigma(doc([n, node({ id: 'editable', absoluteTransform: [1, 0, 0, 1, 200, 0] })]), {
      w: 640,
      h: 360,
    });
    expect(p.commands.map((c) => c.layerType)).toEqual(['shape', 'image']);
    expect(p.commands[1]!.raster?.png).toBe(n.raster!.png);
  });
  it('requires actual text styles while allowing nested matte intersections', () => {
    expect(unsupportedReasons(node({ type: 'TEXT', text: { characters: 'a', runs: [] } }))).toContain(
      'missing text styles',
    );
    expect(
      unsupportedReasons(node({ type: 'FRAME', clipsContent: true, children: [node({ clipsContent: true })] })),
    ).toEqual([]);
  });
  it('promotes unsafe clipboard crops to the smallest enclosing subtree', () => {
    const root = node({
      type: 'GROUP',
      children: [
        node({ id: 'a', effects: [{ type: 'GLASS' }] }),
        node({ id: 'b', absoluteTransform: [1, 0, 0, 1, 40, 0] }),
      ],
    });
    const prepared = clipboardFallbacks({
      ...doc([root, node({ id: 'separate', absoluteTransform: [1, 0, 0, 1, 400, 0] })]),
      source: 'clipboard',
    });
    expect(prepared.whole).toBe(false);
    expect(prepared.nodes[0]!.unsupported).toContain('overlapping clipboard preview');
  });
  it('uses a whole composite only when no safe enclosing selected subtree exists', () => {
    expect(clipboardFallbacks(doc([node({ effects: [{ type: 'GLASS' }] }), node({ id: 'b' })])).whole).toBe(
      true,
    );
  });
  it('rejects duplicates, unexpected schema fields, invalid numbers and excessive nesting', () => {
    expect(() => validateFigmaDoc(doc([node(), node()]))).toThrow();
    expect(() => validateFigmaDoc({ ...doc(), token: 'x' })).toThrow();
    expect(() => validateFigmaDoc(doc([node({ w: Infinity })]))).toThrow();
    let n = node();
    for (let i = 0; i < 120; i++) n = node({ id: String(i), children: [n] });
    expect(() => validateFigmaDoc(doc([n]))).toThrow();
  });
});
it('promotes clipboard crops that would bake a painted ancestor into an animated child', () => {
  const root = node({ type: 'FRAME', children: [node({ id: 'child', effects: [{ type: 'GLASS' }] })] });
  const prepared = clipboardFallbacks({ ...doc([root]), source: 'clipboard' });
  expect(prepared.nodes[0]!.unsupported).toContain('overlapping clipboard preview');
  expect(prepared.whole).toBe(false);
});
it('keeps a clip nested under a sibling mask editable', () => {
  const clipped = node({
    id: 'frame',
    type: 'FRAME',
    clipsContent: true,
    children: [node({ id: 'child' })],
    raster: { png: 'iVBORw0KGgo=', bounds: { x: 0, y: 0, w: 100, h: 100 } },
  });
  const p = convertFigma(doc([node({ id: 'mask', isMask: true, maskType: 'ALPHA' }), clipped]), { w: 640, h: 360 });
  expect(p.commands[0]).toMatchObject({ layerType: 'group', matte: 'mask:node' });
  expect(p.issues).toEqual([]);
  expect(p.commands.find(c=>c.ref==='child:node')!.matte).toBe('frame:clip');
});

it.each([
  ['WIDTH_AND_HEIGHT', 0, 0],
  ['HEIGHT', 80, 0],
  ['NONE', 80, 60],
  ['TRUNCATE', 80, 60],
] as const)('keeps the %s text sizing mode and typography', (autoResize, boxWidth, boxHeight) => {
  const p = convertFigma(
    doc([
      node({
        type: 'TEXT',
        w: 80,
        h: 60,
        text: {
          characters: 'Hello Figma',
          autoResize,
          leading: 30,
          tracking: 1.5,
          runs: [
            {
              start: 0,
              end: 11,
              font: 'Unavailable synthetic font',
              style: 'Regular',
              size: 24,
              fills: [{ type: 'SOLID', color: { r: 1, g: 1, b: 1 } }],
            },
          ],
        },
      }),
    ]),
    { w: 640, h: 360 },
  );
  expect(p.issues).toEqual([]);
  expect(p.commands[0]!.content).toMatchObject({
    boxWidth,
    boxHeight,
    font: 'Unavailable synthetic font',
    size: 24,
    leading: 1.25,
    tracking: 1.5,
  });
});
