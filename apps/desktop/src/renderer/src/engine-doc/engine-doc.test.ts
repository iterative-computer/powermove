/*
 * The write barrier, the mirror, and the native evaluator against the legacy
 * evaluator, with the real pm-wasm module (built by
 * engine/crates/pm-wasm/scripts/build.mjs into public/engine/).
 */
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it, vi } from 'vitest';

import { canonicalProject, canonicalProjectHash, canonicalStringify } from '../../../shared/canonical-json';
import { applyOps, diffOps } from '../../../shared/patch-ids';
import { decodePatchOps } from '../../../shared/patch-binary';
import { makePM } from '../legacy/__tests__/make-pm';
import { installEngineDoc, type EngineDoc } from './install';
import { parseMode } from './flags';

const MODULE = fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url));
const built = existsSync(MODULE);
if (!built && process.env.PM_WASM_REQUIRED) throw new Error(`pm-wasm is not built (${MODULE}); run node engine/crates/pm-wasm/scripts/build.mjs`);
let compiled: Promise<WebAssembly.Module> | null = null;
const loadModule = () => (compiled ??= WebAssembly.compile(readFileSync(MODULE)));

/** A seeded PRNG (mulberry32). */
function rng(seed: number) {
  return () => {
    seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

async function editor(mode = 'audit', lines: string[] = []) {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/compositor');
  const doc: EngineDoc = installEngineDoc(PM, { mode: parseMode(mode), loadModule, sink: line => lines.push(line), auditMs: 1e9 });
  PM.proj = PM.mkProject({ dur: 10 });
  PM.time = 0;
  expect(await doc.whenReady).toBe(true);
  doc.auditor?.stop();
  return { PM, doc, lines };
}

/** Random 2D content: parents, groups, keys (bezier, hold, springs), expressions, effects, masks. */
function populate(PM: any, random: () => number, count = 24) {
  const types = ['solid', 'shape', 'null', 'image', 'video', 'precomp', 'adjustment', 'shader', 'audio'];
  const layers: any[] = [];
  for (let i = 0; i < count; i++) {
    const type = types[Math.floor(random() * types.length)]!;
    const layer = PM.mkLayer(type, { name: `L${i}`, from: Math.round(random() * 4 * 30) / 30, dur: 1 + Math.round(random() * 6 * 30) / 30 });
    for (const key of ['position.x', 'position.y', 'rotation', 'scale.x', 'opacity', 'skew', 'anchor.x']) {
      const prop = layer.p[key];
      if (!prop) continue;
      const roll = random();
      if (roll < 0.3) {
        const keys = 2 + Math.floor(random() * 3);
        prop.kf = [];
        for (let k = 0; k < keys; k++) prop.kf.push(PM.KF(Math.round((k + random()) * 30) / 30, Math.round(random() * 400 - 200), ['linear', 'power', 'hold', 'easeOut'][Math.floor(random() * 4)]));
        prop.kf.sort((a: any, b: any) => a.t - b.t);
      } else if (roll < 0.4) prop.expr = ['value + 10', 'time * 30', 'Math.sin(time) * 50 + value', 'wiggle(2, 10)', 'value *'][Math.floor(random() * 5)];
      else prop.v = Math.round(random() * 300);
    }
    if (type === 'solid' && random() < 0.5) layer.d.w = PM.P(Math.round(random() * 400) + 10, { kf: [PM.KF(0, 10), PM.KF(2, 300)] });
    if (random() < 0.3) layer.fx = [{ id: PM.uid('F'), type: 'blur', on: true, p: { amount: PM.P(Math.round(random() * 20), random() < 0.5 ? { kf: [PM.KF(0, 0), PM.KF(1, 30)] } : {}) } }];
    if (random() < 0.2 && layer.masks) layer.masks = [PM.mkMask('rect', PM.proj)];
    if (random() < 0.2) layer.lock = true;
    layers.push(layer);
  }
  // Parents (no cycles by construction) and a group.
  layers.forEach((layer, i) => { if (i > 0 && random() < 0.3) layer.parent = layers[Math.floor(random() * i)]!.id; });
  PM.proj.layers = layers;
  PM.ProjectIndex.invalidate();
  PM.touch();
  const members = layers.filter(l => !l.parent && !l.lock && l.type !== 'audio').slice(0, 3).map(l => l.id);
  if (members.length > 1) PM.groupLayers(members, 'Group');
  PM.touch();
}

function exercise(PM: any, doc: EngineDoc, random: () => number) {
  const api = doc.api!;
  const layers: any[] = PM.proj.layers;
  for (const L of layers) {
    for (const t of [0, 0.5, 1.25, 2, 3.999, 7, -1]) {
      api.active(L, t);
      api.localMatrix(L, t);
      api.worldMatrix(L, t);
      api.transformParentMatrix(L, t);
      api.resolveContent(L, t);
      for (const key of Object.keys(L.p || {})) {
        api.ev(L, key, t);
        api.evP(L, L.p[key], t, key);
      }
      for (const { key, prop } of api.allProps(L)) if (prop && typeof prop === 'object') api.evP(L, prop, t, key);
      api.local3D(L, t);
      api.world3D(L, t);
      api.planeMatrix(L, t);
      api.perspectiveAmount(L, t);
      api.bounds(L, t);
      if (L.type === 'group') { api.groupBounds(L, t); api.groupSpan(L); }
    }
    api.is3DLayer(L);
    api.groupAncestors(L);
    for (const key of ['position.x', 'rotation', 'c.w', 'nope', `${L.fx?.[0]?.id}.amount`]) api.findProp(L, key);
    for (const other of layers.slice(0, 5)) api.wouldCycle(L, other.id);
    api.wouldCycle(L, null);
    if (L.p?.rotation?.kf?.length) api.hasKeyAt(L, L.p.rotation, L.from + L.p.rotation.kf[0].t);
  }
  for (let i = 0; i < 40; i++) {
    const x = random() * PM.proj.w, y = random() * PM.proj.h;
    api.pick(x, y, random() * 5);
    api.pick(x, y, random() * 5, { includeLocked: true });
  }
  api.transformRoots(layers.map(l => l.id));
  api.expandGroups(layers.filter(l => l.type === 'group').map(l => l.id));
}

const mismatches = (doc: EngineDoc) => doc.dual!.stats.samples;

describe('bus announcements', () => {
  it('does not treat a runtime-only assets event as a document write', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/compositor');
    // The module never loads: only the bus wrapper is under test.
    installEngineDoc(PM, { mode: parseMode('audit'), loadModule: () => new Promise<WebAssembly.Module>(() => {}), sink: () => {}, auditMs: 1e9 });
    const announce = vi.fn();
    PM.bus.__engineDocAnnounce = announce;
    const heard: unknown[] = [];
    PM.bus.on('assets', (change: unknown) => heard.push(change));
    PM.bus.emit('assets', { runtimeOnly: true });
    expect(announce).not.toHaveBeenCalled();
    PM.bus.emit('assets');
    PM.bus.emit('assets', { runtimeOnly: false });
    PM.bus.emit('layers', { runtimeOnly: true });
    expect(announce).toHaveBeenCalledTimes(3);
    expect(heard).toEqual([{ runtimeOnly: true }, undefined, { runtimeOnly: false }]);
  });
});

describe.skipIf(!built)('engine-doc with pm-wasm', () => {
  it('answers every routed query like the legacy evaluator on random projects', async () => {
    for (const seed of [1, 2, 3, 4, 5]) {
      const { PM, doc } = await editor();
      const random = rng(seed);
      populate(PM, random);
      exercise(PM, doc, random);
      expect(mismatches(doc), `seed ${seed}`).toEqual([]);
      expect(doc.dual!.total().compared).toBeGreaterThan(1000);
      expect(doc.auditor!.audit()).toBe(true);
    }
  });

  it('keeps the mirror equal to the document through edits, undo, redo, and structural changes', async () => {
    const { PM, doc } = await editor();
    const random = rng(42);
    populate(PM, random, 16);
    const check = (label = '') => {
      doc.barrier!.flush();
      const mirror = JSON.parse(doc.wasm!.toJson());
      expect(diffOps(mirror, JSON.parse(JSON.stringify(canonicalProject(PM.proj)))), label).toEqual([]);
      expect(doc.wasm!.hash()).toBe(canonicalProjectHash(PM.proj));
      expect(doc.barrier!.shadowMatches().ok).toBe(true);
    };
    check();
    for (let step = 0; step < 60; step++) {
      const layers = PM.proj.layers;
      const L = layers[Math.floor(random() * layers.length)];
      switch (step % 6) {
        case 0: PM.Edit.apply({ type: 'set_property', target: L.id, path: 'position.x', value: Math.round(random() * 500) }); break;
        case 1: PM.Edit.apply({ type: 'set_property', target: L.id, path: 'opacity', value: Math.round(random() * 100), time: Math.round(random() * 60) / 30 }); break;
        case 2: PM.hist.do('Reorder', () => { const [moved] = layers.splice(0, 1); layers.splice(Math.floor(random() * layers.length), 0, moved); }); break;
        case 3: PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('solid', { name: `N${step}` }), Math.floor(random() * layers.length))); break;
        case 4: PM.hist.undo(); break;
        case 5: PM.hist.redo(); break;
      }
      check(`step ${step}`);
      // Edits are visible to the very next query (flush before query).
      const cur = PM.proj.layers[0];
      const native = doc.api!.worldMatrix(cur, 1), legacy = PM.worldMatrix(cur, 1);
      native.forEach((v: number, i: number) => expect(Math.abs(v - legacy[i]!)).toBeLessThanOrEqual(1e-9 + 1e-9 * Math.abs(legacy[i]!)));
    }
    // Direct writes with a bare touch() (production): a structural diff.
    PM.proj.layers[0].p['position.y'].v = 1234;
    PM.touch();
    check();
    expect(doc.barrier!.stats.refused).toBe(0);
    expect(mismatches(doc)).toEqual([]);
  });

  it('sends structural edits as id-keyed operations and property edits as leaf patches', async () => {
    const { PM, doc } = await editor();
    populate(PM, rng(7), 30);
    doc.barrier!.flush();
    const batches: any[] = [];
    doc.subscribe(batch => batches.push(batch));
    PM.hist.do('Reorder', () => { const layers = PM.proj.layers; layers.push(layers.shift()); });
    doc.barrier!.flush();
    PM.Edit.apply({ type: 'set_property', target: PM.proj.layers[0].id, path: 'rotation', value: 33 });
    doc.barrier!.flush();
    expect(batches).toHaveLength(2);
    // An order patch names the 30-odd ids; a whole-array leaf set would carry every layer.
    const layersJson = JSON.stringify(PM.proj.layers).length;
    expect(batches[0].body.length).toBeLessThan(layersJson / 4);
    expect(batches[0].origin).toBe('edit');
    const ops = decodePatchOps(batches[1].body);
    expect(ops.some((op: any) => 'op' in op)).toBe(false);
    expect(ops.some((op: any) => op.path.includes('rotation'))).toBe(true);
    expect(decodePatchOps(batches[0].body).filter((op: any) => 'op' in op).map((op: any) => op.op)).toEqual(['order']);
    expect(batches[1].revision).toBe(batches[0].revision + 1);
    // Neither touches the assets; an asset edit does.
    expect(batches.map(b => b.assets)).toEqual([false, false]);
    PM.hist.do('Rename asset', () => { PM.proj.assets = { ...(PM.proj.assets || {}), x: { id: 'x', name: 'x.svg', kind: 'image' } }; });
    PM.doc.touched([['assets']]);
    doc.barrier!.flush();
    expect(batches[2]?.assets).toBe(true);
  });

  it('tells its pending listeners once when a write leaves something to flush', async () => {
    const { PM, doc } = await editor();
    populate(PM, rng(11), 6);
    doc.barrier!.flush();
    let told = 0;
    const off = doc.onPending(() => { told += 1; });
    PM.Edit.apply({ type: 'set_property', target: PM.proj.layers[0].id, path: 'rotation', value: 12 });
    PM.Edit.apply({ type: 'set_property', target: PM.proj.layers[0].id, path: 'rotation', value: 13 });
    expect(told).toBe(1);
    doc.barrier!.flush();
    PM.Edit.apply({ type: 'set_property', target: PM.proj.layers[0].id, path: 'rotation', value: 14 });
    expect(told).toBe(2);
    off();
    doc.barrier!.flush();
    PM.Edit.apply({ type: 'set_property', target: PM.proj.layers[0].id, path: 'rotation', value: 15 });
    expect(told).toBe(2);
  });

  it('resyncs when the project changes and diffs when a version of it replaces it', async () => {
    const { PM, doc } = await editor();
    populate(PM, rng(9), 6);
    doc.barrier!.flush();
    const resyncs = doc.barrier!.stats.resyncs;
    PM.replaceProject(JSON.parse(JSON.stringify(PM.proj)));
    doc.barrier!.flush();
    expect(doc.barrier!.stats.resyncs).toBe(resyncs);
    PM.replaceProject(PM.mkProject({ name: 'Other' }));
    doc.barrier!.flush();
    expect(doc.barrier!.stats.resyncs).toBe(resyncs + 1);
    expect(doc.wasm!.hash()).toBe(canonicalProjectHash(PM.proj));
  });

  it('carries load only on whole-document resyncs, so a receiver following the stream rules stays equal', async () => {
    const { PM, doc } = await editor('on');
    populate(PM, rng(21), 6);
    const snap = doc.snapshot()!;
    // A receiver as contract 5 describes it: `load` applies at any revision,
    // every other batch only at its base revision.
    const receiver = { revision: snap.revision, doc: JSON.parse(snap.json) as any };
    const batches: any[] = [];
    doc.subscribe(batch => {
      batches.push(batch);
      const ops = decodePatchOps(batch.body);
      if (batch.origin === 'load') {
        expect(ops.length).toBe(1);
        expect(ops[0]).toMatchObject({ path: [], exists: true });
        expect('op' in ops[0]!).toBe(false);
      } else expect(batch.baseRevision).toBe(receiver.revision);
      receiver.doc = applyOps(receiver.doc, ops);
      receiver.revision = batch.revision;
    });
    const expectReceiverEqual = () => {
      doc.barrier!.flush();
      expect(receiver.revision).toBe(doc.barrier!.revision);
      expect(canonicalStringify(receiver.doc)).toBe(canonicalStringify(canonicalProject(PM.proj)));
    };
    // Another composition, opened: rotateProject replaces PM.proj with a new
    // object of the same project.
    const comp = PM.Comps.create({ name: 'Second', w: 640, h: 360, dur: 3 }, { open: false });
    PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('solid')));
    expectReceiverEqual();
    const before = batches.length;
    expect(PM.Comps.open(comp)).toBe(true);
    expectReceiverEqual();
    const rotated = batches.slice(before);
    expect(rotated.length).toBeGreaterThan(0);
    expect(rotated.every(b => b.origin === 'structuralDiff')).toBe(true);
    PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('shape')));
    PM.hist.undo();
    PM.hist.redo();
    expectReceiverEqual();
    expect(PM.Comps.open(Object.keys(PM.proj.comps)[0])).toBe(true);
    expectReceiverEqual();
    // Another version of the same project: incremental.
    const copy = JSON.parse(JSON.stringify(PM.proj));
    copy.layers[0].name = 'Renamed in the copy';
    PM.replaceProject(copy);
    expectReceiverEqual();
    expect(batches.at(-1).origin).toBe('structuralDiff');
    // Another project: a resync.
    PM.replaceProject(PM.mkProject({ name: 'Other' }));
    expectReceiverEqual();
    expect(batches.at(-1).origin).toBe('load');
    const loads = batches.filter(b => b.origin === 'load');
    expect(loads.length).toBe(doc.barrier!.stats.resyncs - 1);
  });

  it('the consistency check finds writes the barrier missed and resyncs', async () => {
    const lines: string[] = [];
    const { PM, doc } = await editor('on', lines);
    populate(PM, rng(11), 4);
    expect(doc.check('test')!.ok).toBe(true);
    // A write with no touch(): invisible until the check.
    PM.proj.layers[1].name = 'Changed behind the barrier';
    const result = doc.check('save')!;
    expect(result.ok).toBe(false);
    expect(result.missed.some(p => p.endsWith('.name'))).toBe(true);
    expect(lines.some(line => line.includes('"consistency"'))).toBe(true);
    expect(doc.check('save')!.ok).toBe(true);
  });

  it('the auditor reports a write the barrier missed and repairs the mirror', async () => {
    const lines: string[] = [];
    const { PM, doc } = await editor('audit', lines);
    populate(PM, rng(12), 4);
    expect(doc.auditor!.audit()).toBe(true);
    PM.proj.layers[0].p.opacity.v = 12;
    expect(doc.auditor!.audit()).toBe(false);
    expect(doc.auditor!.stats.missedWrites).toBe(1);
    expect(lines.some(line => line.includes('"missed-write"'))).toBe(true);
    expect(doc.auditor!.audit()).toBe(true);
    expect(doc.api!.ev(PM.proj.layers[0], 'opacity', 0)).toBe(PM.ev(PM.proj.layers[0], 'opacity', 0));
  });

  it('returns live objects and fills expression errors on live channels', async () => {
    const { PM, doc } = await editor();
    const L = PM.mkLayer('solid');
    L.p.rotation.expr = 'time *';
    L.fx = [{ id: 'F1', type: 'blur', on: true, p: { amount: PM.P(5) } }];
    PM.hist.do('Add', () => PM.addLayer(L));
    const live = PM.proj.layers[0];
    expect(doc.api!.findProp(live, 'F1.amount')).toBe(live.fx[0].p.amount);
    const props = doc.api!.allProps(live);
    expect(props.find((p: any) => p.key === 'rotation').prop).toBe(live.p.rotation);
    doc.api!.ev(live, 'rotation', 0);
    expect(doc.api!.errors.get(live.p.rotation)).toBe('Expression syntax is invalid.');
    PM.hist.do('Fix', () => { live.p.rotation.expr = 'time * 2'; });
    expect(doc.api!.ev(live, 'rotation', 3)).toBe(6);
    expect(doc.api!.errors.has(live.p.rotation)).toBe(false);
    // A detached layer (api.model.mkLayer) is evaluated ad hoc.
    const detached = PM.mkLayer('null');
    detached.p['position.x'].v = 77;
    expect(doc.api!.ev(detached, 'position.x', 0)).toBe(77);
    expect(doc.api!.allProps(detached).length).toBe(PM.allProps(detached).length);
    expect(doc.api!.pick(live.p['position.x'].v, live.p['position.y'].v, 0)).toBe(live);
    expect(mismatches(doc)).toEqual([]);
  });

  it('gives a new receiver the document and continues revisions after a replaced instance', async () => {
    const { PM, doc } = await editor('on');
    const batches: any[] = [];
    doc.subscribe(b => batches.push(b));
    PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('solid')));
    doc.barrier!.flush();
    const snap = doc.snapshot()!;
    expect(snap.revision).toBe(doc.barrier!.revision);
    expect(snap.encoding).toBe('canonicalJsonV1');
    expect(snap.hash).toBe(canonicalProjectHash(PM.proj));
    expect(JSON.parse(snap.json)).toEqual(JSON.parse(JSON.stringify(canonicalProject(PM.proj))));
    // A receiver that starts from the snapshot applies the batches after it.
    batches.length = 0;
    PM.hist.do('Move', () => { PM.proj.layers[0].p['position.x'].v = 321; });
    doc.barrier!.flush();
    expect(batches.map(b => b.baseRevision)).toEqual([snap.revision]);
    // A trap replaces the instance: revisions keep moving forward.
    const before = doc.barrier!.revision;
    doc.wasm!.dead = true;
    await new Promise(r => setTimeout(r, 1100));
    for (let i = 0; i < 50 && !doc.ready; i++) await new Promise(r => setTimeout(r, 50));
    expect(doc.ready).toBe(true);
    expect(doc.barrier!.revision).toBeGreaterThan(before);
    const load = batches.at(-1);
    expect(load.origin).toBe('load');
    expect(load.revision).toBeGreaterThan(before);
  });
});

