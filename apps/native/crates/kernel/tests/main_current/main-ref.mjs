// Development-only access to the shipped Electron app's code at current main.
// Fixture generation and the reverse (main reads a V2 save) oracle import the
// app's own model, editing, hydrate, figma and file-format modules; nothing
// here is part of the native app. Run with Bun 1.4 (TypeScript imports).
import {execFileSync} from 'node:child_process';
import {existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

export const MAIN = '6d5bceb63218b96e751d77ac0450a3a3f22293ec';
export const here = path.dirname(fileURLToPath(import.meta.url));
export const repo = execFileSync('git', ['rev-parse', '--show-toplevel'], {cwd: here, encoding: 'utf8'}).trim();

/** `apps/desktop/src` at MAIN, exported once from this repository's objects
 * into a scratch tree this script owns. Bun auto-installs main's third-party
 * packages at the versions pinned in main's `apps/desktop/package.json`.
 * main's vitest alias `powermove` (kernel/api.ts) and its workspace packages
 * (`@powermove/*`, through each package's exports map) become relative
 * imports; nothing else in the sources changes. */
function mainSource() {
  const out = path.join(tmpdir(), `pm-main-src-${MAIN.slice(0, 12)}`);
  if (!existsSync(path.join(out, '.exported'))) {
    const staging = `${out}.${process.pid}`;
    rmSync(staging, {recursive: true, force: true});
    mkdirSync(staging, {recursive: true});
    const tar = execFileSync('git', ['archive', MAIN, 'apps/desktop/src', 'apps/desktop/package.json', 'packages'], {cwd: repo, maxBuffer: 1 << 30});
    execFileSync('tar', ['-x', '-C', staging], {input: tar});
    const targets = {powermove: path.join(staging, 'apps/desktop/src/renderer/src/kernel/api.ts')};
    for (const name of readdirSync(path.join(staging, 'packages'))) {
      const manifest = path.join(staging, 'packages', name, 'package.json');
      if (!existsSync(manifest)) continue;
      const pkg = JSON.parse(readFileSync(manifest, 'utf8'));
      const exports = typeof pkg.exports === 'string' ? {'.': pkg.exports} : pkg.exports ?? {'.': pkg.main ?? './index.ts'};
      for (const [sub, file] of Object.entries(exports)) {
        const target = typeof file === 'string' ? file : file.import ?? file.default;
        if (target) targets[pkg.name + sub.slice(1)] = path.join(staging, 'packages', name, target);
      }
    }
    const desktop = path.join(staging, 'apps/desktop/package.json');
    const pkg = JSON.parse(readFileSync(desktop, 'utf8'));
    for (const deps of [pkg.dependencies, pkg.devDependencies])
      for (const [dep, version] of Object.entries(deps ?? {})) if (String(version).startsWith('workspace:')) delete deps[dep];
    writeFileSync(desktop, JSON.stringify(pkg, null, 2));
    const roots = [path.join(staging, 'apps/desktop/src'), path.join(staging, 'packages')];
    for (const root of roots) for (const file of readdirSync(root, {recursive: true})) {
      if (!/\.(ts|js|mjs)$/.test(file)) continue;
      const full = path.join(root, file);
      const text = readFileSync(full, 'utf8');
      const next = text.replace(/(from |import\()'((?:powermove|@powermove\/[^']+))'/g, (match, lead, spec) => {
        if (!targets[spec]) return match;
        let rel = path.relative(path.dirname(full), targets[spec]);
        if (!rel.startsWith('.')) rel = './' + rel;
        return `${lead}'${rel}'`;
      });
      if (next !== text) writeFileSync(full, next);
    }
    writeFileSync(path.join(staging, '.exported'), MAIN + '\n');
    rmSync(out, {recursive: true, force: true});
    renameSync(staging, out);
  }
  return path.join(out, 'apps/desktop/src');
}
export const SRC = mainSource() + '/';

// Svelte rune modules (`*.svelte.ts`) evaluate at import; plain values suffice.
const rune = v => v;
rune.raw = rune; rune.snapshot = v => structuredClone(v);
Object.assign(globalThis, {$state: rune, $derived: Object.assign(v => v, {by: f => f()}),
  $effect: Object.assign(() => {}, {root: f => f(), pre: () => {}}), $props: () => ({})});

// Deterministic output: a fixed clock and no diagnostics noise on stdout.
let clock = Date.UTC(2026, 10, 1);
Date.now = () => (clock += 1000);
console.warn = () => {};
console.log = (...args) => process.stderr.write(args.join(' ') + '\n');

