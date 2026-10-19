/*
 * "Use New Engine" on (docs/engine/write-path.md, "Named failures and exact
 * answers"): detached inputs are answered natively from their JSON, a trap
 * is answered by a fresh instance, and what cannot be answered is a named
 * failure; the development modes stay development-only.
 */
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it, vi } from 'vitest';

import { makePM } from '../legacy/__tests__/make-pm';
import { makeVectorPath, makeVertex } from '../legacy/core/vector-paths';
import { installEngineDoc, type EngineDoc } from './install';
import { parseMode, profileOverride, workerMode } from './flags';
import type { FontProvider } from './fonts';
import { createRequire } from 'node:module';

const MODULE = process.env['POWERMOVE_PM_WASM'] ?? fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url));
const BRIDGE = process.env['POWERMOVE_PM_BRIDGE'] ?? fileURLToPath(new URL('../../../../../../engine/target/pm-bridge/pm_bridge.node', import.meta.url));
let compiled: Promise<WebAssembly.Module> | null = null;
const loadModule = () => (compiled ??= WebAssembly.compile(readFileSync(MODULE)));

async function editor(mode: string, strict = true, fonts?: FontProvider) {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/compositor');
  const doc: EngineDoc = installEngineDoc(PM, { mode: parseMode(mode), loadModule, sink: () => {}, auditMs: 1e9, strict, ...(fonts ? { fonts } : {}) });
  PM.proj = PM.mkProject({ dur: 10 });
  PM.time = 0;
  expect(await doc.whenReady).toBe(true);
  doc.auditor?.stop();
  return { PM, doc };
}

/** Two parented solids and a shape layer with a two-level path group, some keys and an expression. */
function content(PM: any) {
  const parent = PM.mkLayer('solid', { name: 'Parent' });
  parent.p['position.x'].kf = [PM.KF(0, 100), PM.KF(2, 400)];
  parent.p.rotation.v = 30;
  const child = PM.mkLayer('solid', { name: 'Child', parent: parent.id });
  child.p['position.y'].expr = 'time * 40 + value';
  const shape = PM.mkLayer('shape', { name: 'Shape' });
  const group = makeVectorPath(PM, 'Group');
  group.p.rotation = PM.P(15, { kf: [PM.KF(0, 0), PM.KF(1, 90)] });
  group.p.x.v = 40;
  const path = makeVectorPath(PM, 'Path');
  path.parent = group.id;
  path.vertices = [makeVertex(PM, 0, 0), makeVertex(PM, 100, 0), makeVertex(PM, 100, 60)];
  path.p.scaleX.v = 150;
  shape.d.paths = [group, path];
  PM.proj.layers = [child, parent, shape];
  PM.ProjectIndex.invalidate();
  PM.touch();
  return { parent, child, shape, group, path };
}

const clone = (v: any) => JSON.parse(JSON.stringify(v));
const fallbacks = (doc: EngineDoc) => (doc.stats() as any).native.fallbacks as Record<string, number>;

