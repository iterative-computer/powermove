/**
 * Parity corpus capture from the legacy engine.
 *
 *   bun engine/corpus/src/capture.ts [--id <capture-id>] [--root <dir>]
 *        [--only <prefix,...>] [--skip-synthetic] [--skip-scenarios] [--skip-real]
 *        [--real <file.pmv>] [--real-step <seconds>] [--no-eval] [--restart-every <n>]
 *        [--label <text>] [--text-only]
 *
 * Writes to <root>/<capture-id>/ (default root: ~/Library/Application Support/
 * Powermove Engine Corpus). Never writes inside the repository. See
 * docs/engine/corpus.md for the layout and formats.
 */
import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { launchLegacyApp, type LegacyApp } from './app';
import { installBlenderRecorder, pairSnapshots, takeBlenderJobs, BLENDER_FORMAT, type SnapshotPass } from './blender';
import { desktopRoot, EXTENSION_EFFECTS, parseRender3D, repoRoot, scene3d, shaderPresets } from './catalog';
import { generateCorpus, stableJson, type GeneratedProject } from './generate';
import { legacyCaptureProjects } from './legacy-capture';
import { MISSING_MEDIA, resolveMedia, type MediaSource } from './media';
import { sha256, timeLabel, writeFile, writeFloat, writePng, straightTopDown, type FileRecord } from './output';
import { legacyScript } from './inject';
import { installCorpusPage, renderingPerformanceScenarios } from './page';
import { allProjects, type CorpusProject } from './projects';
import { textOnly, textOnlyProjects } from './text-only';
import { compareSubset, type Issue } from './validate';
import { verifyCapture } from './verify';

export const DEFAULT_ROOT = path.join(os.homedir(), 'Library/Application Support/Powermove Engine Corpus');
export const DEFAULT_REAL = path.join(os.homedir(), 'Documents/Powermove ad.pmv');

interface Options {
  id?: string; root: string; only: string[]; synthetic: boolean; scenarios: boolean; real: boolean;
  realPath: string; realStep: number; evaluate: boolean; restartEvery: number; label: string;
  /** Text in isolation (`text-only.ts`). */
  textOnly: boolean;
}

interface SampleRecord {
  time: number; width: number; height: number;
  png?: FileRecord; alphaPng?: FileRecord; float?: { data: FileRecord; header: FileRecord }; eval?: FileRecord;
  /** Transparent float renders at other motion-blur sample counts (`blurSamples`). */
  blur?: Array<{ samples: number; float: { data: FileRecord; header: FileRecord } }>;
  blender?: FileRecord; blenderSnapshots?: { final: number; preview: number; export?: number; exportAlpha?: number }; blenderError?: string;
  glErrors: string[]; warnings: string[]; prepareError?: string; error?: string; ms: number;
}

interface ProjectRecord {
  id: string; group?: string; title?: string; covers?: string[]; source: 'synthetic' | 'real';
  document?: FileRecord; loaded?: FileRecord; media: Record<string, { assetId: string; kind: string; source?: MediaSource; error?: string }>;
  hydrateIssues: Issue[]; keyIdRewrites: string[]; extensionChecks: Array<{ extension: string; effect: string; registered: boolean; paramsMatch: boolean }>; extensions?: string[]; missingDefinitions: string[]; mediaPending: string[]; settle?: { stable: boolean; renders: number; prepareError?: string };
  blender?: { settings: Record<string, unknown>; compositions: string[]; assets: FileRecord[]; problems: string[] };
  samples: SampleRecord[]; pageErrors: string[]; error?: string; ms: number;
}

function parseArgs(argv: string[]): Options {
  const options: Options = { root: DEFAULT_ROOT, only: [], synthetic: true, scenarios: true, real: true, realPath: DEFAULT_REAL, realStep: 2, evaluate: true, restartEvery: 30, label: 'v0', textOnly: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]!, next = () => { const value = argv[++i]; if (value === undefined) throw new Error(`${arg} needs a value`); return value; };
    if (arg === '--id') options.id = next();
    else if (arg === '--root') options.root = path.resolve(next());
    else if (arg === '--only') options.only = next().split(',').filter(Boolean);
    else if (arg === '--skip-synthetic') options.synthetic = false;
    else if (arg === '--skip-scenarios') options.scenarios = false;
    else if (arg === '--skip-real') options.real = false;
    else if (arg === '--real') options.realPath = path.resolve(next());
    else if (arg === '--real-step') options.realStep = Number(next());
    else if (arg === '--no-eval') options.evaluate = false;
    else if (arg === '--label') options.label = next();
    else if (arg === '--text-only') options.textOnly = true;
    else if (arg === '--restart-every') options.restartEvery = Math.max(1, Number(next()));
    else throw new Error(`Unknown argument ${arg}`);
  }
  if (!(options.realStep > 0)) throw new Error('--real-step must be positive');
  return options;
}

