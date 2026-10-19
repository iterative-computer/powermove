// @vitest-environment happy-dom
/*
 * The kernel's public API over the engine (owner amendments 23 and 24): an
 * extension at apiVersion 1 to 3 calls `api.anim.*`, the editor helpers and
 * the model lookups exactly as before, and with the native evaluator on
 * those answers come from the real pm-wasm module (engine-doc), with the
 * same values, the same live objects and synchronous returns as today's
 * functions. No mock PM: the legacy modules, the engine document and the
 * kernel are the app's own (built pm-wasm in public/engine/).
 */
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { makePM } from '../legacy/__tests__/make-pm';
import { installEngineDoc, type EngineDoc } from '../engine-doc/install';
import { parseMode } from '../engine-doc/flags';
import * as helpers from './editor-helpers';
import { installKernel, type InstalledKernel } from './install';
import { resetExtensionsStore } from './extensions.svelte';
import { RUNTIME_GLOBAL } from './runtime-globals';

const MODULE = path.resolve(__dirname, '../../public/engine/pm_wasm.wasm');
const built = existsSync(MODULE);
if (!built && process.env.PM_WASM_REQUIRED) throw new Error(`pm-wasm is not built (${MODULE})`);
let compiled: Promise<WebAssembly.Module> | null = null;
const loadModule = () => (compiled ??= WebAssembly.compile(readFileSync(MODULE)));

let installed: InstalledKernel | null = null;
afterEach(() => {
  installed?.uninstall();
  installed = null;
  resetExtensionsStore();
  delete (globalThis as Record<string, unknown>)[RUNTIME_GLOBAL];
  delete (globalThis as Record<string, unknown>).PM;
});

const near = (a: unknown, b: unknown): boolean => {
  if (typeof a === 'number' && typeof b === 'number') return Math.abs(a - b) <= 1e-9 * Math.max(1, Math.abs(b));
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((v, i) => near(v, b[i]));
  if (a && b && typeof a === 'object' && typeof b === 'object') {
    const ka = Object.keys(a).sort(), kb = Object.keys(b).sort();
    return near(ka, kb) && ka.every((k) => near((a as any)[k], (b as any)[k]));
  }
  return a === b;
};

async function setup() {
  const PM: any = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/compositor');
  const doc: EngineDoc = installEngineDoc(PM, { mode: parseMode('on'), loadModule, sink: () => {}, auditMs: 1e9 });
  PM.proj = PM.mkProject({ dur: 4, w: 640, h: 360 });
  PM.time = 0;
  const parent = PM.mkLayer('null', { name: 'Parent', from: 0, dur: 4 });
  parent.p['rotation'] = PM.P(0, { kf: [PM.KF(0, 0), PM.KF(2, 45, 'easeOut')] });
  const title = PM.mkLayer('solid', { name: 'Title', from: 0.5, dur: 3 });
  title.parent = parent.id;
  title.p['position.x'] = PM.P(100, { kf: [PM.KF(0.5, 100), PM.KF(2.5, 400, 'power')] });
  title.p.opacity.expr = 'value * 0.5 + time * 10';
  const group = PM.mkLayer('group', { name: 'Group', from: 0, dur: 4 });
  const child = PM.mkLayer('shape', { name: 'Child', from: 0, dur: 4 });
  child.group = group.id;
  PM.proj.layers = [title, parent, group, child];
  PM.touch?.();
  expect(await doc.whenReady).toBe(true);
  (globalThis as Record<string, unknown>).PM = PM;
  installed = installKernel(PM);
  return { PM, doc, api: installed.api('engine-facade-probe'), title, parent, group, child };
}

