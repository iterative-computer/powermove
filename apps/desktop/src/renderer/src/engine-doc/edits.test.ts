/*
 * pm-edit's kernels in the edit pipeline (engine-doc/edits.ts) with the real
 * pm-wasm module: the same edits run with the flag off (today's TypeScript),
 * `on` (the kernels' operations applied), and `audit` (the TypeScript edit
 * compared with the kernel's operations), and must leave the same document.
 */
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { canonicalProject } from '../../../shared/canonical-json';
import { alignDelta, deltaInParent, unionBox, type AlignMode } from '../../../extensions/inspector/align';
import { makePM } from '../legacy/__tests__/make-pm';
import { is3DLayer } from '../legacy/core/space-3d';
import { initializeLayer3D, parseLayer3DData } from '../core/scene3d/layers';
import { materialPreset } from '../core/scene3d/materials';
import { adoptKeys, compareSplit, editDiffers, knownIds, timeGrid } from './edits';
import { installEngineDoc, type EngineDoc } from './install';
import { parseMode } from './flags';

const MODULE = fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url));
const built = existsSync(MODULE);
if (!built && process.env.PM_WASM_REQUIRED) throw new Error(`pm-wasm is not built (${MODULE}); run node engine/crates/pm-wasm/scripts/build.mjs`);
let compiled: Promise<WebAssembly.Module> | null = null;
const loadModule = () => (compiled ??= WebAssembly.compile(readFileSync(MODULE)));

function rng(seed: number) {
  return () => {
    seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

async function editor(mode: string, project: string) {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/compositor');
  const doc: EngineDoc = installEngineDoc(PM, { mode: parseMode(mode), loadModule, sink: () => {}, auditMs: 1e9 });
  PM.proj = JSON.parse(project);
  PM.time = 0;
  PM.ProjectIndex?.invalidate?.();
  PM.touch();
  if (mode !== 'off') expect(await doc.whenReady).toBe(true);
  doc.auditor?.stop();
  return { PM, doc };
}

/** Ids of a source project's own (`k-4` becomes `ksrc4`), so the editors'
 *  fresh ids (the same `prefix-n` counter) never collide with them. */
function own(v: any): any {
  if (Array.isArray(v)) return v.map(own);
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [own(k), own(x)]));
  return typeof v === 'string' ? v.replace(/^([A-Za-z]+)-(\d+)$/, '$1src$2') : v;
}

/** A random 2D project: animated parent chains, rotated and faded groups,
 *  sized layers, on a 29.97 or 24 fps composition. */
function project(seed: number): string {
  const random = rng(seed);
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  const fps = random() < 0.5 ? 29.97 : 24;
  PM.proj = PM.mkProject({ dur: 3, fps });
  const layers: any[] = [];
  const types = ['solid', 'shape', 'null', 'solid', 'shape'];
  for (let i = 0; i < 9; i++) {
    const layer = PM.mkLayer(types[i % types.length]!, { name: `L${i}`, from: random() < 0.6 ? 0 : Math.round(random() * 20) / fps, dur: 3 });
    for (const key of ['position.x', 'position.y', 'rotation', 'scale.x', 'scale.y', 'anchor.x', 'opacity']) {
      const prop = layer.p[key];
      if (!prop) continue;
      const base = key.startsWith('scale') ? 60 + random() * 120 : key === 'opacity' ? 30 + random() * 70 : key === 'rotation' ? random() * 360 - 180 : random() * 600 - 100;
      prop.v = base;
      if (random() < 0.25) {
        prop.kf = [PM.KF(0, base, 'linear'), PM.KF(Math.round((0.5 + random() * 2) * fps) / fps, base + (random() - 0.5) * 100, 'easeOut')];
      }
    }
    if (layer.d && 'w' in layer.d) { layer.d.w = 40 + Math.round(random() * 300); layer.d.h = 40 + Math.round(random() * 200); }
    layers.push(layer);
  }
  layers.forEach((layer, i) => { if (i > 0 && random() < 0.35) layer.parent = layers[Math.floor(random() * i)]!.id; });
  PM.proj.layers = layers;
  PM.ProjectIndex.invalidate();
  // A rotated, faded group around two layers.
  const group = PM.groupLayers([layers[1]!.id, layers[2]!.id], 'Rig');
  group.p.rotation.v = 20 + random() * 40;
  group.p.opacity.v = 60;
  group.p['scale.x'].v = 120;
  return JSON.stringify(own(PM.proj));
}