describe.skipIf(!existsSync(MODULE))('exact answers with the new engine on', () => {
  it('answers detached channels, parents and paths natively, equal to the legacy answers', async () => {
    const { PM, doc } = await editor('audit');
    const { parent, child, shape, group, path } = content(PM);
    const api = doc.api!;
    const detachedParent = clone(parent);
    detachedParent.p.rotation.v = 45; // a parent that differs from the one in the composition
    const detachedProp = clone(child.p['position.y']);
    detachedProp.kf = [PM.KF(0.5, 3), PM.KF(1, 9)];
    const detachedPath = clone(path);
    detachedPath.p.scaleY.v = 50;
    const orphan = clone(path);
    orphan.parent = 'no-such-group';
    for (const t of [0, 0.5, 1, 1.75]) {
      api.transformParentMatrix(child, t, detachedParent);
      api.transformParentMatrix(child, t, PM.mkLayer('null'));
      api.parent3D(child, t, detachedParent);
      api.hasKeyAt(child, detachedProp, t + child.from);
      api.pathValues(shape, detachedPath, t);
      api.groupMatrix(shape, detachedPath, t);
      api.groupMatrix(shape, orphan, t);
      api.groupMatrix(shape, clone(group), t);
    }
    // The live key comes back for a detached channel (found by index).
    expect(api.hasKeyAt(child, detachedProp, 0.5 + child.from)).toBe(detachedProp.kf[0]);
    const dual = doc.dual!;
    expect(dual.total().mismatched).toBe(0);
    for (const op of ['transformParentMatrix', 'space3d.parent3D', 'hasKeyAt', 'pathValues', 'groupMatrix']) expect(dual.stats.compared[op]).toBeGreaterThan(0);
    expect(Object.keys(fallbacks(doc)).filter((k) => /detached|context|trap|unavailable/.test(k))).toEqual([]);
  });

  it('answers a trapped question on a fresh instance, natively', async () => {
    const { PM, doc } = await editor('on');
    const { child } = content(PM);
    const api = doc.api!;
    const expected = api.worldMatrix(child, 1);
    const wasm = doc.wasm!;
    const query = wasm.query.bind(wasm);
    let traps = 1;
    wasm.query = (request: object) => {
      if (traps-- > 0) { wasm.dead = true; throw new WebAssembly.RuntimeError('unreachable'); }
      return query(request);
    };
    expect(api.worldMatrix(child, 1)).toEqual(expected);
    expect(doc.wasm).not.toBe(wasm);
    expect(fallbacks(doc)).toEqual({});
    expect(doc.failures()).toEqual([]);
  });

  it('reports a question that traps twice as a named failure', async () => {
    const { PM, doc } = await editor('on');
    const { child } = content(PM);
    const failures: any[] = [];
    PM.bus.on('engine:failure', (f: any) => failures.push(f));
    const { PmWasm } = await import('./wasm');
    const original = PmWasm.prototype.query;
    PmWasm.prototype.query = function (this: any) { this.dead = true; throw new WebAssembly.RuntimeError('unreachable'); };
    try {
      expect(() => doc.api!.worldMatrix(child, 1)).toThrow(/worldMatrix.*trap/);
    } finally { PmWasm.prototype.query = original; }
    expect(failures.map((f) => [f.code, f.retry])).toContainEqual(['pm-wasm-trap', false]);
    expect(doc.failures()[0]!.message).toMatch(/trapped twice answering worldMatrix/);
    // The engine keeps answering natively after it.
    expect(doc.ready).toBe(true);
    doc.api!.worldMatrix(child, 1);
    expect(fallbacks(doc)['worldMatrix:trap']).toBe(1);
  });

  it('names a module that does not load, and loads it on retry', async () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/compositor');
    let available = false;
    const doc = installEngineDoc(PM, { mode: parseMode('on'), strict: true, sink: () => {}, loadModule: () => available ? loadModule() : Promise.reject(new Error('404 fetching the module')) });
    PM.proj = PM.mkProject({ dur: 10 });
    expect(await doc.whenReady).toBe(false);
    expect(doc.failures().map((f) => [f.code, f.retry])).toEqual([['pm-wasm-unavailable', true]]);
    available = true;
    expect(await doc.retry()).toBe(true);
    expect(doc.ready).toBe(true);
  });

  it('answers nested composition paths and a root without compId natively', async () => {
    const { PM, doc } = await editor('audit');
    const { child } = content(PM);
    const nested = { id: 'deep', fps: 24, layers: [clone(child)], w: 640, h: 480, dur: 10 };
    PM.proj.comps.outer = { id: 'outer', layers: [], comps: { deep: nested } };
    PM.scope = [nested];
    PM.curComp = () => PM.scope.at(-1) ?? PM.proj;
    PM.doc.touched([['comps']]);
    doc.api!.worldMatrix(nested.layers[0], 1);
    doc.api!.evP(nested.layers[0], nested.layers[0].p['position.y'], 1, 'position.y');
    PM.proj.comps = {};
    PM.doc.touched([['comps']]);
    doc.api!.worldMatrix(nested.layers[0], 1);
    PM.scope = [];
    delete PM.proj.compId;
    PM.touch();
    doc.api!.worldMatrix(child, 1);
    expect(fallbacks(doc)).toEqual({});
    expect(doc.dual!.total().mismatched).toBe(0);
  });

  it.runIf(existsSync(BRIDGE))('recovers a repeatedly failing font provider on retry without a legacy answer', async () => {
    const { FontRegistry } = createRequire(import.meta.url)(BRIDGE) as any;
    const registry = new FontRegistry();
    let available = false;
    const { PM, doc } = await editor('on', true, {
      sync: true,
      answer: q => { if (!available) throw new Error('temporarily unavailable'); return registry.answer(q); },
      subset: (b, i) => registry.subset(b, i), whole: (b, i) => registry.whole(b, i),
    });
    const text = PM.mkLayer('text');
    PM.proj.layers = [text]; PM.touch();
    for (let i = 0; i < 3; i++) expect(() => doc.api!.textLayout(text, 0)).toThrow(/fonts/);
    expect(doc.api!.fonts!.disabled).toBe(true);
    available = true;
    expect(await doc.retry()).toBe(true);
    expect(doc.api!.fonts!.disabled).toBe(false);
    expect(doc.api!.textLayout(text, 0)).toBeTruthy();
    expect(doc.failures().some(f => f.code === 'font-service-unavailable' && f.retry)).toBe(true);
  });

  it('ignores worker evaluator environment overrides in production', () => {
    for (const override of ['audit', 'audit-proxy', 'strict', 'off', 'on', undefined]) {
      expect(workerMode(override, true, false)).toEqual(parseMode('on'));
      expect(workerMode(override, false, false)).toEqual(parseMode('off'));
    }
    expect(workerMode('audit', true, true).dual).toBe(true);
    expect(workerMode('strict', false, true).proxy).toBe('throw');
  });

  it('never calls a legacy answer for a strict refusal, flush failure or exception', async () => {
    const { PM, doc } = await editor('on');
    const { child } = content(PM);
    const legacy = vi.spyOn(PM, 'worldMatrix');
    const query = doc.wasm!.query.bind(doc.wasm);
    for (const reply of [{ host: true }, { ink: true }, { fonts: true }, { legacy: true }]) {
      doc.wasm!.query = () => reply;
      expect(() => doc.api!.worldMatrix(child, 0)).toThrow(/new engine could not answer/);
    }
    doc.wasm!.query = query;
    const flush = doc.barrier!.flush.bind(doc.barrier);
    doc.barrier!.flush = () => { throw new Error('flush failed'); };
    expect(() => doc.api!.worldMatrix(child, 0)).toThrow(/new engine could not answer/);
    doc.barrier!.flush = flush;
    expect(legacy).not.toHaveBeenCalled();
    const editLegacy = vi.fn();
    for (const reply of [{ host: true }, { ink: true }, { fonts: true }, { legacy: true }]) {
      doc.wasm!.query = () => reply;
      expect(() => doc.edits!.edit('captureFontAnchor', {}, editLegacy, () => undefined)).toThrow(/new engine could not answer/);
    }
    expect(editLegacy).not.toHaveBeenCalled();
    doc.wasm!.query = query;
    expect(doc.failures().length).toBeGreaterThan(0);
  });

  it('keeps the development modes out of a profile override in production builds', () => {
    expect(profileOverride('audit', false)!.name).toBe('on');
    expect(profileOverride('strict', false)!.dual).toBe(false);
    expect(profileOverride('off', false)!.evaluator).toBe(false);
    expect(profileOverride('audit', true)!.name).toBe('audit');
    expect(profileOverride(null, false)).toBeNull();
  });

  it.runIf(existsSync(BRIDGE))("loads an open project's faces before anything asks, so an editor's first text question is native", async () => {
    // The editor's provider is asynchronous (main relays to the font service).
    const { FontRegistry } = createRequire(import.meta.url)(BRIDGE) as { FontRegistry: new () => { answer(q: string): string; subset(b: string, i: number): Uint8Array } };
    const registry = new FontRegistry();
    const provider: FontProvider = { sync: false, answer: async (q) => registry.answer(q), subset: async (b, i) => registry.subset(b, i) };
    const { PM, doc } = await editor('on', true, provider);
    const title = PM.mkLayer('text', { name: 'Title' });
    title.d.text = 'Warm faces';
    PM.proj.layers = [title];
    PM.ProjectIndex.invalidate();
    PM.touch();
    await doc.warmFonts();
    const layout = doc.api!.textLayout(title, 0);
    expect(layout).toBeTruthy();
    expect(fallbacks(doc)).toEqual({});
    // Without warming, the same first question waits on the provider and falls back.
    const { PM: cold, doc: coldDoc } = await editor('on', true, { ...provider });
    const other = cold.mkLayer('text', { name: 'Title' });
    other.d.text = 'Cold faces';
    cold.proj.layers = [other];
    cold.ProjectIndex.invalidate();
    cold.touch();
    expect(() => coldDoc.api!.textLayout(other, 0)).toThrow(/textLayout.*fonts/);
    expect(fallbacks(coldDoc)).toEqual({ 'textLayout:fonts': 1 });
  });
});
