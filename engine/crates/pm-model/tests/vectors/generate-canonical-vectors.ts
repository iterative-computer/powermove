/**
 * Generates tests/vectors/canonical-vectors.json from the TypeScript sources the
 * engine must agree with:
 *
 * - apps/desktop/src/shared/canonical-json.ts: canonical serialization, hash,
 *   canonical project form (pm-model's json::write, hash, canonical);
 * - apps/desktop/src/renderer/src/legacy/core/compositions.ts: rotateProject,
 *   canonicalizePatches, realizePatches, normalizeCompositions;
 * - apps/desktop/src/shared/patch.ts: applyPatch and diffPatches, to record
 *   edit sessions the way history.ts does.
 *
 *   bun engine/crates/pm-model/tests/vectors/generate-canonical-vectors.ts
 *
 * Sessions replay the shadow-replay auditor (docs/rust-engine.md, section 6,
 * item 5): every edit is recorded as `diffPatches(before, after)` on the stored
 * project, canonicalized with the open composition's id, and the canonical form
 * of the result is recorded with its hash. Opening another composition is not
 * an undoable edit and its raw patches are not canonical; it is recorded as the
 * diff of the canonical forms before and after. Each step records the hash of
 * the canonical form; the Rust test also checks its own realizePatches port by
 * replaying the realized patches on the stored project.
 */
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { canonicalHash, canonicalProject, canonicalStringify, realizeProject } from '../../../../../apps/desktop/src/shared/canonical-json';
import { applyPatch, diffPatches, type Patch } from '../../../../../apps/desktop/src/shared/patch';
import { COMP_FIELDS, canonicalizePatches, normalizeCompositions, realizePatches, rotateProject } from '../../../../../apps/desktop/src/renderer/src/legacy/core/compositions';

const here = import.meta.dir;
const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value));
const text = (value: unknown) => canonicalStringify(value) ?? null;

// ── Serialization vectors ────────────────────────────────────────────────────
const tricky = [
  '{"b":1,"a":2,"10":3,"2":4,"":5,"A":6,"é":7,"😀":8,"￿":9}',
  '[0,-0,1,-1,0.1,1e21,1e-7,1.5e-7,123456789012345680000,5e-324,1.7976931348623157e308,0.30000000000000004,656607186168349.2,96197621992425.12,-635542944025049.2,9007199254740993,1e400]',
  '"\\u0000\\u0001\\u001f \\b\\f\\n\\r\\t \\" \\\\ / \\u007f \\u2028\\u2029 \\ud800 \\udfff \\ud83d\\ude00 x\\udc00\\ud800y"',
  '{"__proto__":{"x":1},"constructor":2,"toString":3}',
  '{"nested":{"z":[{"b":1,"a":[]},{}],"y":null,"x":true},"w":false}',
  '[[],{},"",[[[]]]]',
  '{"\\ud800":1,"\\udc00":2,"\\ue000":3,"\\ud83d\\ude00":4}',
];
const serialization = tricky.map(source => {
  const value = JSON.parse(source);
  const canonical = canonicalStringify(value)!;
  return { source, canonical, hash: canonicalHash(value) };
});

// ── Projects ─────────────────────────────────────────────────────────────────
const corpusDir = path.join(here, '..', 'fixtures', 'corpus');
const corpus = readdirSync(corpusDir).filter(name => name.endsWith('.json')).sort()
  .map(name => ({ name, project: JSON.parse(readFileSync(path.join(corpusDir, name), 'utf8')) }));

