import { describe, expect, it } from 'vitest';
import { makePM } from '../__tests__/make-pm';
import { drawnShapeCommand, resizeParametricShape, type ShapeHandle } from './parametric-shapes';
import { isProperty } from './content-properties';

function fixture() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/compositor');
  PM.proj = PM.mkProject({ w: 640, h: 480, dur: 5 }); PM.time = 2;
  const layer = PM.mkLayer('shape', { from: 1, dur: 4, d: { w: 100, h: 80, stroke: 0 }, p: { 'position.x': 200, 'position.y': 150 } });
  PM.proj.layers = [layer]; PM.ProjectIndex.invalidate(); PM.touch();
  const context = () => ({ time: 2, parentMatrix: PM.transformParentMatrix(layer, 2),
    evaluate: (l: any, path: string, t: number) => path.startsWith('c.')
      ? isProperty(l.d[path.slice(2)]) ? PM.evP(l, l.d[path.slice(2)], t, path) : l.d[path.slice(2)] : PM.ev(l, path, t) });
  const point = (x: number, y: number) => {
    PM.beginEval(2);
    const bounds = PM.GL.bounds(layer, 2), m = PM.worldMatrix(layer, 2);
    const lx = bounds.x0 + x * bounds.w, ly = bounds.y0 + y * bounds.h;
    return { x: m[0] * lx + m[2] * ly + m[4], y: m[1] * lx + m[3] * ly + m[5] };
  };
  return { PM, layer, context, point };
}

it.each(['rect', 'ellipse', 'polygon', 'star'])('writes drawn %s dimensions with scale 100', shape => {
  const { PM } = fixture();
  const result = PM.Edit.apply({ type: 'add_layer', layerType: 'shape', content: { shape, w: 100, h: 100, radius: 20 },
    properties: { 'position.x': 230, 'position.y': 170, 'scale.x': 310, 'scale.y': 90 } }, { origin: 'canvas' });
  expect(result.ok).toBe(true);
  const added = PM.proj.layers.find((l: any) => l.d.w === 310);
  expect(added.d).toMatchObject({ w: 310, h: 90, radius: 18 });
  expect(PM.proj.edits.at(-1).operations[0].content).toMatchObject({ w: 310, h: 90 });
  expect(added.p['scale.x'].v).toBe(100); expect(added.p['scale.y'].v).toBe(100);
  expect(PM.GL.bounds(added, 2)).toMatchObject({ w: 310, h: 90 });
  PM.hist.undo(); expect(PM.proj.layers.some((l: any) => l.id === added.id)).toBe(false);
  PM.hist.redo(); expect(PM.L(added.id).d.w).toBe(310);
});

it('does not reinterpret explicit scale commands from other origins or mutate the command', () => {
  const command = { type: 'add_layer', layerType: 'shape', content: { w: 100, h: 100 }, properties: { 'scale.x': 200 } };
  expect(drawnShapeCommand(command, 'agent')).toBe(command);
  expect(drawnShapeCommand(command, 'canvas').content.w).toBe(200);
  expect(command.content.w).toBe(100);
});