/** The alignment strip's commands (AlignmentStrip.svelte `alignCommands`). */
function legacyAlign(PM: any, layers: any[], mode: AlignMode, time: number): any[] {
  const worldBox = (layer: any) => {
    if (is3DLayer(PM, layer)) return null;
    const b = PM.GL.bounds(layer, time);
    if (!b) return null;
    const m = PM.worldMatrix(layer, time);
    const corners = [[b.x0, b.y0], [b.x1, b.y0], [b.x1, b.y1], [b.x0, b.y1]].map(([x, y]) => ({ x: m[0] * x + m[2] * y + m[4], y: m[1] * x + m[3] * y + m[5] }));
    return { x0: Math.min(...corners.map(p => p.x)), y0: Math.min(...corners.map(p => p.y)), x1: Math.max(...corners.map(p => p.x)), y1: Math.max(...corners.map(p => p.y)) };
  };
  const boxed = layers.filter(l => l && !l.lock && l.type !== 'audio').map(layer => ({ layer, box: worldBox(layer) })).filter((x): x is { layer: any; box: any } => !!x.box);
  const bounds = unionBox(boxed.map(x => x.box));
  if (!bounds) return [];
  const world = alignDelta(bounds, { x0: 0, y0: 0, x1: PM.proj.w, y1: PM.proj.h }, mode);
  if (!world.dx && !world.dy) return [];
  const commands: any[] = [];
  for (const { layer } of boxed) {
    const parent = layer.parent ? PM.L(layer.parent) : null;
    const local = deltaInParent(parent ? PM.worldMatrix(parent, time) : null, world.dx, world.dy);
    if (local.dx) commands.push({ type: 'set_property', target: layer.id, path: 'position.x', value: PM.round(Number(PM.ev(layer, 'position.x', time)) + local.dx, 3), time, mode: 'auto', preserveHandEdits: false });
    if (local.dy) commands.push({ type: 'set_property', target: layer.id, path: 'position.y', value: PM.round(Number(PM.ev(layer, 'position.y', time)) + local.dy, 3), time, mode: 'auto', preserveHandEdits: false });
  }
  return commands;
}

const byName = (PM: any, name: string) => PM.proj.layers.find((l: any) => l.name === name);
const ok = (PM: any, command: any) => {
  const result = PM.Edit.apply(command, { origin: 'user' });
  return result.ok;
};

/**
 * The edits, by layer name so they pick the same layers in every editor:
 * reparenting, unparenting, grouping (pivot placement), moving into and out
 * of groups, ungrouping (animated: baked), the size anchor through a resize
 * and a Pan Behind, alignment, and Pre-compose › Leave all attributes.
 * Returns what each step answered.
 */