describe.skipIf(!built)('the identity registry across a replaced instance', () => {
  it('ignores ids of the trapped instance when their owners are collected', async () => {
    // A FinalizationRegistry whose callbacks the test fires (a real one runs
    // them only when the collector decides to).
    const registries: Array<{ cleanup: (held: any) => void; held: any[] }> = [];
    const Real = globalThis.FinalizationRegistry;
    (globalThis as any).FinalizationRegistry = class {
      private entry: { cleanup: (held: any) => void; held: any[] };
      constructor(cleanup: (held: any) => void) { this.entry = { cleanup, held: [] }; registries.push(this.entry); }
      register(_owner: object, held: any) { this.entry.held.push(held); }
      unregister() { return false; }
    };
    let PM: any, doc: EngineDoc;
    try { ({ PM, doc } = await editor('on')); } finally { (globalThis as any).FinalizationRegistry = Real; }
    const animated = (name: string, from: number) => {
      const L = PM.mkLayer('solid', { name });
      for (const key of ['position.x', 'position.y', 'rotation', 'opacity']) L.p[key].kf = [PM.KF(0, from), PM.KF(2, from + 100, 'easeOut')];
      return L;
    };
    PM.hist.do('Add', () => { PM.addLayer(animated('A', 10)); PM.addLayer(animated('B', 500)); });
    const [A, B] = ['A', 'B'].map(name => PM.proj.layers.find((l: any) => l.name === name));
    const keys = ['position.x', 'position.y', 'rotation', 'opacity'];
    const ask = (L: any) => keys.flatMap(key => [doc.api!.ev(L, key, 1.25), doc.api!.evP(L, L.p[key], 1.25, key)]);
    const expected = (L: any) => keys.flatMap(key => [PM.ev(L, key, 1.25), PM.evP(L, L.p[key], 1.25, key)]);
    // Interned in the first instance: A's channels get the low ids.
    expect(ask(A)).toEqual(expected(A));
    // Detached scopes also own a finalizer; select the property-id registry.
    const registry = registries.find(r => r.held.some(h => h && typeof h === 'object' && 'generation' in h))!;
    const before = registry.held.splice(0);
    expect(before.length).toBeGreaterThan(0);
    // A trap; the loader replaces the instance.
    doc.wasm!.dead = true;
    await new Promise(r => setTimeout(r, 1100));
    for (let i = 0; i < 50 && !doc.ready; i++) await new Promise(r => setTimeout(r, 50));
    expect(doc.ready).toBe(true);
    // In the new instance B is interned first, so its ids are the ones A had.
    expect(ask(B)).toEqual(expected(B));
    const heldId = (h: any): number => (typeof h === 'number' ? h : h.id);
    const reused = new Set(registry.held.map(heldId));
    expect(before.some(h => reused.has(heldId(h)))).toBe(true);
    // A's old owners are collected: nothing of the new instance is released.
    for (const held of before) registry.cleanup(held);
    expect(ask(B)).toEqual(expected(B));
    expect(ask(A)).toEqual(expected(A));
    expect(registry.held.every(h => h.generation === registry.held[0].generation)).toBe(true);
    expect(before.every(h => h.generation < registry.held[0].generation)).toBe(true);
  });
});

