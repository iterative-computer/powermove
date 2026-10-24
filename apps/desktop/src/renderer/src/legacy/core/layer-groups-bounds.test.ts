import { describe, expect, it } from 'vitest';
import { installLayerGroups } from './layer-groups';
import { isModelGroup, LAYER3D_DEFINITIONS } from '../../core/scene3d/layers';
import { modelGroupBounds } from '../../core/scene3d/service';
import { editScene } from '../../core/scene3d/operations';
import { makePM } from '../__tests__/make-pm';

const identity = [1, 0, 0, 1, 0, 0];
function fixture(layers: any[]) {
  let version = 0;
  const PM: any = {
    proj: { layers, dur: 10 }, curComp() { return this.proj; },
    animVersion: () => version, touch: () => version++,
    worldMatrix: (layer: any) => layer.matrix || identity,
    mul: (a: number[], b: number[]) => [a[0]! * b[0]! + a[2]! * b[1]!, a[1]! * b[0]! + a[3]! * b[1]!,
      a[0]! * b[2]! + a[2]! * b[3]!, a[1]! * b[2]! + a[3]! * b[3]!,
      a[0]! * b[4]! + a[2]! * b[5]! + a[4]!, a[1]! * b[4]! + a[3]! * b[5]! + a[5]!],
    active: (layer: any, time: number) => layer.on !== false && time >= (layer.from ?? 0) && time < (layer.from ?? 0) + (layer.dur ?? 10),
    GL: { bounds: (layer: any) => layer.bounds ?? null },
  };
  installLayerGroups(PM); return PM;
}

// Preserve the original array-based reduction as an independent reference for
// transformed corners, missing geometry, membership and numeric edge cases.
function previousBounds(PM: any, group: any, time: number) {
  if (isModelGroup(PM, group)) return modelGroupBounds(PM, group, time);
  if (!PM.worldMatrix) return null;
  const m = PM.worldMatrix(group, time), det = m[0] * m[3] - m[1] * m[2];
  if (Math.abs(det) < 1e-10) return null;
  const inv = [m[3] / det, -m[1] / det, -m[2] / det, m[0] / det,
    (m[2] * m[5] - m[3] * m[4]) / det, (m[1] * m[4] - m[0] * m[5]) / det];
  const points: { x: number; y: number }[] = [];
  for (const child of PM.curComp().layers) {
    if (child.type === 'group' || !PM.groupAncestors(child, PM.curComp().layers).some((item: any) => item.id === group.id) || !PM.active(child, time)) continue;
    const bounds = PM.GL?.bounds?.(child, time), world = PM.worldMatrix(child, time), local = PM.mul(inv, world);
    if (bounds) for (const x of [bounds.x0, bounds.x1]) for (const y of [bounds.y0, bounds.y1]) points.push({ x: local[0] * x + local[2] * y + local[4], y: local[1] * x + local[3] * y + local[5] });
    else if (child.type !== 'audio') points.push({ x: local[4], y: local[5] });
  }
  if (!points.length) return null;
  const x0 = Math.min(...points.map(p => p.x)), x1 = Math.max(...points.map(p => p.x));
  const y0 = Math.min(...points.map(p => p.y)), y1 = Math.max(...points.map(p => p.y));
  return { x0, y0, x1, y1, w: x1 - x0, h: y1 - y0, ax: 0, ay: 0 };
}
function previousExpand(PM: any, ids: any[]) {
  return PM.proj.layers.filter((layer: any) => ids.includes(layer.id)
    || PM.groupAncestors(layer, PM.proj.layers).some((group: any) => ids.includes(group.id))).map((layer: any) => layer.id);
}
function previousSpan(PM: any, group: any) {
  const ids = new Set(previousExpand(PM, [group.id]));
  const children = PM.proj.layers.filter((layer: any) => ids.has(layer.id) && layer.type !== 'group');
  const from = children.length ? Math.min(...children.map((layer: any) => layer.from)) : 0;
  const end = children.length ? Math.max(...children.map((layer: any) => layer.from + layer.dur)) : PM.proj.dur;
  return { from, dur: end - from };
}
function sameNumbers(actual: any, expected: any) {
  if (expected === null) { expect(actual).toBeNull(); return; }
  expect(Object.keys(actual)).toEqual(Object.keys(expected));
  for (const key of Object.keys(expected)) expect(Object.is(actual[key], expected[key]), key).toBe(true);
}