function git(...args: string[]): string {
  return execFileSync('git', args, { cwd: repoRoot, encoding: 'utf8' }).trim();
}

/** A content hash over the built app (out/), so a capture names the exact legacy build it used. */
function buildFingerprint(): { sha256: string; files: number; newest: string } {
  const root = path.join(desktopRoot, 'out');
  const files: string[] = [];
  const walk = (dir: string) => { for (const entry of readdirSync(dir, { withFileTypes: true })) { const full = path.join(dir, entry.name); if (entry.isDirectory()) walk(full); else files.push(full); } };
  walk(root);
  files.sort();
  let newest = 0;
  const hash = new Bun.CryptoHasher('sha256');
  for (const file of files) { hash.update(path.relative(root, file)); hash.update(readFileSync(file)); newest = Math.max(newest, statSync(file).mtimeMs); }
  return { sha256: hash.digest('hex'), files: files.length, newest: new Date(newest).toISOString() };
}

function machine(): Record<string, unknown> {
  const sysctl = (key: string) => { try { return execFileSync('sysctl', ['-n', key], { encoding: 'utf8' }).trim(); } catch { return null; } };
  const swVers = (flag: string) => { try { return execFileSync('sw_vers', [flag], { encoding: 'utf8' }).trim(); } catch { return null; } };
  return {
    platform: process.platform, arch: process.arch, cpu: sysctl('machdep.cpu.brand_string'), model: sysctl('hw.model'),
    memoryBytes: os.totalmem(), osProductVersion: swVers('-productVersion'), osBuild: swVers('-buildVersion'),
  };
}

const decode = (base64: string) => new Uint8Array(Buffer.from(base64, 'base64'));

class Session {
  legacy!: LegacyApp;
  opened = 0;
  extensions: string[] = [];
  constructor(readonly options: Options, readonly legacyHelpers: string) {}

  async start(extensions: string[] = []): Promise<void> {
    this.extensions = [...extensions].sort();
    this.legacy = await launchLegacyApp({ extensions: this.extensions });
    await installBlenderRecorder(this.legacy);
    await this.legacy.evaluate(installCorpusPage);
    if (await this.legacy.evaluateScript<boolean>(this.legacyHelpers) !== true) throw new Error('The legacy helper bundle did not install in the renderer');
    this.opened = 0;
  }

  async restart(extensions = this.extensions): Promise<void> {
    await this.legacy.close();
    await this.start(extensions);
  }

  /** Restart every `restartEvery` projects, and whenever a project needs a different set of fixture extensions. */
  async prepareFor(extensions: string[] = []): Promise<void> {
    const wanted = [...extensions].sort();
    if (wanted.join('\n') !== this.extensions.join('\n') || this.opened >= this.options.restartEvery) await this.restart(wanted);
  }
}

