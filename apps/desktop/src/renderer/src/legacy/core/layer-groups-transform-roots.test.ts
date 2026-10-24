import { describe, expect, it } from 'vitest';
import { groupAncestors, installLayerGroups } from './layer-groups';
import { makePM } from '../__tests__/make-pm';

const nativeFind = Array.prototype.find;
function fixture(layers: any) {
  const PM: any = { proj: { layers }, curComp() { return this.proj; }, animVersion: () => 0 };
  installLayerGroups(PM); return PM;
}

const previousAncestorCaches = new WeakMap<object, (layer: any, layers: any[]) => any[]>();
function previousAncestors(PM: any) {
  let ancestors = previousAncestorCaches.get(PM);
  if (ancestors) return ancestors;
  let indexes = new WeakMap<any[], { version: number; length: number; byId: Map<any, any>; ancestors: Map<any, any[]> }>();
  ancestors = (layer, layers) => {
    if (!layer?.group) return [];
    const version = PM.animVersion?.() ?? -1;
    let index = indexes.get(layers);
    if (!index || index.version !== version || index.length !== layers.length || version === -1) {
      index = { version, length: layers.length, byId: new Map(layers.map(item => [item.id, item])), ancestors: new Map() };
      indexes.set(layers, index);
    }
    let result = index.ancestors.get(layer);
    if (!result) { result = groupAncestors(layer, layers, index.byId); index.ancestors.set(layer, result); }
    return result;
  };
  PM.bus?.on?.('layers', () => { indexes = new WeakMap(); });
  PM.bus?.on?.('project', () => { indexes = new WeakMap(); });
  previousAncestorCaches.set(PM, ancestors);
  return ancestors;
}

// Preserve the original independent parent search and group cache: parent
// lookup uses the first matching ID, while group lookup keeps the last one.
function previousRoots(PM: any, ids: any[]) {
  const ancestors = previousAncestors(PM);
  const layers = PM.curComp().layers;
  const selected = layers.filter((layer: any) => ids.includes(layer.id) && layer.type !== 'audio' && !layer.lock
    && !ancestors(layer, layers).some((group: any) => group.lock));
  const selectedIds = new Set(selected.map((layer: any) => layer.id));
  return selected.filter((layer: any) => {
    const seen = new Set();
    for (let current = layer; current && !seen.has(current.id); current = layers.find((item: any) => item.id === current.parent)) {
      seen.add(current.id);
      if (current !== layer && selectedIds.has(current.id)
        || ancestors(current, layers).some((group: any) => selectedIds.has(group.id))) return false;
    }
    return true;
  });
}
function result(run: () => any) {
  try { return { value: run().map((layer: any) => layer.index) }; }
  catch (error) { return { error: error instanceof Error ? error.constructor.name : String(error) }; }
}
function compare(factory: () => { layers: any; ids: any[]; trace?: any[]; after?: () => any }) {
  const before = factory(), after = factory();
  const expected = result(() => previousRoots(fixture(before.layers), before.ids));
  const actual = result(() => fixture(after.layers).transformRoots(after.ids));
  expect(actual).toEqual(expected);
  expect(after.trace).toEqual(before.trace);
  expect(after.after?.()).toEqual(before.after?.());
  return actual;
}

