/*
 * The shader gate corpus (plan section 11, "No shader is dropped"; section
 * 25; amendment 9): every GLSL source in the repository, the Store, the docs
 * samples, and the generated agent corpus, with
 *
 * - what the engine receives: a shader layer's `d.code` and `d.uniforms`
 *   values, or the definition snapshot the kernel builds
 *   (`kernel/definition-snapshots.ts`) with each variant's parameter values
 *   as a document holds them, and
 * - the exact uniform calls and texture units the legacy compositor issues
 *   (`compositor.ts` `nativeContentQuad` for layers, `runEffects` and
 *   `runTransition` for effects and transitions), which
 *   `../effects/webgl.ts --corpus <dir>` replays in WebGL2.
 *
 * Sources (spike S3's collector, extended):
 *   shader-presets   PM.SHADER_PRESETS and PM.SHADER_TEMPLATE
 *   builtin-layers   Studio Cube, the missing-layer checkerboard
 *   docs             apps/desktop/docs/samples, EXTENSIONS.md code blocks
 *   cloud-samples    apps/cloud/samples
 *   e2e              e2e fixtures, register(...) literals and shader-layer code in e2e specs
 *   unit-tests       GLSL literals in kernel, legacy, and panel unit tests
 *   scenes           onboarding and www hero scene.json
 *   store            every public Store release (`fetch-store.ts`; activated in a sandbox)
 *   generated-s3     spike S3's agent-style corpus
 *   generated-agent  `generated/*.json` (`generate.ts`)
 *   local            --local <project.pmv | extensions dir> (private: ids are hashes, no source text in reports)
 *
 * Built-in effects and transitions are the effects lane's corpus
 * (`../effects/collect.ts`) and are not repeated here.
 *
 * Usage: bun engine/crates/pm-render/tools/shaders/collect.ts [--store <dir>] [--no-store] [--local <path>]... [--out <dir>]
 * Default output: ~/Library/Application Support/Powermove Engine Corpus/shaders-<UTC stamp>/
 */
import { createHash } from 'node:crypto';
import { existsSync } from 'node:fs';
import { mkdir, readdir, readFile, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, relative, resolve, sep } from 'node:path';
import vm from 'node:vm';

import { activateModule, type Registered } from './activate';
import { PREAMBLES, REPO, VERTEX, snapshots } from '../effects/preambles';
import { HEIGHT, WIDTH, makeImages } from '../effects/images';
import { BUILTIN_LAYERS, SHADER_HEADER, SHADER_PRESETS, SHADER_TEMPLATE, extensionLayers, missingLayerBody, parseUniforms } from './sources';

const APP = join(REPO, 'apps', 'desktop');
const RENDERER = join(APP, 'src', 'renderer', 'src');
const CORPUS_ROOT = join(homedir(), 'Library', 'Application Support', 'Powermove Engine Corpus');
const args = process.argv.slice(2);
const flag = (name: string) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined);
const stamp = new Date().toISOString().replace(/[-:]/g, '').replace(/\..*/, '').replace('T', '-');
const OUT = resolve(flag('--out') ?? join(CORPUS_ROOT, `shaders-${stamp}`));
if (OUT.startsWith(resolve(REPO) + sep)) throw new Error('the shader corpus is never written inside the repository');
const withStore = !args.includes('--no-store');
const storeDir = flag('--store') ?? (existsSync(CORPUS_ROOT) ? (await readdir(CORPUS_ROOT)).filter((d) => d.startsWith('store-')).sort().map((d) => join(CORPUS_ROOT, d)).at(-1) : undefined);
const localPaths = args.flatMap((arg, i, all) => (arg === '--local' && all[i + 1] ? [all[i + 1]!] : []));
const { GENERATED } = await import(join(REPO, 'engine', 'spikes', 's3-shaders', 'corpus', 'generated', 'index.ts'));
const { decodeProjectContainer } = await import(join(APP, 'src/shared/project-container.ts'));

/* legacy/core/util.ts PM.hex2rgb, verbatim (its install needs a DOM). */
const hexDigits = (hex: any): string => {
  const s = String(hex ?? '').replace('#', '');
  return s.length === 3 || s.length === 4 ? s.split('').map((c) => c + c).join('') : s;
};
const hex2rgb = (hex: any) => {
  const s = hexDigits(hex);
  return [0, 2, 4].map((i) => (parseInt(s.slice(i, i + 2), 16) || 0) / 255);
};

