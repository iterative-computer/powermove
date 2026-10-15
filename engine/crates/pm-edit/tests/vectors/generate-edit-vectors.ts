/**
 * Generates tests/vectors/edit-vectors.json.gz by running today's TypeScript
 * edits under bun on seeded random documents: pose- and aim-preserving
 * reparenting (`parenting.ts`), move to group, ungroup (rebase and bake) and
 * a new group's pivot (`layer-groups.ts`), the alignment strip's commands
 * (`extensions/inspector/align.ts` with the strip's own box rule), the size
 * anchor's capture and its rescale after an anchor edit (`size-anchor.ts`,
 * `font-anchor.ts`), Pre-compose › Leave all attributes
 * (`compositions.ts`), material peer synchronization
 * (`scene3d/materials.ts`), and split text over given layouts
 * (`split-text.ts`). `tests/edit_parity.rs` runs pm-edit's kernels on the
 * same inputs and compares the documents they leave.
 *
 *   bun engine/crates/pm-edit/tests/vectors/generate-edit-vectors.ts
 *
 * PM_EDIT_SCALE multiplies the number of documents; PM_EDIT_SEED draws
 * another set; PM_EDIT_OUT writes elsewhere. PM_EDIT_CORPUS=<capture dir>
 * runs the same edits on every `loaded.json` of a corpus capture instead of
 * random documents (write those vectors outside the repository: the corpus
 * holds a private project). Numbers are written as the hex of their IEEE-754
 * bits so NaN, infinities, and -0 survive.
 */
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { gzipSync } from 'node:zlib';
import { install as installEasing } from '../../../../../apps/desktop/src/renderer/src/legacy/core/easing';
import { install as installAnim } from '../../../../../apps/desktop/src/renderer/src/legacy/core/anim';
import { install as installModel } from '../../../../../apps/desktop/src/renderer/src/legacy/core/model';
import * as S3 from '../../../../../apps/desktop/src/renderer/src/legacy/core/space-3d';
import * as Parenting from '../../../../../apps/desktop/src/renderer/src/legacy/core/parenting';
import { captureSizeAnchor, sizeAnchorReference } from '../../../../../apps/desktop/src/renderer/src/legacy/core/size-anchor';
import { rescaleAnchorBounds } from '../../../../../apps/desktop/src/renderer/src/legacy/core/font-anchor';
import { leaveAttributeBounds, shiftLeaveAnchor } from '../../../../../apps/desktop/src/renderer/src/legacy/core/compositions';
import { splitTextLayers } from '../../../../../apps/desktop/src/renderer/src/legacy/core/split-text';
import { shaderPeers, synchronizeShaderPeers } from '../../../../../apps/desktop/src/renderer/src/core/scene3d/materials';
import { alignDelta, deltaInParent, unionBox, type AlignMode } from '../../../../../apps/desktop/src/extensions/inspector/align';
import { adoptTemporalEase } from '../../../../../apps/desktop/src/renderer/src/core/anim/temporal-ease';
import { canonicalProject } from '../../../../../apps/desktop/src/shared/canonical-json';
import { install as installCompositor } from '../../../../../apps/desktop/src/renderer/src/legacy/gl/compositor';

const SCALE = Number(process.env.PM_EDIT_SCALE ?? 1);
const CORPUS = process.env.PM_EDIT_CORPUS ?? '';
const OUT = process.env.PM_EDIT_OUT ?? new URL('./edit-vectors.json.gz', import.meta.url).pathname;
if (CORPUS && !process.env.PM_EDIT_OUT) throw new Error('PM_EDIT_CORPUS needs PM_EDIT_OUT outside the repository');

/* ── deterministic randomness ───────────────────────────────────────────── */
let seed = Number(process.env.PM_EDIT_SEED ?? 0xed17);
function rand(): number {
  seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}
const pick = <T>(items: readonly T[]): T => items[Math.floor(rand() * items.length)]!;
const chance = (p: number) => rand() < p;
const range = (a: number, b: number) => a + (b - a) * rand();
const int = (a: number, b: number) => Math.floor(range(a, b + 1));
const subset = <T>(items: readonly T[], p: number): T[] => items.filter(() => chance(p));

/* ── encoding ───────────────────────────────────────────────────────────── */
const view = new DataView(new ArrayBuffer(8));
function hex(x: number): string {
  view.setFloat64(0, x);
  return view.getBigUint64(0).toString(16).padStart(16, '0');
}
/** A value as the document keeps it: members holding `undefined` dropped (a
 *  save drops them; the canonical document never has them). */