describe('transform root search', () => {
  it('matches the previous search across seeded duplicate IDs, cycles, groups, audio and locks', () => {
    let seed = 291824;
    const random = () => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed / 2 ** 32; };
    for (let sample = 0; sample < 300; sample++) {
      const count = 8 + Math.floor(random() * 50), unusual = [null, undefined, NaN, 0, -0, '', 'duplicate', 'missing'];
      const ids = Array.from({ length: count }, (_, index) => random() < .25 ? unusual[Math.floor(random() * unusual.length)] : `id${index}`);
      const layers = ids.map((id, index) => ({ id, index, type: random() < .2 ? 'group' : random() < .15 ? 'audio' : 'shape',
        lock: random() < .12, parent: random() < .6 ? ids[Math.floor(random() * count)] : null,
        group: random() < .3 ? ids[Math.floor(random() * count)] : null }));
      const selected = ids.filter(() => random() < .6);
      if (sample % 3 === 0) selected.push(...selected.slice(0, 4));
      compare(() => ({ layers: layers.map(layer => ({ ...layer })), ids: selected.slice() }));
    }
  });

  it('preserves first-occurrence parent lookup, strict NaN matching and SameValueZero selection', () => {
    const input = [
      { id: 'child', parent: 'parent' }, { id: 'parent', parent: 'missing', lock: true },
      { id: 'parent', parent: 'selected' }, { id: 'selected', parent: null },
      { id: NaN, parent: NaN }, { id: -0, parent: 0 }, { id: null, parent: undefined }, { id: undefined, parent: 'missing' },
    ];
    const actual = compare(() => ({ layers: input.map((layer, index) => ({ ...layer, index, type: 'shape' })),
      ids: ['child', 'selected', NaN, 0, null, undefined, 'child'] }));
    expect(actual).toEqual({ value: [0, 4, 5, 7] });
  });

  it('uses the later duplicate group for locks and links while parent lookup still uses the first', () => {
    const make = (firstLocked: boolean, laterLocked: boolean) => {
      const outer = { id: 'outer', type: 'group', index: 0, parent: null, group: null };
      const first = { id: 'duplicate', type: 'group', index: 1, parent: null, group: null, lock: firstLocked };
      const later = { id: 'duplicate', type: 'group', index: 2, parent: null, group: 'outer', lock: laterLocked };
      const child = { id: 'child', type: 'shape', index: 3, parent: null, group: 'duplicate' };
      return { layers: [outer, first, later, child], child, first, later, outer };
    };
    const unlockedFirst = make(false, true);
    expect(groupAncestors(unlockedFirst.child, unlockedFirst.layers)).toEqual([unlockedFirst.first]);
    expect(previousRoots(fixture(unlockedFirst.layers), ['child'])).toEqual([]);
    expect(compare(() => ({ ...make(false, true), ids: ['child'] }))).toEqual({ value: [] });
    expect(compare(() => ({ ...make(true, false), ids: ['child'] }))).toEqual({ value: [3] });
    expect(compare(() => ({ ...make(false, false), ids: ['outer', 'child'] }))).toEqual({ value: [0] });
    const parentCase = make(false, false);
    parentCase.child.group = null as any; parentCase.child.parent = 'duplicate' as any;
    // The later duplicate belongs to the selected outer group; the first
    // parent does not, so this child remains a transform root.
    const PM = fixture(parentCase.layers);
    expect(PM.transformRoots(['outer', 'child'])).toEqual([parentCase.outer, parentCase.child]);
    expect(previousRoots(PM, ['outer', 'child'])).toEqual([parentCase.outer, parentCase.child]);
  });

  it('observes ID, parent, lock and array replacement edits without needing invalidation', () => {
    const parent = { id: 'parent', type: 'shape', parent: null, index: 0 };
    const child: any = { id: 'child', type: 'shape', parent: null, index: 1 };
    const PM = fixture([parent, child]);
    expect(PM.transformRoots(['parent', 'child'])).toEqual([parent, child]);
    child.parent = 'parent'; expect(PM.transformRoots(['parent', 'child'])).toEqual([parent]);
    parent.id = 'renamed'; expect(PM.transformRoots(['renamed', 'child'])).toEqual([parent, child]);
    child.lock = true; expect(PM.transformRoots(['renamed', 'child'])).toEqual([parent]);
    PM.proj.layers = [child, { ...parent, id: 'parent' }]; child.lock = false;
    expect(PM.transformRoots(['parent', 'child'])).toEqual([PM.proj.layers[1]]);
  });

  it('visits sparse parent-search entries and preserves sparse selected-ID behavior', () => {
    compare(() => {
      const layers = new Array(3); layers[0] = { id: 'child', parent: 'missing', type: 'shape', index: 0 };
      layers[2] = { id: undefined, parent: null, type: 'shape', index: 2 };
      const ids = new Array(2); ids[0] = 'child';
      return { layers, ids };
    });
  });

  it('keeps native lookup property order and reads the live parent for every candidate', () => {
    compare(() => {
      const trace: string[] = [];
      const layers = Array.from({ length: 5 }, (_, index) => ({ index, type: 'shape',
        get id() { trace.push(`id:${index}`); return `id${index}`; },
        get parent() { trace.push(`parent:${index}`); return null; },
      }));
      Object.defineProperty(layers, 'find', { get() { trace.push('find'); return nativeFind; } });
      return { layers, ids: ['id0', 'id4'], trace };
    });
  });

  it('preserves proxy reads without adding reflection or reading a changing length again', () => {
    compare(() => {
      const trace: string[] = [];
      const layers = new Proxy([{ id: 'child', parent: null, type: 'shape', index: 0 },
        { id: 'other', parent: null, type: 'shape', index: 1 }], {
        get(target, key, receiver) { trace.push(`get:${String(key)}`); return Reflect.get(target, key, receiver); },
        has(target, key) { trace.push(`has:${String(key)}`); return Reflect.has(target, key); },
        getOwnPropertyDescriptor(target, key) { trace.push(`descriptor:${String(key)}`); return Reflect.getOwnPropertyDescriptor(target, key); },
      });
      return { layers, ids: ['child'], trace };
    });
  });

  it('snapshots lookup length before a parent getter appends a matching layer', () => {
    compare(() => {
      const trace: string[] = []; let appended = false;
      const layers: any[] = [{ id: 'child', type: 'shape', index: 0,
        get parent() {
          trace.push('parent');
          if (!appended) { appended = true; layers.push({ id: 'parent', type: 'shape', parent: null, index: 2 }); }
          return 'parent';
        } }, { id: 'other', type: 'shape', parent: null, index: 1 }];
      return { layers, ids: ['child', 'parent'], trace, after: () => layers.length };
    });
  });

  it('observes lookup-method getters before the length snapshot and follows changing entries', () => {
    compare(() => {
      const trace: string[] = []; let replaced = false;
      const layers: any[] = [{ id: 'child', type: 'shape', parent: 'parent', index: 0 },
        { id: 'other', type: 'shape', parent: null, index: 1 }];
      Object.defineProperty(layers, 'find', { get() {
        trace.push('find');
        if (!replaced) { replaced = true; layers[1] = { id: 'parent', type: 'shape', parent: null, index: 2 }; }
        return nativeFind;
      } });
      return { layers, ids: ['child'], trace };
    });
  });

  it('observes entries changed by a parent getter during the search', () => {
    compare(() => {
      const trace: string[] = [];
      const layers: any[] = [{ id: 'child', type: 'shape', index: 0, get parent() {
        trace.push('parent'); layers[1] = { id: 'parent', type: 'shape', parent: null, index: 2 }; return 'parent';
      } }, { id: 'other', type: 'shape', parent: null, index: 1 }];
      return { layers, ids: ['child'], trace };
    });
  });

  it('keeps native generic lookup length coercion, including fractional lengths and BigInt rejection', () => {
    for (const length of ['2.9', NaN, -3, 2n]) compare(() => {
      const trace: string[] = [];
      const layers: any = { 0: { id: 'child', type: 'shape', parent: null, index: 0 },
        1: { id: 'other', type: 'shape', parent: null, index: 1 },
        filter(callback: any) { return [this[0]].filter(callback); }, find: nativeFind,
        get length() { trace.push('length'); return length; } };
      return { layers, ids: ['child'], trace };
    });
    for (const length of [0x100000000, 0x100000001 + .9, Infinity]) compare(() => {
      const trace: string[] = [];
      const layers: any = { 0: { id: 'child', type: 'shape', parent: 'child', index: 0 },
        filter(callback: any) { return [this[0]].filter(callback); }, find: nativeFind,
        get length() { trace.push('length'); return length; } };
      return { layers, ids: ['child'], trace };
    });
  });

  it('honors a prototype search override installed after module import, before PM installation', () => {
    Array.prototype.find = function(this: any, callback: any, thisArg?: any) {
      this.trace?.push('custom prototype'); return nativeFind.call(this, callback, thisArg);
    };
    try {
      compare(() => {
        const trace: string[] = [];
        const layers: any = [{ id: 'child', type: 'shape', parent: null, index: 0 }];
        layers.trace = trace;
        return { layers, ids: ['child'], trace };
      });
    } finally { Array.prototype.find = nativeFind; }
  });

  it('retains custom find receiver, callback lifetime and dynamically replaced method behavior', () => {
    compare(() => {
      const trace: any[] = [], callbacks: any[] = [];
      const parent: any = { id: 'parent', type: 'shape', parent: null, index: 1 };
      const layers: any[] = [{ id: 'child', type: 'shape', parent: 'parent', index: 0 }, parent];
      Object.defineProperty(layers, 'find', { value: function(this: any, callback: any) {
        trace.push(this === layers); callbacks.push(callback); return nativeFind.call(this, callback);
      } });
      return { layers, ids: ['child'], trace, after: () => {
        parent.parent = 'probe';
        return [callbacks[0]({ id: 'probe' }), result(() => callbacks[1]({ id: 'probe' }))];
      } };
    });
    compare(() => {
      const trace: string[] = []; let calls = 0;
      const layers = [{ id: 'child', parent: 'parent', type: 'shape', index: 0 },
        { id: 'parent', parent: null, type: 'shape', index: 1 }];
      Object.defineProperty(layers, 'find', { get() {
        trace.push('find'); return calls++ % 2 ? function(this: any, callback: any) {
          trace.push('custom'); return nativeFind.call(this, callback);
        } : nativeFind;
      } });
      return { layers, ids: ['child'], trace };
    });
  });

  it('does not depend on a custom find function exposing its own call method', () => {
    compare(() => {
      const layers = [{ id: 'child', parent: null, type: 'shape', index: 0 }];
      const find = function(this: any, callback: any) { return nativeFind.call(this, callback); };
      Object.defineProperty(find, 'call', { value() { throw new Error('Function.call must not be read'); } });
      Object.defineProperty(layers, 'find', { value: find });
      return { layers, ids: ['child'] };
    });
  });

  it('keeps real editing, grouping, locking and undo selections compatible', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    PM.proj = PM.mkProject({ dur: 5 });
    const layers = Array.from({ length: 5 }, (_, index) => PM.mkLayer('shape', { name: `Layer ${index}` }));
    PM.proj.layers = [...layers]; PM.ProjectIndex.invalidate();
    expect(PM.Edit.apply({ type: 'group_layers', targets: [layers[1].id, layers[3].id] }).ok).toBe(true);
    const group = PM.firstSel();
    const ids = [group.id, ...layers.map(layer => layer.id)];
    expect(PM.transformRoots(ids)).toEqual(previousRoots(PM, ids));
    group.lock = true; PM.touch(); expect(PM.transformRoots(ids)).toEqual([layers[0], layers[2], layers[4]]);
    group.lock = false; PM.touch(); expect(PM.hist.undo()).toBe(true);
    expect(PM.transformRoots(ids)).toEqual(previousRoots(PM, ids));
    expect(PM.hist.redo()).toBe(true); expect(PM.transformRoots(ids)).toEqual(previousRoots(PM, ids));
  });
});