/* ── corpus types (read by tests/shader_gate.rs and ../effects/webgl.ts) ── */
interface Call { fn: '1f' | '2f' | '3f' | '4f' | '1i' | 'm3'; name: string; alt?: string; v: number[] }
interface Unit { unit: number; src: string }
interface Pass { calls: Call[]; units: Unit[] }
interface Variant {
  name: string; width: number; height: number; time: number;
  /** Layers: the layer's timing and the open composition's rate. */
  from?: number; dur?: number; fps?: number;
  /** Shader layers: `d.uniforms` values by name (a document's plain values). */
  uniforms?: Record<string, unknown>;
  /** Extension layers, effects, transitions: values in definition order (null: the default). */
  params?: unknown[];
  input?: string; backdrop?: string | null; region?: [number, number] | null; full?: [number, number];
  from_?: string; to?: string; progress?: number;
  passes: Pass[];
}
interface Case {
  id: string; group: string; kind: 'shader' | 'extension' | 'effect' | 'transition'; label: string; source: string;
  frag: string; lineOffset: number; code?: string; definition?: any; covers?: string[]; private?: boolean;
  provenance?: unknown; variants: Variant[];
}

function setU(name: string, v: number[], alt?: string): Call {
  if (v.length === 9) return { fn: 'm3', name, v };
  const fn = (['1f', '2f', '3f', '4f'] as const)[v.length - 1]!;
  return alt ? { fn, name, alt, v } : { fn, name, v };
}
const setI = (name: string, v: number): Call => ({ fn: '1i', name, v: [v] });
const fullQuad = (w: number, h: number) => [w, 0, 0, 0, h, 0, 0, 0, 1];
const clamp = (v: number, a: number, b: number) => (v < a ? a : v > b ? b : v);

/* ── variant values ────────────────────────────────────────────────── */
const ALT_COLORS = ['#3FA7D6', '#F2545B', '#7DCE82', '#FFD23F', '#8E6CFF', '#1B998B'];
type Param = { k: string; type?: string; def: any; min?: number; max?: number };
type SetName = 'defaults' | 'alt' | 'coerced';
function value(param: Param, index: number, set: SetName): any {
  const min = Number(param.min), max = Number(param.max);
  const ranged = Number.isFinite(min) && Number.isFinite(max) && max > min;
  if (set === 'defaults') return null;
  if (set === 'alt') {
    if (param.type === 'color') return ALT_COLORS[index % ALT_COLORS.length];
    if (param.type === 'toggle') return !param.def;
    return ranged ? min + 0.7 * (max - min) : Number(param.def) * 1.5 + 1;
  }
  if (param.type === 'color') return ['#F80', '#3fa7d6', 'C0FFEE', '#1234'][index % 4];
  if (param.type === 'toggle') return param.def ? 0 : 1;
  return String(ranged ? min + 0.3 * (max - min) : param.def);
}

/* ── layers (compositor.ts nativeContentQuad) ──────────────────────── */
const LAYER_VARIANTS = [
  { name: 'defaults' as const, time: 1.25, fps: 30 },
  { name: 'alt' as const, time: 3.7, fps: 30 },
  { name: 'coerced' as const, time: 2.0, fps: 24 },
];
const FROM = 0.25, DUR = 4;
function layerBaseCalls(W: number, H: number, T: number, fps: number): Call[] {
  return [
    setU('u_m', fullQuad(W, H)), setU('u_res', [W, H]), setU('u_uv', [0, 0, 1, 1]), setU('iResolution', [W, H]),
    setU('iTime', [T - FROM]), setU('iGlobalTime', [T]), setU('iProgress', [clamp((T - FROM) / Math.max(DUR, 1e-4), 0, 1)]),
    setI('iFrame', Math.round(T * fps)), setU('iMouse', [0, 0]),
  ];
}

/** A shader layer's `d.uniforms` as the Inspector creates it (every `@param` uniform, its default). */
function shaderUniformValues(code: string, set: SetName): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  parseUniforms(code).forEach((def: any, index: number) => {
    if (def.name in out) return;
    const param: Param = { k: def.name, type: def.control === 'color' ? 'color' : def.control === 'toggle' ? 'toggle' : 'number', def: def.def, min: def.min, max: def.max };
    out[def.name] = value(param, index, set) ?? def.def;
  });
  return out;
}

function shaderCase(group: string, id: string, code: string, label: string, source: string, extra: Partial<Case> = {}): Case {
  const frag = SHADER_HEADER + '\n' + code;
  const udefs: any[] = parseUniforms(code || '');
  const variants = LAYER_VARIANTS.map((v): Variant => {
    const uniforms = shaderUniformValues(code, v.name);
    const calls = layerBaseCalls(WIDTH, HEIGHT, v.time, v.fps);
    for (const un in uniforms) {
      const def = udefs.find((u) => u.name === un);
      const val = uniforms[un];
      if (def && def.control === 'color') calls.push(setU(un, hex2rgb(String(val))));
      else if (def && def.control === 'toggle') calls.push(setI(un, val ? 1 : 0));
      else calls.push(setU(un, [Number(val) || 0]));
    }
    return { name: v.name, width: WIDTH, height: HEIGHT, time: v.time, from: FROM, dur: DUR, fps: v.fps, uniforms, passes: [{ calls, units: [] }] };
  });
  return { id: `${group}/${id}`, group, kind: 'shader', label, source, frag, lineOffset: SHADER_HEADER.split('\n').length, code, variants, ...extra };
}