function run(PM: any, doc: EngineDoc, seed: number): unknown[] {
  const random = rng(seed * 7919);
  const answers: unknown[] = [];
  const time = (n: number) => { PM.time = Math.round(n * PM.proj.fps) / PM.proj.fps; };
  const off = random() < 0.5 ? 0.0123 : 0;
  time(0.4); PM.time += off;
  answers.push(ok(PM, { type: 'set_layer', target: byName(PM, 'L5').id, patch: { parent: byName(PM, 'L3').id } }));
  time(1.1);
  answers.push(ok(PM, { type: 'set_layer', target: byName(PM, 'L5').id, patch: { parent: null } }));
  answers.push(ok(PM, { type: 'set_layer', target: byName(PM, 'L7').id, patch: { parent: byName(PM, 'L1').id } }));
  time(0.7);
  answers.push(ok(PM, { type: 'group_layers', targets: [byName(PM, 'L4').id, byName(PM, 'L6').id], name: 'Pair' }));
  answers.push(ok(PM, { type: 'move_to_group', targets: [byName(PM, 'L8').id], group: byName(PM, 'Pair').id }));
  answers.push(ok(PM, { type: 'move_to_group', targets: [byName(PM, 'L2').id], group: null }));
  time(1.5); PM.time += off;
  answers.push(ok(PM, { type: 'ungroup_layers', targets: [byName(PM, 'Rig').id] }));
  answers.push(ok(PM, { type: 'ungroup_layers', targets: [byName(PM, 'Pair').id] }));
  // The size anchor: a resize captures it, a Pan Behind rescales it.
  const sized = byName(PM, 'L0');
  answers.push(ok(PM, { type: 'set_content', target: sized.id, patch: { w: 260 }, time: PM.time }));
  answers.push(ok(PM, { type: 'set_property', target: sized.id, path: 'anchor.x', value: 37, time: PM.time, mode: 'static' }));
  answers.push(ok(PM, { type: 'set_content', target: sized.id, patch: { h: 90 }, time: PM.time }));
  // Alignment (the strip's commands, then applied as the strip does).
  for (const mode of ['left', 'vcenter', 'right'] as AlignMode[]) {
    const layers = ['L0', 'L3', 'L5', 'L7'].map(n => byName(PM, n));
    const commands = doc.edits ? doc.edits.align(layers, mode, PM.time, () => legacyAlign(PM, layers, mode, PM.time)) : legacyAlign(PM, layers, mode, PM.time);
    answers.push(commands.map((c: any) => [c.target, c.path, c.value]));
    if (commands.length) PM.Edit.apply(commands, { origin: 'inspector' });
  }
  // Pre-compose › Leave all attributes.
  const leave = byName(PM, 'L3');
  const comp = PM.Comps.precompose([leave.id], { mode: 'leave', name: 'Left' });
  answers.push(comp ? { w: comp.w, h: comp.h } : null);
  return answers;
}

/** The canonical project with created compositions named by their names
 *  (their ids are fresh on every run; the ids inside are compared by
 *  `editDiffers`). */
function canonical(PM: any, known: Set<string>): any {
  const project = adoptKeys(canonicalProject(JSON.parse(JSON.stringify(PM.proj))));
  // The edit log names created layers by id under members of its own
  // (`group`, `targets`): compared by its labels.
  if (Array.isArray(project.edits)) project.edits = project.edits.map((e: any) => e.label ?? e.type ?? null);
  const rename = new Map<string, string>();
  for (const [id, comp] of Object.entries<any>(project.comps ?? {})) if (!known.has(id)) rename.set(id, `#comp:${comp.name}`);
  const walk = (v: any): any => {
    if (Array.isArray(v)) return v.map(walk);
    if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [rename.get(k) ?? k, walk(x)]));
    return typeof v === 'string' && rename.has(v) ? rename.get(v) : v;
  };
  return walk(project);
}