function enc(v: unknown): any {
  if (v === undefined) return { u: 1 };
  if (v === null) return { z: 1 };
  if (typeof v === 'number') return { n: hex(v) };
  if (typeof v === 'string') return { s: v };
  if (typeof v === 'boolean') return { b: v };
  if (Array.isArray(v)) return { a: v.map(x => (x === undefined ? { z: 1 } : enc(x))) };
  if (typeof v === 'object') {
    const o: Record<string, unknown> = {};
    for (const k of Object.keys(v as object).sort()) if ((v as any)[k] !== undefined) o[k] = enc((v as any)[k]);
    return { o };
  }
  return { s: String(v) };
}
function run(fn: () => unknown): any {
  try { return enc(fn()); } catch (e) { return { throw: (e as Error)?.message ?? 'Error' }; }
}
/**
 * What an edit changed in the canonical compositions, against the document
 * `before` it: for each composition whose layers changed, the layer ids in
 * their new order and the layers that differ from the layer with the same id
 * before (`[index, layer]`). Unchanged compositions are left out; the replay
 * holds the kernel to the input there.
 */
function layersAfter(before: string): any {
  const then = canonicalProject(settle(JSON.parse(before))).comps;
  const now = canonicalProject(settle(PM.proj)).comps;
  const same = (a: unknown, b: unknown) => JSON.stringify(enc(a)) === JSON.stringify(enc(b));
  const out: Record<string, unknown> = {};
  for (const id of [...new Set([...Object.keys(then), ...Object.keys(now)])].sort()) {
    const was: any[] = then[id]?.layers ?? [], is: any[] = now[id]?.layers ?? [];
    if (same(was, is)) continue;
    const changed: any[] = [];
    is.forEach((layer, i) => {
      const previous = was[i]?.id === layer?.id ? was[i] : was.find(l => l?.id === layer?.id);
      if (!same(previous, layer)) changed.push([i, layer]);
    });
    out[id] = { ids: is.map(l => l?.id), changed };
  }
  return out;
}

/**
 * Keys as evaluation leaves them: `temporalKeys` adopts the legacy `eo`/`ei`/
 * `hold` handles of every numeric track into native ease the first time it
 * evaluates it, so the documents the app edits hold adopted keys, and a
 * TypeScript edit adopts whatever it evaluates (a key it just created, when a
 * later sample reads that channel). Inputs are settled, and so is each
 * result before it is recorded; pm-edit writes the keys it creates adopted.
 */
function settle<T>(value: T): T {
  const walk = (v: any) => {
    if (Array.isArray(v)) { v.forEach(walk); return; }
    if (!v || typeof v !== 'object') return;
    if (Array.isArray(v.kf) && v.kf.length && v.kf.every((k: any) => k && typeof k === 'object' && typeof k.v === 'number')) adoptTemporalEase(v.kf);
    for (const key of Object.keys(v)) walk(v[key]);
  };
  walk(value);
  return value;
}

/* ── the legacy registry ────────────────────────────────────────────────── */
let uid = 0;
const PM: any = {
  clamp: (v: any, a: any, b: any) => (v < a ? a : v > b ? b : v),
  uid: (prefix = 'id') => `${prefix}${++uid}`,
  scope: [],
  bus: { on() {}, emit() {} },
  invalidate() {},
};
// util.ts installs browser logging; these two are its `snapF` and `round`.
PM.snapF = (t: any, fps: any) => Math.round(t * fps) / fps;
PM.round = (v: any, n = 2) => { const p = 10 ** n; return Math.round(v * p) / p; };
installEasing(PM);
installAnim(PM);
installModel(PM);
installCompositor(PM);
PM.scope = [];
PM.selectLayers = () => {};
const normalizeGroupStack = PM.normalizeGroupStack;
/** Runs `fn` with the stack normalization the renderer does after a kernel
 *  (`changed()`) switched off: the kernels return what the edit writes
 *  before it. */
function withoutNormalize<T>(fn: () => T): T {
  PM.normalizeGroupStack = () => {};
  try { return fn(); } finally { PM.normalizeGroupStack = normalizeGroupStack; }
}
function open(doc: any, time: number): any[] {
  PM.proj = typeof doc === 'string' ? JSON.parse(doc) : JSON.parse(JSON.stringify(doc));
  PM.scope = [];
  PM.time = time;
  PM.ProjectIndex?.invalidate?.();
  PM.touch();
  return PM.proj.layers;
}