function extensionCase(group: string, id: string, def: any, source: string, extra: Partial<Case> = {}): Case | null {
  extensionLayers.validateExtensionLayerDefinition(def);
  if (def.renderer?.kind !== 'fragment') return null;
  const snapshot = snapshots.layerSnapshot(def, extra.group === 'store' ? 'store' : 'local', PREAMBLES);
  const variants = LAYER_VARIANTS.map((v): Variant => {
    const params = def.params.map((p: Param, i: number) => value(p, i, v.name));
    const calls = layerBaseCalls(WIDTH, HEIGHT, v.time, v.fps);
    def.params.forEach((param: Param, index: number) => {
      const val = params[index] ?? param.def;
      if (param.type === 'color') calls.push(setU('u_' + param.k, hex2rgb(String(val))));
      else if (param.type === 'toggle') calls.push(setI('u_' + param.k, val ? 1 : 0));
      else calls.push(setU('u_' + param.k, [Number(val) || 0]));
    });
    return { name: v.name, width: WIDTH, height: HEIGHT, time: v.time, from: FROM, dur: DUR, fps: v.fps, params, passes: [{ calls, units: [] }] };
  });
  return { id: `${group}/${id}`, group, kind: 'extension', label: def.label, source, definition: snapshot, frag: snapshot.glsl.source, lineOffset: snapshot.glsl.bodyLineOffset, variants, ...extra };
}

/* ── effects and transitions (compositor.ts runEffects, runTransition) ── */
function paramCall(param: Param, index: number, val: any): Call {
  const name = 'u_' + param.k;
  const alt = (param.type === 'color' ? 'u_c' : 'u_p') + index;
  if (param.type === 'color') return setU(name, hex2rgb(String(val)), alt);
  if (param.type === 'toggle') return setU(name, [val ? 1 : 0], alt);
  return setU(name, [Number(val) || 0], alt);
}
function effectCase(group: string, id: string, def: any, source: string, extra: Partial<Case> = {}): Case {
  const snapshot = snapshots.effectSnapshot(def, extra.group === 'store' ? 'store' : 'local', PREAMBLES);
  const flags = snapshot.flags;
  const W = WIDTH, H = HEIGHT;
  const variants: Variant[] = [];
  const add = (name: string, set: SetName, time: number, input: string, backdrop: string, region: [number, number] | null, full: [number, number]) => {
    const params = def.params.map((p: Param, i: number) => value(p, i, set));
    const origin = region ?? [0, 0];
    const uv = [origin[0] / full[0], origin[1] / full[1], W / full[0], H / full[1]];
    const passes: Pass[] = [];
    for (let pass = 0; pass < flags.passes; pass++) {
      const calls: Call[] = [setI('u_tex', 0)];
      const units: Unit[] = [{ unit: 0, src: pass === 0 ? input : `pass:${pass - 1}` }];
      if (flags.keepOrig) { calls.push(setI('u_orig', 1)); units.push({ unit: 1, src: input }); }
      if (flags.backdrop) { calls.push(setI('u_backdrop', 2)); units.push({ unit: 2, src: backdrop }); }
      calls.push(setU('u_m', fullQuad(W, H)), setU('u_res', [W, H]), setU('u_uv', uv), setU('u_coordRes', [...full]), setU('u_pxOrigin', [...origin]));
      calls.push(setU('u_texel', [1 / W, 1 / H]), setU('u_time', [time]), setI('u_pass', pass));
      def.params.forEach((param: Param, index: number) => calls.push(paramCall(param, index, params[index] ?? param.def)));
      passes.push({ calls, units });
    }
    variants.push({ name, width: W, height: H, time, params, input, backdrop, region, full, passes });
  };
  add('defaults', 'defaults', 1.25, 'layer', 'backdrop', null, [W, H]);
  add('alt', 'alt', 3.7, 'layer', 'backdrop', null, [W, H]);
  add('coerced', 'coerced', 2.0, 'layer', 'backdrop', null, [W, H]);
  if (flags.viewportSafe) add('viewport', 'defaults', 1.25, 'layer', 'backdrop', [W / 2, H / 2], [W * 2, H * 2]);
  add('hdr', 'defaults', 1.25, 'layer-hdr', 'backdrop-hdr', null, [W, H]);
  return { id: `${group}/${id}`, group, kind: 'effect', label: def.label, source, definition: snapshot, frag: snapshot.glsl.source, lineOffset: snapshot.glsl.bodyLineOffset, variants, ...extra };
}
function transitionCase(group: string, id: string, def: any, source: string, extra: Partial<Case> = {}): Case {
  const snapshot = snapshots.transitionSnapshot(def, extra.group === 'store' ? 'store' : 'local', PREAMBLES);
  const W = WIDTH, H = HEIGHT;
  const variants: Variant[] = [];
  for (const [name, set, progress, time] of [['p0.37', 'defaults', 0.37, 1.25], ['p0.81-alt', 'alt', 0.81, 3.7], ['p0-coerced', 'coerced', 0, 2], ['p1', 'defaults', 1, 2.5]] as const) {
    const params = def.params.map((p: Param, i: number) => value(p, i, set));
    const calls: Call[] = [setI('u_from', 0), setI('u_to', 1), setU('u_m', fullQuad(W, H)), setU('u_res', [W, H]), setU('u_uv', [0, 0, 1, 1])];
    calls.push(setU('u_texel', [1 / W, 1 / H]), setU('u_time', [time]), setU('u_prog', [progress]));
    def.params.forEach((param: Param, index: number) => calls.push(paramCall(param, index, params[index] ?? param.def)));
    variants.push({ name, width: W, height: H, time, params, from_: 'from', to: 'to', progress, passes: [{ calls, units: [{ unit: 0, src: 'from' }, { unit: 1, src: 'to' }] }] });
  }
  return { id: `${group}/${id}`, group, kind: 'transition', label: def.label, source, definition: snapshot, frag: snapshot.glsl.source, lineOffset: snapshot.glsl.bodyLineOffset, variants, ...extra };
}