describe.skipIf(!built)('pm-edit in the edit pipeline', () => {
  it('leaves the document today\'s TypeScript edits leave, with the flag on and in audit', async () => {
    const used: Record<string, number> = {};
    const compared: Record<string, number> = {};
    for (const seed of [1, 2, 3, 4, 5, 6]) {
      const json = project(seed);
      const known = knownIds(JSON.parse(json));
      for (const id of [JSON.parse(json).compId, ...Object.keys(JSON.parse(json).comps ?? {})]) known.add(id);
      const legacy = await editor('off', json);
      const want = run(legacy.PM, legacy.doc, seed);

      const native = await editor('on', json);
      const got = run(native.PM, native.doc, seed);
      expect(editDiffers(got, want, known), `seed ${seed} answers`).toBeNull();
      expect(editDiffers(canonical(native.PM, known), canonical(legacy.PM, known), known), `seed ${seed} document`).toBeNull();
      const stats = native.doc.edits!.stats;
      for (const [name, n] of Object.entries(stats.applied)) used[name] = (used[name] ?? 0) + n;
      expect(Object.keys(stats.fallbacks), `seed ${seed} fallbacks`).toEqual([]);
      expect(native.doc.check('test')?.ok ?? true).toBe(true);
      expect(native.doc.auditor).toBeNull();

      // Undo walks every kernel edit back to the project as it was.
      while (native.PM.hist.undo()) { /* to the start */ }
      expect(editDiffers(canonical(native.PM, known), adoptKeys(canonicalProject(JSON.parse(json))), known), `seed ${seed} undo`).toBeNull();

      const audit = await editor('audit', json);
      run(audit.PM, audit.doc, seed);
      const dual = audit.doc.dual!.stats;
      expect(dual.samples.filter(s => String(s.op).startsWith('edit.')), `seed ${seed} dual`).toEqual([]);
      const editOps = Object.keys(dual.compared).filter(op => op.startsWith('edit.'));
      for (const op of editOps) compared[op] = (compared[op] ?? 0) + dual.compared[op]!;
      expect(editOps.reduce((n, op) => n + (dual.mismatched[op] ?? 0), 0), `seed ${seed} dual mismatches`).toBe(0);
      expect(audit.doc.auditor!.audit()).toBe(true);
    }
    // Every kernel the run reaches was answered natively.
    for (const name of ['reparent', 'groupPlacement', 'moveToGroup', 'ungroup', 'captureSizeAnchor', 'anchorReference', 'anchorMoved', 'align', 'leaveAttributes']) {
      expect(used[name], name).toBeGreaterThan(0);
      expect(compared[`edit.${name}`], `edit.${name} compared`).toBeGreaterThan(0);
    }
  });

  it('runs the legacy edit for a camera reparented into a space that moves with it', async () => {
    // A keyframed camera, and a group parented to it holding a solid: putting
    // the camera under the solid (which `PM.wouldCycle` allows) makes its
    // parent space read its own transform, which pm-edit leaves to the
    // legacy edit (`{legacy}`).
    const source = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    source.proj = source.mkProject({ dur: 3, fps: 29.97 });
    const camera = source.mkLayer('null', { name: 'Camera', from: 0, dur: 3 });
    Object.assign(camera, { type: 'extension', threeD: true, d: { definition: 'powermove.3d.camera', data: { camera: { p: { targetX: source.P(10), targetY: source.P(20), targetZ: source.P(-30) } } } } });
    camera.p['position.x'].kf = [source.KF(0, 0, 'linear'), source.KF(2, 400, 'linear')];
    const solid = source.mkLayer('solid', { name: 'Solid', from: 0, dur: 3 });
    solid.p['position.x'].v = 40;
    source.proj.layers = [camera, solid];
    source.ProjectIndex.invalidate();
    const group = source.groupLayers([solid.id], 'Rig');
    group.p.rotation.v = 30;
    group.parent = camera.id;
    const json = JSON.stringify(own(source.proj));
    const known = knownIds(JSON.parse(json));
    const reparent = (PM: any) => {
      PM.time = 0.5;
      return ok(PM, { type: 'set_layer', target: byName(PM, 'Camera').id, patch: { parent: byName(PM, 'Solid').id } });
    };
    const legacy = await editor('off', json);
    reparent(legacy.PM);
    const native = await editor('on', json);
    reparent(native.PM);
    const stats = native.doc.edits!.stats;
    expect(stats.calls.reparent).toBe(1);
    expect(stats.fallbacks).toEqual({ 'reparent:legacy': 1 });
    expect(stats.applied.reparent ?? 0).toBe(0);
    expect(byName(native.PM, 'Camera').parent).toBe(byName(native.PM, 'Solid').id);
    expect(editDiffers(canonical(native.PM, known), canonical(legacy.PM, known), known)).toBeNull();
    // The legacy edit did rewrite the camera's aim.
    expect(byName(native.PM, 'Camera').d.data.camera.p.targetX.v).not.toBe(10);
  });
});