/* ── random documents ───────────────────────────────────────────────────── */
const EASES = ['linear', 'easeIn', 'easeOut', 'power', 'backOut', 'hold'];
const EXPRESSIONS = ['value * 2', 'value + 10', 't * 30', 'time', 'value'];
function channel(v: number, spread: number, animated: boolean): any {
  const prop: any = { v, kf: [], expr: null };
  if (animated) {
    const n = int(1, 4);
    let t = range(-0.5, 0.5);
    for (let i = 0; i < n; i++) {
      const ease = pick(EASES);
      const { eo, ei } = PM.Ease.handles(ease === 'hold' ? 'linear' : ease);
      const key: any = { t, v: v + range(-spread, spread), eo, ei, hold: ease === 'hold', i: `k${++uid}` };
      if (chance(0.15)) { key.inEase = { speed: range(-50, 50), influence: 33 }; key.outEase = { speed: range(-50, 50), influence: 33 }; }
      prop.kf.push(key);
      t += range(0.3, 1.5);
    }
  }
  if (chance(0.06)) prop.expr = pick(EXPRESSIONS);
  return prop;
}
const animated = () => chance(0.3);

function transformChannels(three: boolean): Record<string, any> {
  const p: Record<string, any> = {
    'position.x': channel(range(-300, 900), 200, animated()),
    'position.y': channel(range(-200, 600), 200, animated()),
    'anchor.x': channel(chance(0.5) ? 0 : range(-80, 80), 40, animated()),
    'anchor.y': channel(chance(0.5) ? 0 : range(-80, 80), 40, animated()),
    'scale.x': channel(chance(0.05) ? 0 : pick([100, 100, range(-150, 250)]), 60, animated()),
    'scale.y': channel(chance(0.05) ? 0 : pick([100, 100, range(-150, 250)]), 60, animated()),
    rotation: channel(chance(0.4) ? 0 : range(-400, 400), 180, animated()),
    skew: channel(chance(0.7) ? 0 : range(-60, 60), 30, animated()),
    opacity: channel(chance(0.6) ? 100 : range(0, 100), 50, animated()),
  };
  if (three || chance(0.15)) {
    p['position.z'] = channel(range(-500, 800), 300, animated());
    p['anchor.z'] = channel(chance(0.5) ? 0 : range(-50, 50), 20, animated());
    p['scale.z'] = channel(pick([100, range(-50, 200)]), 50, animated());
    p['rotation.x'] = channel(chance(0.5) ? 0 : range(-180, 180), 90, animated());
    p['rotation.y'] = channel(chance(0.5) ? 0 : range(-180, 180), 90, animated());
    for (const axis of ['x', 'y', 'z']) p[`orientation.${axis}`] = channel(chance(0.7) ? 0 : pick([90, -90, 180, 33.3]), 30, animated());
    p.perspective = channel(range(10, 120), 30, animated());
  }
  return p;
}

const TYPES = ['solid', 'shape', 'image', 'video', 'null', 'precomp', 'shader', 'audio', 'adjustment', 'text'];
/** A random composition. `bounded`: only layers whose bounds bun can answer
 *  (no text, cameras, lights: those need canvas text and three.js). */
function makeDoc(index: number, bounded: boolean): any {
  const n = int(2, 10);
  const layers: any[] = [];
  for (let i = 0; i < n; i++) {
    const role = !bounded && chance(0.12) ? pick(['camera', 'light']) : null;
    const type = role ? 'extension' : chance(0.22) ? 'group' : pick(bounded ? TYPES.filter(t => t !== 'text') : TYPES);
    const three = chance(0.25);
    const layer: any = {
      id: `L${i}`, name: `Layer ${i}`, type, from: chance(0.6) ? 0 : pick([range(-1, 2), 1 / 30 * int(1, 40)]), dur: range(0.5, 6),
      on: true, lock: false, solo: false, threeD: three, parent: null, group: null, matteSource: null,
      p: transformChannels(three), d: {},
    };
    if (type === 'shape') layer.d = { shape: pick(['rect', 'ellipse']), w: range(10, 400), h: range(10, 300), stroke: chance(0.5) ? 0 : range(0, 30), color: '#ff0000' };
    if (['solid', 'image', 'video', 'shader', 'precomp'].includes(type)) layer.d = { w: chance(0.1) ? 0 : range(10, 2000), h: chance(0.1) ? 0 : range(10, 2000) };
    if (type === 'text') layer.d = { text: 'Hello', size: 40, font: 'Inter', weight: 600, align: 'center', color: '#ffffff' };
    if (chance(bounded ? 0.45 : 0.15) && ['shape', 'solid', 'image', 'video', 'shader', 'precomp'].includes(type)) layer.d.sizeAnchorBounds = { x0: range(-100, 0), y0: range(-100, 0), x1: range(1, 200), y1: range(1, 200) };
    if (role) {
      layer.d = { definition: `powermove.3d.${role}`, data: { [role]: { p: {
        targetX: channel(range(-5, 5), 3, animated()), targetY: channel(range(-5, 5), 3, animated()), targetZ: channel(range(-5, 5), 3, animated()),
      } } } };
      layer.threeD = true;
    }
    if (type === 'group' && chance(0.2)) layer.p.opacity = channel(range(20, 90), 10, animated());
    layers.push(layer);
  }
  // Groups and parents (no cycles: a parent comes earlier in a random order).
  const groups = layers.filter(l => l.type === 'group');
  for (const [i, layer] of layers.entries()) {
    const earlierGroups = groups.filter(g => layers.indexOf(g) < i);
    if (earlierGroups.length && chance(0.5)) layer.group = pick(earlierGroups).id;
    const parents = layers.slice(0, i).filter(l => l.type !== 'audio');
    if (parents.length && chance(0.3)) layer.parent = pick(parents).id;
  }
  const fps = pick([24, 25, 30, 29.97, 60, 23.976]);
  return {
    id: `P${index}`, compId: `C${index}`, compName: 'Edits', name: 'Edits', w: pick([640, 1920, 1080]), h: pick([360, 1080, 1920]),
    fps, dur: range(1, 4), layers, comps: {}, params: {},
  };
}