/* ── case bookkeeping ──────────────────────────────────────────────── */
const cases: Case[] = [];
const notes: string[] = [];
const seen = new Set<string>();
function push(c: Case | null) {
  if (!c) return;
  if (seen.has(c.id)) throw new Error(`duplicate case id ${c.id}`);
  seen.add(c.id);
  cases.push(c);
}
const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
function safely(what: string, f: () => Case | null) {
  try { push(f()); } catch (error) { notes.push(`${what}: not a renderable definition (${String(error).slice(0, 140)})`); }
}
function addRegistered(group: string, prefix: string, items: Registered[], source: string, extra: Partial<Case> = {}) {
  items.forEach(({ kind, def }, index) => {
    const id = extra.private ? `${prefix}/${index}` : `${prefix}/${slug(def.id)}`;
    const label = extra.private ? { label: 'Local extension' } : {};
    const e = { group, ...extra, ...label };
    if (kind === 'effect') safely(id, () => effectCase(group, id, def, source, e));
    else if (kind === 'transition') safely(id, () => transitionCase(group, id, def, source, e));
    else safely(id, () => extensionCase(group, id, def, source, e));
  });
}
async function activate(entry: string) {
  const a = await activateModule(entry);
  for (const n of a.notes) notes.push(`${relative(REPO, entry)}: ${n}`);
  return a.registered;
}