describe.skipIf(!built)('writes inside open transactions', () => {
  it('re-marks a scoped transaction\'s held paths on every announcement, and diffs inside a plain one', async () => {
    const { PM, doc } = await editor('on');
    PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('solid')));
    const L = PM.proj.layers[0];
    // A gesture: one track() at the start, many writes, touch() after each.
    PM.hist.beginScoped('Drag');
    PM.hist.track([['layers', 0]]);
    for (const x of [10, 20, 30]) {
      L.p['position.x'].v = x;
      PM.touch();
      expect(doc.api!.ev(L, 'position.x', 0)).toBe(x);
    }
    PM.hist.commit('Drag');
    // A plain transaction: queries inside it see its writes after touch().
    PM.hist.begin('Move');
    L.p['position.y'].v = 77;
    PM.touch();
    expect(doc.api!.ev(L, 'position.y', 0)).toBe(77);
    PM.hist.commit('Move');
    expect(doc.check('test')!.ok).toBe(true);
  });

  it('answers a query inside a plain transaction from writes nobody announced yet', async () => {
    const { PM, doc } = await editor('on');
    PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('solid')));
    const L = PM.proj.layers[0];
    const added = PM.mkLayer('solid');
    doc.barrier!.flush();
    // The pen tool: begin, insert, then ask before any touch().
    PM.hist.begin('Draw');
    L.p['position.x'].v = 41;
    PM.proj.layers.unshift(added);
    expect(doc.api!.ev(L, 'position.x', 0)).toBe(41);
    expect(doc.api!.ev(added, 'position.x', 0)).toBe(PM.ev(added, 'position.x', 0));
    expect(doc.api!.stats.repairs).toBe(0);
    PM.hist.commit('Draw');
    expect(doc.check('test')!.ok).toBe(true);
  });

  it('does not announce the speculative diff a flush marks inside an open transaction', async () => {
    const { PM, doc } = await editor('on');
    PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('solid')));
    const L = PM.proj.layers[0];
    doc.barrier!.flush();
    // A listener that flushes whenever told, as the engine connection does:
    // the speculative marks of an open plain transaction must not tell it,
    // or every flush would ask for the next one.
    let told = 0;
    doc.onPending(() => {
      told += 1;
      // Bounded, so a regression fails here instead of starving the loop.
      if (told < 8) queueMicrotask(() => doc.barrier!.flush());
    });
    PM.hist.begin('Drag');
    for (let run = 0; run < 4; run++) {
      doc.api!.ev(L, 'position.x', 0);
      await Promise.resolve();
      await Promise.resolve();
    }
    expect(told).toBe(0);
    PM.hist.commit('Drag');
    expect(doc.check('test')!.ok).toBe(true);
  });

  it('diffs once per synchronous run inside a plain transaction', async () => {
    const { PM, doc } = await editor('on');
    PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('solid')));
    const L = PM.proj.layers[0];
    doc.barrier!.flush();
    const diffs = () => doc.barrier!.stats.structuralDiffs;
    PM.hist.begin('Drag');
    const before = diffs();
    for (let i = 0; i < 5; i++) doc.api!.ev(L, 'position.x', 0);
    expect(diffs()).toBe(before + 1);
    await Promise.resolve();
    L.p['position.x'].v = 12;
    expect(doc.api!.ev(L, 'position.x', 0)).toBe(12);
    expect(diffs()).toBe(before + 2);
    PM.hist.commit('Drag');
    expect(doc.check('test')!.ok).toBe(true);
  });

  it('diffs the entered composition on a touch() without paths', async () => {
    const { PM, doc } = await editor('on');
    const sub = PM.mkProject({ w: 100, h: 100, dur: 2 });
    const inner = PM.mkLayer('solid', {}, sub);
    sub.layers.push(inner);
    PM.hist.do('Nest', () => { PM.proj.comps[sub.id] = sub; });
    doc.barrier!.flush();
    PM.scope = [PM.proj.comps[sub.id]];
    try {
      inner.p['position.x'].v = 33;
      PM.touch();
      expect(doc.api!.ev(inner, 'position.x', 0)).toBe(33);
      expect(doc.barrier!.shadowMatches().ok).toBe(true);
    } finally { PM.scope = []; }
  });

  it('diffs compositions added outside the open one on a structural diff', async () => {
    const { PM, doc } = await editor('on');
    doc.barrier!.flush();
    const id = 'Cadded01';
    PM.proj.comps = { ...(PM.proj.comps || {}), [id]: { id, name: 'Nested', w: 100, h: 100, fps: 30, dur: 2, bg: '#000000', layers: [] } };
    PM.touch();
    doc.barrier!.flush();
    expect(doc.barrier!.shadowMatches().ok).toBe(true);
    expect(doc.check('test')!.ok).toBe(true);
  });

  it('marks an announcement before the bus listeners ask, inside a scoped transaction too', async () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/compositor');
    // The kernel's `project:changed` bridge subscribes before the mirror is
    // installed: the viewer asks as the bus announces.
    let ask = () => {};
    PM.bus.on('layers', () => ask());
    const doc: EngineDoc = installEngineDoc(PM, { mode: parseMode('audit'), loadModule, sink: () => {}, auditMs: 1e9 });
    PM.proj = PM.mkProject({ dur: 10 });
    PM.time = 0;
    expect(await doc.whenReady).toBe(true);
    doc.auditor?.stop();
    PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('solid')));
    doc.barrier!.flush();
    const seen: unknown[] = [];
    ask = () => { for (const L of PM.proj.layers) seen.push(doc.api!.ev(L, 'position.x', 0)); };
    // Edit.apply: a scoped transaction tracks `layers`, asks, inserts, announces.
    PM.hist.beginScoped('Add layer');
    PM.hist.track([['layers'], ['comps']]);
    doc.api!.ev(PM.proj.layers[0], 'position.x', 0);
    const added = PM.mkLayer('solid', { p: { 'position.x': 123 } });
    PM.proj.layers.unshift(added);
    PM.bus.emit('layers');
    PM.hist.commit();
    expect(seen).toContain(PM.ev(added, 'position.x', 0));
    expect(doc.api!.stats.repairs).toBe(0);
    expect(doc.dual!.total().stale).toBe(0);
    expect(doc.auditor!.audit()).toBe(true);
  });

  it('hands a commit\'s paths over before the commit\'s listeners ask (the pen tool\'s overlay)', async () => {
    const { PM, doc } = await editor('audit');
    PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('solid')));
    const L = PM.proj.layers[0];
    const masks = (L.masks ||= []);
    PM.hist.do('Mask', () => masks.push(PM.mkMask('rect', PM.proj)));
    doc.barrier!.flush();
    const vertices: any[] = masks[0].path?.vertices ?? (masks[0].path = { id: 'p', vertices: [] }).vertices;
    const answers: unknown[] = [];
    // The viewer redraws its overlay (and so asks) as history announces the
    // new entry, before the commit's own touch().
    const ask = () => { for (const v of vertices) answers.push(doc.api!.evP(L, v.p.x, 0, `mp.${masks[0].id}.v.${v.id}.x`)); };
    PM.bus.on('history', ask);
    PM.bus.on('history:project-patch', ask);
    for (const x of [10, 20, 30]) {
      PM.hist.begin('Draw path vertex');
      // Asked before the write inside the transaction, as the pen tool does.
      ask();
      vertices.push({ id: PM.uid('v'), p: { x: PM.P(x), y: PM.P(x) } });
      PM.hist.commit();
    }
    expect(answers).toContain(30);
    expect(doc.dual!.total().stale).toBe(0);
    expect(doc.auditor!.audit()).toBe(true);
  });

  it('serves structural answers again until the composition shape changes', async () => {
    const { PM, doc } = await editor('on');
    const api = doc.api!;
    for (const name of ['A', 'B', 'C']) PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('solid', { name })));
    const byName = (name: string) => PM.proj.layers.find((l: any) => l.name === name);
    PM.groupLayers([byName('A').id, byName('B').id], 'Group');
    const a = byName('A'), c = byName('C'), group = PM.proj.layers.find((l: any) => l.type === 'group');
    const ids = (list: any[]) => list.map(l => l.id);
    expect(ids(api.groupAncestors(a))).toEqual([group.id]);
    expect(api.wouldCycle(group, a.id)).toBe(true);
    expect(api.is3DLayer(a)).toBe(false);
    const hits = api.stats.shapeHits;
    // A channel edit (and the provenance it records) leaves the shape alone.
    expect(PM.Edit.apply({ type: 'set_property', target: a.id, path: 'opacity', value: 40 }).ok).toBe(true);
    const first = api.groupAncestors(a);
    expect(ids(first)).toEqual([group.id]);
    first.pop();
    expect(ids(api.groupAncestors(a)), 'callers get their own copy').toEqual([group.id]);
    expect(api.wouldCycle(group, a.id)).toBe(true);
    expect(api.is3DLayer(a)).toBe(false);
    expect(api.stats.shapeHits).toBe(hits + 4);
    // Membership, the 3D switch, and parenting change the answers.
    PM.hist.do('Move', () => { c.group = group.id; PM.touch(); });
    expect(ids(api.groupAncestors(c))).toEqual([group.id]);
    PM.hist.do('3D', () => { group.threeD = true; PM.touch(); });
    expect(api.is3DLayer(a)).toBe(true);
    PM.hist.do('Parent', () => { group.parent = c.id; PM.touch(); });
    expect(api.wouldCycle(c, group.id)).toBe(true);
    expect(api.stats.shapeHits).toBe(hits + 4);
    // Undo moves the shape back; the answers follow.
    PM.hist.undo(); PM.hist.undo(); PM.hist.undo();
    expect(ids(api.groupAncestors(c))).toEqual([]);
    expect(api.is3DLayer(a)).toBe(false);
    expect(api.stats.repairs).toBe(0);
  });

  it('sends an edit before its listeners run while an engine session wants it', async () => {
    const { PM, doc } = await editor('on');
    PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('solid', { name: 'A' })));
    doc.barrier!.flush();
    const layer = PM.proj.layers[0];
    const order: string[] = [];
    const offBatch = doc.subscribe(batch => order.push(`batch ${batch.origin}`));
    PM.bus.on('layers', () => order.push('layers'));
    PM.bus.on('history', () => order.push('history'));
    // Without an engine session: the batch waits for a query or the end of the task.
    expect(PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'opacity', value: 40 }).ok).toBe(true);
    expect(order).toEqual(['layers', 'history']);
    doc.barrier!.flush();
    expect(order).toEqual(['layers', 'history', 'batch edit']);
    order.length = 0;
    let wanted = true;
    const off = doc.flushBeforeListeners(() => wanted);
    expect(PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'opacity', value: 50 }).ok).toBe(true);
    expect(order[0]).toBe('batch edit');
    expect(order.slice(1)).toEqual(['layers', 'history']);
    // The batch carried the edit: nothing is left for the end of the task.
    doc.barrier!.flush();
    expect(order.filter(o => o.startsWith('batch'))).toHaveLength(1);
    expect(JSON.parse(doc.wasm!.toJson()).comps[PM.proj.compId].layers[0].p.opacity.v).toBe(50);
    order.length = 0;
    wanted = false;
    expect(PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'opacity', value: 60 }).ok).toBe(true);
    expect(order).toEqual(['layers', 'history']);
    off();
    offBatch();
  });

  it('names the patch paths that can change a composition shape', async () => {
    const { changesShape } = await import('./barrier');
    expect(changesShape([])).toBe(true);
    expect(changesShape(['compId'])).toBe(true);
    expect(changesShape(['comps'])).toBe(true);
    expect(changesShape(['comps', 'C1'])).toBe(true);
    expect(changesShape(['comps', 'C1', 'layers'])).toBe(true);
    expect(changesShape(['comps', 'C1', 'layers', 3])).toBe(true);
    for (const member of ['id', 'type', 'parent', 'group', 'threeD', 'lock']) expect(changesShape(['comps', 'C1', 'layers', 3, member])).toBe(true);
    expect(changesShape(['comps', 'C1', 'layers', 3, 'p', 'opacity', 'v'])).toBe(false);
    expect(changesShape(['comps', 'C1', 'layers', 3, 'fx'])).toBe(false);
    expect(changesShape(['comps', 'C1', 'w'])).toBe(false);
    expect(changesShape(['edits'])).toBe(false);
    expect(changesShape(['revision'])).toBe(false);
    expect(changesShape(['layers', 0, 'group'])).toBe(true);
    expect(changesShape(['layers', 0, 'p'])).toBe(false);
  });

  it('treats ProjectIndex.invalidate() as an announcement of the layer structure', async () => {
    const { PM, doc } = await editor('audit');
    PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('solid')));
    doc.barrier!.flush();
    expect(typeof PM.ProjectIndex?.invalidate).toBe('function');
    // Layers replaced outside history, the index invalidated, then a
    // selection that redraws the overlay asks before any touch().
    const layer = PM.mkLayer('solid', { p: { 'position.x': 321 } });
    PM.proj.layers = [layer];
    PM.ProjectIndex.invalidate();
    const legacy = PM.ev(layer, 'position.x', 0);
    expect(doc.api!.ev(layer, 'position.x', 0)).toBe(legacy);
    expect(doc.api!.groupAncestors(layer)).toEqual([]);
    expect(doc.api!.stats.repairs).toBe(0);
    expect(doc.dual!.total().stale).toBe(0);
    expect(doc.auditor!.audit()).toBe(true);
  });
});