/* ── edits ──────────────────────────────────────────────────────────────── */
type Case = Record<string, unknown>;

/** The renderer's own checks before it asks a kernel (locks, cycles,
 *  styled groups): their throws are not the kernels'. */
const PRECHECKS = ['Group not found', 'Layer not found', 'Unlock layers before moving', 'Grouping would create a cycle',
  'Select a group to ungroup', 'Unlock the group first', 'Unlock the group contents before ungrouping', 'Remove visual compositing from'];
const precheck = (result: any) => typeof result?.throw === 'string' && PRECHECKS.some(m => result.throw.startsWith(m));

function reparentCases(doc: string, T: number, out: Case[]): void {
  const L = open(doc, T);
  for (let k = 0; k < 3 && L.length; k++) {
    const li = int(0, L.length - 1);
    const layer = L[li];
    if (layer.type === 'audio') continue;
    // `setLayer`'s checks: a transform-capable parent that is not the layer
    // and makes no cycle; the parent link must change.
    const options = [null, ...L.map((_, i) => i)].filter(pi => {
      if (pi === null) return layer.parent != null;
      const parent = L[pi];
      return parent !== layer && PM.TYPE_META[parent.type]?.transform !== false && !PM.wouldCycle(layer, parent.id) && layer.parent !== parent.id;
    });
    if (!options.length) continue;
    const pi = pick(options);
    open(doc, T);
    const live = PM.proj.layers;
    const result = run(() => { Parenting.preserveParentPose(PM, live[li], pi === null ? null : live[pi], T); return null; });
    out.push({ kind: 'reparent', t: hex(T), layer: li, parent: pi, out: result.throw ? result : enc(layersAfter(doc)) });
  }
}

function moveCases(doc: string, T: number, out: Case[]): void {
  const L = open(doc, T);
  const groups = L.map((l, i) => [l, i] as const).filter(([l]) => l.type === 'group');
  for (let k = 0; k < 3; k++) {
    const L2 = open(doc, T);
    const ids = subset(L2.map(l => l.id), 0.35);
    if (!ids.length) continue;
    const target = groups.length && chance(0.75) ? pick(groups)[0].id : null;
    // The kernel runs after the renderer's checks; their throws are not its.
    const selected = L2.filter(l => ids.includes(l.id));
    if (new Set(ids).size !== selected.length) continue;
    const group = target ? PM.L(target) : null;
    if (group && (ids.includes(group.id) || PM.groupAncestors(group).some((g: any) => ids.includes(g.id)))) continue;
    const roots = selected.filter(l => !PM.groupAncestors(l).some((g: any) => ids.includes(g.id))).map(l => L2.indexOf(l));
    const targetIndex = group ? L2.indexOf(group) : null;
    const result = run(() => withoutNormalize(() => PM.moveToGroup(ids, target)));
    if (precheck(result)) continue;
    out.push({ kind: 'moveToGroup', t: hex(T), roots, target: targetIndex, out: result.throw ? result : enc(layersAfter(doc)) });
  }
}