export const load = p => import(SRC + p);
const {makePM} = await load('renderer/src/legacy/__tests__/make-pm.ts');
const {EFFECTS} = await load('extensions/effects-basic/effects.ts');
export {EFFECTS};

const noop = () => {};
const element = () => ({dataset: {}, style: {}, classList: {add: noop, remove: noop, contains: () => false},
  append: noop, addEventListener: noop, remove: noop, focus: noop, select: noop, click: noop, setAttribute: noop});

/** A main editor registry, as main's own tests build it: the real model,
 * editing and history installers and the effects-basic catalog. */
export function editor(project = {name: 'Main fixture', w: 640, h: 360, fps: 30, dur: 4}, store = null) {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history',
    ...(store ? ['core/projects'] : []), 'core/editing', 'gl/shaders');
  // The renderer's store over main's FileStore (electron-shim, separateHistory).
  if (store) PM.store = {separateHistory: true, get: (key, fallback) => store.get(key) ?? fallback,
    has: key => store.get(key) !== undefined, set: (key, value) => { store.set(key, value); return true; }, del: key => store.delete(key)};
  for (const definition of EFFECTS) PM.Kernel.registerEffect('effects-basic', definition);
  PM.Kernel.services.register('shaderHooks', {syncShaderUniforms: noop});
  PM.proj = PM.mkProject(project);
  return PM;
}

/** Main's open-time hydrate (`legacy/app.ts`), installed exactly as
 * `effects-hydrate.test.ts` does. Returns `raw => hydrated project`. */
export async function hydrator() {
  const PM = editor();
  const {install} = await load('renderer/src/legacy/app.ts');
  Object.defineProperty(globalThis, 'window', {configurable: true, value: {
    document: {documentElement: element(), body: element(), createElement: element, getElementById: element},
    matchMedia: () => ({matches: false, addEventListener: noop}), addEventListener: noop, removeEventListener: noop,
    requestAnimationFrame: () => 1, setTimeout: () => 1, clearTimeout: noop, setInterval: () => 1, Blob}});
  const boot = JSON.parse(JSON.stringify(PM.proj));
  Object.assign(PM, {
    bootVersion: 0,
    Projects: {pickBoot: () => boot, list: () => [], get: () => boot, put: noop, remove: noop, getState: () => null,
      putState: noop, openProjects: () => [], tabs: () => [boot.id], rename: noop, trashList: () => []},
    WS: {init: noop, restoreSnapshot: noop, snapshot: () => ({}), editing: false},
    setTime(value) { PM.time = value; }, Audio: {normalizeLayer: noop}, rasterClear: noop,
    assets: {restoreProject: async () => ({stale: true, missing: []}), clear: noop, map: new Map(), get: () => null},
    Export: {snapshot: () => ''}, h: () => element(), $: () => element(), icon: element, PANELS: {}, perf: {}});
  PM.Kernel.services.register('timeline', {pps: 90, scrollT: 0, scrollY: 0, graph: false, frameView: noop});
  PM.Kernel.services.register('viewer', {layout: noop});
  PM.Kernel.services.register('inspector', {refresh: noop});
  install(PM);
  return {PM, hydrate: raw => PM.hydrateProject(structuredClone(raw))};
}

/** Main's native Save writer (`main/project-save.ts`): PMV4 with the
 * document first, then each owned media range, a checked index and footer. */
export async function writePmv4(destination, documentJSON, media) {
  const {ProjectSave} = await load('main/project-save.ts');
  const bytes = new TextEncoder().encode(documentJSON);
  rmSync(destination, {force: true});
  const save = await ProjectSave.create(destination, undefined,
    media.map(m => ({id: m.id, revision: m.revision, type: m.type, length: m.data.length})), bytes.length,
    async temporary => { renameSync(temporary, destination); return destination; }, noop);
  const chunks = (id, data) => { const out = []; for (let i = 0; i < data.length; i += 1 << 20) out.push([id, data.subarray(i, i + (1 << 20))]); return out; };
  for (const [id, data] of [...chunks(null, bytes), ...media.flatMap(m => chunks(m.id, m.data))]) await save.write(id, data);
  return save.finish();
}

/** The document string main's Save writes (`legacy/app.ts` saveProject). */
export const fileDocument = (version, project, history, ws) =>
  `{"v":${JSON.stringify(version ?? null)},"proj":${JSON.stringify(project)},"history":${JSON.stringify(history ?? null)},"ws":${JSON.stringify(ws ?? null)}}`;