describe('dual answers: allowed differences (amendment 20)', () => {
  it('allows 180 against -180 and nothing else near it', async () => {
    const { DualAnswers } = await import('./dual');
    const dual = new DualAnswers();
    const id = <T,>(v: T) => v;
    expect(dual.compare('ev', { value: 180 }, { value: -180 }, () => ({}), id)).toBe(true);
    expect(dual.compare('evP', { value: [-180.0000000001, 3] }, { value: [180, 3] }, () => ({}), id)).toBe(true);
    expect(dual.stats.allowed).toEqual({ 'ev:atan2': 1, 'evP:atan2': 1 });
    expect(dual.compare('ev', { value: 179 }, { value: -181 }, () => ({}), id)).toBe(false);
    expect(dual.compare('ev', { value: 540 }, { value: 180 }, () => ({}), id)).toBe(false);
    // Bounds keep their own tolerance: no branch-cut class there.
    expect(dual.compare('gl.bounds', { value: { x0: 180 } }, { value: { x0: -180 } }, () => ({}), id)).toBe(false);
    expect(dual.total().mismatched).toBe(3);
  });

  it('classes a bounds difference as fragile when the legacy answer moves under a perturbation', async () => {
    const { PM, doc } = await editor();
    PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('solid')));
    const L = PM.proj.layers[0];
    const legacy = (doc.api as any).legacy;
    const original = legacy.bounds;
    // A legacy answer that jumps at t = 1 (a key edge): fragile there.
    legacy.bounds = (layer: any, t: number) => { const b = original(layer, t); return b && t >= 1 ? { ...b, x0: b.x0 + 5 } : b; };
    doc.api!.bounds(L, 1);
    expect(doc.dual!.stats.fragile['gl.bounds']).toBe(1);
    expect(doc.dual!.stats.allowed['gl.bounds:fragile']).toBe(1);
    expect(doc.dual!.stats.mismatched['gl.bounds'] ?? 0).toBe(0);
    // Away from the edge the same difference is a mismatch.
    doc.api!.bounds(L, 2);
    expect(doc.dual!.stats.mismatched['gl.bounds']).toBe(1);
    expect(doc.dual!.fragileBudget()).toEqual({ fragile: 1, differed: 1, ok: false });
    legacy.bounds = original;
  });

  it('classes a pick on a layer\'s edge as fragile, though no nudge of the point moves the answer', async () => {
    const { PM, doc } = await editor();
    const solid = PM.mkLayer('solid');
    PM.hist.do('Add', () => PM.addLayer(solid));
    const L = PM.proj.layers[0];
    const b = PM.GL.bounds(L, 0);
    const legacy = (doc.api as any).legacy;
    const original = legacy.pick;
    // A legacy pick that never moves under a nudge but differs from the native answer.
    legacy.pick = () => null;
    const plane = legacy.planeMatrix(L, 0);
    const at = (u: number, v: number) => legacy.projectPoint(plane, { x: u, y: v });
    const edge = at(b.x0, (b.y0 + b.y1) / 2);
    doc.api!.pick(edge.x, edge.y, 0);
    expect(doc.dual!.stats.fragile['gl.pick']).toBe(1);
    expect(doc.dual!.stats.allowed['gl.pick:fragile']).toBe(1);
    expect(doc.dual!.stats.mismatched['gl.pick'] ?? 0).toBe(0);
    // The middle of the layer is no edge: the same difference is a mismatch.
    const middle = at((b.x0 + b.x1) / 2, (b.y0 + b.y1) / 2);
    doc.api!.pick(middle.x, middle.y, 0);
    expect(doc.dual!.stats.mismatched['gl.pick']).toBe(1);
    legacy.pick = original;
  });

  it('classes the bounds of a group with an ill-conditioned matrix as fragile', async () => {
    const { PM, doc } = await editor();
    PM.hist.do('Add', () => { PM.addLayer(PM.mkLayer('solid', { name: 'A' })); PM.addLayer(PM.mkLayer('solid', { name: 'B' })); });
    PM.groupLayers(PM.proj.layers.map((l: any) => l.id), 'Group');
    PM.touch();
    const group = PM.proj.layers.find((l: any) => l.type === 'group');
    expect(group).toBeTruthy();
    const legacy = (doc.api as any).legacy;
    const original = { bounds: legacy.bounds, worldMatrix: legacy.worldMatrix };
    legacy.bounds = (layer: any, t: number) => { const r = original.bounds(layer, t); return r && { ...r, x0: r.x0 + 1 }; };
    // Well conditioned: a mismatch.
    doc.api!.bounds(group, 0);
    expect(doc.dual!.stats.mismatched['gl.bounds']).toBe(1);
    expect(doc.dual!.stats.fragile['gl.bounds'] ?? 0).toBe(0);
    // Squashed to 1e-6 on one axis (condition about 1e6): fragile.
    legacy.worldMatrix = () => [1, 0, 0, 1e-6, 0, 0];
    doc.api!.bounds(group, 0);
    expect(doc.dual!.stats.fragile['gl.bounds']).toBe(1);
    expect(doc.dual!.stats.allowed['gl.bounds:fragile']).toBe(1);
    expect(doc.dual!.stats.mismatched['gl.bounds']).toBe(1);
    Object.assign(legacy, original);
  });
});