describe('group bounds and spans', () => {
  it('matches original bounds and spans across nested, inactive, audio and transformed members', () => {
    let seed = 128417;
    const random = () => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed / 2 ** 32; };
    for (let sample = 0; sample < 200; sample++) {
      const matrix = () => Array.from({ length: 6 }, () => random() * 20 - 10);
      const root = { id: 'root', type: 'group', group: null, matrix: matrix() };
      const inner = { id: 'inner', type: 'group', group: 'root' };
      const layers = [root, inner, ...Array.from({ length: 30 }, (_, i) => ({
        id: `layer${i}`, type: i % 7 === 0 ? 'audio' : 'shape', group: ['root', 'inner', null, 'missing'][i % 4],
        matrix: matrix(), on: random() > .2, from: random() * 3, dur: random() * 5,
        bounds: random() > .25 ? { x0: random() * -40, x1: random() * 60, y0: random() * -30, y1: random() * 80 } : null,
      }))];
      const PM = fixture(layers), time = random() * 8;
      sameNumbers(PM.groupBounds(root, time), previousBounds(PM, root, time));
      sameNumbers(PM.groupSpan(root), previousSpan(PM, root));
    }
  });

  it('preserves NaN, infinities and signed zero in missing-geometry origins and spans', () => {
    for (const x of [0, -0, Infinity, -Infinity, NaN]) for (const y of [0, -0, Infinity, -Infinity, NaN]) {
      const group = { id: 'group', type: 'group' }, child = { id: 'child', type: 'shape', group: 'group', matrix: [1, 0, 0, 1, x, y], from: x, dur: y };
      const PM = fixture([group, child]);
      PM.active = () => true; PM.mul = (_a: any, b: any) => b;
      sameNumbers(PM.groupBounds(group, 0), previousBounds(PM, group, 0));
      sameNumbers(PM.groupSpan(group), previousSpan(PM, group));
    }
  });

  it('preserves corner getter order without extra coordinate reads', () => {
    const group = { id: 'group', type: 'group' }, reads: string[] = [], bounds = {};
    for (const [key, value] of Object.entries({ x0: 1, x1: 2, y0: 3, y1: 4 })) {
      Object.defineProperty(bounds, key, { get: () => { reads.push(key); return value; } });
    }
    const PM = fixture([group, { id: 'child', type: 'shape', group: 'group', bounds }]);
    const expected = previousBounds(PM, group, 0), previousReads = reads.splice(0);
    sameNumbers(PM.groupBounds(group, 0), expected);
    expect(reads).toEqual(previousReads);
    expect(reads).toEqual(['x0', 'x1', 'y0', 'y1', 'y0', 'y1']);
  });

  it('preserves non-finite corners and transforms, empty/audio groups and singular matrices', () => {
    const group: any = { id: 'group', type: 'group' }, child: any = { id: 'child', type: 'shape', group: 'group' };
    const PM = fixture([group, child]);
    for (const value of [Infinity, -Infinity, NaN, -0, 0]) {
      child.bounds = { x0: value, x1: value, y0: value, y1: value }; PM.touch();
      sameNumbers(PM.groupBounds(group, 0), previousBounds(PM, group, 0));
      group.matrix = [value, 0, 0, 1, 0, 0];
      sameNumbers(PM.groupBounds(group, 0), previousBounds(PM, group, 0));
      delete group.matrix;
    }
    group.matrix = [0, 0, 0, 1, 0, 0]; expect(PM.groupBounds(group, 0)).toBeNull(); delete group.matrix;
    child.type = 'audio'; delete child.bounds; PM.touch(); expect(PM.groupBounds(group, 0)).toBeNull();
    PM.proj.layers = [group]; PM.touch();
    expect(PM.groupBounds(group, 0)).toBeNull(); expect(PM.groupSpan(group)).toEqual({ from: 0, dur: 10 });
    delete PM.worldMatrix; expect(PM.groupBounds(group, 0)).toBeNull();
  });

  it('retains signed-zero extrema and non-finite combinations across multiple members', () => {
    for (const values of [[0, -0], [-0, -0], [Infinity, -Infinity], [NaN, 1]]) {
      const group = { id: 'group', type: 'group' };
      const PM = fixture([group, ...values.map((value, i) => ({ id: `layer${i}`, type: 'shape', group: 'group',
        matrix: [1, 0, 0, 1, value, value], from: value, dur: -0 }))]);
      PM.active = () => true; PM.mul = (_a: any, b: any) => b;
      sameNumbers(PM.groupBounds(group, 0), previousBounds(PM, group, 0));
      sameNumbers(PM.groupSpan(group), previousSpan(PM, group));
      if (values[0] === 0 && !Object.is(values[0], -0)) {
        expect(Object.is(PM.groupBounds(group, 0).x0, -0)).toBe(true);
        expect(Object.is(PM.groupBounds(group, 0).x1, 0)).toBe(true);
      }
    }
  });

  it('handles a large group without spreading all corners into function arguments', () => {
    const group = { id: 'group', type: 'group' };
    const PM = fixture([group, ...Array.from({ length: 40_000 }, (_, i) => ({
      id: `layer${i}`, type: 'shape', group: 'group', bounds: { x0: -1, x1: 3, y0: -2, y1: 4 },
    }))]);
    expect(PM.groupBounds(group, 0)).toEqual({ x0: -1, x1: 3, y0: -2, y1: 4, w: 4, h: 6, ax: 0, ay: 0 });
  });

  it('retains span cache identity, handles sparse membership selections and refreshes after edits', () => {
    const group = { id: 'group', type: 'group' }, inner = { id: 'inner', type: 'group', group: 'group' };
    const layers: any[] = [group, inner, { id: 'a', type: 'shape', group: 'inner', from: -0, dur: 2 },
      { id: 'b', type: 'audio', group: 'group', from: -1, dur: 5 }, { id: NaN, type: 'shape', from: 8, dur: 1 },
      { id: undefined, type: 'shape', from: 3, dur: 1 }];
    const PM = fixture(layers), span = PM.groupSpan(group);
    sameNumbers(span, previousSpan(PM, group)); expect(PM.groupSpan(group)).toBe(span);
    for (const ids of [['group'], [], ['inner', 'b'], ['group', 'group', ...Array(10)], [NaN, ...Array(20)]]) {
      expect(PM.expandGroups(ids)).toEqual(previousExpand(PM, ids));
    }
    layers[2]!.from = -3; PM.touch();
    sameNumbers(PM.groupSpan(group), previousSpan(PM, group)); expect(PM.groupSpan(group)).not.toBe(span);
    // Existing array callbacks skip holes; a retained hierarchy index can also
    // encounter one after a same-length in-place edit without a version bump.
    delete layers[2];
    const another = { id: 'inner', type: 'group' };
    sameNumbers(PM.groupSpan(another), previousSpan(PM, another));
  });

  it('snapshots span traversal length before a time getter appends another selected ID', () => {
    const create = () => {
      const group = { id: 'group', type: 'group' }, child: any = { id: 'child', type: 'shape', group: 'group', dur: 2 };
      const layers: any[] = [group, child]; let appended = false;
      Object.defineProperty(child, 'from', { get: () => {
        if (!appended) { appended = true; layers.push({ id: 'child', type: 'shape', group: 'group', from: -100, dur: 1 }); }
        return 3;
      } });
      return { PM: fixture(layers), group };
    };
    const actual = create(), original = create();
    sameNumbers(actual.PM.groupSpan(actual.group), previousSpan(original.PM, original.group));
    expect(actual.PM.groupSpan(actual.group)).toEqual({ from: -100, dur: 105 });
  });

  it('snapshots members and preserves all from reads before all end reads for numeric getters', () => {
    const create = () => {
      const group = { id: 'group', type: 'group' }, reads: string[] = [];
      const first: any = { id: 'first', type: 'shape', group: 'group' }, second: any = { id: 'second', type: 'shape', group: 'group' };
      let value = 0;
      for (const [layer, duration] of [[first, 100], [second, 0]] as const) {
        Object.defineProperties(layer, {
          from: { get: () => { reads.push(`${layer.id}.from`); second.type = 'group'; return value++; } },
          dur: { get: () => { reads.push(`${layer.id}.dur`); return duration; } },
        });
      }
      return { PM: fixture([group, first, second]), group, reads };
    };
    const actual = create(), original = create();
    sameNumbers(actual.PM.groupSpan(actual.group), previousSpan(original.PM, original.group));
    expect(actual.reads).toEqual(original.reads);
    expect(actual.reads).toEqual(['first.from', 'second.from', 'first.from', 'first.dur', 'second.from', 'second.dur']);
  });

  it('preserves actual grouping bounds, transformed members and undo', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    PM.proj = PM.mkProject({ dur: 4 }); PM.time = 1;
    const a = PM.mkLayer('shape', { d: { w: 50, h: 30 } }), b = PM.mkLayer('shape', { d: { w: 80, h: 40 } });
    PM.proj.layers = [a, b]; PM.ProjectIndex.invalidate();
    PM.GL.bounds = (layer: any) => ({ x0: -layer.d.w / 2, x1: layer.d.w / 2, y0: -layer.d.h / 2, y1: layer.d.h / 2 });
    const group = PM.groupLayers([a.id, b.id]), original = PM.groupBounds(group, 1);
    sameNumbers(original, previousBounds(PM, group, 1));
    expect(PM.Edit.apply([{ type: 'set_property', target: a.id, path: 'rotation', value: 33 },
      { type: 'set_property', target: b.id, path: 'scale.x', value: -175 }]).ok).toBe(true);
    sameNumbers(PM.groupBounds(group, 1), previousBounds(PM, group, 1));
    expect(PM.hist.undo()).toBe(true);
    sameNumbers(PM.groupBounds(PM.L(group.id), 1), original);
    const nested = PM.groupLayers([group.id]);
    sameNumbers(PM.groupBounds(nested, 1), previousBounds(PM, nested, 1));
  });

  it('keeps the real native model-group bounds path', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    PM.proj = PM.mkProject({ dur: 4 }); PM.time = 0;
    PM.layerDefinition = (id:string) => Object.values(LAYER3D_DEFINITIONS).includes(id as any)
      ? { id, label: '3D', version: 1, params: [], defaults: {}, renderer: { kind: 'layer3d' } } : null;
    expect(editScene(PM, { operation: 'add_object', object: { id: 'model', source: { primitive: 'box' } } }).ok).toBe(true);
    const group = PM.groupLayers(['model']);
    expect(isModelGroup(PM, group)).toBe(true);
    PM.GL.bounds = () => { throw new Error('Native groups must not use the flat corner path'); };
    sameNumbers(PM.groupBounds(group, 0), modelGroupBounds(PM, group, 0));
    expect(PM.groupBounds(group, 0).z1).toBeGreaterThan(PM.groupBounds(group, 0).z0);
  });
});