/** Two compositions whose 3D objects share shader materials (base and slot). */
function materialProject(seed: number): string {
  const random = rng(seed);
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 4, fps: random() < 0.5 ? 29.97 : 30 });
  const fps = PM.proj.fps;
  const channel = () => {
    const prop = PM.P(Math.round(random() * 100) / 100);
    if (random() < 0.7) prop.kf = [PM.KF(Math.round(random() * 20) / fps, random(), 'linear'), PM.KF((30 + Math.round(random() * 20)) / fps, random(), 'easeOut')];
    return prop;
  };
  const shader = (id: string) => {
    const preset: any = materialPreset('surface', id, `Shader ${id}`);
    preset.p.roughness = channel();
    preset.p.metalness = channel();
    return preset;
  };
  const material = (id: string) => ({ ...parseLayer3DData('object', { object: { source: { primitive: 'box' } } }).object.material, shader: shader(id) });
  const object = (name: string, from: number, slots: boolean) => {
    const layer = PM.mkLayer('null', { name, from, dur: 4 });
    layer.type = 'extension';
    const data = parseLayer3DData('object', { object: { source: { primitive: 'box' } } });
    if (slots) data.object.slots = [{ id: 's0', name: 'Slot 0', material: material('A') }, { id: 's1', name: 'Slot 1', material: material('B') }];
    else data.object.material = material('A');
    layer.d = { definition: 'powermove.3d.object', data: parseLayer3DData('object', data) };
    initializeLayer3D(layer);
    return layer;
  };
  PM.proj.layers = [object('O0', 0, true), object('O1', Math.round(random() * 20) / fps, false), object('O2', 0.5, true)];
  PM.proj.comps = { Kother: { id: 'Kother', name: 'Other', w: 640, h: 360, fps: 24, dur: 4, bg: '#000000', layers: [object('O3', 0.25, false)] } };
  return JSON.stringify(own(PM.proj));
}

describe.skipIf(!built)('material peers through pm-edit', () => {
  it('copies shared shader channels to every surface as the TypeScript edit does', async () => {
    let applied = 0, compared = 0;
    for (const seed of [11, 12, 13, 14]) {
      const json = materialProject(seed);
      const known = knownIds(JSON.parse(json));
      for (const id of [JSON.parse(json).compId, 'Kother']) known.add(id);
      const edit = (PM: any) => {
        PM.time = 1;
        const o0 = byName(PM, 'O0');
        const results = [
          PM.Edit.apply({ type: 'set_property', target: o0.id, path: 'slots.s0.shader.roughness', value: 0.25, time: 1, mode: 'auto' }, { origin: 'user' }),
          PM.Edit.apply({ type: 'set_property', target: byName(PM, 'O1').id, path: 'shader.metalness', value: 0.75, time: 2, mode: 'static' }, { origin: 'user' }),
          PM.Edit.apply({ type: 'offset_property', target: o0.id, path: 'slots.s1.shader.roughness', delta: 0.1 }, { origin: 'user' }),
        ];
        return results.map((r: any) => r.ok);
      };
      const legacy = await editor('off', json);
      const want = edit(legacy.PM);
      const native = await editor('on', json);
      const got = edit(native.PM);
      expect(got).toEqual(want);
      const after = canonical(legacy.PM, known);
      expect(editDiffers(canonical(native.PM, known), after, known, timeGrid(after.comps)), `seed ${seed}`).toBeNull();
      applied += native.doc.edits!.stats.applied.syncPeers ?? 0;
      const audit = await editor('audit', json);
      edit(audit.PM);
      const dual = audit.doc.dual!.stats;
      expect(dual.mismatched['edit.syncPeers'] ?? 0, `seed ${seed} dual`).toBe(0);
      compared += dual.compared['edit.syncPeers'] ?? 0;
    }
    expect(applied).toBeGreaterThan(0);
    expect(compared).toBeGreaterThan(0);
  });
});

describe('split text comparison', () => {
  it('reads the created layers out of Edit.mutate\'s result and leaves group membership aside', () => {
    const child = (id: string, text: string, group: string) => ({ id, type: 'text', group, d: { text }, p: {} });
    const plans = [{ children: [child('Lk1', 'Hello', 'g1'), child('Lk2', 'world', 'g1')] }];
    const created = { ok: true, message: 'Source updated', data: { result: [child('Lz8', 'Hello', 'g9'), child('Lz9', 'world', 'g9')], revision: 3 } };
    expect(compareSplit(plans, created, new Set())).toBeNull();
    expect(compareSplit(plans, [child('Lz8', 'Hello', 'g9'), child('Lz9', 'World', 'g9')], new Set())).not.toBeNull();
    expect(compareSplit([{ children: [] }], false, new Set())).toBeNull();
    expect(compareSplit(plans, false, new Set())).toBe('(nothing split)');
  });
});