/* Spike S3's static readers for test sources. */
function registerCallsIn(text: string): Array<{ kind: Registered['kind']; literal: string; line: number }> {
  const out: Array<{ kind: Registered['kind']; literal: string; line: number }> = [];
  const re = /\b(?:(effects|transitions|layers)\.register|register(Effect|Transition|LayerType))\(\s*(?:'[^']*'\s*,\s*)?\{/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) {
    const start = m.index + m[0].length - 1;
    let depth = 0, i = start, quote: string | null = null;
    for (; i < text.length; i++) {
      const ch = text[i]!;
      if (ch === '\\') { i++; continue; }
      if (quote) { if (ch === quote) quote = null; continue; }
      if (ch === "'" || ch === '"' || ch === '`') { quote = ch; continue; }
      if (ch === '{') depth++;
      if (ch === '}' && --depth === 0) break;
    }
    const which = m[1] ?? m[2];
    const kind = which === 'effects' || which === 'Effect' ? 'effect' : which === 'transitions' || which === 'Transition' ? 'transition' : 'layer';
    out.push({ kind, literal: text.slice(start, i + 1), line: text.slice(0, m.index).split('\n').length });
  }
  return out;
}
const transpiler = new Bun.Transpiler({ loader: 'ts' });
const plain = (v: unknown) => JSON.parse(JSON.stringify(v));
function evaluateLiteral(source: string): any {
  try { return evaluateLiteralOnce(source); } catch { return evaluateLiteralOnce(source.replace(/\\`/g, '`').replace(/\\\$\{/g, '${')); }
}
function evaluateLiteralOnce(literal: string): any {
  const js = transpiler.transformSync(`globalThis.__s3 = (${literal});`).replace('globalThis.__s3', '__out.value');
  const target: any = { api: { id: 'ext' }, label: 'Test', __out: {} };
  const scope = new Proxy(target, { has: () => true, get: (t, p) => (p in t ? t[p] : p === Symbol.unscopables ? undefined : (globalThis as any)[p] ?? 'x') });
  vm.runInNewContext(`with (scope) { ${js} }`, { scope });
  return plain(target.__out.value);
}
function shaderLiterals(text: string): Array<{ key: string; value: string; line: number }> {
  const out: Array<{ key: string; value: string; line: number }> = [];
  const re = /\b(frag|fragment|code)\s*:\s*(['"`])/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) {
    const quote = m[2]!;
    let i = m.index + m[0].length, raw = '';
    for (; i < text.length; i++) {
      const ch = text[i]!;
      if (ch === '\\') { raw += ch + text[i + 1]; i++; continue; }
      if (ch === quote) break;
      raw += ch;
    }
    if (quote === '`' && raw.includes('${')) continue;
    out.push({ key: m[1]!, value: vm.runInNewContext(quote + raw + quote), line: text.slice(0, m.index).split('\n').length });
  }
  return out;
}

/* ── 1. shader-layer presets and the template ──────────────────────── */
for (const [name, code] of Object.entries(SHADER_PRESETS)) {
  push(shaderCase('shader-presets', slug(name), code, name, `apps/desktop/src/renderer/src/legacy/gl/shaders.ts (SHADER_PRESETS['${name}'])`));
}
if (!Object.values(SHADER_PRESETS).includes(SHADER_TEMPLATE)) push(shaderCase('shader-presets', 'template', SHADER_TEMPLATE, 'Template', 'legacy/gl/shaders.ts (SHADER_TEMPLATE)'));

/* ── 2. built-in layers ────────────────────────────────────────────── */
for (const def of BUILTIN_LAYERS) safely(def.id, () => extensionCase('builtin-layers', slug(def.id), def, `apps/desktop/src/extensions/layers-3d (${def.id})`, { group: 'builtin-layers' }));
push(shaderCase('builtin-layers', 'missing-placeholder', await missingLayerBody(), 'Missing extension placeholder', 'apps/desktop/src/renderer/src/legacy/gl/compositor.ts (missing)'));

/* ── 3. docs ───────────────────────────────────────────────────────── */
const scratch = join(OUT, 'work');
for (const dir of (await readdir(join(APP, 'docs/samples'))).sort()) {
  const entry = join(APP, 'docs/samples', dir, 'index.ts');
  if (existsSync(entry)) addRegistered('docs', dir, await activate(entry), relative(REPO, entry));
}
{
  const markdown = await readFile(join(APP, 'docs/EXTENSIONS.md'), 'utf8');
  let n = 0;
  for (const block of markdown.matchAll(/```ts\n([\s\S]*?)```/g)) {
    const body = block[1]!;
    if (!/\b(effects|transitions|layers)\.register\(/.test(body)) continue;
    const line = markdown.slice(0, block.index).split('\n').length;
    const file = join(scratch, 'docs-snippets', `extensions-md-${n++}.ts`);
    await mkdir(dirname(file), { recursive: true });
    await writeFile(file, `export default async function activate(api: any) {\n${body}\n}\n`);
    addRegistered('docs', `extensions-md-L${line}`, await activate(file), `apps/desktop/docs/EXTENSIONS.md:${line}`);
  }
}

/* ── 4. repository Store samples ───────────────────────────────────── */
for (const dir of (await readdir(join(REPO, 'apps/cloud/samples'))).sort()) {
  const entry = join(REPO, 'apps/cloud/samples', dir, 'index.ts');
  if (existsSync(entry)) addRegistered('cloud-samples', slug(dir), await activate(entry), relative(REPO, entry));
}

/* ── 5. e2e ────────────────────────────────────────────────────────── */
for (const file of ['e2e/fixtures/compat-typewriter/index.ts', 'test/fixtures/sandboxed-ext/index.ts']) {
  if (existsSync(join(APP, file))) addRegistered('e2e', slug(file), await activate(join(APP, file)), `apps/desktop/${file}`);
}
for (const file of (await readdir(join(APP, 'e2e'))).filter((f) => f.endsWith('.spec.ts') && !f.includes(' 2.')).sort()) {
  const text = await readFile(join(APP, 'e2e', file), 'utf8');
  for (const call of registerCallsIn(text)) {
    let def: any;
    try { def = evaluateLiteral(call.literal); } catch (error) { notes.push(`e2e/${file}:${call.line}: literal not evaluable (${String(error).slice(0, 80)})`); continue; }
    addRegistered('e2e', `${slug(file)}-L${call.line}`, [{ kind: call.kind, def: { ...def, id: typeof def.id === 'string' && def.id !== 'x' ? def.id : `e2e-${call.line}` } }], `apps/desktop/e2e/${file}:${call.line}`);
  }
  for (const literal of shaderLiterals(text)) {
    if (literal.key === 'code' && /void\s+main\s*\(/.test(literal.value)) {
      const id = `${slug(file)}-L${literal.line}-shader`;
      if (!seen.has(`e2e/${id}`)) push(shaderCase('e2e', id, literal.value, 'Shader layer', `apps/desktop/e2e/${file}:${literal.line}`));
    }
  }
}

/* ── 6. unit tests ─────────────────────────────────────────────────── */
{
  const files = [
    'src/renderer/src/kernel/glsl.test.ts', 'src/renderer/src/kernel/registries.test.ts', 'src/renderer/src/kernel/host.test.ts',
    'src/renderer/src/kernel/install.test.ts', 'src/renderer/src/kernel/extension-layers.test.ts', 'src/renderer/src/kernel/sandbox-host-stubs.test.ts',
    'src/renderer/src/kernel/sandbox-schemas.test.ts', 'src/renderer/src/legacy/__tests__/agent-harness.test.ts', 'src/renderer/src/legacy/__tests__/kernel-views.test.ts',
    'src/renderer/src/legacy/__tests__/source-editing.behavior.test.ts', 'src/renderer/src/legacy/__tests__/transitions.behavior.test.ts', 'src/renderer/src/panels/FxBrowserPanel.test.ts',
    'src/renderer/src/kernel/definition-snapshots.test.ts',
  ].filter((f) => existsSync(join(APP, f)));
  const uniq = new Map<string, string>();
  for (const file of files) {
    const text = await readFile(join(APP, file), 'utf8');
    for (const literal of shaderLiterals(text)) {
      if (literal.key === 'code') continue;
      const key = `${literal.key}:${literal.value}`;
      if (!uniq.has(key)) uniq.set(key, `apps/desktop/${file}:${literal.line}`);
    }
  }
  let n = 0;
  for (const [key, source] of uniq) {
    const [kind, ...rest] = key.split(':');
    const body = rest.join(':');
    const id = `unit-${n++}`;
    if (kind === 'fragment') {
      if (/void\s+main/.test(body)) safely(id, () => extensionCase('unit-tests', id, { id: `unit.${n}`, label: 'Unit', version: 1, params: [], renderer: { kind: 'fragment', fragment: body } }, source));
      continue;
    }
    if (body.length < 4 || !/\bo\s*=/.test(body)) continue;
    const isTransition = /\bu_(from|to)\b/.test(body);
    const referenced = [...new Set([...body.matchAll(/\bu_([a-z][a-zA-Z0-9]*)\b/g)].map((m) => m[1]!))]
      .filter((k) => !['tex', 'res', 'texel', 'time', 'prog', 'pass', 'from', 'to', 'orig', 'backdrop', 'coordRes', 'pxOrigin'].includes(k));
    const params = referenced.map((k) => (/color/i.test(k) ? { k, label: k, def: '#C08040', type: 'color' } : { k, label: k, def: 0.5, min: 0, max: 1 }));
    if (isTransition) safely(id, () => transitionCase('unit-tests', id, { id: `unit.tr.${n}`, label: 'Unit', params, frag: body }, source));
    else safely(id, () => effectCase('unit-tests', id, { id: `unit.fx.${n}`, label: 'Unit', group: 'Test', params, frag: body }, source));
  }
  const seenCode = new Set<string>();
  let k = 0;
  for (const file of ['src/renderer/src/legacy/__tests__/library-survivors.test.ts', 'src/renderer/src/panels/shader/ShaderPanel.test.ts'].filter((f) => existsSync(join(APP, f)))) {
    const text = await readFile(join(APP, file), 'utf8');
    for (const m of text.matchAll(/'((?:[^'\\\n]|\\.)*void\s+main\s*\((?:[^'\\\n]|\\.)*fragColor(?:[^'\\\n]|\\.)*)'/g)) {
      const code = vm.runInNewContext(`'${m[1]}'`) as string;
      if (seenCode.has(code)) continue;
      seenCode.add(code);
      push(shaderCase('unit-tests', `shader-code-${k++}`, code, 'Shader layer (test)', `apps/desktop/${file}:${text.slice(0, m.index).split('\n').length}`));
    }
  }
}

/* ── 7. scenes shipped with the app and site ───────────────────────── */
for (const file of ['apps/desktop/src/renderer/public/onboarding/scene.json', 'apps/www/static/hero/scene.json']) {
  if (!existsSync(join(REPO, file))) continue;
  const scene = JSON.parse(await readFile(join(REPO, file), 'utf8'));
  const tag = slug(file.split('/').slice(-3, -1).join('-'));
  for (const def of scene.effects ?? []) safely(def.id, () => effectCase('scenes', `${tag}-${slug(def.id)}`, { ...def, id: `${def.id}.${tag}` }, `${file} (effects.${def.id})`));
  for (const def of scene.transitions ?? []) safely(def.id, () => transitionCase('scenes', `${tag}-${slug(def.id)}`, { ...def, id: `${def.id}.${tag}` }, `${file} (transitions.${def.id})`));
  for (const def of scene.layerTypes ?? []) safely(def.id, () => extensionCase('scenes', `${tag}-${slug(def.id)}`, def, `${file} (layerTypes.${def.id})`));
  const walk = (v: any) => {
    if (Array.isArray(v)) return v.forEach(walk);
    if (v && typeof v === 'object') {
      if (v.type === 'shader' && typeof v.d?.code === 'string') {
        const id = `${tag}-shader-${createHash('sha256').update(v.d.code).digest('hex').slice(0, 8)}`;
        if (!seen.has(`scenes/${id}`)) push(shaderCase('scenes', id, v.d.code, 'Scene shader layer', file));
      }
      Object.values(v).forEach(walk);
    }
  };
  walk(scene);
}

/* ── 8. Store ──────────────────────────────────────────────────────── */
let storeReleases = 0;
if (withStore) {
  if (!storeDir || !existsSync(join(storeDir, 'index.json'))) throw new Error('no Store download: run fetch-store.ts (or pass --no-store)');
  const store = JSON.parse(await readFile(join(storeDir, 'index.json'), 'utf8'));
  for (const release of store.releases) {
    const entry = join(storeDir, release.handle, release.slug, release.version, 'index.ts');
    if (!existsSync(entry)) continue;
    storeReleases++;
    addRegistered('store', `${release.handle}-${release.slug}-${release.version}`, await activate(entry), `${store.origin}/v1/store/x/${release.handle}/${release.slug}/r/${release.version}/file/index.ts`, { group: 'store' });
  }
}

/* ── 9. generated corpora ──────────────────────────────────────────── */
for (const g of GENERATED as any[]) {
  const source = `engine/spikes/s3-shaders/corpus/generated (${g.name})`;
  const extra = { covers: g.covers };
  if (g.kind === 'effect') safely(g.name, () => effectCase('generated-s3', g.name, g.def, source, extra));
  else if (g.kind === 'transition') safely(g.name, () => transitionCase('generated-s3', g.name, g.def, source, extra));
  else if (g.kind === 'extension') safely(g.name, () => extensionCase('generated-s3', g.name, g.def, source, extra));
  else push(shaderCase('generated-s3', g.name, g.code, g.name, source, extra));
}
const generatedDir = join(import.meta.dir, 'generated');
for (const file of existsSync(generatedDir) ? (await readdir(generatedDir)).filter((f) => f.endsWith('.json')).sort() : []) {
  const g = JSON.parse(await readFile(join(generatedDir, file), 'utf8'));
  const source = `engine/crates/pm-render/tools/shaders/generated/${file}`;
  const extra = { covers: g.covers, provenance: g.provenance };
  if (g.kind === 'shader') push(shaderCase('generated-agent', g.name, g.code, g.name, source, extra));
  else if (g.kind === 'extension') safely(g.name, () => extensionCase('generated-agent', g.name, g.definition, source, extra));
  else if (g.kind === 'effect') safely(g.name, () => effectCase('generated-agent', g.name, g.definition, source, extra));
  else safely(g.name, () => transitionCase('generated-agent', g.name, g.definition, source, extra));
}

/* ── 10. private sources on this machine (opt-in) ──────────────────── */
const hash8 = (text: string) => createHash('sha256').update(text).digest('hex').slice(0, 8);
for (const path of localPaths) {
  if (path.endsWith('.pmv')) {
    let project: any;
    try { project = decodeProjectContainer(new Uint8Array(await readFile(path))).document; } catch (error) { notes.push(`local project skipped (${String(error).slice(0, 80)})`); continue; }
    const codes = new Set<string>();
    const walk = (v: unknown, key = '') => {
      if (typeof v === 'string') { if (key === 'code' && /void\s+main\s*\(/.test(v)) codes.add(v); return; }
      if (Array.isArray(v)) { for (const item of v) walk(item, key); return; }
      if (v && typeof v === 'object') for (const [k, x] of Object.entries(v)) walk(x, k);
    };
    walk(project);
    for (const code of codes) {
      const id = `shader-${hash8(code)}`;
      if (!seen.has(`local/${id}`)) push(shaderCase('local', id, code, 'Local shader layer', 'local project (private)', { private: true }));
    }
  } else {
    for (const dir of (await readdir(path)).sort()) {
      const entry = join(path, dir, 'index.ts');
      if (!existsSync(entry)) continue;
      const text = await readFile(entry, 'utf8');
      // A local extension that does not load here (an import the folder
      // does not hold) is noted, not fatal: the others still go in.
      let registered: Awaited<ReturnType<typeof activate>>;
      try { registered = await activate(entry); } catch (error) { notes.push(`local extension ${hash8(text)} skipped (${String(error).slice(0, 120)})`); continue; }
      addRegistered('local', `ext-${hash8(text)}`, registered, 'local extension (private)', { private: true });
    }
  }
}

/* ── real user setups (real extension loader, both settings) ────────── */
const userRoot = join(CORPUS_ROOT, 'user-setups');
const userLoader = flag('--user-loader') ?? join(OUT, 'user-loader.json');
if (!existsSync(userRoot) && !flag('--user-loader')) {
  notes.push('SKIP user-setups: private corpus directory is absent');
} else {
  if (!flag('--user-loader')) {
    const child = Bun.spawn([join(homedir(), 'Library/Caches/powermove-engine/bin/pm-electron-lock'), process.execPath, join(import.meta.dir, 'user-setups.ts'), '--out', userLoader], { stdout: 'inherit', stderr: 'inherit' });
    if (await child.exited !== 0) throw new Error('real user-setup loader capture failed');
  }
  const loader = JSON.parse(await readFile(userLoader, 'utf8'));
  for (const [si, setup] of loader.captures.entries()) {
    const [off, on] = setup.modes;
    if (JSON.stringify(off.registrations) !== JSON.stringify(on.registrations) || JSON.stringify(off.uses) !== JSON.stringify(on.uses)) throw new Error(`user setup ${si}: registrations/parameters differ with the engine setting`);
    if (JSON.stringify(off.health) !== JSON.stringify(on.health)) throw new Error(`user setup ${si}: extension failures differ with the engine setting`);
    for (const health of off.health) {
      if (health.health.state !== 'ok') notes.push(`user setup ${si} extension ${hash8(health.id)}: identical ${health.health.state} in both engines (see private loader capture)`);
    }
    for (const [index, item] of off.registrations.entries()) {
      const id = `setup-${si}/${hash8(item.owner)}/${index}`;
      let c = item.kind === 'effect' ? effectCase('user-setups', id, item.def, 'real extension loader (private)', { private: true })
        : item.kind === 'transition' ? transitionCase('user-setups', id, item.def, 'real extension loader (private)', { private: true })
        : extensionCase('user-setups', id, item.def, 'real extension loader (private)', { private: true });
      if (!c) continue;
      c.label = 'User program';
      const uses = off.uses.filter((u: any) => u.registration === index);
      for (const [ui, use] of uses.entries()) {
        // Real evaluated channels, including every key and segment midpoint,
        // replayed with both production renderers on identical inputs.
        const v: Variant = structuredClone(c.variants[0]!);
        v.name = `project-${use.project}-sample-${ui}`; v.time = use.time; v.params = use.params;
        for (const pass of v.passes) {
          const time = pass.calls.find(call => call.name === 'u_time');
          if (time) time.v = [use.time];
          pass.calls.splice(pass.calls.length - item.def.params.length, item.def.params.length, ...item.def.params.map((param: Param, pi: number) => paramCall(param, pi, use.params[pi] ?? param.def)));
        }
        c.variants.push(v);
      }
      push(c);
    }
  }
}

/* ── write ─────────────────────────────────────────────────────────── */
await mkdir(join(OUT, 'images'), { recursive: true });
const images = makeImages();
for (const image of images) await writeFile(join(OUT, image.file), image.bytes);
await writeFile(join(OUT, 'corpus.json'), JSON.stringify({
  format: 'powermove-shader-gate-corpus', version: 1, created: new Date().toISOString(), storeReleases,
  vertex: VERTEX, images: images.map(({ bytes: _, ...rest }) => rest), cases, notes,
}));
const byGroup: Record<string, Record<string, number>> = {};
for (const c of cases) ((byGroup[c.group] ??= {})[c.kind] = (byGroup[c.group]![c.kind] ?? 0) + 1);
console.log(`${cases.length} cases, ${cases.reduce((n, c) => n + c.variants.length, 0)} variants → ${OUT}`);
console.log(JSON.stringify(byGroup));
for (const n of notes) console.log('note:', n);