async function captureSamples(session: Session, out: string, dir: string, samples: number[], size: [number, number], alpha: boolean, evaluate: boolean, record: ProjectRecord, label: string, blurSamples: number[] = []): Promise<void> {
  for (const time of samples) {
    const started = performance.now();
    const sample: SampleRecord = { time, width: size[0], height: size[1], glErrors: [], warnings: [], ms: 0 };
    try {
      const result = await session.legacy.evaluate(args => (window as any).__corpus.capture(args), { time, width: size[0], height: size[1], alpha, evaluate, blurSamples });
      const name = timeLabel(time);
      const text = { Software: 'Powermove engine corpus (legacy capture)', Source: `${label} @ ${time}s`, Alpha: 'opaque export path (topDownOpaque, over black)' };
      sample.png = writePng(out, `${dir}/frames/${name}.png`, size[0], size[1], decode(result.opaque.base64), text);
      if (result.alpha) {
        sample.alphaPng = writePng(out, `${dir}/frames/${name}.alpha.png`, size[0], size[1], decode(result.alpha.straightBase64), { ...text, Alpha: 'straight alpha, exporter alphaFrame conversion of the transparent render' });
        sample.float = writeFloat(out, `${dir}/frames/${name}.rgba32f`, {
          width: size[0], height: size[1], rowOrder: 'top-down', alpha: 'premultiplied', time, sourcePrecision: 'binary16',
          encoding: 'Legacy compositor working values: display encoding E, Rec.709 primaries, premultiplied, unclamped; the accumulator is an RGBA16F texture, so every value is exactly representable in binary16',
          source: `PM.GL.renderToPixels(t, w, h, ${JSON.stringify(result.alpha.options)}) via gl.readPixels(RGBA, FLOAT)`,
        }, decode(result.alpha.floatBase64));
      }
      for (const blur of result.blur ?? []) {
        (sample.blur ??= []).push({
          samples: blur.samples,
          float: writeFloat(out, `${dir}/frames/${name}.mb${blur.samples}.rgba32f`, {
            width: size[0], height: size[1], rowOrder: 'top-down', alpha: 'premultiplied', time, sourcePrecision: 'binary16',
            encoding: 'Legacy compositor working values: display encoding E, Rec.709 primaries, premultiplied, unclamped; the accumulator is an RGBA16F texture, so every value is exactly representable in binary16',
            source: `PM.GL.renderToPixels(t, w, h, ${JSON.stringify(blur.options)}) via gl.readPixels(RGBA, FLOAT)`,
          }, decode(blur.floatBase64)),
        });
      }
      if (result.evaluated) sample.eval = writeFile(out, `${dir}/eval/${name}.json`, JSON.stringify(result.evaluated) + '\n');
      sample.glErrors = result.glErrors; sample.warnings = result.warnings;
      if (result.prepareError) sample.prepareError = result.prepareError;
    } catch (error) {
      sample.error = (error as Error).message.split('\n')[0]!;
    }
    sample.ms = Math.round(performance.now() - started);
    record.samples.push(sample);
  }
}

/**
 * Blender snapshots for every captured sample of a project with 3D objects: final
 * output (every nested composition and shutter sample) and the paused rendered
 * preview. Runs after the frames, since it turns Blender output on in memory.
 */
async function captureBlender(session: Session, out: string, dir: string, project: CorpusProject, record: ProjectRecord, alpha: boolean): Promise<void> {
  const settings = parseRender3D({ ...(project.blender ?? {}), enabled: true });
  const objectDefinition = scene3d.definitions.object;
  const problems: string[] = [];
  const compositions: string[] = await session.legacy.evaluate(args => (window as any).__corpus.enableBlenderOutput(args), { settings, objectDefinition });
  const written = new Map<string, FileRecord>();
  for (const sample of record.samples) {
    if (sample.error) continue;
    try {
      const stray = await takeBlenderJobs(session.legacy);
      if (stray.length) problems.push(`${stray.length} Blender jobs before the ${sample.time}s pass`);
      const final = await session.legacy.evaluate(t => (window as any).__corpus.blenderFinal(t), sample.time);
      const finalJobs = await takeBlenderJobs(session.legacy);
      const exportPasses: Record<string, SnapshotPass & { options: unknown }> = {};
      for (const [kind, mbSamples] of [['export', 16], ...(alpha ? [['exportAlpha', 20]] : [])] as Array<[string, number]>) {
        const pass = await session.legacy.evaluate(args => (window as any).__corpus.blenderExport(args),
          { time: sample.time, width: sample.width, height: sample.height, mbSamples });
        const jobs = await takeBlenderJobs(session.legacy);
        exportPasses[kind] = { options: pass.options, ...pairSnapshots(out, dir, pass.requests, jobs, written, pass.error ? { error: pass.error } : {}) };
      }
      const preview = await session.legacy.evaluate(args => (window as any).__corpus.blenderPreview(args), { time: sample.time, objectDefinition });
      const previewJobs = await takeBlenderJobs(session.legacy);
      const document = {
        format: BLENDER_FORMAT, version: 1, project: project.id, time: sample.time, settings, compositions,
        final: pairSnapshots(out, dir, final.requests, finalJobs, written, final.error ? { error: final.error } : {}),
        preview: pairSnapshots(out, dir, preview.requests, previewJobs, written, { view: preview.view, ...(preview.error ? { error: preview.error } : {}) }),
        ...exportPasses,
      };
      for (const [kind, pass] of [['final', document.final], ['preview', document.preview], ...Object.entries(exportPasses)] as Array<[string, SnapshotPass]>) {
        if (pass.mismatch) problems.push(`${kind} @ ${sample.time}s: ${pass.mismatch}`);
        if (pass.error) problems.push(`${kind} @ ${sample.time}s: ${pass.error}`);
      }
      sample.blender = writeFile(out, `${dir}/blender/${timeLabel(sample.time)}.json`, JSON.stringify(document) + '\n');
      sample.blenderSnapshots = {
        final: document.final.snapshots.length, preview: document.preview.snapshots.length,
        ...Object.fromEntries(Object.entries(exportPasses).map(([kind, pass]) => [kind, pass.snapshots.length])),
      };
    } catch (error) {
      sample.blenderError = (error as Error).message.split('\n')[0]!;
      problems.push(`@ ${sample.time}s: ${sample.blenderError}`);
    }
  }
  record.blender = { settings, compositions, assets: [...written.values()], problems };
}

