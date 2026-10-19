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
import { groupMatrix, makeVectorPath, makeVertex, pathValues } from '../legacy/core/vector-paths';
import { installEngineDoc, type EngineDoc } from './install';
import { COMP_FIELDS, rootComp } from '../legacy/core/compositions';
import { ROOT_WRAPPER_FIELDS } from './native';
import { parseMode, profileOverride, workerMode } from './flags';
import { runCommandWhenReady } from './readiness';
import type { FontProvider } from './fonts';
import { PmWasm } from './wasm';
import { installBridgeForTests, resetBridgeForTests } from '../kernel/bridge';
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
  it('keeps window font seeds with the synchronous whole-face provider', async () => {
    const seed = (size: number, specified: number) => JSON.stringify({ seeds: [
      { family: 'system-ui', weight: 400, italic: false, size, specified },
    ] });
    const initial = seed(26, 13), changed = seed(30, 15);
    let onChange!: (json: string) => void;
    const applied = vi.spyOn(PmWasm.prototype, 'setFontSeeds');
    installBridgeForTests({ engineFonts: {
      answer: async () => ({ ok: true, value: '{}' }),
      subset: async () => ({ ok: true, value: [] }),
      answerSync: () => ({ ok: true, value: '{}' }),
      wholeSync: () => ({ ok: true, value: [] }),
      seeds: async () => ({ ok: true, value: initial }),
      onSeedsChanged: (listener: (json: string) => void) => { onChange = listener; },
    } } as any);
    try {
      const { doc } = await editor('on');
      expect(doc.api!.fonts!.provider.sync).toBe(true);
      await vi.waitFor(() => expect(applied).toHaveBeenCalledWith(initial));
      onChange(changed);
      expect(applied).toHaveBeenLastCalledWith(changed);
    } finally {
      resetBridgeForTests();
      applied.mockRestore();
    }
  });

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
    const readied: boolean[] = [];
    const stop = doc.onReady(() => readied.push(doc.ready));
    available = true;
    expect(await doc.retry()).toBe(true);
    expect(doc.ready).toBe(true);
    // The extension loader activates what waited (kernel/loader.ts).
    expect(readied).toEqual([true]);
    stop();
  });

  it('shows the named failure again for a request that cannot run, and runs a key held during a Retry after it', async () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/compositor');
    const toasts: Array<[string, any]> = [];
    PM.toast = (message: string, _ms: number, options: any) => { toasts.push([message, options]); };
    let available = false;
    const doc = installEngineDoc(PM, { mode: parseMode('on'), strict: true, sink: () => {}, loadModule: () => available ? loadModule() : Promise.reject(new Error('404 fetching the module')) });
    PM.proj = PM.mkProject({ dur: 10 });
    expect(await doc.whenReady).toBe(false);
    expect(toasts).toHaveLength(1);
    const ran: string[] = [];
    // An app command runs; a document command shows the same named notice, with Retry.
    expect(runCommandWhenReady(PM, 'settings', () => ran.push('settings'))).toBe(1);
    runCommandWhenReady(PM, 'duplicate', () => ran.push('duplicate'));
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(ran).toEqual(['settings']);
    expect(toasts).toHaveLength(2);
    expect(toasts[1]![0]).toBe(toasts[0]![0]);
    expect(toasts[1]![1]).toMatchObject({ error: true, key: 'engine-failure-pm-wasm-unavailable', action: { label: 'Retry' } });
    expect(doc.failures()).toHaveLength(1);
    // A key pressed while the Retry loads runs once it is ready.
    available = true;
    const retry = doc.retry();
    runCommandWhenReady(PM, 'duplicate', () => ran.push(`duplicate:${doc.ready}`));
    expect(ran).toEqual(['settings']);
    expect(await retry).toBe(true);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(ran).toEqual(['settings', 'duplicate:true']);
    expect(toasts).toHaveLength(2);
  });

  it('addresses fresh Comps.get root wrappers through the wrapper view, keeping interned values', async () => {
    const { PM, doc } = await editor('on');
    const { child } = content(PM);
    PM.scope = [{}];
    // PM.Comps.get(openId): a new rootComp(PM.proj) on every call.
    PM.curComp = () => rootComp(PM.proj);
    const query = vi.spyOn(doc.wasm!, 'query');
    const prop = child.p['position.y'];
    for (const t of [0, 0.5, 1, 2]) {
      expect(doc.api!.evP(child, prop, t, 'position.y')).toBe(PM.evP(child, prop, t, 'position.y'));
      doc.api!.worldMatrix(child, t);
    }
    expect(query.mock.calls.every(([r]) => !(r as any).compJson && !(r as any).compJsonText && (r as any).comp === '@root:' + PM.proj.compId)).toBe(true);
    expect((doc.stats() as any).native.fastPath).toBeGreaterThanOrEqual(4);
    expect((doc.stats() as any).native.interned).toBe(1);
    expect(fallbacks(doc)).toEqual({});
  });

  it("gives expressions under a root wrapper the wrapper's members, not the project's (param, comp.name, comp.id)", async () => {
    const { PM, doc } = await editor('on');
    const { child } = content(PM);
    PM.proj.params = { amp: { value: 7 } };
    PM.proj.compName = 'Main';
    const exprs = [
      'param("amp") + (comp.params ? 1000 : 0)',
      'comp.name === "Untitled" ? 5 : comp.name === "Main" ? 6 : 9',
      'comp.id === "' + PM.proj.compId + '" ? 3 : comp.id === "' + PM.proj.id + '" ? 4 : 8',
      '(comp.compId === undefined ? 10 : 20) + (comp.comps ? 100 : 0) + (comp.compName === undefined ? 1 : 2)',
      'comp.fps * 2 + comp.layers.length + fps',
    ];
    const props = exprs.map((expr, i) => { const p = PM.P(i); p.expr = expr; child.p[`x.e${i}`] = p; return p; });
    child.p['position.x'].expr = 'param("amp") * 10 + (comp.name === "Main" ? 1 : 0)';
    PM.touch();
    const native = () => [...props.map((p, i) => doc.api!.evP(child, p, 1, `x.e${i}`)), doc.api!.ev(child, 'position.x', 1)];
    const legacy = () => [...props.map((p, i) => PM.evP(child, p, 1, `x.e${i}`)), PM.ev(child, 'position.x', 1)];
    // Today's answers (legacy/core/anim.ts paramGet and evalProp's `comp`).
    const fpsTerm = 3 * PM.proj.fps + PM.proj.layers.length;
    const open = [1007, 5, 4, 122, fpsTerm, 70];
    const wrapped = [0, 6, 3, 11, fpsTerm, 1];
    const fresh = () => { PM.scope = [{}]; PM.curComp = () => rootComp(PM.proj); };
    const none = () => { PM.scope = []; PM.curComp = () => PM.scope.at(-1) ?? PM.proj; };
    // Scopes alternate at one revision: no interned id, memo, or answer may cross views.
    for (const [scope, want] of [[none, open], [fresh, wrapped], [none, open], [fresh, wrapped]] as const) {
      scope();
      for (let round = 0; round < 2; round++) {
        expect(legacy()).toEqual(want);
        expect(native()).toEqual(legacy());
      }
    }
    // A wrapper held in the scope and then changed without the barrier seeing
    // it (its own members, not the document): the next answer reads it.
    const held = rootComp(PM.proj);
    PM.scope = [held];
    PM.curComp = () => PM.scope.at(-1) ?? PM.proj;
    expect(native()).toEqual(legacy());
    held.params = { amp: { value: 3 } };
    expect(legacy()[0]).toBe(1003);
    expect(native()).toEqual(legacy());
    delete held.params;
    held.name = 'Untitled';
    expect(native()).toEqual(legacy());
    // A member rootComp never has, and one it has, removed.
    (held as any).extra = 1;
    expect(native()).toEqual(legacy());
    delete (held as any).extra;
    delete held.fps;
    expect(native()).toEqual(legacy());
    held.fps = PM.proj.fps;
    expect(native()).toEqual(legacy());
    expect([...ROOT_WRAPPER_FIELDS]).toEqual([...COMP_FIELDS]);
    // A stale wrapper after the project's own field changed.
    held.name = PM.proj.compName;
    PM.proj.fps = 50;
    PM.touch();
    expect(legacy()[4]).not.toBe(2 * 50 + 2 + 50);
    expect(native()).toEqual(legacy());
    // The stored form: a project without compId is its own open composition.
    none();
    delete PM.proj.compId;
    PM.touch();
    fresh();
    expect(native()).toEqual(legacy());
    expect(doc.api!.evP(child, props[0], 1, 'x.e0')).toBe(0);
    expect(fallbacks(doc)).toEqual({});
    expect(doc.failures()).toEqual([]);
  });

  it('keeps one validated detached scope for a scope object made anew on every call, exactly', async () => {
    const { PM, doc } = await editor('on');
    const { child } = content(PM);
    const base: any = clone({ id: 'outside', fps: 24, w: 640, h: 360, dur: 10, layers: PM.proj.layers });
    const L = base.layers[1];
    L.p['x.k'] = PM.P(0, { kf: [PM.KF(0, 0), PM.KF(2, 50)] });
    L.p['x.e'] = PM.P(0); L.p['x.e'].expr = 'param("amp") + comp.fps + comp.layers.length + time';
    PM.scope = [{}];
    PM.curComp = () => ({ ...base });
    const query = vi.spyOn(doc.wasm!, 'query');
    const check = () => {
      for (const t of [0, 0.5, 1.25]) {
        expect(doc.api!.evP(L, L.p['x.k'], t, 'x.k')).toBe(PM.evP(L, L.p['x.k'], t, 'x.k'));
        expect(doc.api!.evP(L, L.p['x.e'], t, 'x.e')).toBe(PM.evP(L, L.p['x.e'], t, 'x.e'));
        expect(doc.api!.ev(L, 'position.y', t)).toBe(PM.ev(L, 'position.y', t));
        expect(doc.api!.worldMatrix(L, t)).toEqual(PM.worldMatrix(L, t));
      }
    };
    check();
    const serialized = () => query.mock.calls.filter(([r]) => (r as any).compJson || (r as any).compJsonText).length;
    const first = serialized();
    for (let i = 0; i < 20; i++) check();
    // Repeated queries reuse the resident scope: no full serialization per call.
    expect(serialized()).toBe(first);
    // Changes reach the next copy. A scope member change is seen at an
    // unchanged document revision; channel edits are announced (PM.touch()
    // clears today's keyframe caches too).
    base.fps = 50;
    check();
    base.params = { amp: { value: 9 } };
    check();
    L.p['x.k'].kf[1].v = 80;
    PM.touch();
    check();
    L.p['x.e'] = PM.P(0); L.p['x.e'].expr = 'comp.w + time';
    PM.touch();
    check();
    base.layers = base.layers.slice(0, 2);
    PM.touch();
    check();
    expect(child).toBeTruthy();
    expect(fallbacks(doc)).toEqual({});
    expect(doc.failures()).toEqual([]);
  });

  it('invalidates numeric scope answers on writes and revives a dead module before reusing them', async () => {
    const { PM, doc } = await editor('on');
    const { child } = content(PM);
    const prop = child.p['position.y'];
    expect(doc.api!.evP(child, prop, 1, 'position.y')).toBe(40);
    expect(doc.api!.evP(child, prop, 1, 'position.y')).toBe(40);
    prop.v = 12;
    PM.touch();
    expect(doc.api!.evP(child, prop, 1, 'position.y')).toBe(52);
    const old = doc.wasm!;
    old.dead = true;
    expect(doc.api!.evP(child, prop, 1, 'position.y')).toBe(52);
    expect(doc.wasm).not.toBe(old);
    expect(doc.failures()).toEqual([]);
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
    const repairs = (doc.stats() as any).native.repairs;
    for (let i = 0; i < 5; i++) doc.api!.worldMatrix(child, 1 + i / 10);
    // The mirror holds the root under its id: no query of it needs a repair.
    expect((doc.stats() as any).native.repairs).toBe(repairs);
    expect(fallbacks(doc)).toEqual({});
    expect(doc.dual!.total().mismatched).toBe(0);
  });

  it("throws the legacy TypeError natively for a detached input that is not an object", async () => {
    const thrown = (fn: () => unknown) => { try { fn(); } catch (error) { return `${(error as Error).name}: ${(error as Error).message}`; } return 'no throw'; };
    for (const mode of ['on', 'audit']) {
      const { PM, doc } = await editor(mode);
      const { child, shape } = content(PM);
      const api = doc.api!;
      for (const input of [undefined, null, 5, 'text', true]) {
        expect(thrown(() => api.hasKeyAt(child, input, 1))).toBe(thrown(() => (input as any).kf.find(() => false)));
        expect(thrown(() => api.pathValues(shape, input, 1))).toBe(thrown(() => pathValues(PM, shape, input as any, 1)));
        expect(thrown(() => api.pathValues(shape, input, 1, 'g.x'))).toBe(thrown(() => pathValues(PM, shape, input as any, 1, 'g.x')));
        expect(thrown(() => api.groupMatrix(shape, input, 1))).toBe(thrown(() => groupMatrix(PM, shape, input as any, 1)));
      }
      expect(fallbacks(doc)).toEqual({});
      expect(doc.failures()).toEqual([]);
      if (mode === 'audit') expect(doc.dual!.total().mismatched).toBe(0);
    }
  });

  it("asks about another composition's own layer list by its address", async () => {
    const { PM, doc } = await editor('audit');
    const group = PM.mkLayer('group', { name: 'G' });
    const inner = PM.mkLayer('solid', { name: 'In' }); inner.group = group.id;
    const matte = PM.mkLayer('solid', { name: 'Matte' });
    const other = { id: 'other', name: 'Other', fps: 30, w: 640, h: 360, dur: 10, layers: [inner, group, matte] };
    const deep = { id: 'deep', fps: 30, w: 640, h: 360, dur: 10, layers: [PM.mkLayer('solid', { name: 'Deep' })] };
    PM.proj.comps = { other, outer: { id: 'outer', layers: [], comps: { deep } } };
    PM.touch();
    const api = doc.api!;
    expect(api.groupAncestors(inner, other.layers)).toEqual([group]);
    expect(api.groupAncestors(matte, other.layers)).toEqual([]);
    expect(api.groupAncestors(deep.layers[0], deep.layers)).toEqual([]);
    expect(api.groupAncestors(clone(inner), other.layers)).toEqual([group]);
    expect(api.validMatteSource(other.layers, inner, matte.id)).toBe(true);
    expect(api.validMatteSource(other.layers, inner, inner.id)).toBe(false);
    // A caller's own list (filtered, as the 3D depth pass makes) and odd inputs.
    const filtered = other.layers.filter((l: any) => l.type !== 'solid' || l === inner);
    expect(api.groupAncestors(inner, filtered)).toEqual([group]);
    expect(api.groupAncestors(inner, [inner])).toEqual([]);
    expect(api.groupAncestors(null, other.layers)).toEqual([]);
    expect(api.groupAncestors(5 as any, other.layers)).toEqual([]);
    expect(api.validMatteSource([inner, matte], inner, matte.id)).toBe(true);
    expect(api.validMatteSource([inner, { ...matte, type: 'null' }], inner, matte.id)).toBe(false);
    expect(() => api.validMatteSource(other.layers, null, matte.id)).toThrow("Cannot read properties of null (reading 'id')");
    expect(api.validMatteSource(other.layers, 7 as any, matte.id)).toBe(true);
    const repairs = (doc.stats() as any).native.repairs;
    for (let i = 0; i < 3; i++) api.groupAncestors(inner, other.layers);
    expect((doc.stats() as any).native.repairs).toBe(repairs);
    expect(fallbacks(doc)).toEqual({});
    expect(doc.dual!.total().mismatched).toBe(0);
  });

  it('addresses a precomposition scope in O(1) and a nested one from a verified cache', async () => {
    const { PM, doc } = await editor('on');
    // The reviewer's shape: 100 layers of 40 keyed properties in the open composition.
    PM.proj.layers = Array.from({ length: 100 }, (_, i) => {
      const L = PM.mkLayer('solid', { name: `L${i}` });
      for (let k = 0; k < 40; k++) L.p[`extra${k}`] = PM.P(k, { kf: Array.from({ length: 5 }, (_, t) => PM.KF(t, t)) });
      return L;
    });
    const pre = { id: 'pre', fps: 30, w: 640, h: 360, dur: 10, layers: [PM.mkLayer('solid', { name: 'In pre' })] };
    const deep = { id: 'deep', fps: 24, w: 640, h: 360, dur: 10, layers: [PM.mkLayer('solid', { name: 'In deep' })] };
    PM.proj.comps = { pre, outer: { id: 'outer', layers: [], comps: { deep } } };
    PM.touch();
    const api = doc.api! as any;
    const time = (comp: any, n: number) => {
      PM.scope = [comp];
      PM.curComp = () => PM.scope.at(-1) ?? PM.proj;
      api.comps();
      const t0 = performance.now();
      for (let i = 0; i < n; i++) api.comps();
      return (performance.now() - t0) / n;
    };
    const preMs = time(pre, 2000);
    const deepMs = time(deep, 2000);
    expect(api.comps().comp).toBe('@path:' + JSON.stringify(['comps', 'outer', 'comps', 'deep']));
    // A move invalidates the cached path: the next answer finds it again.
    PM.proj.comps = { pre, deep, outer: { id: 'outer', layers: [], comps: {} } };
    PM.touch();
    expect(api.comps().comp).toBe('deep');
    PM.proj.comps = { pre, other: { id: 'other', layers: [], comps: { deep } } };
    PM.touch();
    expect(api.comps().comp).toBe('@path:' + JSON.stringify(['comps', 'other', 'comps', 'deep']));
    PM.scope = [pre];
    expect(api.comps().comp).toBe('pre');
    expect(doc.api!.worldMatrix(pre.layers[0], 1)).toBeTruthy();
    PM.scope = [];
    console.log(`[strict] scope address per call: precomposition ${(preMs * 1e3).toFixed(2)} µs, nested ${(deepMs * 1e3).toFixed(2)} µs`);
    expect(preMs).toBeLessThan(0.05);
    expect(deepMs).toBeLessThan(0.05);
    expect(fallbacks(doc)).toEqual({});
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

  it.runIf(existsSync(BRIDGE))('warms nested text and fonts keyed after the layer start', async () => {
    const { FontRegistry } = createRequire(import.meta.url)(BRIDGE) as any;
    const registry = new FontRegistry();
    const asked: string[] = [];
    const { PM, doc } = await editor('on', true, {
      sync: true, answer: q => { asked.push(q); return registry.answer(q); },
      subset: (b, i) => registry.subset(b, i), whole: (b, i) => registry.whole(b, i),
    });
    const title = PM.mkLayer('text');
    title.d.font = PM.P('Helvetica Neue', { kf: [PM.KF(0, 'Helvetica Neue'), PM.KF(3, 'Georgia')] });
    const inner = { id: 'inner', fps: 30, w: 640, h: 360, dur: 10, layers: [title] };
    PM.proj.comps.outer = { id: 'outer', layers: [], comps: { inner } };
    PM.doc.touched([['comps']]);
    await doc.warmFonts();
    expect(asked.join(' ')).toContain('Helvetica Neue');
    expect(asked.join(' ')).toContain('Georgia');
    PM.scope = [inner];
    doc.api!.textLayout(title, 3);
    expect(fallbacks(doc)).toEqual({});
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

  it('evaluates an explicit group list natively and refuses detached edit scopes without legacy answers', async () => {
    const { PM, doc } = await editor('audit');
    const group = PM.mkLayer('group');
    const layer = PM.mkLayer('solid'); layer.group = group.id;
    const list = [layer, group];
    expect(doc.api!.groupAncestors(layer, list)).toEqual([group]);
    expect(fallbacks(doc)).toEqual({});
    expect(doc.dual!.total().mismatched).toBe(0);
    const legacy = vi.fn();
    // align's closure is the caller's own extension code (it evaluates through
    // api.anim/api.render, natively): it answers what the kernel does not cover.
    const caller = vi.fn(() => ['caller']);
    expect(doc.edits!.align([layer], 'left', 0, caller)).toEqual(['caller']);
    expect(doc.edits!.align([], 'left', 0, caller)).toEqual(['caller']);
    expect(caller).toHaveBeenCalledTimes(2);
    expect(doc.edits!.stats.callers).toEqual({ 'align:context': 1, 'align:empty': 1 });
    expect(doc.edits!.stats.fallbacks).toEqual({});
    expect(() => doc.edits!.leaveAttributes(layer, 0, legacy)).toThrow(/edit.leaveAttributes.*context/);
    expect(() => doc.edits!.anchorReference(layer, 'font', 0, legacy)).toThrow(/edit.anchorReference.*context/);
    expect(legacy).not.toHaveBeenCalled();
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