function ungroupCases(doc: string, T: number, out: Case[]): void {
  const L = open(doc, T);
  const groups = L.filter(l => l.type === 'group');
  if (!groups.length) return;
  for (let k = 0; k < 2; k++) {
    const L2 = open(doc, T);
    const ids = subset(groups.map(g => g.id), 0.6);
    if (!ids.length) ids.push(pick(groups).id);
    const indexes = L2.map((l, i) => [l, i] as const).filter(([l]) => ids.includes(l.id) && l.type === 'group').map(([, i]) => i);
    let reply: any = null;
    const result = run(() => { reply = withoutNormalize(() => PM.ungroupLayers(ids)); return null; });
    if (precheck(result)) continue;
    out.push({ kind: 'ungroup', t: hex(T), groups: indexes, out: result.throw ? result : enc({ layers: layersAfter(doc), ids: reply.ids }) });
  }
}

function groupPlacementCases(doc: string, T: number, out: Case[]): void {
  for (let k = 0; k < 2; k++) {
    const L = open(doc, T);
    const candidates = L.filter(l => !l.group);
    if (!candidates.length) return;
    const ids = subset(candidates.map(l => l.id), 0.5);
    if (!ids.length) continue;
    // The kernel places the group `groupLayers` inserted: the document as
    // `PM.groupBounds` sees it is its input.
    const groupBounds = PM.groupBounds;
    let before: string | null = null, index = -1;
    PM.groupBounds = (group: any, time: number) => {
      before = JSON.stringify(settle(PM.proj)); index = PM.proj.layers.indexOf(group);
      return groupBounds(group, time);
    };
    let result: any;
    try { result = run(() => { PM.groupLayers(ids, 'Group'); return null; }); } finally { PM.groupBounds = groupBounds; }
    if (before === null || result.throw) continue;
    out.push({ kind: 'groupPlacement', doc: before, t: hex(T), group: index, out: enc(layersAfter(before!)) });
  }
}

/** The alignment strip's commands (`AlignmentStrip.svelte`, `alignCommands`). */
function alignCommands(layers: any[], mode: AlignMode, time: number): any[] {
  const worldBox = (layer: any) => {
    if (S3.is3DLayer(PM, layer)) return null;
    const bounds = PM.GL.bounds(layer, time);
    if (!bounds) return null;
    const m = PM.worldMatrix(layer, time);
    const corners = [[bounds.x0, bounds.y0], [bounds.x1, bounds.y0], [bounds.x1, bounds.y1], [bounds.x0, bounds.y1]]
      .map(([x, y]) => ({ x: m[0] * x! + m[2] * y! + m[4], y: m[1] * x! + m[3] * y! + m[5] }));
    return { x0: Math.min(...corners.map(p => p.x)), y0: Math.min(...corners.map(p => p.y)), x1: Math.max(...corners.map(p => p.x)), y1: Math.max(...corners.map(p => p.y)) };
  };
  const targets = layers.filter(layer => layer && !layer.lock && layer.type !== 'audio');
  const boxed = targets.map(layer => ({ layer, box: worldBox(layer) })).filter((item): item is { layer: any; box: any } => !!item.box);
  const bounds = unionBox(boxed.map(item => item.box));
  if (!bounds) return [];
  const { w, h } = PM.proj;
  const world = alignDelta(bounds, { x0: 0, y0: 0, x1: w, y1: h }, mode);
  if (!world.dx && !world.dy) return [];
  const commands: any[] = [];
  for (const { layer } of boxed) {
    const parent = layer.parent ? PM.L(layer.parent) : null;
    const local = deltaInParent(parent ? PM.worldMatrix(parent, time) : null, world.dx, world.dy);
    const index = PM.proj.layers.indexOf(layer);
    if (local.dx) commands.push({ layer: index, channel: 'position.x', value: PM.round(Number(PM.ev(layer, 'position.x', time)) + local.dx, 3) });
    if (local.dy) commands.push({ layer: index, channel: 'position.y', value: PM.round(Number(PM.ev(layer, 'position.y', time)) + local.dy, 3) });
  }
  return commands;
}

function alignCases(doc: string, T: number, out: Case[]): void {
  const L = open(doc, T);
  for (let k = 0; k < 3; k++) {
    const chosen = subset(L.map((_, i) => i), 0.4);
    if (!chosen.length) continue;
    const mode = pick(['left', 'hcenter', 'right', 'top', 'vcenter', 'bottom'] as const);
    out.push({ kind: 'align', t: hex(T), layers: chosen, mode, out: run(() => alignCommands(chosen.map(i => L[i]), mode, T)) });
  }
}