const hasObjects = (described: GeneratedProject) => described.layerTypes.includes(`extension:${scene3d.definitions.object}`);

function missingDefinitions(doc: any, hydrated: any): string[] {
  const out: string[] = [];
  const comps = (p: any) => [['root', p], ...Object.entries<any>(p.comps ?? {})] as Array<[string, any]>;
  for (const [where, comp] of comps(hydrated)) for (const layer of comp.layers ?? []) {
    for (const fx of layer.fx ?? []) if (fx.missing) out.push(`${where}/${layer.name}: effect ${fx.type} missing`);
    for (const field of ['transitionIn', 'transitionOut']) if (layer[field]?.missing) out.push(`${where}/${layer.name}: ${field} ${layer[field].type} missing`);
  }
  void doc;
  return out;
}

async function captureSynthetic(session: Session, out: string, project: CorpusProject, described: GeneratedProject, generatedDir: string, evaluate: boolean): Promise<ProjectRecord> {
  const started = performance.now();
  const dir = `projects/${project.id}`;
  const record: ProjectRecord = {
    id: project.id, group: project.group, title: project.title, covers: project.covers, source: 'synthetic',
    media: {}, hydrateIssues: [], keyIdRewrites: [], extensionChecks: [], extensions: project.extensions ?? [], missingDefinitions: [], mediaPending: [], samples: [], pageErrors: [], ms: 0,
  };
  const errorsBefore = session.legacy.pageErrors.length;
  try {
    record.document = writeFile(out, `${dir}/document.json`, stableJson(project.doc));
    const opened = await session.legacy.evaluate(doc => (window as any).__corpus.open(doc), project.doc);
    session.opened++;
    const issues: Issue[] = [];
    compareSubset(JSON.parse(stableJson(project.doc)), opened.hydrated, 'doc', issues);
    // The app's loader reassigns some keyframe ids on every open (effect parameters pass
    // through two id-deduplicating sanitizers in legacy/app.ts); those are recorded apart
    // from value rewrites. loaded.json holds the ids the app settled on.
    const keyId = /\.kf\[\d+\]\.i$/;
    record.keyIdRewrites = issues.filter(issue => keyId.test(issue.path)).map(issue => issue.path);
    record.hydrateIssues = issues.filter(issue => !keyId.test(issue.path));
    record.missingDefinitions = missingDefinitions(project.doc, opened.hydrated);
    for (const extension of project.extensions ?? []) {
      for (const [type, definition] of Object.entries(EXTENSION_EFFECTS).filter(([, item]) => item.extension === extension)) {
        const live: string[] | null = await session.legacy.evaluate((id: string) => (window as any).PM.Kernel.effects.get(id)?.params?.map((param: any) => param.k) ?? null, type);
        const matches = !!live && live.length === definition.params.length && live.every((key, index) => key === definition.params[index]);
        record.extensionChecks.push({ extension, effect: type, registered: !!live, paramsMatch: matches });
        if (!matches) record.hydrateIssues.push({ path: `extension ${extension}`, message: `effect ${type} ${live ? `parameters ${live.join(',')} differ from the corpus list` : 'is not registered'}` });
      }
    }
    const names = described.media.filter(name => !`media:${name}`.startsWith(MISSING_MEDIA));
    if (names.length) {
      const sources = names.map(name => resolveMedia(name, generatedDir));
      await session.legacy.evaluate(() => (window as any).__corpus.mediaInput());
      await session.legacy.setInputFiles('#corpus-media', sources.map(source => path.isAbsolute(source.path) ? source.path : path.join(repoRoot, source.path)));
      const imported: Record<string, { id: string; kind: string; error?: string }> = await session.legacy.evaluate(() => (window as any).__corpus.importMedia());
      const map: Record<string, string> = {};
      for (const source of sources) {
        const result = imported[source.name];
        record.media[source.name] = { assetId: result?.id ?? '', kind: result?.kind ?? '', source, ...(result?.error ? { error: result.error } : {}) };
        if (result?.id) map[source.name] = result.id;
      }
      const { project: rewritten } = await session.legacy.evaluate(m => (window as any).__corpus.rewriteMedia(m), map);
      // Reopen the document with real asset ids: the same path as opening a saved project.
      await session.legacy.evaluate(doc => (window as any).__corpus.open(doc), rewritten);
      record.mediaPending = await session.legacy.evaluate(() => (window as any).__corpus.waitForMedia(30000));
    }
    const loaded = await session.legacy.evaluate(() => JSON.stringify((window as any).PM.proj));
    record.loaded = writeFile(out, `${dir}/loaded.json`, loaded + '\n');
    record.settle = await session.legacy.evaluate(t => (window as any).__corpus.settle(t), project.samples[0]!);
    await captureSamples(session, out, dir, project.samples, described.size, described.alpha, evaluate, record, project.id, project.blurSamples);
    const stray = await takeBlenderJobs(session.legacy);
    if (stray.length) record.hydrateIssues.push({ path: 'blender', message: `${stray.length} Blender jobs while capturing frames` });
    if (hasObjects(described)) await captureBlender(session, out, dir, project, record, described.alpha);
  } catch (error) {
    record.error = (error as Error).message.split('\n')[0]!;
  }
  record.pageErrors = session.legacy.pageErrors.slice(errorsBefore);
  record.ms = Math.round(performance.now() - started);
  writeFile(out, `${dir}/project.json`, JSON.stringify(record, null, 1) + '\n');
  return record;
}