describe.skipIf(!built)('the kernel API over the engine (pm-wasm), as an extension calls it', () => {
  it('anim.* answers from pm-wasm with today\'s values, live objects and synchronous returns', async () => {
    const { PM, doc, api, title, parent, group, child } = await setup();
    const native = doc.api!;
    const spies = ['ev', 'evP', 'active', 'worldMatrix', 'localMatrix', 'transformParentMatrix', 'findProp', 'allProps', 'hasKeyAt', 'wouldCycle', 'resolveContent', 'mul'].map((name) => vi.spyOn(native as any, name));
    for (const t of [0, 0.75, 1.3, 2.5, 3.9]) {
      expect(near(api.anim.ev(title, 'position.x', t), PM.ev(title, 'position.x', t)), `ev ${t}`).toBe(true);
      expect(near(api.anim.ev(title, 'opacity', t), PM.ev(title, 'opacity', t)), `expression ${t}`).toBe(true);
      expect(near(api.anim.evP(parent, parent.p.rotation, t, 'rotation'), PM.evP(parent, parent.p.rotation, t, 'rotation'))).toBe(true);
      expect(api.anim.active(title, t)).toBe(!!PM.active(title, t));
      expect(near(api.anim.worldMatrix(title, t), PM.worldMatrix(title, t)), `world ${t}`).toBe(true);
      expect(near(api.anim.localMatrix(title, t), PM.localMatrix(title, t))).toBe(true);
      expect(near(api.anim.transformParentMatrix(title, t), PM.transformParentMatrix(title, t))).toBe(true);
    }
    // Live objects: the channel and keyframe are the project's own.
    expect(api.anim.findProp(title, 'position.x')).toBe(title.p['position.x']);
    for (const t of [0, 0.5, 1, 2.5, 3]) expect(api.anim.hasKeyAt(title, title.p['position.x'], t), `hasKeyAt ${t}`).toBe(PM.hasKeyAt(title, title.p['position.x'], t) ?? null);
    const keyAt = [0, 0.5, 1, 2.5, 3].map((t) => api.anim.hasKeyAt(title, title.p['position.x'], t)).find(Boolean);
    expect(title.p['position.x'].kf).toContain(keyAt);
    const props = api.anim.allProps(title);
    expect(props.map((p) => p.key)).toEqual(PM.allProps(title).map((p: any) => p.key));
    expect(props.every((p, i) => p.prop === PM.allProps(title)[i].prop)).toBe(true);
    expect(api.anim.wouldCycle(group, child.id)).toBe(!!PM.wouldCycle(group, child.id));
    expect(api.anim.wouldCycle(parent, title.id)).toBe(true);
    expect(near(api.anim.mul([2, 0, 0, 2, 1, 1], [1, 0, 0, 1, 5, 5]), PM.mul([2, 0, 0, 2, 1, 1], [1, 0, 0, 1, 5, 5]))).toBe(true);
    expect(near(api.anim.resolveContent(child, 1), PM.resolveContent(child, 1))).toBe(true);
    // Synchronous returns, as apiVersion 1 to 3 code expects.
    expect(api.anim.ev(title, 'opacity', 1)).not.toBeInstanceOf(Promise);
    // And the engine answered them.
    for (const spy of spies) expect(spy, spy.getMockName()).toHaveBeenCalled();
  });

  it('the editor helpers from the powermove module answer from pm-wasm as the legacy implementations do', async () => {
    const { PM, doc, title } = await setup();
    const legacy = { ...PM, EngineDoc: undefined };
    const resolve = vi.spyOn(doc.api as any, 'resolveContent');
    const evaluate = vi.spyOn(doc.api as any, 'evaluatedValue');
    for (const t of [0.6, 1.4, 2.9]) {
      expect(near(helpers.resolveContent(PM, title, t), helpers.resolveContent(legacy, title, t))).toBe(true);
      expect(near(helpers.evaluatedValue(PM, title, title.p['position.x'], t, 'position.x'), helpers.evaluatedValue(legacy, title, title.p['position.x'], t, 'position.x'))).toBe(true);
      expect(near(helpers.planeMatrix(PM, title, t), helpers.planeMatrix(legacy, title, t))).toBe(true);
    }
    expect(helpers.is3DLayer(PM, title)).toBe(helpers.is3DLayer(legacy, title));
    expect(resolve).toHaveBeenCalled();
    expect(evaluate).toHaveBeenCalled();
  });

  it('model lookups and project reads keep today\'s live identity', async () => {
    const { PM, api, title } = await setup();
    expect(api.project.get()).toBe(PM.proj);
    expect(api.model.layer(title.id)).toBe(title);
    expect(api.model.byName('Title')).toBe(title);
    expect(api.model.curComp()).toBe(PM.proj);
    // An edit through the API lands on the same live objects the engine mirrors.
    const result: any = api.project.apply({ type: 'set_property', target: title.id, path: 'position.y', value: 222 } as any);
    expect(result.ok).toBe(true);
    expect(api.anim.ev(title, 'position.y', 1)).toBe(222);
    expect(PM.ev(title, 'position.y', 1)).toBe(222);
  });
});