const layer = (id: string, extra: Record<string, unknown> = {}) => ({
  id, type: 'solid', name: id.toUpperCase(), from: 0, dur: 5, on: true, lock: false, shy: false, collapsed: false,
  color: '#fff', blend: 'normal', mblur: false, parent: null, fx: [], masks: [], locked_intent: {},
  p: { opacity: { v: 100, kf: [], expr: null }, 'position.x': { v: 0, kf: [], expr: null } },
  d: { color: '#ff0000', w: 100, h: 100, radius: 0 }, ...extra,
});
const synthetic = {
  id: 'project', name: 'Synthetic', revision: 3, created: 1, assets: { img: { id: 'img', name: 'a.png', kind: 'image' } },
  params: {}, edits: [], notes: 'n', exportDefaults: { format: 'mp4', scale: 1, fps: 30, range: 'work', quality: 'high', mblur: false, audio: true, alpha: false },
  compId: 'C1', compName: 'Main', w: 1920, h: 1080, fps: 29.97, dur: 10, bg: '#000000',
  backgroundFill: { type: 'solid', angle: 0, stops: [] }, layers: [layer('a'), layer('b'), layer('c')], markers: [{ id: 'm', t: 1, name: 'M' }],
  work: [0, 10], shutter: 180, audioGain: 0.5, render3d: { enabled: true, engine: 'eevee', samples: 64, previewSamples: 16, previewScale: 0.5, denoise: true, device: 'auto' },
  comps: {
    C2: { id: 'C2', name: 'Second', w: 640, h: 360, fps: 24, dur: 4, bg: '#111111', backgroundFill: { type: 'solid', angle: 0, stops: [] }, layers: [layer('x')], markers: [], work: [0, 4], comps: {}, assets: {}, params: {}, revision: 0, edits: [], created: 2 },
    C3: { id: 'C3', name: '', w: 100, h: 100, fps: 60, dur: 2, layers: [], markers: [], work: [1] },
  },
};
const legacyUnnormalized = {
  id: 'old', name: 'Old project', w: 640, h: 480, fps: 25, dur: 3, bg: '#000', layers: [layer('o')], markers: [], work: [0, 3], assets: {},
  comps: { N1: { name: 'Nested', w: 10, h: 10, fps: 25, dur: 1, layers: [], comps: { N2: { name: 'Deeper', layers: [] } } }, bad: 5 },
};

let uidCounter = 0;
const uid = (prefix: string) => `${prefix}-test-${++uidCounter}`;

/** Hashes keep the file small; the Rust test prints its own text on a mismatch. */
const hashOf = (value: unknown) => (canonicalStringify(value) === undefined ? null : canonicalHash(value));
const inline = new Set(['synthetic', 'legacy-unnormalized']);
const projects = [{ name: 'synthetic', project: synthetic }, { name: 'legacy-unnormalized', project: legacyUnnormalized }, ...corpus];
const projectVectors = projects.map(({ name, project }) => {
  const canonical = canonicalProject(clone(project));
  const rotations: Record<string, string | null> = {};
  for (const id of Object.keys(project.comps || {}).sort()) rotations[id] = hashOf(rotateProject(clone(project), id));
  uidCounter = 0;
  const normalized = normalizeCompositions(clone(project), uid);
  return {
    name,
    // Corpus projects are read from tests/fixtures/corpus/<name> by the Rust test.
    project: inline.has(name) ? JSON.stringify(project) : null,
    canonical: inline.has(name) ? text(canonical) : null,
    hash: canonicalHash(canonical), realized: hashOf(realizeProject(canonical)), rotations, normalized: hashOf(normalized),
  };
});