function sizeAnchorCases(doc: string, T: number, out: Case[]): void {
  const L = open(doc, T);
  for (let li = 0; li < L.length; li++) {
    if (!chance(0.3)) continue;
    // Capture on the first `w` / `h` edit.
    const field = pick(['w', 'h', 'color']);
    const command = chance(0.5) ? { type: 'set_content', patch: { [field]: 10 }, time: T } : { type: 'set_property', path: `c.${field}`, time: T };
    open(doc, T);
    const captured = run(() => { captureSizeAnchor(PM, PM.proj.layers[li], command); return null; });
    out.push({ kind: 'captureSizeAnchor', t: hex(T), layer: li, fields: [field], out: captured.throw ? captured : enc(layersAfter(doc)) });
    // Pan Behind: the reference before the anchor edit, then the rescale.
    const L2 = open(doc, T);
    if (!L2[li].d?.sizeAnchorBounds) continue;
    const reference = run(() => sizeAnchorReference(PM, L2[li], T));
    out.push({ kind: 'anchorReference', t: hex(T), layer: li, out: reference });
    const bounds = sizeAnchorReference(PM, L2[li], T);
    const axis = pick(['x', 'y']);
    if (!bounds || !L2[li].p?.[`anchor.${axis}`]) continue;
    L2[li].p[`anchor.${axis}`].v = Number(L2[li].p[`anchor.${axis}`].v) + range(-50, 50);
    PM.touch();
    const moved = JSON.stringify(settle(PM.proj));
    const result = run(() => { rescaleAnchorBounds(PM, L2[li], 'size', T, bounds); return null; });
    out.push({ kind: 'anchorMoved', doc: moved, t: hex(T), layer: li, bounds: [bounds.x0, bounds.y0, bounds.x1, bounds.y1].map(hex), out: result.throw ? result : enc(layersAfter(moved)) });
  }
}

function leaveCases(doc: string, T: number, out: Case[]): void {
  const L = open(doc, T);
  for (let li = 0; li < L.length; li++) {
    if (!['solid', 'image', 'video', 'precomp', 'shader', 'extension'].includes(L[li].type) || !chance(0.5)) continue;
    open(doc, T);
    const layer = PM.proj.layers[li];
    let bounds: any = null;
    const result = run(() => { bounds = leaveAttributeBounds(PM, layer, T); shiftLeaveAnchor(layer, bounds.ax, bounds.ay); return null; });
    out.push({ kind: 'leaveAttributes', t: hex(T), layer: li, out: result.throw ? result : enc({ bounds, layers: layersAfter(doc) }) });
  }
}

/* ── materials ──────────────────────────────────────────────────────────── */
function shaderChannel(): any {
  return channel(range(0, 1), 1, chance(0.6));
}
function materialDoc(index: number): any {
  const shaders = ['shaderA', 'shaderB'];
  const material = () => ({ shader: chance(0.85) ? { id: pick(shaders), p: Object.fromEntries(['mix', 'glow'].filter(() => chance(0.8)).map(k => [k, shaderChannel()])) } : null, p: {} });
  const object = (i: number, comp: string): any => {
    const d: any = { definition: 'powermove.3d.object', data: { object: {} } };
    if (chance(0.5)) d.data.object.slots = Array.from({ length: int(1, 3) }, (_, s) => ({ id: `s${s}`, name: `Slot ${s}`, material: material() }));
    else d.data.object.material = material();
    return { id: `${comp}O${i}`, name: `Object ${i}`, type: 'extension', from: pick([0, range(-1, 2), 1 / 30 * int(1, 40)]), dur: 5, on: true, threeD: true, parent: null, group: null, p: transformChannels(true), d };
  };
  const fps = pick([24, 30, 29.97]);
  const comp = (id: string) => ({ id, name: id, w: 640, h: 360, fps: pick([fps, 25, 60]), dur: 5, layers: Array.from({ length: int(0, 3) }, (_, i) => object(i, id)) });
  const other = Object.fromEntries(Array.from({ length: int(0, 2) }, (_, i) => [`K${index}x${i}`, comp(`K${index}x${i}`)]));
  return {
    id: `M${index}`, compId: `MC${index}`, compName: 'Materials', name: 'Materials', w: 640, h: 360, fps, dur: 5,
    layers: Array.from({ length: int(1, 4) }, (_, i) => object(i, `MC${index}`)), comps: other, params: {},
  };
}