describe('deepKeyOrder', () => {
  it('orders members as the live content at every depth, and computed bounds as the legacy code', async () => {
    const { deepKeyOrder } = await import('./native');
    const live = { text: { v: 'a', kf: [], expr: null }, animators: [{ id: 'A', name: 'Rise', enabled: true }], data: { objects: [], camera: {} } };
    const answer = { animators: [{ enabled: true, id: 'A', name: 'Rise' }], data: { camera: {}, objects: [] }, fontAnchorBounds: { x0: 1, x1: 2, y0: 3, y1: 4 }, text: 'a' };
    const ordered = deepKeyOrder(answer, live);
    expect(JSON.stringify(ordered)).toBe(JSON.stringify({ text: 'a', animators: [{ id: 'A', name: 'Rise', enabled: true }], data: { objects: [], camera: {} }, fontAnchorBounds: { x0: 1, y0: 3, x1: 2, y1: 4 } }));
  });

  it('keeps a property answered unevaluated (extension params) in the property\'s own order', async () => {
    const { deepKeyOrder } = await import('./native');
    const live = { params: { yaw: { v: 28, kf: [], expr: null }, pitch: { v: { x: 1, y: 2 }, kf: [], expr: null } } };
    const answer = { params: { pitch: { expr: null, kf: [], v: { x: 1, y: 2 } }, yaw: { expr: null, kf: [], v: 28 } } };
    expect(JSON.stringify(deepKeyOrder(answer, live))).toBe(JSON.stringify(live));
  });
});

