/*
 * Undo, redo, history jumps, and a canceled gesture restore a layer by
 * replacing its object in place (the scoped history entry of a per-layer
 * command holds the whole layer, and `applyPatch` assigns a clone). The
 * composition keeps its shape, so structural memos stay valid by
 * `shapeRevision`, but every native answer that hands out or keys on a layer
 * object must follow the replacement: a gesture reads its press-time
 * snapshot from these answers and writes values derived from them
 * (docs/engine/write-path.md, "Answers are live objects").
 */
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { makePM } from '../legacy/__tests__/make-pm';
import { installEngineDoc, type EngineDoc } from './install';
import { parseMode } from './flags';

const MODULE = fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url));
const built = existsSync(MODULE);
if (!built && process.env.PM_WASM_REQUIRED) throw new Error(`pm-wasm is not built (${MODULE}); run node engine/crates/pm-wasm/scripts/build.mjs`);
let compiled: Promise<WebAssembly.Module> | null = null;
const loadModule = () => (compiled ??= WebAssembly.compile(readFileSync(MODULE)));

/** The production mode ("Use New Engine" on): fast paths and memos, no dual answers. */
async function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/compositor');
  const doc: EngineDoc = installEngineDoc(PM, { mode: parseMode('on'), loadModule, sink: () => {}, auditMs: 1e9 });
  PM.proj = PM.mkProject({ dur: 10 });
  PM.time = 1;
  expect(await doc.whenReady).toBe(true);
  return { PM, doc, api: doc.api! };
}

const T = 1;
const CHANNELS = ['position.x', 'position.y', 'scale.x', 'scale.y', 'rotation', 'skew', 'anchor.x', 'anchor.y'];

/** A skewed parent, a child, an animated free layer, and a group of two. */
function scene(PM: any) {
  const parent = PM.mkLayer('null', { name: 'Parent', dur: 10, p: { 'position.x': 320, 'position.y': 180, 'scale.x': 160, 'scale.y': 60, rotation: 10, skew: 25 } });
  const child = PM.mkLayer('solid', { name: 'Child', dur: 10, d: { color: '#FF6B1A', w: 120, h: 80 }, p: { 'position.x': -90 } });
  child.parent = parent.id;
  const free = PM.mkLayer('solid', { name: 'Free', dur: 10, d: { color: '#FFFFFF', w: 200, h: 60 }, p: { 'position.x': 450, 'position.y': 180 } });
  const a = PM.mkLayer('solid', { name: 'A', dur: 10, d: { color: '#4C8DFF', w: 40, h: 40 }, p: { 'position.x': 100, 'position.y': 300 } });
  const b = PM.mkLayer('solid', { name: 'B', dur: 10, d: { color: '#4C8DFF', w: 40, h: 40 }, p: { 'position.x': 160, 'position.y': 300 } });
  PM.proj.layers = [b, a, free, child, parent];
  PM.ProjectIndex.invalidate();
  PM.touch();
  PM.setKey(child, 'scale.x', 0, 100, 'linear');
  PM.setKey(child, 'scale.y', 0, 100, 'linear');
  PM.setKey(free, 'scale.x', 0, 100, 'linear');
  PM.setKey(free, 'scale.y', 0, 100, 'linear');
  PM.groupLayers([a.id, b.id], 'Group');
  PM.touch();
  const byName = (name: string) => PM.proj.layers.find((l: any) => l.name === name);
  return { ids: Object.fromEntries(['Parent', 'Child', 'Free', 'A', 'B', 'Group'].map(n => [n, byName(n).id])) as Record<string, string> };
}

/** One gesture as the viewer runs it: a scoped edit with keyed writes, asking between moves. */
function gesture(PM: any, api: any, ids: string[], label: string, write: (layer: any, step: number) => Record<string, number>) {
  PM.Edit.begin(label, { origin: 'canvas' });
  for (let step = 1; step <= 3; step++) {
    api.transformRoots(ids);
    for (const id of ids) {
      const layer = PM.L(id);
      api.worldMatrix(layer, T);
      api.transformParentMatrix(layer, T);
      for (const key of CHANNELS) api.ev(layer, key, T);
      for (const [path, value] of Object.entries(write(layer, step))) {
        expect(PM.Edit.dispatch({ type: 'set_property', target: id, path, value, time: T, mode: 'auto', preserveHandEdits: false, markIntent: 'human' }).ok).toBe(true);
      }
    }
  }
  return PM.Edit.commit(label);
}

/** What a gesture reads at press time, through the native answers. */
function pressTime(PM: any, api: any, ids: string[]) {
  const roots = api.transformRoots(ids);
  return roots.map((L: any) => ({
    id: L.id,
    live: L === PM.L(L.id),
    world: api.worldMatrix(L, T),
    parent: api.transformParentMatrix(L, T),
    values: CHANNELS.map(key => api.ev(L, key, T)),
    ancestors: api.groupAncestors(L).map((g: any) => ({ live: g === PM.L(g.id), opacity: api.ev(g, 'opacity', T) })),
  }));
}