describe('edit comparison on the tick grid', () => {
  const TICK = 1 / 705_600_000;
  const comp = (keys: any[], from = 1.0667) => ({ C1: { fps: 24, layers: [{ id: 'L1', from, p: { mix: { v: 0, kf: keys, expr: null } } }] } });
  const key = (i: string, t: number, v: number) => ({ i, t, v });

  it('allows created and moved key times the tick rounding of their terms on exact rates', () => {
    const before = comp([key('k1', 0, 0)]);
    const known = knownIds(before);
    const grid = timeGrid(before);
    expect(grid.drift).toBe(0);
    // Material peers on a 24 fps composition, layer start 1.0667: the moved
    // key is -48132983 ticks, 0.86 tick from the float sum.
    const ours = comp([key('k1', -0.06821567885487528, 0.5)]);
    const want = comp([key('k1', -0.06821568007580936, 0.5)]);
    expect(editDiffers(ours, want, known, grid)).toBeNull();
    expect(editDiffers(ours, want, known)).toBeNull();
    // Beyond the allowance (three ticks off) it is a mismatch.
    const far = comp([key('k1', -0.06821568007580936 + 3 * TICK, 0.5)]);
    expect(editDiffers(far, want, known, grid)).toMatch(/\.kf\[0\]\.t$/);
  });

  it('holds values of created keys to section 25\'s tolerance', () => {
    const before = comp([key('k1', 0, 0)], 0);
    const known = knownIds(before);
    const grid = timeGrid(before);
    const track = (v: number, id: string, t = 2.4583333333333335) => [key('n1', 2.4166666666666665, 71.55407422687858), key(id, t, v), key('n3', 2.5, 140.17270896993412)];
    // A created key on its tick (0.9 tick from the float sum) with the
    // legacy value: equal.
    expect(editDiffers(comp(track(71.55407422687858, 'n2', 2.4583333333333335 + 0.9 * TICK), 0), comp(track(71.55407422687858, 'n9'), 0), known, grid)).toBeNull();
    // Values get no sampling allowance, even next to a steep step.
    expect(editDiffers(comp(track(71.55407453068052, 'n2'), 0), comp(track(71.55407422687858, 'n9'), 0), known, grid)).toMatch(/\.kf\[1\]\.v$/);
    expect(editDiffers(comp(track(71.55407453068052, 'k1'), 0), comp(track(71.55407422687858, 'k1'), 0), known, grid)).toMatch(/\.kf\[1\]\.v$/);
  });

  it('allows NTSC key times the float rate\'s drift, not their values', () => {
    const ntsc = (keys: any[]) => ({ C1: { fps: 29.97, layers: [{ id: 'L1', from: 0.43333333333333335, p: { 'position.x': { v: 0, kf: keys, expr: null } } }] } });
    const before = ntsc([key('k1', 0, 0)]);
    const known = knownIds(before);
    const grid = timeGrid(before);
    expect(grid.drift).toBeGreaterThan(9e-7);
    // Frame 28 of 29.97 less an off-grid start: the tick time is 28 frames
    // of 30000/1001 (0.93 µs before 28 / 29.97).
    const legacy = 28 / 29.97 - 0.43333333333333335, ticked = 28 * 1001 / 30000 - 0.43333333333333335;
    expect(editDiffers(ntsc([key('n2', ticked, 1493.745000595005)]), ntsc([key('n9', legacy, 1493.745000595005)]), known, grid)).toBeNull();
    expect(editDiffers(ntsc([key('n2', ticked, 1493.745000595005)]), ntsc([key('n9', legacy, 1493.745000595005)]), known)).toMatch(/\.kf\[0\]\.t$/);
    expect(editDiffers(ntsc([key('n2', ticked, 1493.746950608196)]), ntsc([key('n9', legacy, 1493.745000595005)]), known, grid)).toMatch(/\.kf\[0\]\.v$/);
  });
});
