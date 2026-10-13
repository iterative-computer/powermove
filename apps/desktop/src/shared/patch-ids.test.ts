// Runs under `bun test` and vitest. The engine side (engine/crates/pm-model,
// tests/ops.rs and tests/patch_binary.rs) checks its generator and applier
// against vectors generated from this module.
import { describe, expect, it } from 'vitest';
import { randomPair } from '../../../../engine/crates/pm-model/tests/vectors/generate-patch-binary-vectors';
import { canonicalStringify } from './canonical-json';
import { applyPatch, diffPatches } from './patch';
import { encodePatchOps } from './patch-binary';
import { applyOps, diffArrayOps, diffOps, IdOpMismatch, isIdKeyed, type PatchOp } from './patch-ids';

const clone = <T>(value: T): T => (value === undefined ? value : JSON.parse(JSON.stringify(value)));
const layer = (id: string, name = id) => ({ id, name, p: { x: { v: 0, kf: [] } } });
const comp = (ids: string[]) => ({ compId: 'c1', comps: { c1: { id: 'c1', layers: ids.map((id) => layer(id)) } } });
const L = ['comps', 'c1', 'layers'];

describe('diffOps', () => {
  it('emits deletes, an order, inserts, then content edits', () => {
    const before = comp(['a', 'b', 'c', 'd']);
    const after = comp(['d', 'n', 'a', 'c']);
    after.comps.c1.layers[2]!.name = 'renamed';
    expect(diffOps(before, after)).toEqual([
      { op: 'delete', path: L, id: 'b' },
      { op: 'order', path: L, ids: ['d', 'a', 'c'] },
      { op: 'insert', path: L, index: 1, value: layer('n') },
      { path: [...L, 2, 'name'], exists: true, value: 'renamed' },
    ]);
  });

  it('sends a reorder as ids only', () => {
    const before = comp(['a', 'b', 'c']);
    expect(diffOps(before, comp(['c', 'a', 'b']))).toEqual([{ op: 'order', path: L, ids: ['c', 'a', 'b'] }]);
  });

  it('keeps diffPatches leaf edits when the ids do not move', () => {
    const before = comp(['a', 'b']);
    const after = clone(before);
    after.comps.c1.layers[1]!.p.x.v = 3;
    expect(diffOps(before, after)).toEqual([{ path: [...L, 1, 'p', 'x', 'v'], exists: true, value: 3 }]);
  });

  it('falls back to the whole array when items lack ids, repeat ids, or share none', () => {
    for (const [before, after] of [
      [[{ v: 1 }, { v: 2 }], [{ v: 2 }]],
      [[{ id: 'a' }, { v: 2 }], [{ v: 2 }]],
      [[{ id: 'a' }, { id: 'a' }], [{ id: 'a' }]],
      [[{ id: 'a' }], [{ id: 'b' }]],
      [[], [{ id: 'a' }]],
      [[1, 2], [2]],
      [[{ id: 1 }, { id: 2 }], [{ id: 2 }]],
    ] as [any[], any[]][]) {
      expect(isIdKeyed(before) && isIdKeyed(after) && before.some((x) => after.some((y) => y.id === x.id))).toBe(false);
      expect(diffArrayOps(before, after, ['list'])).toEqual([{ path: ['list'], exists: true, value: after }]);
    }
  });

  it('visits members in UTF-16 code unit order', () => {
    const keys = ['b', 'a', '10', '2', '', '😀'];
    const before = Object.fromEntries(keys.map((k) => [k, 1]));
    const after = Object.fromEntries(keys.map((k) => [k, 2]));
    expect(diffOps(before, after).map((op) => op.path[0])).toEqual(['10', '2', 'a', 'b', '😀', '']);
  });

  it('does not touch its inputs', () => {
    const before = comp(['a', 'b', 'c']);
    const after = comp(['c', 'x']);
    const [b, a] = [JSON.stringify(before), JSON.stringify(after)];
    const ops = diffOps(before, after);
    expect([JSON.stringify(before), JSON.stringify(after)]).toEqual([b, a]);
    // The operations own their values.
    (ops.find((op) => 'op' in op && op.op === 'insert') as { value: any }).value.name = 'mutated';
    expect(after.comps.c1.layers[1]!.name).toBe('x');
  });
});