describe.skipIf(!built)('the tracking Proxy', () => {
  it('names the path of keys adopted in a composition that is not open', async () => {
    const { PM, doc } = await editor('audit');
    const sub = PM.mkProject({ w: 100, h: 100, dur: 2 });
    const star = PM.mkLayer('solid', {}, sub);
    star.p.rotation.kf = [
      { t: 0, v: 0, eo: [0.4, 0], inInterp: 'bezier', outInterp: 'bezier' },
      { t: 1, v: 90, ei: [0.6, 1], inInterp: 'bezier', outInterp: 'bezier' },
    ];
    sub.layers.push(star);
    PM.hist.do('Nest', () => { PM.proj.comps[sub.id] = sub; });
    doc.barrier!.flush();
    const diffs = doc.barrier!.stats.structuralDiffs;
    // Evaluating the nested layer adopts its keys (a write in comps[sub.id]).
    PM.ev(star, 'rotation', 0.5);
    expect(Object.prototype.propertyIsEnumerable.call(star.p.rotation.kf[0], 'eo')).toBe(false);
    doc.barrier!.flush();
    expect(doc.barrier!.stats.structuralDiffs).toBe(diffs);
    expect(doc.auditor!.audit()).toBe(true);
    expect(doc.check('test')!.ok).toBe(true);
  });

  it('treats legacy key adoption during evaluation as a known writer, and keeps the mirror right', async () => {
    const { PM, doc } = await editor('strict');
    PM.hist.do('Add', () => PM.addLayer(PM.mkLayer('solid')));
    // Keys in the stored legacy form: evaluation adopts them (writes speed and
    // influence, turns eo/ei into views), which strict mode would throw on if
    // it were not a writer.
    PM.hist.do('Keys', () => {
      PM.proj.layers[0].p.rotation.kf = [
        { t: 0, v: 0, eo: [0.4, 0], inInterp: 'bezier', outInterp: 'bezier' },
        { t: 1, v: 90, ei: [0.6, 1], inInterp: 'bezier', outInterp: 'bezier' },
      ];
    });
    const L = PM.proj.layers[0];
    expect(() => PM.ev(L, 'rotation', 0.5)).not.toThrow();
    expect(doc.tracker!.violations()).toEqual([]);
    expect(Object.prototype.propertyIsEnumerable.call(PM.proj.layers[0].p.rotation.kf[0], 'eo')).toBe(false);
    expect(doc.api!.ev(L, 'rotation', 0.5)).toBe(PM.ev(L, 'rotation', 0.5));
    expect(doc.auditor!.audit()).toBe(true);
    expect(mismatches(doc)).toEqual([]);
  });

  it('diffs on a touch() that announces writes made through a reference the Proxy never handed out', async () => {
    const { PM, doc } = await editor('audit-proxy');
    const raw = PM.mkLayer('solid');
    PM.hist.do('Add', () => PM.addLayer(raw));
    // `raw` is the factory's object, not a proxy: the Proxy cannot see this.
    raw.p['position.x'].v = 432;
    PM.touch();
    expect(doc.api!.ev(PM.proj.layers[0], 'position.x', 0)).toBe(432);
    expect(doc.dual!.total().stale).toBe(0);
    expect(doc.auditor!.audit()).toBe(true);
  });
});