// ── Auditor sessions ─────────────────────────────────────────────────────────
let seed = 0xc0ffee;
const rand = () => { seed |= 0; seed = (seed + 0x6d2b79f5) | 0; let t = Math.imul(seed ^ (seed >>> 15), 1 | seed); t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t; return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
const pick = <T>(items: readonly T[]): T => items[Math.floor(rand() * items.length)]!;

type Step = { label: string; patches: Patch[]; hash: string };

function edit(project: any): { label: string; next: any; rotation: boolean } {
  const next = clone(project);
  const comps = Object.keys(next.comps || {});
  const r = rand();
  if (r < 0.12 && comps.length) return { label: 'open composition', next: rotateProject(next, pick(comps)), rotation: true };
  const layers: any[] = r < 0.25 && comps.length && Array.isArray(next.comps[comps[0]!].layers) ? next.comps[comps[0]!].layers : next.layers;
  if (r < 0.4 && layers.length > 1) { layers.push(layers.shift()); return { label: 'reorder layers', next, rotation: false }; }
  if (r < 0.5) { layers.splice(Math.floor(rand() * (layers.length + 1)), 0, layer('n' + Math.floor(rand() * 1e6))); return { label: 'add layer', next, rotation: false }; }
  if (r < 0.58 && layers.length) { layers.splice(Math.floor(rand() * layers.length), 1); return { label: 'delete layer', next, rotation: false }; }
  if (r < 0.66) { next.fps = pick([24, 25, 29.97, 30000 / 1001, 60]); next.dur = pick([5, 10, 12.5]); return { label: 'composition settings', next, rotation: false }; }
  if (r < 0.72) { if (next.audioGain === undefined) next.audioGain = 2; else delete next.audioGain; return { label: 'optional field', next, rotation: false }; }
  if (r < 0.78) { next.name = 'Renamed ' + Math.floor(rand() * 100); next.revision = (next.revision || 0) + 1; return { label: 'project fields', next, rotation: false }; }
  if (r < 0.84) { next.compName = 'Open ' + Math.floor(rand() * 100); return { label: 'rename open composition', next, rotation: false }; }
  if (r < 0.88) { next.comps = { ...next.comps, ['K' + Math.floor(rand() * 1000)]: { id: 'K', name: 'New comp', w: 10, h: 10, fps: 30, dur: 1, layers: [], markers: [], work: [0, 1] } }; return { label: 'new composition', next, rotation: false }; }
  const target = layers.length ? pick(layers) : null;
  if (target) {
    target.p ??= {};
    const channel = target.p.opacity ??= { v: 100, kf: [], expr: null };
    if (rand() < 0.5) channel.v = Math.round(rand() * 100);
    else channel.kf = [...(channel.kf || []), { t: Math.round(rand() * 300) / 30, v: Math.round(rand() * 100), i: 'linear' }];
  }
  return { label: 'property edit', next, rotation: false };
}

const sessions = projects.filter(p => typeof p.project.compId === 'string').slice(0, 12).map(({ name, project }) => {
  let current = clone(project);
  const steps: Step[] = [];
  for (let i = 0; i < 20; i++) {
    const { label, next, rotation } = edit(current);
    let patches: Patch[];
    if (rotation) patches = diffPatches(canonicalProject(clone(current)), canonicalProject(clone(next))).forward;
    else {
      const { forward, backward } = diffPatches(current, next);
      patches = canonicalizePatches(forward, backward, current.compId).forward;
    }
    // The renderer realizes canonical patches onto the stored project it holds.
    const realized = rotation ? [] : realizePatches(clone(patches), current);
    const replayed = rotation ? next : applyPatch(clone(current), clone(realized));
    if (canonicalStringify(replayed) !== canonicalStringify(next)) throw new Error(`${name}: realizePatches does not reproduce ${label}`);
    current = next;
    const canonical = canonicalProject(clone(current));
    steps.push({ label, patches: clone(patches), hash: canonicalHash(canonical) });
  }
  return { name, project: inline.has(name) ? JSON.stringify(project) : null, steps };
});

const out = {
  generatedBy: 'engine/crates/pm-model/tests/vectors/generate-canonical-vectors.ts',
  compFields: COMP_FIELDS, serialization, projects: projectVectors, sessions,
};
writeFileSync(path.join(here, 'canonical-vectors.json'), JSON.stringify(out) + '\n');
console.log(`wrote ${serialization.length} serialization vectors, ${projectVectors.length} projects, ${sessions.length} sessions of ${sessions[0]?.steps.length} steps`);
