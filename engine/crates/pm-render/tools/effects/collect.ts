/*
 * The shader parity corpus for pm-render's effects (plan section 11 and
 * section 25's shader gate): the 16 built-in effects and 4 built-in
 * transitions at several parameter sets, times, crops, and E-encoded
 * inputs above 1.0; spike S3's generated agent-style effects and
 * transitions (every row of the section 11 contract); and the 394-probe
 * coverage sweep. Each case carries
 *
 * - the definition snapshot the engine receives (built by the kernel's own
 *   `kernel/definition-snapshots.ts` from the real preambles), and the
 *   parameter values per variant as a document holds them (the engine
 *   converts them itself), and
 * - the exact uniform calls and texture units the legacy compositor issues
 *   per pass (`runEffects`, `runTransition`, `setParam`, `setU`, `setI`),
 *   which `webgl.ts` replays in WebGL2.
 *
 * Usage: bun engine/crates/pm-render/tools/effects/collect.ts [--out <dir>]
 * Default output: ~/Library/Application Support/Powermove Engine Corpus/glsl-<UTC stamp>/
 * (corpus.json, images/*.f16). Nothing is written inside the repository.
 */
import { mkdir, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';

import { HEIGHT, WIDTH, makeImages } from './images';
import { BUILTIN_EFFECTS, BUILTIN_TRANSITION_DEFS, PREAMBLES, REPO, VERTEX, snapshots } from './preambles';
import { probes, program } from './probes';

const { GENERATED } = await import(join(REPO, 'engine', 'spikes', 's3-shaders', 'corpus', 'generated', 'index.ts'));

const args = process.argv.slice(2);
const stamp = new Date().toISOString().replace(/[-:]/g, '').replace(/\..*/, '').replace('T', '-');
const OUT = args.includes('--out')
  ? resolve(args[args.indexOf('--out') + 1]!)
  : join(homedir(), 'Library', 'Application Support', 'Powermove Engine Corpus', `glsl-${stamp}`);
if (resolve(OUT).startsWith(resolve(REPO) + '/')) throw new Error('the shader corpus is never written inside the repository');

/* legacy/core/util.ts PM.hex2rgb, verbatim (its install needs a DOM). */
const hexDigits = (hex: any): string => {
  const s = String(hex ?? '').replace('#', '');
  return s.length === 3 || s.length === 4 ? s.split('').map((c) => c + c).join('') : s;
};
const hex2rgb = (hex: any) => {
  const s = hexDigits(hex);
  return [0, 2, 4].map((i) => (parseInt(s.slice(i, i + 2), 16) || 0) / 255);
};

interface Call { fn: '1f' | '2f' | '3f' | '4f' | '1i' | 'm3'; name: string; alt?: string; v: number[] }
interface Unit { unit: number; src: string }
interface Pass { calls: Call[]; units: Unit[] }
interface Variant {
  name: string; width: number; height: number; time: number;
  /** Per definition parameter, as a document holds it; null: the default. */
  params: unknown[];
  input?: string; backdrop?: string | null; region?: [number, number] | null; full?: [number, number];
  from?: string; to?: string; progress?: number;
  passes: Pass[];
}
interface Case {
  id: string; group: string; kind: 'effect' | 'transition'; label: string;
  definition: any; frag: string; lineOffset: number; covers?: string[]; variants: Variant[];
}

/* compositor.ts setU / setI */
function setU(name: string, v: number[], alt?: string): Call {
  if (v.length === 9) return { fn: 'm3', name, v };
  const fn = (['1f', '2f', '3f', '4f'] as const)[v.length - 1]!;
  return alt ? { fn, name, alt, v } : { fn, name, v };
}
const setI = (name: string, v: number): Call => ({ fn: '1i', name, v: [v] });
const fullQuad = (w: number, h: number) => [w, 0, 0, 0, h, 0, 0, 0, 1];

/* compositor.ts setParam */
function paramCall(param: any, index: number, value: any): Call {
  const name = 'u_' + param.k;
  const alt = (param.type === 'color' ? 'u_c' : 'u_p') + index;
  if (param.type === 'color') return setU(name, hex2rgb(String(value)), alt);
  if (param.type === 'toggle') return setU(name, [value ? 1 : 0], alt);
  return setU(name, [Number(value) || 0], alt);
}

/* ── parameter sets ────────────────────────────────────────────────── */

const ALT_COLORS = ['#3FA7D6', '#F2545B', '#7DCE82', '#FFD23F', '#8E6CFF', '#1B998B'];
function rng(seed: number) {
  return () => {
    seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const hex = (r: () => number) => '#' + [0, 0, 0].map(() => Math.floor(r() * 256).toString(16).padStart(2, '0')).join('').toUpperCase();

type Set = { name: string; time: number; value: (param: any, index: number) => unknown };
const range = (p: any): [number, number] | null => {
  const min = Number(p.min), max = Number(p.max);
  return Number.isFinite(min) && Number.isFinite(max) && max > min ? [min, max] : null;
};
const SETS: Set[] = [
  { name: 'defaults', time: 1.25, value: () => null },
  { name: 'alt', time: 3.7, value: (p, i) => p.type === 'color' ? ALT_COLORS[i % ALT_COLORS.length] : p.type === 'toggle' ? !p.def : (range(p) ? range(p)![0] + 0.7 * (range(p)![1] - range(p)![0]) : Number(p.def) * 1.5 + 1) },
  { name: 'min', time: 0, value: (p) => p.type === 'color' ? '#000000' : p.type === 'toggle' ? false : range(p)?.[0] ?? 0 },
  { name: 'max', time: 7.31, value: (p) => p.type === 'color' ? '#FFFFFF' : p.type === 'toggle' ? true : range(p)?.[1] ?? 1 },
  ...[11, 29].map((seed, k): Set => ({
    name: `random${k + 1}`, time: 0.5 + 9 * rng(seed)(),
    value: (p, i) => {
      // Seeded by the parameter itself, so every definition draws its own values.
      let h = seed * 31 + i;
      for (const c of `${p.k}:${p.min}:${p.max}`) h = (Math.imul(h, 31) + c.charCodeAt(0)) | 0;
      const r = rng(h);
      return p.type === 'color' ? hex(r) : p.type === 'toggle' ? r() > 0.5 : range(p) ? range(p)![0] + r() * (range(p)![1] - range(p)![0]) : Number(p.def) * (0.5 + r());
    },
  })),
  /* setParam's conversions: numbers as strings, toggles as numbers, short colors. */
  { name: 'coerced', time: 2.0, value: (p, i) => p.type === 'color' ? ['#F80', '#3fa7d6', 'C0FFEE', '#1234'][i % 4] : p.type === 'toggle' ? 1 : String(range(p) ? range(p)![0] + 0.3 * (range(p)![1] - range(p)![0]) : p.def) },
];

/* ── effects ───────────────────────────────────────────────────────── */

/* compositor.ts runEffects */
function effectPasses(def: any, legacy: { passes: number; keepOrig: boolean; backdrop: boolean }, values: unknown[], v: { time: number; input: string; backdrop: string | null; region: [number, number] | null; full: [number, number] }): Pass[] {
  const W = WIDTH, H = HEIGHT;
  const resolution = v.full;
  const origin = v.region ?? [0, 0];
  const uv = [origin[0] / resolution[0], origin[1] / resolution[1], W / resolution[0], H / resolution[1]];
  const passes: Pass[] = [];
  for (let pass = 0; pass < legacy.passes; pass++) {
    const calls: Call[] = [setI('u_tex', 0)];
    const units: Unit[] = [{ unit: 0, src: pass === 0 ? v.input : `pass:${pass - 1}` }];
    if (legacy.keepOrig) { calls.push(setI('u_orig', 1)); units.push({ unit: 1, src: v.input }); }
    if (legacy.backdrop) { calls.push(setI('u_backdrop', 2)); if (v.backdrop) units.push({ unit: 2, src: v.backdrop }); }
    calls.push(setU('u_m', fullQuad(W, H)), setU('u_res', [W, H]), setU('u_uv', uv), setU('u_coordRes', [...resolution]), setU('u_pxOrigin', [...origin]));
    calls.push(setU('u_texel', [1 / W, 1 / H]), setU('u_time', [v.time]), setI('u_pass', pass));
    // effectParamValue: no value → the definition's (raw) default.
    def.params.forEach((param: any, index: number) => calls.push(paramCall(param, index, values[index] ?? param.def)));
    passes.push({ calls, units });
  }
  return passes;
}

function effectCase(id: string, group: string, def: any, origin: string, extra: Partial<Case> = {}, sets: Set[] = SETS): Case {
  const snapshot = snapshots.effectSnapshot(def, origin, PREAMBLES);
  const legacy = { passes: snapshot.flags.passes, keepOrig: snapshot.flags.keepOrig, backdrop: snapshot.flags.backdrop };
  const variants: Variant[] = [];
  const full: [number, number] = [WIDTH, HEIGHT];
  const add = (name: string, set: Set, input: string, backdrop: string | null, region: [number, number] | null, fullSize: [number, number]) => {
    const values = def.params.map((p: any, i: number) => set.value(p, i));
    variants.push({ name, width: WIDTH, height: HEIGHT, time: set.time, params: values, input, backdrop, region, full: fullSize, passes: effectPasses(def, legacy, values, { time: set.time, input, backdrop, region, full: fullSize }) });
  };
  for (const set of sets) add(set.name, set, 'layer', 'backdrop', null, full);
  // A quarter of a 2× composition (`u_coordRes`, `u_pxOrigin`, `u_uv`), for viewport-safe effects.
  if (snapshot.flags.viewportSafe) {
    add('viewport', sets[0]!, 'layer', 'backdrop', [WIDTH / 2, HEIGHT / 2], [WIDTH * 2, HEIGHT * 2]);
    add('viewport-alt', sets[1] ?? sets[0]!, 'layer', 'backdrop', [WIDTH / 2, HEIGHT / 2], [WIDTH * 2, HEIGHT * 2]);
  }
  // E-encoded values above 1.0 in the layer and the backdrop.
  add('hdr', sets[0]!, 'layer-hdr', 'backdrop-hdr', null, full);
  add('hdr-alt', sets[1] ?? sets[0]!, 'layer-hdr', 'backdrop-hdr', null, full);
  return { id, group, kind: 'effect', label: def.label, definition: snapshot, frag: snapshot.glsl.source, lineOffset: snapshot.glsl.bodyLineOffset, variants, ...extra };
}

/* ── transitions ───────────────────────────────────────────────────── */

const PROGRESS = [0, 0.37, 0.81, 1];
function transitionCase(id: string, group: string, def: any, origin: string, extra: Partial<Case> = {}): Case {
  const snapshot = snapshots.transitionSnapshot(def, origin, PREAMBLES);
  const W = WIDTH, H = HEIGHT;
  const variants: Variant[] = [];
  const add = (name: string, set: Set, progress: number, from: string, to: string) => {
    const values = def.params.map((p: any, i: number) => set.value(p, i));
    const calls: Call[] = [setI('u_from', 0), setI('u_to', 1), setU('u_m', fullQuad(W, H)), setU('u_res', [W, H]), setU('u_uv', [0, 0, 1, 1])];
    calls.push(setU('u_texel', [1 / W, 1 / H]), setU('u_time', [set.time]), setU('u_prog', [progress]));
    // `prop ? PM.evP(…) : pd.def`
    def.params.forEach((param: any, index: number) => calls.push(paramCall(param, index, values[index] ?? param.def)));
    variants.push({ name, width: W, height: H, time: set.time, params: values, from, to, progress, passes: [{ calls, units: [{ unit: 0, src: from }, { unit: 1, src: to }] }] });
  };
  for (const set of SETS.slice(0, 2).concat(SETS.slice(4, 5))) {
    for (const p of PROGRESS) add(`${set.name}-p${p}`, set, p, 'from', 'to');
  }
  add('hdr', SETS[0]!, 0.5, 'layer-hdr', 'backdrop-hdr');
  return { id, group, kind: 'transition', label: def.label, definition: snapshot, frag: snapshot.glsl.source, lineOffset: snapshot.glsl.bodyLineOffset, variants, ...extra };
}

/* ── the corpus ────────────────────────────────────────────────────── */

const cases: Case[] = [];
for (const def of BUILTIN_EFFECTS) cases.push(effectCase(`builtin/effect/${def.id}`, 'builtin-effects', def, 'builtIn'));
for (const def of BUILTIN_TRANSITION_DEFS) cases.push(transitionCase(`builtin/transition/${def.id}`, 'builtin-transitions', def, 'builtIn'));

let generated = 0;
for (const g of GENERATED as any[]) {
  if (g.kind === 'effect') {
    cases.push(effectCase(`generated/${g.name}`, 'generated', g.def, 'local', { covers: g.covers }, SETS.slice(0, 2)));
    generated++;
  } else if (g.kind === 'transition') {
    cases.push(transitionCase(`generated/${g.name}`, 'generated', g.def, 'local', { covers: g.covers }));
    generated++;
  }
}

/* The coverage sweep: one-pass raw effects over `layer`. */
probes.forEach((probe, index) => {
  const def = { id: `probe${index}`, label: `${probe.group}/${probe.name}`, group: 'Probes', params: [], frag: program(probe), rawShader: true };
  const snapshot = snapshots.effectSnapshot(def, 'local', PREAMBLES);
  const full: [number, number] = [WIDTH, HEIGHT];
  cases.push({
    id: `probe/${probe.group}/${probe.name}`, group: 'probes', kind: 'effect', label: def.label, definition: snapshot,
    frag: snapshot.glsl.source, lineOffset: 0,
    variants: [{ name: 'defaults', width: WIDTH, height: HEIGHT, time: 1.25, params: [], input: 'layer', backdrop: null, region: null, full,
      passes: effectPasses(def, { passes: 1, keepOrig: false, backdrop: false }, [], { time: 1.25, input: 'layer', backdrop: null, region: null, full }) }],
  });
});

const ids = new Set<string>();
for (const c of cases) {
  if (ids.has(c.id)) throw new Error(`duplicate case ${c.id}`);
  ids.add(c.id);
}

await mkdir(join(OUT, 'images'), { recursive: true });
const images = makeImages();
for (const image of images) await writeFile(join(OUT, image.file), image.bytes);
await writeFile(join(OUT, 'corpus.json'), JSON.stringify({
  format: 'powermove-shader-corpus', version: 1, created: new Date().toISOString(),
  vertex: VERTEX, images: images.map(({ bytes: _, ...rest }) => rest), cases,
}));
const variants = cases.reduce((n, c) => n + c.variants.length, 0);
console.log(`${cases.length} cases (${BUILTIN_EFFECTS.length} built-in effects, ${BUILTIN_TRANSITION_DEFS.length} transitions, ${generated} generated, ${probes.length} probes), ${variants} variants → ${OUT}`);