async function captureScenarios(session: Session, out: string): Promise<Record<string, unknown>> {
  await session.legacy.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Rendering performance scenarios' }) }));
  });
  session.opened++;
  const result = await session.legacy.evaluate(renderingPerformanceScenarios);
  const sizes: Record<string, [number, number]> = { resize: [160, 90] };
  const images: Record<string, unknown> = {};
  const hashes: Record<string, string> = {};
  for (const [label, base64] of Object.entries(result.images)) {
    const bytes = decode(base64);
    const [width, height] = sizes[label] ?? [320, 180];
    hashes[label] = sha256(bytes);
    const opaque = !['alpha', 'transparent-background', 'group-effect', 'matte', 'nested-boundary'].includes(label);
    images[label] = {
      width, height, opaque,
      raw: writeFile(out, `scenarios/rendering-performance/${label}.rgba8`, bytes),
      png: writePng(out, `scenarios/rendering-performance/${label}.png`, width, height, straightTopDown(bytes, width, height), { Source: `rendering-performance scenario ${label}`, Alpha: opaque ? 'opaque' : 'straight alpha from premultiplied readback' }),
    };
  }
  const summary = {
    source: 'apps/desktop/e2e/rendering-performance.spec.ts, "shared renderer preserves pixels across grouped previews and captures"',
    rawFormat: 'exact PM.GL.renderToPixels bytes: RGBA8, bottom-up rows, premultiplied (opaque captures have alpha 255); sha256 matches the spec\'s hashes',
    count: Object.keys(hashes).length, hashes, images, flatMatches: result.flatMatches, glErrors: result.errors, glError: result.glError,
  };
  writeFile(out, 'scenarios/rendering-performance/hashes.json', JSON.stringify({ hashes }, null, 2) + '\n');
  writeFile(out, 'scenarios/rendering-performance/scenarios.json', JSON.stringify(summary, null, 1) + '\n');
  return summary;
}