/** The same reads through the legacy evaluator on the live layers (today's answers). */
function legacyPressTime(PM: any, ids: string[]) {
  return PM.transformRoots(ids).map((L: any) => ({
    id: L.id,
    live: true,
    world: PM.worldMatrix(L, T),
    parent: PM.transformParentMatrix(L, T),
    values: CHANNELS.map(key => PM.ev(L, key, T)),
    ancestors: PM.groupAncestors(L).map((g: any) => ({ live: true, opacity: PM.ev(g, 'opacity', T) })),
  }));
}

const close = (a: any, b: any) => expect(JSON.parse(JSON.stringify(a, (_k, v) => typeof v === 'number' ? Math.round(v * 1e9) / 1e9 : v)))
  .toEqual(JSON.parse(JSON.stringify(b, (_k, v) => typeof v === 'number' ? Math.round(v * 1e9) / 1e9 : v)));

describe.skipIf(!built)('native answers after history replaces layer objects', () => {
  it('a press after undo reads the restored layers, not the undone gesture', async () => {
    const { PM, doc, api } = await editor();
    const { ids } = scene(PM);
    const selection = [ids.Child!, ids.Free!];
    const before = legacyPressTime(PM, selection);
    close(pressTime(PM, api, selection), before);
    // Resize the multi-selection (keys at T on both), then undo it.
    expect(gesture(PM, api, selection, 'Resize selection', (L, step) => ({ 'scale.x': 100 + step * 3.827, 'scale.y': 100 + step * 3.827, 'position.x': PM.ev(L, 'position.x', T) + step }))).toMatchObject({ ok: true });
    const resized = legacyPressTime(PM, selection);
    expect(resized[0].values[2]).not.toBe(before[0].values[2]);
    close(pressTime(PM, api, selection), resized);
    expect(PM.hist.undo()).toBe(true);
    close(legacyPressTime(PM, selection), before);
    close(pressTime(PM, api, selection), before);
    expect(doc.check('test')!.ok).toBe(true);
  });

  it('a rotate after undo writes values derived from the restored layers', async () => {
    const { PM, doc, api } = await editor();
    const { ids } = scene(PM);
    const selection = [ids.Child!, ids.Free!];
    const scaleBefore = selection.map(id => [PM.ev(PM.L(id), 'scale.x', T), PM.ev(PM.L(id), 'scale.y', T)]);
    gesture(PM, api, selection, 'Resize selection', (_L, step) => ({ 'scale.x': 100 + step * 3.827, 'scale.y': 111.481 }));
    expect(PM.hist.undo()).toBe(true);
    // The rotate gesture's press: snapshots from the roots, writes by id.
    const snapshots = api.transformRoots(selection).map((L: any) => ({ L, sx: api.ev(L, 'scale.x', T), sy: api.ev(L, 'scale.y', T), world: api.worldMatrix(L, T) }));
    for (const s of snapshots) {
      const [a = NaN, b = NaN] = s.world;
      const [pa = NaN, pb = NaN] = api.transformParentMatrix(s.L, T);
      const sx = Math.hypot(a, b), parentSx = Math.hypot(pa, pb);
      // The world scale the press saw is the live layer's scale through its parent.
      expect(sx / parentSx * 100).toBeCloseTo(s.sx, 6);
    }
    PM.Edit.begin('Rotate selection', { origin: 'canvas' });
    for (const s of snapshots) PM.Edit.dispatch({ type: 'set_property', target: s.L.id, path: 'scale.x', value: s.sx, time: T, mode: 'auto', preserveHandEdits: false, markIntent: 'human' });
    PM.Edit.commit('Rotate selection');
    expect(selection.map(id => [PM.ev(PM.L(id), 'scale.x', T), PM.ev(PM.L(id), 'scale.y', T)])).toEqual(scaleBefore);
    expect(doc.check('test')!.ok).toBe(true);
  });

  it('follows redo, repeated undo and redo, and a history jump', async () => {
    const { PM, doc, api } = await editor();
    const { ids } = scene(PM);
    const selection = [ids.Child!, ids.Free!];
    const states = [legacyPressTime(PM, selection)];
    gesture(PM, api, selection, 'Scale', (_L, step) => ({ 'scale.x': 100 + step * 10 }));
    states.push(legacyPressTime(PM, selection));
    gesture(PM, api, selection, 'Rotate', (_L, step) => ({ rotation: step * 15 }));
    states.push(legacyPressTime(PM, selection));
    gesture(PM, api, selection, 'Move', (L, step) => ({ 'position.x': PM.ev(L, 'position.x', T) + step * 7, 'position.y': 12 * step }));
    states.push(legacyPressTime(PM, selection));
    close(pressTime(PM, api, selection), states[3]);
    // Step back one at a time, then forward, asking at each step.
    for (const at of [2, 1, 0]) { expect(PM.hist.undo()).toBe(true); close(pressTime(PM, api, selection), states[at]); }
    for (const at of [1, 2, 3]) { expect(PM.hist.redo()).toBe(true); close(pressTime(PM, api, selection), states[at]); }
    // A history jump: several entries in one synchronous run before the next press.
    PM.hist.undo(); PM.hist.undo(); PM.hist.undo();
    close(pressTime(PM, api, selection), states[0]);
    PM.hist.redo(); PM.hist.redo();
    close(pressTime(PM, api, selection), states[2]);
    expect(doc.check('test')!.ok).toBe(true);
  });

  it('follows a canceled gesture (the scoped rollback replaces the layers too)', async () => {
    const { PM, api } = await editor();
    const { ids } = scene(PM);
    const selection = [ids.Child!, ids.Free!];
    const before = legacyPressTime(PM, selection);
    pressTime(PM, api, selection);
    PM.Edit.begin('Rotate selection', { origin: 'canvas' });
    for (const id of selection) PM.Edit.dispatch({ type: 'set_property', target: id, path: 'rotation', value: 45, time: T, mode: 'auto' });
    pressTime(PM, api, selection);
    expect(PM.Edit.cancel()).toBe(true);
    close(pressTime(PM, api, selection), before);
  });

  it('hands out the live group after undo restores a group layer', async () => {
    const { PM, api } = await editor();
    const { ids } = scene(PM);
    const members = [ids.A!, ids.B!];
    const groupOpacity = () => PM.ev(PM.L(ids.Group), 'opacity', T);
    const opacity0 = groupOpacity();
    expect(api.groupAncestors(PM.L(ids.A)).map((g: any) => g === PM.L(ids.Group))).toEqual([true]);
    gesture(PM, api, [ids.Group!], 'Fade group', (_L, step) => ({ opacity: 100 - step * 20 }));
    expect(groupOpacity()).toBe(40);
    expect(PM.hist.undo()).toBe(true);
    for (const id of members) {
      const [group] = api.groupAncestors(PM.L(id));
      expect(group, 'the group object the document holds now').toBe(PM.L(ids.Group));
      expect(api.ev(group, 'opacity', T)).toBe(opacity0);
      expect(api.worldMatrix(group, T)).toEqual(PM.worldMatrix(PM.L(ids.Group), T));
    }
    expect(api.transformRoots(members).every((L: any) => L === PM.L(L.id))).toBe(true);
  });

  it('answers a layer object history replaced from that object, as the legacy evaluator does', async () => {
    const { PM, api } = await editor();
    const { ids } = scene(PM);
    const held = { child: PM.L(ids.Child), free: PM.L(ids.Free) };
    // Interned before the edit, so the fast path has ids for the held objects.
    for (const L of Object.values(held)) for (const key of CHANNELS) api.ev(L, key, T);
    gesture(PM, api, [ids.Child!, ids.Free!], 'Resize selection', (_L, step) => ({ 'scale.x': 100 + step * 3.827 }));
    expect(PM.hist.undo()).toBe(true);
    // `held` objects carry the gesture's values and were replaced by the
    // undo: no longer the document's. Every native answer reads them as the
    // legacy code does, so one press never mixes two states of a layer.
    const check = () => {
      for (const L of Object.values(held)) {
        expect(L).not.toBe(PM.L(L.id));
        expect(CHANNELS.map(key => api.ev(L, key, T))).toEqual(CHANNELS.map(key => PM.ev(L, key, T)));
        expect(CHANNELS.map(key => api.evP(L, L.p[key], T, key))).toEqual(CHANNELS.map(key => PM.evP(L, L.p[key], T, key)));
        close(api.worldMatrix(L, T), PM.worldMatrix(L, T));
      }
      // And the live layers answer from the document.
      for (const id of [ids.Child!, ids.Free!]) {
        const L = PM.L(id);
        expect(CHANNELS.map(key => api.ev(L, key, T))).toEqual(CHANNELS.map(key => PM.ev(L, key, T)));
        expect(CHANNELS.map(key => api.evP(L, L.p[key], T, key))).toEqual(CHANNELS.map(key => PM.evP(L, L.p[key], T, key)));
        close(api.worldMatrix(L, T), PM.worldMatrix(L, T));
      }
    };
    expect(PM.ev(held.free, 'scale.x', T)).not.toBe(PM.ev(PM.L(ids.Free), 'scale.x', T));
    check();
    // Asked again at the same revision (the fast path's stamp is current now).
    check();
    // A later edit elsewhere moves the revision without touching these layers.
    expect(PM.Edit.apply({ type: 'set_property', target: ids.Parent, path: 'opacity', value: 50 }).ok).toBe(true);
    check();
  });
});