describe('applyOps', () => {
  it('applies id-keyed operations and leaf patches in order', () => {
    const doc = comp(['a', 'b', 'c']);
    const out = applyOps(doc, [
      { op: 'delete', path: L, id: 'b' },
      { op: 'order', path: L, ids: ['c', 'a'] },
      { op: 'insert', path: L, index: 1, value: layer('n') },
      { path: [...L, 0, 'name'], exists: true, value: 'first' },
    ]);
    expect(out).toBe(doc);
    expect(out.comps.c1.layers.map((l: any) => [l.id, l.name])).toEqual([['c', 'first'], ['n', 'n'], ['a', 'a']]);
  });

  it('throws IdOpMismatch when the array does not match, leaving earlier operations applied', () => {
    const cases: PatchOp[] = [
      { op: 'delete', path: L, id: 'zz' },
      { op: 'delete', path: ['comps', 'c2', 'layers'], id: 'a' },
      { op: 'delete', path: ['comps', 'c1'], id: 'a' },
      { op: 'order', path: L, ids: ['a'] },
      { op: 'order', path: L, ids: ['a', 'c'] },
      { op: 'order', path: L, ids: ['a', 'a', 'b'] },
      { op: 'insert', path: L, index: 0, value: layer('a') },
      { op: 'insert', path: L, index: 4, value: layer('n') },
      { op: 'insert', path: L, index: 0, value: { name: 'no id' } },
    ];
    for (const op of cases) {
      const doc = comp(['a', 'b', 'c']);
      expect(() => applyOps(doc, [{ op: 'delete', path: L, id: 'c' }, op]), JSON.stringify(op)).toThrow(IdOpMismatch);
      expect(doc.comps.c1.layers.map((l) => l.id)).toEqual(['a', 'b']);
    }
    expect(() => applyOps(undefined, [{ op: 'delete', path: [], id: 'a' }])).toThrow(IdOpMismatch);
    expect(() => applyOps({ l: [{ id: 'a' }, { v: 1 }] }, [{ op: 'delete', path: ['l'], id: 'a' }])).toThrow(IdOpMismatch);
  });

  it('copies inserted values', () => {
    const value = layer('n');
    const doc = applyOps(comp(['a']), [{ op: 'insert', path: L, index: 0, value }]);
    value.name = 'changed later';
    expect(doc.comps.c1.layers[0].name).toBe('n');
  });
});

describe('diffOps against diffPatches (property)', () => {
  it('gives the same document as patch.ts leaf patches, forward and back, on 3,000 random pairs', () => {
    const kinds: Record<string, number> = { leaf: 0, order: 0, insert: 0, delete: 0 };
    for (let seed = 0; seed < 3000; seed++) {
      const [before, after] = randomPair(0x7e570000 + seed);
      const ops = diffOps(before, after);
      for (const op of ops) kinds['op' in op ? op.op : 'leaf']!++;
      const byOps = applyOps(clone(before), ops);
      const byLeaf = applyPatch(clone(before), diffPatches(before, after).forward);
      expect(canonicalStringify(byOps), `seed ${seed}`).toBe(canonicalStringify(byLeaf));
      expect(canonicalStringify(byOps), `seed ${seed}`).toBe(canonicalStringify(after));
      const back = applyOps(byOps, diffOps(after, before));
      expect(canonicalStringify(back), `seed ${seed} undo`).toBe(canonicalStringify(before));
    }
    for (const [kind, n] of Object.entries(kinds)) expect(n, kind).toBeGreaterThan(300);
  });
});

describe('structural edits stay small (S5)', () => {
  // An 884-layer composition like S5's synthetic project: about 1.2 KB a layer.
  const big = () => comp(Array.from({ length: 884 }, (_, i) => `layer-${i}`)).comps.c1.layers.map((l, i) => ({
    ...l, type: i % 7 ? 'shape' : 'text', from: i / 30, dur: 5,
    p: Object.fromEntries(['position.x', 'position.y', 'scale.x', 'scale.y', 'rotation', 'opacity', 'anchor.x', 'anchor.y'].map((k) => [k, { v: i, kf: [], expr: null }])),
    d: { fill: '#336699', stroke: null, path: Array.from({ length: 40 }, (_, k) => [k, k * 2, k * 3]) },
  }));

  it('sends only what moved, never the stringified array', () => {
    const layers = big();
    const before = { compId: 'c1', comps: { c1: { id: 'c1', layers } } };
    const wholeArray = new TextEncoder().encode(JSON.stringify(layers)).length;
    expect(wholeArray).toBeGreaterThan(884 * 800);
    const edits: [string, any[]][] = [
      ['reorder', [layers[500], ...layers.slice(0, 500), ...layers.slice(501)]],
      ['insert', [...layers.slice(0, 10), { ...layers[0], id: 'new-layer' }, ...layers.slice(10)]],
      ['delete', layers.filter((_, i) => i !== 300)],
    ];
    for (const [kind, next] of edits) {
      const after = { compId: 'c1', comps: { c1: { id: 'c1', layers: next } } };
      const body = encodePatchOps(diffOps(before, after));
      // Reorder carries 884 ids (about 9 KB); insert and delete one layer at most.
      expect(body.length, kind).toBeLessThan(wholeArray / 50);
      expect(diffPatches(before, after).forward[0]!.path, kind).toEqual(L); // what diffPatches would have sent whole
    }
  });
});