async function captureReal(session: Session, out: string, file: string, step: number, evaluate: boolean, isolateText = false): Promise<ProjectRecord> {
  const started = performance.now();
  const slug = path.basename(file, path.extname(file)).toLowerCase().replace(/[^a-z0-9]+/g, '-');
  const dir = `real/${slug}`;
  const record: ProjectRecord = { id: `real/${slug}`, source: 'real', media: {}, hydrateIssues: [], keyIdRewrites: [], extensionChecks: [], missingDefinitions: [], mediaPending: [], samples: [], pageErrors: [], ms: 0 };
  const errorsBefore = session.legacy.pageErrors.length;
  const originalHash = sha256(readFileSync(file));
  try {
    // Open a copy inside the isolated profile so nothing can write to the original.
    const copy = path.join(session.legacy.userData, path.basename(file));
    copyFileSync(file, copy);
    await session.legacy.evaluateMain(({ dialog }, filePath: string) => { dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] }); }, copy);
    const before = await session.legacy.evaluate(() => (window as any).PM.proj.id);
    await session.legacy.evaluate(() => (window as any).PM.openProject());
    session.opened++;
    await session.legacy.waitFor((id: string) => (window as any).PM.proj.id !== id, before, 120_000);
    await session.legacy.evaluate(async () => { await (window as any).PM.projectAssetsReady; });
    record.mediaPending = await session.legacy.evaluate(() => (window as any).__corpus.waitForMedia(180000));
    if (isolateText) {
      const snapshot = await session.legacy.evaluate(() => JSON.stringify((window as any).PM.proj));
      const isolated = textOnly(JSON.parse(snapshot));
      await session.legacy.evaluate((doc: any) => { const PM = (window as any).PM; PM.replaceProject(doc); PM.invalidate(); }, isolated);
    }
    const info = await session.legacy.evaluate(() => { const PM = (window as any).PM; return { w: PM.proj.w, h: PM.proj.h, dur: PM.proj.dur, fps: PM.proj.fps, name: PM.proj.name, layers: PM.proj.layers.length, comps: Object.keys(PM.proj.comps || {}).length, assets: Object.keys(PM.proj.assets || {}).length }; });
    record.title = info.name;
    record.covers = [`${info.layers} root layers`, `${info.comps} nested compositions`, `${info.assets} assets`, `${info.w}x${info.h} at ${info.fps} fps, ${info.dur}s`];
    const loaded = await session.legacy.evaluate(() => JSON.stringify((window as any).PM.proj));
    record.loaded = writeFile(out, `${dir}/loaded.json`, loaded + '\n');
    const samples: number[] = [];
    const lastFrame = Math.max(0, info.dur - 1 / info.fps);
    for (let t = 0; t < lastFrame; t += step) samples.push(Math.round(t * 1e6) / 1e6);
    samples.push(Math.round(lastFrame * 1e6) / 1e6);
    record.settle = await session.legacy.evaluate(t => (window as any).__corpus.settle(t, 30000), samples[0]!);
    await captureSamples(session, out, dir, samples, [info.w, info.h], false, evaluate, record, record.id);
    const copyHash = sha256(readFileSync(copy));
    if (copyHash !== originalHash) record.error = `The opened copy changed on disk (${copyHash} vs ${originalHash})`;
  } catch (error) {
    record.error = (error as Error).message.split('\n')[0]!;
  }
  if (sha256(readFileSync(file)) !== originalHash) record.error = `${file} changed during capture`;
  record.pageErrors = session.legacy.pageErrors.slice(errorsBefore);
  record.ms = Math.round(performance.now() - started);
  writeFile(out, `${dir}/project.json`, JSON.stringify({ ...record, file: path.basename(file), fileSha256: originalHash }, null, 1) + '\n');
  return record;
}