describe('parametric shape resize geometry', () => {
  it.each(['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'] as ShapeHandle[])('keeps the opposite handle fixed for %s with anchor, rotation, skew and parent transforms', handle => {
    const { PM, layer, context, point } = fixture();
    const parent = PM.mkLayer('null', { p: { rotation: -23, 'scale.x': 130, 'scale.y': 80 } });
    PM.proj.layers.push(parent); layer.parent = parent.id;
    layer.p.rotation.v = 37; layer.p.skew.v = 12;
    layer.p['scale.x'].v = -140; layer.p['scale.y'].v = 70;
    layer.p['anchor.x'].v = -50; layer.p['anchor.y'].v = 20; PM.touch();
    const hx = handle.includes('w') ? -1 : handle.includes('e') ? 1 : 0;
    const hy = handle.includes('n') ? -1 : handle.includes('s') ? 1 : 0;
    const before = point((1 - hx) / 2, (1 - hy) / 2);
    const snapshot = JSON.stringify(layer);
    const result = resizeParametricShape(layer, handle, { x: 21, y: -15 }, {}, context())!;
    expect(JSON.stringify(layer)).toBe(snapshot);
    expect(PM.Edit.apply(result.commands, { origin: 'canvas' }).ok).toBe(true);
    const after = point((1 - hx) / 2, (1 - hy) / 2);
    expect(after.x).toBeCloseTo(before.x, 8); expect(after.y).toBeCloseTo(before.y, 8);
    expect(layer.p['scale.x'].v).toBe(-140); expect(layer.p['scale.y'].v).toBe(70);
  });

  it('moves the dragged corner by the exact composition delta', () => {
    const { PM, layer, context, point } = fixture(); layer.p.rotation.v = 32; PM.touch();
    const before = point(1, 1), result = resizeParametricShape(layer, 'se', { x: 32, y: 16 }, {}, context())!;
    PM.Edit.apply(result.commands, { origin: 'canvas' });
    const after = point(1, 1);
    expect(after.x - before.x).toBeCloseTo(32, 8); expect(after.y - before.y).toBeCloseTo(16, 8);
  });

  it.each(['e', 'se'] as ShapeHandle[])('supports shift proportions and alt from center for %s', handle => {
    const { PM, layer, context, point } = fixture();
    layer.p['anchor.x'].v = 90; layer.p.rotation.v = 28; PM.touch();
    const before = point(.5, .5), result = resizeParametricShape(layer, handle, { x: 30, y: 20 }, { shift: true, alt: true }, context())!;
    expect(result.width / result.height).toBeCloseTo(1.25, 9);
    PM.Edit.apply(result.commands, { origin: 'canvas' }); const after = point(.5, .5);
    expect(after.x).toBeCloseTo(before.x, 8); expect(after.y).toBeCloseTo(before.y, 8);
  });

  it('updates animated width, height and position keys at the current layer-local time', () => {
    const { PM, layer, context } = fixture();
    layer.d.w = PM.P(100, { kf: [PM.KF(0, 100), PM.KF(3, 160)] });
    layer.d.h = PM.P(80, { kf: [PM.KF(0, 80), PM.KF(3, 110)] });
    for (const path of ['position.x', 'position.y']) layer.p[path].kf = [PM.KF(0, layer.p[path].v), PM.KF(3, layer.p[path].v + 30)];
    PM.touch(); const result = resizeParametricShape(layer, 'se', { x: 30, y: 20 }, {}, context())!;
    expect(result.width).toBeCloseTo(150); expect(result.height).toBeCloseTo(110);
    expect(PM.Edit.apply(result.commands, { origin: 'canvas' }).ok).toBe(true);
    for (const prop of [layer.d.w, layer.d.h, layer.p['position.x'], layer.p['position.y']]) {
      expect(prop.kf).toHaveLength(3); expect(prop.kf[1].t).toBe(1);
    }
    PM.hist.undo(); expect(PM.L(layer.id).d.w.kf).toHaveLength(2);
  });

  it('keeps a pre-existing size-anchor reference stable on subsequent drags', () => {
    const { PM, layer, context, point } = fixture();
    layer.p['anchor.x'].v = -50;
    PM.Edit.apply({ type: 'set_content', target: layer.id, patch: { w: 200 } });
    const before = point(0, .5), result = resizeParametricShape(layer, 'e', { x: 50, y: 0 }, {}, context())!;
    PM.Edit.apply(result.commands, { origin: 'canvas' });
    expect(point(0, .5).x).toBeCloseTo(before.x, 8);
  });

  it('clamps crossing handles and refuses singular transforms or unevaluated animation', () => {
    const { layer } = fixture();
    expect(resizeParametricShape(layer, 'w', { x: 1000, y: 0 })!.width).toBe(.01);
    layer.p['scale.x'].v = 0; expect(resizeParametricShape(layer, 'e', { x: 1, y: 0 })).toBeNull();
    layer.p['scale.x'].v = 100; layer.d.w = { v: 100, kf: [{ t: 0, v: 100 }], expr: null };
    expect(resizeParametricShape(layer, 'e', { x: 1, y: 0 })).toBeNull();
  });
});

it('does not silently overwrite expression-driven dimensions', () => {
  const { layer, context } = fixture();
  layer.d.w = { v: 100, kf: [], expr: '100 + t * 5' };
  expect(resizeParametricShape(layer, 'e', { x: 10, y: 0 }, {}, context())).toBeNull();
});
