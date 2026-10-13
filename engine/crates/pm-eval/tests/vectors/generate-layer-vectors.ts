/**
 * Generates tests/vectors/layer-vectors.json.gz by running the real TypeScript
 * layer queries under bun: `legacy/core/anim.ts` (localMatrix, worldMatrix,
 * transformParentMatrix, worldOpacity, wouldCycle, active), `space-3d.ts`,
 * `parenting.ts`, `layer-groups.ts`, `matte.ts`, `retiming.ts`,
 * `size-anchor.ts`, `font-anchor.ts`, and `text-animation.ts`, installed on a
 * minimal `PM` the way the app installs them. `tests/layer_parity.rs`
 * replays every case.
 *
 *   bun engine/crates/pm-eval/tests/vectors/generate-layer-vectors.ts
 *
 * PM_LAYER_SCALE multiplies the number of random documents; PM_LAYER_SEED
 * draws another set; PM_LAYER_OUT writes elsewhere. Deterministic: a seeded
 * PRNG drives every case. Numbers are written as the hex of their IEEE-754
 * bits so NaN, infinities, and -0 survive.
 */
import { writeFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import { install as installEasing } from '../../../../../apps/desktop/src/renderer/src/legacy/core/easing';
import { install as installAnim } from '../../../../../apps/desktop/src/renderer/src/legacy/core/anim';
import { install as installModel } from '../../../../../apps/desktop/src/renderer/src/legacy/core/model';
import * as S3 from '../../../../../apps/desktop/src/renderer/src/legacy/core/space-3d';
import * as Parenting from '../../../../../apps/desktop/src/renderer/src/legacy/core/parenting';
import { validMatteSource } from '../../../../../apps/desktop/src/renderer/src/legacy/core/matte';
import { sourceTime } from '../../../../../apps/desktop/src/renderer/src/legacy/core/retiming';
import { sizeBounds, sizeAnchorOffset } from '../../../../../apps/desktop/src/renderer/src/legacy/core/size-anchor';
import { fontAnchorOffset } from '../../../../../apps/desktop/src/renderer/src/legacy/core/font-anchor';
import * as TA from '../../../../../apps/desktop/src/renderer/src/legacy/core/text-animation';

const SCALE = Number(process.env.PM_LAYER_SCALE ?? 1);
const OUT = process.env.PM_LAYER_OUT ?? new URL('./layer-vectors.json.gz', import.meta.url).pathname;

/* ── deterministic randomness ───────────────────────────────────────────── */
let seed = Number(process.env.PM_LAYER_SEED ?? 0x1a7e5);
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

/* ── encoding ───────────────────────────────────────────────────────────── */
const view = new DataView(new ArrayBuffer(8));
function hex(x: number): string {
  view.setFloat64(0, x);
  return view.getBigUint64(0).toString(16).padStart(16, '0');
}
function sortDeep<T>(value: T): T {
  if (Array.isArray(value)) return value.map(sortDeep) as T;
  if (value && typeof value === 'object') {
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(value).sort()) out[key] = sortDeep((value as any)[key]);
    return out as T;
  }
  return value;
}
function enc(v: unknown): any {
  if (v === undefined) return { u: 1 };
  if (v === null) return { z: 1 };
  if (typeof v === 'number') return { n: hex(v) };
  if (typeof v === 'string') return { s: v };
  if (typeof v === 'boolean') return { b: v };
  if (Array.isArray(v)) return { a: v.map(enc) };
  if (typeof v === 'object') {
    const o: Record<string, unknown> = {};
    for (const k of Object.keys(v as object).sort()) o[k] = enc((v as any)[k]);
    return { o };
  }
  return { s: String(v) };
}
function run(fn: () => unknown): any {
  try { return enc(fn()); } catch (e) { return { throw: (e as Error)?.message ?? 'Error' }; }
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
installEasing(PM);
installAnim(PM);
installModel(PM);
PM.scope = [];

/* ── random documents ───────────────────────────────────────────────────── */
const EASES = ['linear', 'easeIn', 'easeOut', 'power', 'backOut', 'hold'];
function channel(v: number, spread: number, animated: boolean): any {
  if (!animated) return { v, kf: [], expr: null };
  const n = int(2, 4), keys: any[] = [];
  let t = range(-0.5, 0.5);
  for (let i = 0; i < n; i++) {
    const ease = pick(EASES);
    const { eo, ei } = PM.Ease.handles(ease === 'hold' ? 'linear' : ease);
    keys.push({ t, v: v + range(-spread, spread), eo, ei, hold: ease === 'hold', i: `k${i}` });
    t += range(0.3, 1.5);
  }
  return { v, kf: keys, expr: null };
}
const animated = () => chance(0.35);

function transformChannels(three: boolean): Record<string, any> {
  const p: Record<string, any> = {
    'position.x': channel(range(-300, 900), 200, animated()),
    'position.y': channel(range(-200, 600), 200, animated()),
    'anchor.x': channel(chance(0.5) ? 0 : range(-80, 80), 40, animated()),
    'anchor.y': channel(chance(0.5) ? 0 : range(-80, 80), 40, animated()),
    'scale.x': channel(chance(0.1) ? 0 : range(-150, 250), 60, animated()),
    'scale.y': channel(chance(0.1) ? 0 : range(-150, 250), 60, animated()),
    rotation: channel(chance(0.3) ? 0 : range(-400, 400), 180, animated()),
    skew: channel(chance(0.6) ? 0 : range(-85, 85), 30, animated()),
    opacity: channel(range(-20, 130), 50, animated()),
  };
  if (three || chance(0.2)) {
    p['position.z'] = channel(range(-500, 800), 300, animated());
    p['anchor.z'] = channel(chance(0.5) ? 0 : range(-50, 50), 20, animated());
    p['scale.z'] = channel(range(-50, 200), 50, animated());
    p['rotation.x'] = channel(chance(0.4) ? 0 : range(-180, 180), 90, animated());
    p['rotation.y'] = channel(chance(0.4) ? 0 : range(-180, 180), 90, animated());
    for (const axis of ['x', 'y', 'z']) p[`orientation.${axis}`] = channel(chance(0.6) ? 0 : pick([90, -90, 180, 33.3]), 30, animated());
    p.perspective = channel(chance(0.2) ? pick([0, -5, 0.0001]) : range(10, 120), 30, animated());
  }
  if (chance(0.05)) delete p.skew;
  if (chance(0.05)) p.rotation = { v: '45', kf: [], expr: null };
  return p;
}

const TYPES = ['solid', 'shape', 'image', 'video', 'null', 'text', 'precomp', 'shader', 'audio', 'adjustment'];
function makeDoc(index: number): any {
  const n = int(3, 12);
  const layers: any[] = [];
  for (let i = 0; i < n; i++) {
    const role = chance(0.12) ? pick(['camera', 'light']) : null;
    const type = role ? 'extension' : chance(0.2) ? 'group' : pick(TYPES);
    const three = chance(0.35);
    const layer: any = {
      id: chance(0.04) && i > 0 ? layers[0].id : `L${i}`,
      name: `Layer ${i}`, type, from: chance(0.7) ? 0 : range(-1, 2), dur: range(0.5, 6),
      on: chance(0.9) ? true : chance(0.5) ? false : { v: true, kf: [{ t: 1, v: false, i: 'k0' }], expr: null },
      lock: chance(0.15), solo: chance(0.05), threeD: three,
      parent: null, group: null, matteSource: null,
      p: transformChannels(three),
      d: {},
    };
    if (type === 'shape') layer.d = { shape: pick(['rect', 'ellipse', 'star']), w: chance(0.1) ? 0 : range(10, 400), h: range(10, 300), stroke: chance(0.5) ? 0 : range(0, 30), color: '#ff0000' };
    if (type === 'solid' || type === 'image' || type === 'video' || type === 'shader' || type === 'precomp') layer.d = { w: chance(0.2) ? 0 : range(10, 2000), h: chance(0.2) ? 0 : range(10, 2000) };
    if (type === 'video' || type === 'precomp' || type === 'audio') {
      Object.assign(layer.d, { trim: chance(0.5) ? 0 : range(0, 3) });
      const kind = rand();
      if (kind < 0.3) layer.d.speed = range(0.25, 3);
      else if (kind < 0.6) layer.d.speed = channel(1, 1.5, true);
      else if (kind < 0.7) { layer.d.timeRemap = { v: true, kf: [], expr: null }; layer.d.sourceTime = channel(1, 2, true); }
    }
    if (chance(0.2) && ['shape', 'solid', 'image', 'video', 'shader', 'precomp'].includes(type)) layer.d.sizeAnchorBounds = { x0: range(-100, 0), y0: range(-100, 0), x1: range(1, 200), y1: range(1, 200) };
    if (role) {
      layer.d = { definition: `powermove.3d.${role}`, data: { [role]: { p: {
        targetX: channel(range(-5, 5), 3, animated()), targetY: channel(range(-5, 5), 3, animated()), targetZ: channel(range(-5, 5), 3, animated()),
      } } } };
      layer.threeD = true;
    }
    layers.push(layer);
  }
  // Groups, parents, mattes (cycles included).
  const groups = layers.filter(l => l.type === 'group');
  for (const layer of layers) {
    if (groups.length && chance(0.5)) layer.group = pick(groups).id;
    if (chance(0.35)) layer.parent = pick(layers).id;
    if (chance(0.1)) layer.matteSource = pick([...layers.map(l => l.id), 'missing']);
  }
  return sortDeep({
    id: `P${index}`, compId: `C${index}`, name: 'Vectors', w: pick([640, 1920, 1080, 0]), h: pick([360, 1080, 1920]),
    fps: pick([24, 25, 30, 29.97, 60]), dur: range(4, 10), layers, comps: {}, params: {},
  });
}

/* ── text animation ─────────────────────────────────────────────────────── */
function layoutFor(text: string): any {
  const lines = text.split('\n');
  const out: any = { characters: [], words: [], lines: [] };
  let ci = 0, wi = 0, li = 0, source = 0;
  lines.forEach((line, row) => {
    const width = line.length * 9.5;
    const startX = -width / 2;
    if (line.length) out.lines.push({ text: line, x: startX, y: row * 30, w: width, line: row, sourceStart: source, index: li });
    const lineUnit = li;
    if (line.length) li++;
    let x = startX, local = -1, wasSpace = true;
    for (const ch of Array.from(line)) {
      const space = /^\s+$/u.test(ch);
      if (!space && wasSpace) local++;
      if (!space) out.characters.push({ text: ch, x, y: row * 30, w: 9.5, line: row, sourceStart: source, index: ci++, lineUnit, word: wi + local });
      wasSpace = space; x += 9.5; source += ch.length;
    }
    for (const m of line.matchAll(/\S+/gu)) out.words.push({ text: m[0], x: startX + m.index! * 9.5, y: row * 30, w: m[0].length * 9.5, line: row, sourceStart: m.index!, index: wi++ });
    source += 1;
  });
  return out;
}
function animator(i: number): any {
  const mode = pick(['stagger', 'range', 'wave', undefined]);
  const p: Record<string, any> = {};
  const set = (k: string, v: any) => { p[k] = chance(0.3) ? channel(v, 20, true) : { v, kf: [], expr: null }; };
  for (const k of ['opacity', 'x', 'y', 'scale', 'rotation', 'skew', 'tracking', 'blur']) if (chance(0.5)) set(k, range(-120, 160));
  if (chance(0.3)) p.color = { v: pick(['#ff0000', '#0f08', '#12345678', 'rgb(10, 200, 30)', 'rgba(1.5,2,3,0.5)', 'red', '']), kf: [], expr: null };
  for (const k of ['amount', 'delay', 'duration', 'stagger', 'start', 'end', 'offset', 'smoothness', 'speed', 'spread']) if (chance(0.5)) set(k, range(-20, 120) / (k === 'delay' || k === 'duration' || k === 'stagger' ? 40 : 1));
  if (chance(0.3)) p.unit = { v: pick(['characters', 'words', 'lines', 'other']), kf: [], expr: null };
  const a: any = { id: `a${i}`, p, enabled: chance(0.9) ? true : false };
  if (mode) a.mode = mode;
  if (chance(0.6)) a.unit = pick(['characters', 'words', 'lines', 'bogus']);
  if (chance(0.6)) a.order = pick(['forward', 'reverse', 'center', 'edges', 'random']);
  if (chance(0.5)) a.seed = pick([1, 7, 123456, -3.5, 0, 4294967297]);
  if (chance(0.5)) a.easing = pick(['spring', 'easeOut', 'power', 'linear', 'cubic', 'backOut']);
  if (chance(0.5)) a.direction = pick(['in', 'out']);
  if (chance(0.5)) a.shape = pick(['square', 'rampUp', 'rampDown', 'triangle', 'round']);
  if (chance(0.2)) a.unitOffset = int(0, 5);
  if (chance(0.2)) a.unitTotal = int(0, 30);
  return a;
}
const TEXTS = ['Hello world', 'One', 'Two lines\nof text', '  spaced   out  ', 'é café 👍🏽 family 👨‍👩‍👧', '日本語のテキスト', 'a\tb c d', ''];
function textDoc(index: number): any {
  const text = pick(TEXTS);
  const layer: any = {
    id: 'T', type: 'text', name: 'Text', from: range(-1, 1), dur: 6, on: true, p: transformChannels(false),
    d: { text, size: pick([16, 30, 0, 72]), align: pick(['left', 'center', 'right']), color: pick(['#ffffff', 'rgb(1,2,3)', '#abc']), font: 'Inter', weight: 600,
      animators: Array.from({ length: int(0, 3) }, (_, i) => animator(i)),
      styles: Array.from({ length: int(0, 2) }, (_, i) => ({ id: `s${i}`, start: int(0, 4), end: pick([int(2, 9), '5', null]), p: { color: { v: '#00ff00', kf: [], expr: null }, size: { v: 40, kf: [], expr: null } } })) },
  };
  return { doc: sortDeep({ id: `T${index}`, compId: `TC${index}`, w: 640, h: 360, fps: 30, dur: 6, layers: [layer], comps: {}, params: {} }), text };
}

/* ── cases ──────────────────────────────────────────────────────────────── */
const docs: any[] = [];
const COUNT = 36 * SCALE;
for (let index = 0; index < COUNT; index++) {
  const doc = makeDoc(index);
  PM.proj = JSON.parse(JSON.stringify(doc));
  PM.ProjectIndex?.invalidate?.();
  PM.touch();
  const L = PM.proj.layers;
  const cases: any[] = [];
  const times = [0, range(-1, 5), doc.dur];
  for (const T of times) {
    PM.time = T;
    cases.push({ op: 'depth', t: hex(T), out: run(() => S3.depthOrderedLayers(PM, L, T).map((l: any) => L.indexOf(l))) });
    for (let i = 0; i < L.length; i++) {
      const layer = L[i];
      const c = (op: string, fn: () => unknown, extra: any = {}) => cases.push({ op, layer: i, t: hex(T), ...extra, out: run(fn) });
      c('active', () => PM.active(layer, T));
      c('local', () => PM.localMatrix(layer, T));
      c('world', () => PM.worldMatrix(layer, T));
      c('parent2d', () => PM.transformParentMatrix(layer, T));
      c('opacity', () => PM.worldOpacity(layer, T));
      c('local3d', () => S3.local3D(PM, layer, T));
      c('parent3d', () => S3.parent3D(PM, layer, T));
      c('world3d', () => S3.world3D(PM, layer, T));
      c('world3dpos', () => S3.world3D(PM, layer, T, true));
      c('is3d', () => S3.is3DLayer(PM, layer));
      c('persp', () => S3.perspectiveAmount(PM, layer, T));
      c('plane', () => S3.planeMatrix(PM, layer, T));
      c('planepos', () => S3.planeMatrix(PM, layer, T, true));
      c('affine', () => S3.affinePlane(S3.planeMatrix(PM, layer, T)));
      try { const w = S3.world3D(PM, layer, T); c('inverse3d', () => S3.inverse3D(w), { world: w.map(hex) }); } catch { /* covered by world3d */ }
      // Decompositions take their matrices as inputs, so ill-conditioned
      // poses compare the decomposition itself rather than amplify the
      // last-bit differences of sin and cos upstream.
      try {
        const world3 = S3.world3D(PM, layer, T), parent3 = S3.parent3D(PM, layer, T);
        c('pose3d', () => S3.poseValues3D(PM, layer, world3, T, parent3), { world: world3.map(hex), parent: parent3.map(hex) });
      } catch { /* the inputs throw: covered by world3d and parent3d */ }
      try {
        const world = PM.worldMatrix(layer, T), parent = PM.transformParentMatrix(layer, T);
        c('pose2d', () => Parenting.worldPoseValues(PM, layer, world, T, parent), { world: world.map(hex), parent: parent.map(hex) });
        c('pose2dzero', () => Parenting.worldPoseValues(PM, layer, world, T, parent, true), { world: world.map(hex), parent: parent.map(hex) });
      } catch { /* covered by world and parent2d */ }
      c('aim', () => Parenting.worldAim3D(PM, layer, T));
      try {
        const aim = Parenting.worldAim3D(PM, layer, T), parent3 = S3.parent3D(PM, layer, T);
        if (aim) c('aimvalues', () => Parenting.aimValues3D(PM, layer, aim, T, parent3), { aim: aim.map(hex), parent: parent3.map(hex) });
      } catch { /* covered by aim */ }
      const x = range(-100, 1000), y = range(-100, 700), b = { x0: range(-300, 0), y0: range(-300, 0), x1: range(0, 300), y1: range(0, 300) };
      c('contains', () => S3.planeContains(PM, layer, T, x, y, b), { x: hex(x), y: hex(y), b: [b.x0, b.y0, b.x1, b.y1].map(hex) });
      const target = pick([...L.map((l: any) => l.id), 'missing', null]);
      c('cycle', () => PM.wouldCycle(layer, target), { target: enc(target) });
      c('matte', () => validMatteSource(L, layer, target), { target: enc(target) });
      c('sizeBounds', () => sizeBounds(PM, layer, T));
      c('sizeAnchor', () => sizeAnchorOffset(PM, layer, T));
      if (['video', 'precomp', 'audio'].includes(layer.type)) c('sourceTime', () => sourceTime(PM, layer, T));
      if (layer.type === 'group') {
        c('span', () => PM.groupSpan(layer));
        c('expand', () => PM.expandGroups([layer.id]));
      }
    }
    const ids = L.filter(() => chance(0.5)).map((l: any) => l.id);
    cases.push({ op: 'roots', t: hex(T), ids: ids.map(enc), out: run(() => PM.transformRoots(ids).map((l: any) => L.indexOf(l))) });
  }
  docs.push({ doc: JSON.stringify(doc), cases });
}

const texts: any[] = [];
for (let index = 0; index < 48 * SCALE; index++) {
  const { doc, text } = textDoc(index);
  PM.proj = JSON.parse(JSON.stringify(doc));
  PM.ProjectIndex?.invalidate?.();
  PM.touch();
  const layer = PM.proj.layers[0];
  const layout = layoutFor(text);
  const cases: any[] = [];
  for (const T of [0, range(0, 1), range(0, 3), range(3, 8)]) {
    cases.push({ t: hex(T), out: run(() => {
      const state = TA.textAnimationState(PM, layer, T, layout);
      const d = PM.resolveContent(layer, T);
      const glyphs = TA.animatedGlyphs(PM, layer, T, d, layout, state);
      return { animators: state.animators, styles: state.styles, glyphs: glyphs.map((g: any) => ({
        index: g.index, x: g.x, matrix: g.matrix, opacity: g.opacity, blur: g.blur, color: g.color, tracking: g.tracking,
        trackingShift: g.trackingShift, style: { color: g.style.color, size: g.style.size, font: g.style.font } })), identity: glyphs.map((g: any) => TA.isIdentityGlyph(g)) };
    }) });
  }
  texts.push({ doc: JSON.stringify(doc), layout: JSON.stringify(layout), cases });
}

// Direct helpers.
const helpers: any[] = [];
const UNIT_STRINGS = ['', ' ', 'a b', 'é́x', '👨‍👩‍👧‍👦 👍🏽', '   x 　y', '한국어 텍스트', 'a‍b', 'x﻿y', 'तुम्हारा', '🇯🇵🇺🇸', 'a\r\nb'];
for (const unit of ['characters', 'words', 'lines']) for (let i = 0; i < 20 * SCALE; i++) {
  const lines = Array.from({ length: int(0, 3) }, () => ({ text: pick(UNIT_STRINGS) + (chance(0.5) ? pick(UNIT_STRINGS) : '') }));
  helpers.push({ op: 'countUnits', unit, lines: lines.map(l => l.text), out: run(() => TA.countTextUnits(lines, unit as any)) });
}
for (let i = 0; i < 60 * SCALE; i++) {
  const count = int(0, 40), index = int(0, Math.max(0, count - 1)), s = pick([1, 7, 99, 0, -1, 4294967296 + 5, 2.7]);
  for (const order of ['forward', 'reverse', 'center', 'edges', 'random', undefined]) helpers.push({ op: 'rank', order: enc(order), index, count, seed: hex(s), out: run(() => TA.unitRank(order as any, index, count, s)) });
}
for (let i = 0; i < 200 * SCALE; i++) {
  const p: any = {};
  for (const k of ['start', 'end', 'offset', 'smoothness']) if (chance(0.7)) p[k] = pick([range(-50, 150), NaN, Infinity, '30', null]);
  const rank = range(-2, 30), count = int(0, 30), shape = pick(['square', 'rampUp', 'rampDown', 'triangle', 'round', undefined]);
  helpers.push({ op: 'weight', p: enc(p), rank: hex(rank), count, shape: enc(shape), out: run(() => TA.animatorWeight(p, rank, count, shape as any)) });
}
for (const easing of ['spring', 'linear', 'easeOut', 'power', 'backOut', 'cubic', undefined, '']) for (const progress of [-1, 0, 1e-9, 0.1, 0.25, 0.5, 0.75, 0.999, 1, 2, NaN, range(0, 1), range(0, 1)]) {
  helpers.push({ op: 'ease', easing: enc(easing), progress: hex(progress), out: run(() => TA.easeProgress(PM, easing, progress)) });
}
for (let i = 0; i < 100 * SCALE; i++) {
  const ref = chance(0.9) ? { x0: pick([range(-100, 0), NaN, '1']), y0: range(-100, 0), x1: range(-50, 200), y1: range(-50, 200) } : null;
  const bounds = chance(0.9) ? { x0: range(-100, 0), y0: range(-100, 0), x1: range(0, 300), y1: pick([range(0, 300), Infinity]) } : null;
  const ax = pick([range(-50, 50), '3', NaN]), ay = range(-50, 50);
  helpers.push({ op: 'fontAnchor', ref: enc(ref), bounds: enc(bounds), ax: enc(ax), ay: enc(ay), out: run(() => fontAnchorOffset(ref, bounds, ax as any, ay)) });
}

writeFileSync(OUT, gzipSync(JSON.stringify({ docs, texts, helpers }), { level: 9 }));
const total = docs.reduce((n, d) => n + d.cases.length, 0) + texts.reduce((n, d) => n + d.cases.length, 0) + helpers.length;
console.log(`wrote ${OUT}: ${docs.length} documents, ${texts.length} text documents, ${total} cases`);
