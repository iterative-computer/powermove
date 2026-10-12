/*
 * Builds the S3 corpus: every GLSL source Powermove renders today, wrapped into
 * the full program the legacy compositor compiles, plus the exact uniform calls
 * and texture units the compositor issues for each pass.
 *
 * Generators are the real ones: kernel/glsl.ts (through the kernel's effect and
 * transition registries), kernel/extension-layers.ts, and the preambles that
 * legacy/gl/shaders.ts and transitions.ts install.
 *
 * Usage: bun scripts/collect.ts [--no-store] [--local <path>]...
 * Output: work/corpus.json, work/images/*.f16 (gitignored; Store sources are
 * third-party code and never committed).
 *
 * --local adds private sources from this machine as the `local` group: a
 * `.pmv` project (shader-layer code in its layers and edit history) or an
 * extensions folder (each subfolder's index.ts). Their code stays in work/;
 * case ids are content hashes and the report prints no source text for them.
 */
import { mkdir, readdir, readFile, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { dirname, join, relative } from 'node:path';
import vm from 'node:vm';

import { GENERATED, type GeneratedShader } from '../corpus/generated';
import { makeImages, WIDTH, HEIGHT } from './images';

const SPIKE = join(import.meta.dir, '..');
const REPO = join(SPIKE, '..', '..', '..');
const APP = join(REPO, 'apps', 'desktop');
const RENDERER = join(APP, 'src', 'renderer', 'src');
const WORK = join(SPIKE, 'work');
const withStore = !process.argv.includes('--no-store');
const localPaths = process.argv.flatMap((arg, i, all) => (arg === '--local' && all[i + 1] ? [all[i + 1]!] : []));

/* ── the real generators ───────────────────────────────────────────── */
const shaders = await import(join(RENDERER, 'legacy/gl/shaders.ts'));
const transitions = await import(join(RENDERER, 'legacy/gl/transitions.ts'));
const extensionLayers = await import(join(RENDERER, 'kernel/extension-layers.ts'));
const PM: any = { uid: () => 'fx', P: (v: unknown) => ({ v }) };
shaders.install(PM);
transitions.install(PM);
const kernel = PM.Kernel;

/* legacy/core/util.ts PM.hex2rgb, verbatim (its install needs a DOM). */
const hexDigits = (hex: any): string => {
  const s = String(hex ?? '').replace('#', '');
  return s.length === 3 || s.length === 4 ? s.split('').map((c) => c + c).join('') : s;
};
const hex2rgb = (hex: any) => {
  const s = hexDigits(hex);
  return [0, 2, 4].map((i) => (parseInt(s.slice(i, i + 2), 16) || 0) / 255);
};

/* ── corpus types (mirrored by src/case.rs) ────────────────────────── */
interface Call { fn: '1f' | '2f' | '3f' | '4f' | '1i' | 'm3'; name: string; alt?: string; v: number[] }
interface Unit { unit: number; src: string }
interface Pass { calls: Call[]; units: Unit[] }
interface Variant { name: string; width: number; height: number; passes: Pass[] }
interface Case {
  id: string; group: string; kind: 'effect' | 'transition' | 'shader' | 'extension';
  label: string; source: string; frag: string; lineOffset: number; variants: Variant[];
  /** Contract features exercised (generated corpus), for the coverage table. */
  covers?: string[];
  flags?: Record<string, unknown>;
  /** Private local source: the report prints no source text or labels. */
  private?: boolean;
}

/* compositor.ts setU: how a JS value list becomes a glUniform call. */
function setU(name: string, v: number[] | Float32Array, alt?: string): Call {
  if (v.length === 9) return { fn: 'm3', name, v: Array.from(v) };
  const fn = (['1f', '2f', '3f', '4f'] as const)[v.length - 1]!;
  return alt ? { fn, name, alt, v: Array.from(v) } : { fn, name, v: Array.from(v) };
}
const setI = (name: string, v: number): Call => ({ fn: '1i', name, v: [v] });
const fullQuad = (w: number, h: number) => [w, 0, 0, 0, h, 0, 0, 0, 1];

/* ── parameter values per variant ──────────────────────────────────── */
const ALT_COLORS = ['#3FA7D6', '#F2545B', '#7DCE82', '#FFD23F', '#8E6CFF', '#1B998B'];
type Param = { k: string; type?: string; def: any; min?: number; max?: number };
function paramValue(param: Param, index: number, variant: string): any {
  if (variant !== 'alt') return param.def;
  if (param.type === 'color') return ALT_COLORS[index % ALT_COLORS.length];
  if (param.type === 'toggle') return !param.def;
  const min = Number(param.min), max = Number(param.max);
  if (Number.isFinite(min) && Number.isFinite(max) && max > min) return min + 0.7 * (max - min);
  return Number(param.def) * 1.5 + 1;
}
/* compositor.ts setParam */
function paramCall(param: Param, index: number, value: any): Call {
  const name = 'u_' + param.k;
  const alt = (param.type === 'color' ? 'u_c' : 'u_p') + index;
  if (param.type === 'color') return setU(name, hex2rgb(String(value)), alt);
  if (param.type === 'toggle') return setU(name, [value ? 1 : 0], alt);
  return setU(name, [Number(value) || 0], alt);
}

const EFFECT_VARIANTS = [
  { name: 'defaults', time: 1.25, viewport: false },
  { name: 'alt', time: 3.7, viewport: false },
  { name: 'viewport', time: 1.25, viewport: true },
];

/* compositor.ts runEffects */
function effectVariants(fx: { passes: number; params: Param[]; keepOrig: boolean; backdrop: boolean }, viewportSafe: boolean): Variant[] {
  return EFFECT_VARIANTS.filter((v) => !v.viewport || viewportSafe).map((variant) => {
    const W = WIDTH, H = HEIGHT;
    const resolution = variant.viewport ? [W * 2, H * 2] : [W, H];
    const origin = variant.viewport ? [W / 2, H / 2] : [0, 0];
    const uv = [origin[0]! / resolution[0]!, origin[1]! / resolution[1]!, W / resolution[0]!, H / resolution[1]!];
    const passes: Pass[] = [];
    for (let pass = 0; pass < fx.passes; pass++) {
      const calls: Call[] = [setI('u_tex', 0)];
      const units: Unit[] = [{ unit: 0, src: pass === 0 ? 'layer' : `pass:${pass - 1}` }];
      if (fx.keepOrig) { calls.push(setI('u_orig', 1)); units.push({ unit: 1, src: 'layer' }); }
      if (fx.backdrop) { calls.push(setI('u_backdrop', 2)); units.push({ unit: 2, src: 'backdrop' }); }
      calls.push(setU('u_m', fullQuad(W, H)), setU('u_res', [W, H]), setU('u_uv', uv), setU('u_coordRes', resolution), setU('u_pxOrigin', origin));
      calls.push(setU('u_texel', [1 / W, 1 / H]), setU('u_time', [variant.time]), setI('u_pass', pass));
      fx.params.forEach((param, index) => calls.push(paramCall(param, index, paramValue(param, index, variant.name))));
      passes.push({ calls, units });
    }
    return { name: variant.name, width: W, height: H, passes };
  });
}

/* compositor.ts runTransition */
function transitionVariants(params: Param[]): Variant[] {
  return [
    { name: 'defaults', prog: 0.37, time: 1.25 },
    { name: 'alt', prog: 0.81, time: 3.7 },
  ].map((variant) => {
    const W = WIDTH, H = HEIGHT;
    const calls: Call[] = [setI('u_from', 0), setI('u_to', 1), setU('u_m', fullQuad(W, H)), setU('u_res', [W, H]), setU('u_uv', [0, 0, 1, 1])];
    calls.push(setU('u_texel', [1 / W, 1 / H]), setU('u_time', [variant.time]), setU('u_prog', [variant.prog]));
    params.forEach((param, index) => calls.push(paramCall(param, index, paramValue(param, index, variant.name))));
    return { name: variant.name, width: W, height: H, passes: [{ calls, units: [{ unit: 0, src: 'from' }, { unit: 1, src: 'to' }] }] };
  });
}

const LAYER_FROM = 0.25, LAYER_DUR = 4, FPS = 30;
const LAYER_VARIANTS = [
  { name: 'defaults', time: 1.25 },
  { name: 'alt', time: 3.7 },
];
function layerBaseCalls(W: number, H: number, T: number): Call[] {
  const clamp = (v: number) => Math.min(1, Math.max(0, v));
  return [
    setU('u_m', fullQuad(W, H)), setU('u_res', [W, H]), setU('u_uv', [0, 0, 1, 1]), setU('iResolution', [W, H]),
    setU('iTime', [T - LAYER_FROM]), setU('iGlobalTime', [T]), setU('iProgress', [clamp((T - LAYER_FROM) / Math.max(LAYER_DUR, 1e-4))]),
    setI('iFrame', Math.round(T * FPS)), setU('iMouse', [0, 0]),
  ];
}

/* compositor.ts shader layer: uniforms come from PM.parseUniforms (udefs) */
function shaderLayerVariants(code: string): Variant[] {
  const udefs: any[] = PM.parseUniforms(code);
  return LAYER_VARIANTS.map((variant) => {
    const W = WIDTH, H = HEIGHT;
    const calls = layerBaseCalls(W, H, variant.time);
    udefs.forEach((def, index) => {
      let value: any = def.def;
      if (variant.name === 'alt') {
        if (def.control === 'color') value = ALT_COLORS[index % ALT_COLORS.length];
        else if (def.control === 'toggle') value = !def.def;
        else value = def.min + 0.7 * (def.max - def.min);
      }
      if (def.control === 'color') calls.push(setU(def.name, hex2rgb(String(value))));
      else if (def.control === 'toggle') calls.push(setI(def.name, value ? 1 : 0));
      else calls.push(setU(def.name, [Number(value) || 0]));
    });
    return { name: variant.name, width: W, height: H, passes: [{ calls, units: [] }] };
  });
}

/* compositor.ts extension fragment layer */
function extensionLayerVariants(params: Param[]): Variant[] {
  return LAYER_VARIANTS.map((variant) => {
    const W = WIDTH, H = HEIGHT;
    const calls = layerBaseCalls(W, H, variant.time);
    params.forEach((param, index) => {
      const value = paramValue(param, index, variant.name);
      if (param.type === 'color') calls.push(setU('u_' + param.k, hex2rgb(String(value))));
      else if (param.type === 'toggle') calls.push(setI('u_' + param.k, value ? 1 : 0));
      else calls.push(setU('u_' + param.k, [Number(value) || 0]));
    });
    return { name: variant.name, width: W, height: H, passes: [{ calls, units: [] }] };
  });
}

const lineOffsetOf = (program: string, user: string): number => {
  const at = program.lastIndexOf(user);
  return at < 0 ? 0 : program.slice(0, at).split('\n').length - 1;
};

/* ── case builders over registered definitions ─────────────────────── */
const cases: Case[] = [];
const seen = new Set<string>();
let registrations = 0;
function push(c: Case) {
  if (seen.has(c.id)) throw new Error(`duplicate case id ${c.id}`);
  seen.add(c.id);
  cases.push(c);
}

function effectCase(group: string, id: string, def: any, source: string, extra: Partial<Case> = {}) {
  const extensionId = `s3-${group}-${registrations++}`;
  const definition = { ...def, id: def.id };
  kernel.registerEffect(extensionId, definition);
  const fx = PM.FX[definition.id];
  push({
    id: `${group}/${id}`, group, kind: 'effect', label: def.label, source,
    frag: fx.frag, lineOffset: def.rawShader ? 0 : lineOffsetOf(fx.frag, def.frag),
    variants: effectVariants(fx, def.viewportSafe !== false),
    flags: { passes: fx.passes, keepOrig: fx.keepOrig, backdrop: fx.backdrop, rawShader: !!def.rawShader, viewportSafe: def.viewportSafe !== false },
    ...extra,
  });
}
function transitionCase(group: string, id: string, def: any, source: string, extra: Partial<Case> = {}) {
  kernel.registerTransition(`s3-${group}-${registrations++}`, def);
  const legacy = PM.transitionDef(def.id);
  push({
    id: `${group}/${id}`, group, kind: 'transition', label: def.label, source,
    frag: legacy.frag, lineOffset: def.rawShader ? 0 : lineOffsetOf(legacy.frag, def.frag),
    variants: transitionVariants(def.params), flags: { rawShader: !!def.rawShader }, ...extra,
  });
}
function extensionCase(group: string, id: string, def: any, source: string, extra: Partial<Case> = {}) {
  extensionLayers.validateExtensionLayerDefinition(def);
  if (def.renderer?.kind !== 'fragment') return;
  const frag = extensionLayers.extensionLayerFragment(def, PM.SHADER_HEADER);
  push({
    id: `${group}/${id}`, group, kind: 'extension', label: def.label, source,
    frag, lineOffset: lineOffsetOf(frag, def.renderer.fragment), variants: extensionLayerVariants(def.params), ...extra,
  });
}
function shaderCase(group: string, id: string, code: string, label: string, source: string, extra: Partial<Case> = {}) {
  const frag = PM.SHADER_HEADER + '\n' + code;
  push({ id: `${group}/${id}`, group, kind: 'shader', label, source, frag, lineOffset: lineOffsetOf(frag, code), variants: shaderLayerVariants(code), ...extra });
}

const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');

/* ── extension modules (docs, samples, fixtures, Store) ────────────── */
/** Definitions from a vm context use that realm's prototypes; copy into this realm. */
const plain = (value: unknown) => JSON.parse(JSON.stringify(value));
const activationNotes: string[] = [];
// Fixtures start background probes (sandbox checks) that reject outside a browser.
process.on('unhandledRejection', (error) => activationNotes.push(`background task rejected: ${String(error).slice(0, 120)}`));
interface Registered { kind: 'effect' | 'transition' | 'layer'; def: any }
async function activateModule(entry: string, project = { w: WIDTH, h: HEIGHT }): Promise<Registered[]> {
  const built = await Bun.build({ entrypoints: [entry], format: 'cjs', target: 'browser', external: ['powermove', '*.svelte'] });
  if (!built.success) throw new Error(`build ${entry}: ${built.logs.join('\n')}`);
  const code = await built.outputs[0]!.text();
  const out: Registered[] = [];
  const disposable = { dispose() {} };
  const noop: any = new Proxy(function () {}, { get: () => noop, apply: () => noop });
  const api: any = new Proxy({
    id: 'ext',
    effects: { register: (def: any) => { out.push({ kind: 'effect', def: plain(def) }); return disposable; }, list: () => [], get: () => undefined },
    transitions: { register: (def: any) => { out.push({ kind: 'transition', def: plain(def) }); return disposable; }, list: () => [], get: () => undefined },
    layers: { register: (def: any) => { out.push({ kind: 'layer', def: plain(def) }); return disposable; }, list: () => [], get: () => undefined },
    project: new Proxy({ get: async () => ({ ...project, fps: FPS, layers: [] }) }, { get: (t: any, p) => t[p] ?? noop }),
    events: { on: () => disposable },
    onDispose: () => {},
    log: () => {},
  }, { get: (target: any, prop) => target[prop] ?? noop });
  const module = { exports: {} as any };
  // A fresh context without require/process: these modules only build definitions.
  // `require` only resolves to inert stubs (Svelte components, the powermove types module).
  const require = () => new Proxy({ default: noop }, { get: (t: any, p) => t[p] ?? noop });
  vm.runInNewContext(code, { module, exports: module.exports, require, console: { log() {}, warn() {}, error() {} }, Promise, setTimeout, clearTimeout, document: noop, window: noop, navigator: noop, performance: noop, requestAnimationFrame: noop, cancelAnimationFrame: noop, OffscreenCanvas: noop, Image: noop, fetch: noop, addEventListener: noop, removeEventListener: noop }, { filename: entry, timeout: 2000 });
  const activate = module.exports.default ?? module.exports.activate;
  if (typeof activate === 'function') {
    try {
      await activate(api);
    } catch (error) {
      // Registrations made before the failure still count; the fixture's other
      // activation work (sandbox probes, DOM measurement) is irrelevant here.
      activationNotes.push(`${relative(REPO, entry)}: activation stopped after ${out.length} registrations (${String(error).slice(0, 120)})`);
    }
  }
  return out;
}

function addRegistered(group: string, prefix: string, items: Registered[], source: string, extra: Partial<Case> = {}) {
  items.forEach(({ kind, def }, index) => {
    const id = extra.private ? `${prefix}/${index}` : `${prefix}/${slug(def.id)}`;
    const label = extra.private ? { label: 'Local extension' } : {};
    if (kind === 'effect') effectCase(group, id, def, source, { ...extra, ...label });
    else if (kind === 'transition') transitionCase(group, id, def, source, { ...extra, ...label });
    else extensionCase(group, id, def, source, { ...extra, ...label });
  });
}

/* Evaluate `api.<registry>.register({...})` object literals in test files. */
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
function evaluateLiteral(source: string): any {
  try {
    return evaluateLiteralOnce(source);
  } catch {
    // The literal sits inside a template string (page code): unescape once.
    return evaluateLiteralOnce(source.replace(/\\`/g, '`').replace(/\\\$\{/g, '${'));
  }
}
function evaluateLiteralOnce(literal: string): any {
  const js = transpiler.transformSync(`globalThis.__s3 = (${literal});`).replace('globalThis.__s3', '__out.value');
  const target: any = { api: { id: 'ext' }, label: 'Test', __out: {} };
  // Unknown identifiers in the fixture (test-local variables) read as 'x'.
  const scope = new Proxy(target, { has: () => true, get: (t, p) => (p in t ? t[p] : p === Symbol.unscopables ? undefined : (globalThis as any)[p] ?? 'x') });
  vm.runInNewContext(`with (scope) { ${js} }`, { scope });
  return plain(target.__out.value);
}

/* String literals assigned to frag/fragment/code keys (unit-test fixtures). */
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
    const value = vm.runInNewContext(quote + raw + quote);
    out.push({ key: m[1]!, value, line: text.slice(0, m.index).split('\n').length });
  }
  return out;
}

/* ── 1. built-in effects and transitions ───────────────────────────── */
const { EFFECTS } = await import(join(APP, 'src/extensions/effects-basic/effects.ts'));
for (const def of EFFECTS) effectCase('builtin-effects', def.id, def, `apps/desktop/src/extensions/effects-basic/effects.ts (${def.id})`);
const { BUILTIN_TRANSITIONS } = await import(join(APP, 'src/extensions/transitions-basic/transitions.ts'));
for (const def of BUILTIN_TRANSITIONS) transitionCase('builtin-transitions', def.id, def, `apps/desktop/src/extensions/transitions-basic/transitions.ts (${def.id})`);

/* ── 2. Studio Cube and other built-in layer definitions ───────────── */
const { STUDIO_CUBE } = await import(join(APP, 'src/extensions/layers-3d/studio-cube.ts'));
extensionCase('builtin-layers', 'studio-cube', STUDIO_CUBE, 'apps/desktop/src/extensions/layers-3d/studio-cube.ts');
{
  // The checkerboard the compositor draws for a missing extension layer (compositor.ts contentQuad).
  const text = await readFile(join(RENDERER, 'legacy/gl/compositor.ts'), 'utf8');
  const match = /const missing = `([\s\S]*?)`;/.exec(text);
  if (!match) throw new Error('missing-layer placeholder not found in compositor.ts');
  shaderCase('builtin-layers', 'missing-placeholder', match[1]!, 'Missing extension placeholder', 'apps/desktop/src/renderer/src/legacy/gl/compositor.ts (missing)');
}

/* ── 3. shader-layer presets ───────────────────────────────────────── */
for (const [name, code] of Object.entries(PM.SHADER_PRESETS as Record<string, string>)) {
  shaderCase('shader-presets', slug(name), code, name, `apps/desktop/src/renderer/src/legacy/gl/shaders.ts (SHADER_PRESETS['${name}'])`);
}

/* ── 4. docs: samples and EXTENSIONS.md examples ───────────────────── */
for (const dir of await readdir(join(APP, 'docs/samples'))) {
  const entry = join(APP, 'docs/samples', dir, 'index.ts');
  if (!existsSync(entry)) continue;
  addRegistered('docs', dir, await activateModule(entry), relative(REPO, entry));
}
{
  const markdown = await readFile(join(APP, 'docs/EXTENSIONS.md'), 'utf8');
  const blocks = [...markdown.matchAll(/```ts\n([\s\S]*?)```/g)];
  let n = 0;
  for (const block of blocks) {
    const body = block[1]!;
    if (!/\b(effects|transitions|layers)\.register\(/.test(body)) continue;
    const line = markdown.slice(0, block.index).split('\n').length;
    const file = join(WORK, 'docs-snippets', `extensions-md-${n++}.ts`);
    await mkdir(dirname(file), { recursive: true });
    await writeFile(file, `export default async function activate(api: any) {\n${body}\n}\n`);
    addRegistered('docs', `extensions-md-L${line}`, await activateModule(file), `apps/desktop/docs/EXTENSIONS.md:${line}`);
  }
}

/* ── 5. repository Store samples (apps/cloud/samples) ──────────────── */
for (const dir of (await readdir(join(REPO, 'apps/cloud/samples'))).sort()) {
  const entry = join(REPO, 'apps/cloud/samples', dir, 'index.ts');
  if (!existsSync(entry)) continue;
  addRegistered('cloud-samples', slug(dir), await activateModule(entry), relative(REPO, entry));
}

/* ── 6. e2e and test fixtures ──────────────────────────────────────── */
for (const file of ['e2e/fixtures/typewriter-effect/index.ts', 'test/fixtures/sandboxed-ext/index.ts']) {
  addRegistered('e2e', slug(file), await activateModule(join(APP, file)), `apps/desktop/${file}`);
}
for (const file of (await readdir(join(APP, 'e2e'))).filter((f) => f.endsWith('.spec.ts') && !f.includes(' 2.')).sort()) {
  const text = await readFile(join(APP, 'e2e', file), 'utf8');
  for (const call of registerCallsIn(text)) {
    const def = evaluateLiteral(call.literal);
    addRegistered('e2e', `${slug(file)}-L${call.line}`, [{ kind: call.kind, def: { ...def, id: typeof def.id === 'string' && def.id !== 'x' ? def.id : `e2e-${call.line}` } }], `apps/desktop/e2e/${file}:${call.line}`);
  }
  // Shader layers created through project edits carry user GLSL in `code`.
  for (const literal of shaderLiterals(text)) {
    if (literal.key === 'code' && /void\s+main\s*\(/.test(literal.value)) {
      shaderCase('e2e', `${slug(file)}-L${literal.line}-shader`, literal.value, 'Shader layer', `apps/desktop/e2e/${file}:${literal.line}`);
    }
  }
}

/* ── 7. unit-test fixtures ─────────────────────────────────────────── */
{
  const files = [
    'src/renderer/src/kernel/glsl.test.ts', 'src/renderer/src/kernel/registries.test.ts', 'src/renderer/src/kernel/host.test.ts',
    'src/renderer/src/kernel/install.test.ts', 'src/renderer/src/kernel/extension-layers.test.ts', 'src/renderer/src/kernel/sandbox-host-stubs.test.ts',
    'src/renderer/src/kernel/sandbox-schemas.test.ts', 'src/renderer/src/legacy/__tests__/agent-harness.test.ts', 'src/renderer/src/legacy/__tests__/kernel-views.test.ts',
    'src/renderer/src/legacy/__tests__/source-editing.behavior.test.ts', 'src/renderer/src/legacy/__tests__/transitions.behavior.test.ts', 'src/renderer/src/panels/FxBrowserPanel.test.ts',
  ];
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
    const value = rest.join(':');
    const id = `unit-${n++}`;
    if (kind === 'fragment') {
      if (!/void\s+main/.test(value)) continue; // validator rejects these (by design of the test)
      extensionCase('unit-tests', id, { id: `unit.${n}`, label: 'Unit', version: 1, params: [], renderer: { kind: 'fragment', fragment: value } }, source);
      continue;
    }
    // Effect or transition bodies: declare the params they reference as floats.
    const isTransition = /\bu_(from|to)\b/.test(value);
    const referenced = [...new Set([...value.matchAll(/\bu_([a-z][a-zA-Z0-9]*)\b/g)].map((m) => m[1]!))]
      .filter((k) => !['tex', 'res', 'texel', 'time', 'prog', 'pass', 'from', 'to', 'orig', 'backdrop', 'coordRes', 'pxOrigin'].includes(k));
    const params = referenced.map((k) => (/color/i.test(k) ? { k, label: k, def: '#C08040', type: 'color' } : { k, label: k, def: 0.5, min: 0, max: 1 }));
    if (value.length < 4 || !/\bo\s*=/.test(value)) continue; // 'RAW', 'converted', 'x'.repeat(...): not shader bodies
    if (isTransition) transitionCase('unit-tests', id, { id: `unit.tr.${n}`, label: 'Unit', params, frag: value }, source);
    else effectCase('unit-tests', id, { id: `unit.fx.${n}`, label: 'Unit', group: 'Test', params, frag: value }, source);
  }
}

/* Shader-layer code in tests (`d.code = '…'`, `const SHADER_CODE = '…'`). */
{
  const files = ['src/renderer/src/legacy/__tests__/library-survivors.test.ts', 'src/renderer/src/panels/shader/ShaderPanel.test.ts'];
  const seenCode = new Set<string>();
  let n = 0;
  for (const file of files) {
    const text = await readFile(join(APP, file), 'utf8');
    for (const m of text.matchAll(/'((?:[^'\\\n]|\\.)*void\s+main\s*\((?:[^'\\\n]|\\.)*fragColor(?:[^'\\\n]|\\.)*)'/g)) {
      const code = vm.runInNewContext(`'${m[1]}'`) as string;
      if (seenCode.has(code)) continue;
      seenCode.add(code);
      const line = text.slice(0, m.index).split('\n').length;
      shaderCase('unit-tests', `shader-code-${n++}`, code, 'Shader layer (test)', `apps/desktop/${file}:${line}`);
    }
  }
}

/* ── 8. exported scenes shipped with the app and site ──────────────── */
for (const file of ['apps/desktop/src/renderer/public/onboarding/scene.json', 'apps/www/static/hero/scene.json']) {
  const scene = JSON.parse(await readFile(join(REPO, file), 'utf8'));
  for (const def of scene.effects ?? []) effectCase('scenes', `${slug(file.split('/').slice(-3, -1).join('-'))}-${slug(def.id)}`, { ...def, id: `${def.id}.${slug(file)}` }, `${file} (effects.${def.id})`);
  for (const def of scene.transitions ?? []) transitionCase('scenes', `${slug(file)}-${slug(def.id)}`, { ...def, id: `${def.id}.${slug(file)}` }, `${file} (transitions.${def.id})`);
  for (const def of scene.layerTypes ?? []) extensionCase('scenes', `${slug(file)}-${slug(def.id)}`, def, `${file} (layerTypes.${def.id})`);
}

/* ── 9. Store (public releases, fetched by fetch-store.ts) ─────────── */
if (withStore) {
  const index = join(WORK, 'store', 'index.json');
  if (!existsSync(index)) throw new Error('work/store/index.json missing: run `bun scripts/fetch-store.ts` (or pass --no-store)');
  const store = JSON.parse(await readFile(index, 'utf8'));
  for (const release of store.releases) {
    const entry = join(WORK, 'store', release.handle, release.slug, release.version, 'index.ts');
    if (!existsSync(entry)) continue;
    addRegistered('store', `${release.handle}-${release.slug}-${release.version}`, await activateModule(entry), `${store.origin}/v1/store/x/${release.handle}/${release.slug}/r/${release.version}/file/index.ts`);
  }
}

/* ── 10. private sources on this machine (opt-in) ──────────────────── */
const hash8 = (text: string) => createHash('sha256').update(text).digest('hex').slice(0, 8);
for (const path of localPaths) {
  if (path.endsWith('.pmv')) {
    // PMV4: a header line, then the project JSON, then binary payloads.
    const bytes = await readFile(path);
    const header = bytes.subarray(0, 5).toString('latin1');
    if (header !== 'PMV4\n') { activationNotes.push(`local project skipped (format ${JSON.stringify(header.trim())})`); continue; }
    const text = bytes.subarray(5).toString('utf8');
    let depth = 0, end = 0, quote = false;
    for (let i = 0; i < text.length; i++) {
      const ch = text[i]!;
      if (quote) { if (ch === '\\') i++; else if (ch === '"') quote = false; continue; }
      if (ch === '"') quote = true;
      else if (ch === '{') depth++;
      else if (ch === '}' && --depth === 0) { end = i + 1; break; }
    }
    const project = JSON.parse(text.slice(0, end));
    // Shader-layer code: current layers plus every distinct version in the edit history.
    const codes = new Set<string>();
    const walk = (value: unknown, key = '') => {
      if (typeof value === 'string') { if (key === 'code' && /void\s+main\s*\(/.test(value)) codes.add(value); return; }
      if (Array.isArray(value)) { for (const item of value) walk(item, key); return; }
      if (value && typeof value === 'object') for (const [k, v] of Object.entries(value)) walk(v, k);
    };
    walk(project.proj);
    for (const code of codes) {
      const id = `shader-${hash8(code)}`;
      if (seen.has(`local/${id}`)) continue;
      shaderCase('local', id, code, 'Local shader layer', 'local project (private)', { private: true });
    }
  } else {
    for (const dir of (await readdir(path)).sort()) {
      const entry = join(path, dir, 'index.ts');
      if (!existsSync(entry)) continue;
      const text = await readFile(entry, 'utf8');
      addRegistered('local', `ext-${hash8(text)}`, await activateModule(entry), 'local extension (private)', { private: true });
    }
  }
}

/* ── 11. generated agent corpus ────────────────────────────────────── */
for (const g of GENERATED as GeneratedShader[]) {
  const extra = { covers: g.covers };
  if (g.kind === 'effect') effectCase('generated', g.name, g.def, `engine/spikes/s3-shaders/corpus/generated (${g.name})`, extra);
  else if (g.kind === 'transition') transitionCase('generated', g.name, g.def, `engine/spikes/s3-shaders/corpus/generated (${g.name})`, extra);
  else if (g.kind === 'extension') extensionCase('generated', g.name, g.def, `engine/spikes/s3-shaders/corpus/generated (${g.name})`, extra);
  else shaderCase('generated', g.name, g.code, g.name, `engine/spikes/s3-shaders/corpus/generated (${g.name})`, extra);
}

/* ── write ─────────────────────────────────────────────────────────── */
await mkdir(join(WORK, 'images'), { recursive: true });
const images = makeImages();
for (const image of images) await writeFile(join(WORK, image.file), image.bytes);
await writeFile(
  join(WORK, 'corpus.json'),
  JSON.stringify({ vertex: PM.VERT, images: images.map(({ bytes: _, ...rest }) => rest), cases }, null, 1),
);
const byGroup = new Map<string, number>();
for (const c of cases) byGroup.set(c.group, (byGroup.get(c.group) ?? 0) + 1);
console.log(`${cases.length} cases:`, Object.fromEntries(byGroup));
for (const note of activationNotes) console.log('note:', note);