function materialCases(doc: string, T: number, out: Case[]): void {
  const L = open(doc, T);
  for (let li = 0; li < L.length; li++) {
    const object = L[li].d?.data?.object;
    const slots = object?.slots?.length ? object.slots.map((s: any) => ({ prefix: `slots.${s.id}.shader`, material: s.material })) : [{ prefix: 'shader', material: object?.material }];
    for (const slot of slots) {
      for (const param of Object.keys(slot.material?.shader?.p || {})) {
        if (!chance(0.6)) continue;
        const channelPath = `${slot.prefix}.${param}`;
        const L2 = open(doc, T);
        const layer = L2[li];
        const prop = PM.findProp(layer, channelPath) || layer.p[channelPath];
        if (!prop) continue;
        // The edit the pipeline made before synchronizing: a new value, a
        // key at a frame, or a moved key.
        const roll = rand();
        if (roll < 0.4) prop.v = range(0, 1);
        else if (roll < 0.8) PM.setKeyOn(prop, Math.round(range(0, 3) * PM.proj.fps) / PM.proj.fps - layer.from, range(0, 1), 'linear', PM.proj.fps);
        else if (prop.kf.length) prop.kf[0].t += 1 / PM.proj.fps;
        PM.touch();
        const before = JSON.stringify(settle(PM.proj));
        const result = run(() => { synchronizeShaderPeers(PM, shaderPeers(PM, layer, channelPath), prop, layer); return null; });
        out.push({ kind: 'syncPeers', doc: before, t: hex(T), layer: li, channel: channelPath, out: result.throw ? result : enc(layersAfter(before)) });
      }
    }
  }
}

/* ── split text ─────────────────────────────────────────────────────────── */
/** A synthetic layout in UTF-16 units (pm-text lays out real text; these
 *  cases compare what the split does with a layout). */
function layoutFor(text: string): any {
  const out: any = { characters: [], words: [], lines: [] };
  let ci = 0, wi = 0, li = 0, source = 0;
  text.split('\n').forEach((line, row) => {
    const width = line.length * 9.5, startX = -width / 2;
    const lineUnit = li;
    if (line.length) out.lines.push({ text: line, x: startX, y: row * 30, w: width, line: row, sourceStart: source, index: li++ });
    let x = startX, local = -1, wasSpace = true, offset = 0;
    for (const ch of Array.from(line)) {
      const space = /^\s+$/u.test(ch);
      if (!space && wasSpace) local++;
      if (!space) out.characters.push({ text: ch, x, y: row * 30, w: 9.5 * ch.length, line: row, sourceStart: source + offset, index: ci++, lineUnit, word: wi + local });
      wasSpace = space; x += 9.5 * ch.length; offset += ch.length;
    }
    for (const m of line.matchAll(/\S+/gu)) out.words.push({ text: m[0], x: startX + m.index! * 9.5, y: row * 30, w: m[0].length * 9.5, line: row, sourceStart: source + m.index!, index: wi++ });
    source += line.length + 1;
  });
  return out;
}
const TEXTS = ['Hello world', 'One', 'Two lines\nof text', '  spaced   out  ', 'é café 👍🏽 ok', '日本語 テキスト', ''];
function splitDoc(index: number): any {
  const layers = Array.from({ length: int(1, 3) }, (_, i) => {
    const text = pick(TEXTS);
    return {
      id: `T${i}`, name: `Text ${i}`, type: 'text', from: range(-1, 1), dur: 6, on: true, lock: false, solo: chance(0.2), parent: null, group: null, p: transformChannels(false),
      d: {
        text, size: pick([16, 30, 72]), align: pick(['left', 'center', 'right']), color: '#ffffff', font: 'Inter', weight: 600, paragraph: chance(0.3),
        ...(chance(0.3) ? { fontAnchorBounds: { x0: -50, y0: -20, x1: 50, y1: 20 } } : {}),
        animators: Array.from({ length: int(0, 2) }, (_, a) => ({
          id: `a${a}`, enabled: true, ...(chance(0.6) ? { unit: pick(['characters', 'words', 'lines', 'bogus']) } : {}),
          ...(chance(0.3) ? { unitOffset: int(0, 4) } : {}), ...(chance(0.3) ? { unitTotal: int(0, 20) } : {}),
          p: { opacity: channel(range(0, 100), 20, chance(0.3)), ...(chance(0.4) ? { unit: { v: pick(['characters', 'words', 'lines', 'x']), kf: [], expr: null } } : {}) },
        })),
        styles: Array.from({ length: int(0, 2) }, (_, s) => ({ id: `s${s}`, start: int(0, 4), end: int(2, 12), p: { color: { v: '#00ff00', kf: [], expr: null } } })),
      },
    };
  });
  return { id: `S${index}`, compId: `SC${index}`, compName: 'Split', name: 'Split', w: 640, h: 360, fps: 30, dur: 6, layers, comps: {}, params: {} };
}