export async function capture(options: Options): Promise<string> {
  const head = git('rev-parse', 'HEAD');
  const dirty = git('status', '--porcelain', '--untracked-files=no').length > 0;
  const stamp = new Date().toISOString().replace(/[-:]/g, '').replace(/\..+/, '').replace('T', '-');
  const id = options.id ?? `${options.label.replace(/[^A-Za-z0-9._-]+/g, "-")}-${stamp}-${head.slice(0, 8)}`;
  const out = path.join(options.root, id);
  if (path.resolve(out).startsWith(repoRoot + path.sep)) throw new Error('Refusing to write captured data inside the repository');
  if (existsSync(out)) throw new Error(`${out} already exists; choose another --id`);
  mkdirSync(out, { recursive: true });
  const startedAt = new Date().toISOString();
  const started = performance.now();
  const log = (line: string) => { console.log(line); writeFileSync(path.join(out, 'capture.log'), `${new Date().toISOString()} ${line}\n`, { flag: 'a' }); };
  log(`Capture ${id} into ${out}`);

  const generatedDir = path.join(out, 'generated');
  const corpus = generateCorpus(generatedDir);
  const described = new Map(corpus.projects.map(item => [item.id, item]));
  // Projects that install fixture extensions run last, in their own sessions, so extension
  // hooks (for example a text raster wrapper) never touch the plain projects' frames.
  const extensionKey = (item: CorpusProject) => [...(item.extensions ?? [])].sort().join(',');
  // Today's engine draws level 2 text at level 1: those documents have no
  // legacy reference and go to the native semantic gate (legacy-capture.ts).
  const split = legacyCaptureProjects(options.textOnly ? textOnlyProjects(allProjects()) : allProjects());
  const nativeOnly = split.nativeOnly.map(item => item.id);
  const projects = split.capture
    .filter(item => !options.only.length || options.only.some(prefix => item.id.startsWith(prefix)))
    .map((item, index) => ({ item, index }))
    .sort((a, b) => extensionKey(a.item).localeCompare(extensionKey(b.item)) || a.index - b.index)
    .map(({ item }) => item);
  if (nativeOnly.length) log(`Left to the native semantic gate (level 2 text, no legacy reference): ${nativeOnly.length} projects`);
  const build = buildFingerprint();
  const session = new Session(options, await legacyScript());
  await session.start();
  const environment = await session.legacy.evaluate(() => (window as any).__corpus.environment());
  const appVersions = await session.legacy.evaluateMain(({ app }) => ({ app: app.getVersion(), electron: process.versions.electron, chrome: process.versions.chrome, node: process.versions.node }));
  const extracted = shaderPresets();
  const live = await session.legacy.evaluate(() => (window as any).__corpus.shaderPresets());
  const shaderPresetCheck = Object.keys(extracted).every(name => live[name] === extracted[name]) && Object.keys(live).length === Object.keys(extracted).length;
  log(`App ${appVersions.app}, Electron ${appVersions.electron}, GPU ${environment.renderer}; shader presets ${shaderPresetCheck ? 'match' : 'DIFFER'}`);

  const records: ProjectRecord[] = [];
  try {
    if (options.synthetic) {
      for (const [index, project] of projects.entries()) {
        await session.prepareFor(project.extensions);
        const record = await captureSynthetic(session, out, project, described.get(project.id)!, generatedDir, options.evaluate);
        records.push(record);
        const failed = record.samples.filter(sample => sample.error).length;
        log(`[${index + 1}/${projects.length}] ${project.id}: ${record.samples.length - failed}/${project.samples.length} samples${record.error ? `, ERROR ${record.error}` : ''}${failed ? `, ${failed} failed` : ''}${record.hydrateIssues.length ? `, ${record.hydrateIssues.length} hydrate rewrites` : ''}${record.keyIdRewrites.length ? `, ${record.keyIdRewrites.length} keyframe ids reassigned` : ''}${record.missingDefinitions.length ? `, missing: ${record.missingDefinitions.join('; ')}` : ''} (${record.ms} ms)`);
      }
    }
    let scenarios: Record<string, unknown> | null = null;
    if (options.scenarios) {
      // A fresh app, as the spec gets from its fixture before openEditor().
      try { await session.restart([]); scenarios = await captureScenarios(session, out); log(`Scenarios: ${Object.keys((scenarios as any).hashes).length} images, flatMatches ${(scenarios as any).flatMatches}`); }
      catch (error) { scenarios = { error: (error as Error).message.split('\n')[0] }; log(`Scenarios failed: ${(scenarios as any).error}`); }
    }
    let real: ProjectRecord | null = null;
    if (options.real) {
      if (!existsSync(options.realPath)) log(`Real project ${options.realPath} not found; skipped`);
      else {
        await session.restart([]);
        real = await captureReal(session, out, options.realPath, options.realStep, options.evaluate, options.textOnly);
        log(`Real project ${real.title ?? real.id}: ${real.samples.filter(s => !s.error).length}/${real.samples.length} samples${real.error ? `, ERROR ${real.error}` : ''} (${real.ms} ms)`);
      }
    }
    const all = [...records, ...(real ? [real] : [])];
    const files = all.flatMap(record => [
      ...record.samples.flatMap(sample => [sample.png, sample.alphaPng, sample.float?.data, sample.float?.header, ...(sample.blur ?? []).flatMap(b => [b.float.data, b.float.header]), sample.eval, sample.blender].filter(Boolean) as FileRecord[]),
      ...(record.blender?.assets ?? []),
    ]);
    const manifest = {
      format: 'powermove-engine-corpus', version: 1, id, label: options.label,
      createdAt: startedAt, finishedAt: new Date().toISOString(), durationMs: Math.round(performance.now() - started),
      engine: { commit: head, dirty, branch: git('rev-parse', '--abbrev-ref', 'HEAD'), build },
      app: appVersions, gpu: environment, machine: machine(),
      settings: {
        opaque: 'PM.GL.renderToPixels(t, w, h, {mblur: true, mbSamples: 16, shutter, opaque: true, topDownOpaque: true}) after PM.prepareFrame(t), PM.quality = 1',
        transparent: 'PM.GL.renderToPixels(t, w, h, {transparent: true, mblur: true, mbSamples: 20, shutter}); float readback captured, plus the exporter alphaFrame straight-alpha conversion',
        evaluation: 'every channel from PM.allProps through PM.ev / PM.evP, resolved content, local and world matrices, world opacity, planeMatrix for 3D layers, GL.bounds, text layout, caption bounds, path values, precomp source times (recursive), scene3d.describe()',
        blender: `after the frames, render3d {enabled: true, ...project settings} on every composition with 3D objects; final snapshots from PM.prepareFrame(t) (default options: composition size, 10 shutter samples when motion blur is on) labelled by blenderFrameRequests; export snapshots from PM.prepareFrame(t, {width, height, mblur: true, mbSamples: 16}) and, for transparent projects, exportAlpha with mbSamples 20, labelled the same way; preview snapshots from scene3d.renderFrame() at t; ${'the main-process blender:run handler records jobs and returns an empty render, so Blender never runs'}`,
      },
      shaderPresetCheck,
      generated: { dir: 'generated', projects: corpus.projects.length, media: corpus.media },
      nativeOnly,
      totals: {
        projects: all.length, synthetic: records.length, real: real ? 1 : 0,
        frames: all.reduce((sum, record) => sum + record.samples.filter(sample => sample.png).length, 0),
        floatFrames: all.reduce((sum, record) => sum + record.samples.reduce((n, sample) => n + (sample.float ? 1 : 0) + (sample.blur?.length ?? 0), 0), 0),
        evalDumps: all.reduce((sum, record) => sum + record.samples.filter(sample => sample.eval).length, 0),
        blenderDumps: all.reduce((sum, record) => sum + record.samples.filter(sample => sample.blender).length, 0),
        blenderSnapshots: all.reduce((sum, record) => sum + record.samples.reduce((n, sample) => n + (sample.blenderSnapshots ? sample.blenderSnapshots.final + sample.blenderSnapshots.preview : 0), 0), 0),
        blenderExportSnapshots: all.reduce((sum, record) => sum + record.samples.reduce((n, sample) => n + (sample.blenderSnapshots?.export ?? 0) + (sample.blenderSnapshots?.exportAlpha ?? 0), 0), 0),
        projectsWithBlenderProblems: all.filter(record => record.blender?.problems.length).map(record => record.id),
        failedSamples: all.reduce((sum, record) => sum + record.samples.filter(sample => sample.error).length, 0),
        failedProjects: all.filter(record => record.error).map(record => ({ id: record.id, error: record.error })),
        projectsWithGlErrors: all.filter(record => record.samples.some(sample => sample.glErrors.length)).map(record => record.id),
        projectsWithPageErrors: all.filter(record => record.pageErrors.length).map(record => record.id),
        projectsWithHydrateRewrites: all.filter(record => record.hydrateIssues.length).map(record => record.id),
        projectsWithKeyIdRewrites: all.filter(record => record.keyIdRewrites.length).map(record => record.id),
        unsettled: all.filter(record => record.settle && !record.settle.stable).map(record => record.id),
        bytes: files.reduce((sum, file) => sum + file.bytes, 0),
      },
      projects: all.map(record => ({
        id: record.id, source: record.source, group: record.group, title: record.title, covers: record.covers,
        dir: record.source === 'real' ? `real/${record.id.slice(5)}` : `projects/${record.id}`,
        document: record.document, loaded: record.loaded, media: record.media, error: record.error,
        extensions: record.extensions, extensionChecks: record.extensionChecks, hydrateIssues: record.hydrateIssues, keyIdRewrites: record.keyIdRewrites.length, missingDefinitions: record.missingDefinitions, mediaPending: record.mediaPending, settle: record.settle,
        blender: record.blender,
        samples: record.samples.map(sample => ({ time: sample.time, width: sample.width, height: sample.height, png: sample.png, alphaPng: sample.alphaPng, float: sample.float, blur: sample.blur, eval: sample.eval, blender: sample.blender, blenderSnapshots: sample.blenderSnapshots, blenderError: sample.blenderError, glErrors: sample.glErrors, warnings: sample.warnings, prepareError: sample.prepareError, error: sample.error, ms: sample.ms })),
        pageErrors: record.pageErrors.length,
      })),
      scenarios,
    };
    writeFile(out, 'manifest.json', JSON.stringify(manifest, null, 1) + '\n');
    const verified = verifyCapture(out);
    for (const problem of verified.problems) log(`verify: ${problem}`);
    for (const file of verified.unlisted) log(`verify: unlisted ${file}`);
    log(`Verified ${verified.checked} files: ${verified.problems.length} problems, ${verified.unlisted.length} unlisted`);
    log(`Done: ${manifest.totals.frames} frames, ${manifest.totals.floatFrames} float frames, ${manifest.totals.evalDumps} evaluation dumps, ${(manifest.totals.bytes / 1e6).toFixed(1)} MB, ${manifest.totals.failedSamples} failed samples`);
  } finally {
    await session.legacy.close();
  }
  return out;
}

if (import.meta.main) {
  capture(parseArgs(process.argv.slice(2))).then(out => { console.log(out); process.exit(0); }, error => { console.error(error); process.exit(1); });
}