function splitCases(doc: string, T: number, out: Case[]): void {
  const L = open(doc, T);
  const sources = subset(L.map((_, i) => i), 0.7);
  if (!sources.length) sources.push(0);
  const mode = pick(['words', 'characters', 'lines'] as const);
  const layouts = sources.map(i => layoutFor(String(L[i].d.text)));
  const offsets = sources.map(() => ({ x: chance(0.5) ? 0 : range(-20, 20), y: chance(0.5) ? 0 : range(-20, 20) }));
  open(doc, T);
  const live = PM.proj.layers;
  const byLayer = new Map(sources.map((i, k) => [live[i], k]));
  PM.textLayout = (layer: any) => layouts[byLayer.get(layer)!];
  PM.raster = (layer: any) => ({ fontOffset: offsets[byLayer.get(layer)!] });
  PM.Edit = { mutate: (_label: string, fn: () => unknown) => fn() };
  const groupLayers = PM.groupLayers;
  PM.groupLayers = () => ({ id: 'group' });
  let created: any;
  try {
    const result = run(() => { created = splitTextLayers(PM, sources.map(i => live[i].id), mode); return null; });
    out.push({ kind: 'splitText', t: hex(T), sources, mode, layouts: JSON.stringify(layouts), offsets: offsets.map(o => [hex(o.x), hex(o.y)]),
      out: result.throw ? result : enc(created === false ? [] : created) });
  } finally { PM.groupLayers = groupLayers; delete PM.textLayout; delete PM.raster; }
}

/* ── documents ──────────────────────────────────────────────────────────── */
const docs: any[] = [];
const add = (doc: any, makeCases: (doc: string, T: number, out: Case[]) => void, times: number[]) => {
  const text = JSON.stringify(settle(doc));
  const cases: Case[] = [];
  for (const T of times) makeCases(text, T, cases);
  if (cases.length) docs.push({ doc: text, cases });
};
const timesFor = (doc: any) => [0, Math.round(range(0, doc.dur) * doc.fps) / doc.fps, range(-0.5, doc.dur)];

if (CORPUS) {
  const files: string[] = [];
  const walk = (dir: string) => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) { if (!['frames', 'eval', 'media', 'generated'].includes(name)) walk(path); }
      else if (name === 'loaded.json') files.push(path);
    }
  };
  walk(CORPUS);
  files.sort();
  for (const file of files) {
    const project = JSON.parse(readFileSync(file, 'utf8'));
    const doc = project.proj ?? project;
    if (!Array.isArray(doc?.layers) || typeof doc.compId !== 'string') continue;
    const real = file.includes('/real/');
    const times = real ? [Math.round(range(0, doc.dur) * doc.fps) / doc.fps] : timesFor(doc).slice(0, 2);
    // Bounds questions on text need canvas measurement and 3D objects need
    // three.js: the replay skips what pm-edit leaves to the host, and the
    // generator still records every TypeScript answer.
    add(doc, (text, T, out) => {
      const before = out.length;
      reparentCases(text, T, out); moveCases(text, T, out); ungroupCases(text, T, out);
      alignCases(text, T, out); groupPlacementCases(text, T, out); sizeAnchorCases(text, T, out); leaveCases(text, T, out); materialCases(text, T, out);
      // The real project is large: keep a sample of its cases.
      if (real) out.splice(before, out.length - before, ...out.slice(before).filter(() => chance(0.25)));
    }, times);
    console.log(`${file.slice(CORPUS.length)}: ${docs.at(-1)?.cases.length ?? 0} cases`);
  }
} else {
  for (let index = 0; index < 40 * SCALE; index++) {
    const doc = makeDoc(index, false);
    add(doc, (text, T, out) => { reparentCases(text, T, out); moveCases(text, T, out); ungroupCases(text, T, out); }, timesFor(doc));
  }
  for (let index = 0; index < 30 * SCALE; index++) {
    const doc = makeDoc(1000 + index, true);
    add(doc, (text, T, out) => { alignCases(text, T, out); groupPlacementCases(text, T, out); sizeAnchorCases(text, T, out); leaveCases(text, T, out); }, timesFor(doc));
  }
  for (let index = 0; index < 30 * SCALE; index++) add(materialDoc(index), materialCases, [0, range(0, 4)]);
  for (let index = 0; index < 30 * SCALE; index++) add(splitDoc(index), splitCases, [0, range(0, 3)]);
}

writeFileSync(OUT, gzipSync(JSON.stringify({ docs }), { level: 9 }));
const counts: Record<string, number> = {};
for (const d of docs) for (const c of d.cases) counts[c.kind as string] = (counts[c.kind as string] ?? 0) + 1;
console.log(`wrote ${OUT}: ${docs.length} documents, ${JSON.stringify(counts)}`);
// The compositor module keeps timers alive; leave once the file is written.
process.exit(0);
