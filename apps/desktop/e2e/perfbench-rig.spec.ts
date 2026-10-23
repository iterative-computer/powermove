/*
 * The 120 Hz frame rig (docs/engine/perfbench.md, "The 120 Hz rig"): today's
 * viewer (Use New Engine off) and the engine (on), driven through the same
 * deterministic sequence of frames, one per 120 Hz refresh (8.33 ms budget).
 *
 * Chromium's begin-frame control (HeadlessExperimental.beginFrame with
 * --enable-begin-frame-control) exists only in headless Chromium; Electron's
 * windowed renderer refuses the domain (the rig records the refusal). So the
 * rig steps a virtual clock instead: step k asks for the frame of refresh k
 * (a playback step sets the time to k/120 s on a 120 fps composition; a drag
 * step moves a layer one pixel) and waits until that frame is presented
 * (`preview:presented`, which both paths emit). Each step records:
 *
 * - the page's main-thread work in the step (CDP Performance.getMetrics
 *   TaskDuration, minus the rig's own calibrated round trip),
 * - the engine's render-thread work for the step (its stats: each frame's
 *   span from the start of its compose to its GPU completion, the compose's
 *   CPU within it, and the pm-perf work counters),
 * - the page's messages posted and received and their structured-clone bytes
 *   (estimated by walking the payload), and today's WebGL calls (draws,
 *   framebuffer binds, uploads and their bytes, readbacks, programs, shader
 *   compiles) through prototype hooks installed before the app boots,
 * - the step's wall time from the request to the presentation.
 *
 * A step fits the budget when its main-thread work and (engine) the
 * render thread's frame time (compose through GPU completion) are each within
 * 8.33 ms; frames presented, frames over
 * budget ("dropped" at 120 Hz), and per-step distributions are reported.
 *
 * Fixtures: the synthetic project (public, generated in the page) and, when
 * $PM_PERFBENCH_PRIVATE names a manifest outside the repository, the
 * projects it lists (`{ "projects": [{ "label", "file", "extensions"?, "from"?, "steps"? }] }`;
 * labels only are reported). Nothing private is written to the repository.
 *
 * Opt in (heavy; one Electron at a time):
 *   PM_PERFBENCH_RIG=1 pm-electron-lock bunx playwright test e2e/perfbench-rig.spec.ts --workers=1
 * Results go to $PM_PERFBENCH_RIG_OUT (JSON lines; default /tmp/powermove-perfbench-rig.jsonl).
 */
import { cp, mkdtemp, readFile, appendFile, rm } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import type { CDPSession, Page } from 'playwright';

import { buildSyntheticProject } from '../src/main/engine/integration/synthetic-project';
import { expect, launchApp, test, type LaunchedApp } from './helpers/app';

test.skip(!process.env.PM_PERFBENCH_RIG, 'the 120 Hz rig runs on request (PM_PERFBENCH_RIG=1)');

const OUT = process.env.PM_PERFBENCH_RIG_OUT ?? path.join(os.tmpdir(), 'powermove-perfbench-rig.jsonl');
const STEPS = Number(process.env.PM_PERFBENCH_RIG_STEPS ?? 240);
const BUDGET_MS = 1000 / 120;
type Mode = 'today' | 'engine';
const ENV: Record<Mode, Record<string, string>> = { today: { POWERMOVE_NEW_ENGINE: '0' }, engine: { POWERMOVE_NEW_ENGINE: '1' } };

/** Page hooks: WebGL calls, messages and their clone bytes. Installed before the app's scripts run. */
function installHooks(): void {
  const w = window as any;
  if (w.__pmRig) return;
  const c: Record<string, number> = {};
  const add = (k: string, n = 1) => { c[k] = (c[k] ?? 0) + n; };
  /** Structured-clone bytes, estimated: typed data by length, strings at 2 bytes a code unit, 8 per number. */
  const bytes = (v: unknown, depth = 0, seen = new Set<unknown>()): number => {
    if (v == null) return 1;
    switch (typeof v) {
      case 'number': case 'bigint': return 8;
      case 'boolean': return 1;
      case 'string': return 4 + (v as string).length * 2;
      case 'object': {
        if (seen.has(v) || depth > 12) return 4;
        seen.add(v);
        if (v instanceof ArrayBuffer) return v.byteLength;
        if (ArrayBuffer.isView(v)) return (v as ArrayBufferView).byteLength;
        if (typeof ImageBitmap !== 'undefined' && v instanceof ImageBitmap) return v.width * v.height * 4;
        if (typeof MessagePort !== 'undefined' && v instanceof MessagePort) return 8;
        let n = 8;
        if (Array.isArray(v)) for (const x of v) n += bytes(x, depth + 1, seen);
        else if (v instanceof Map) for (const [k, x] of v) n += bytes(k, depth + 1, seen) + bytes(x, depth + 1, seen);
        else for (const k of Object.keys(v as object)) n += 4 + k.length * 2 + bytes((v as any)[k], depth + 1, seen);
        return n;
      }
      default: return 0;
    }
  };
  const wrapSend = (proto: any, name: string) => {
    if (!proto?.postMessage) return;
    const orig = proto.postMessage;
    proto.postMessage = function (this: unknown, data: unknown, ...rest: unknown[]) {
      add(`page.messages_posted`); add(`page.posted_${name}`); add('page.clone_bytes_posted', bytes(data));
      return orig.call(this, data, ...rest);
    };
  };
  wrapSend(typeof MessagePort !== 'undefined' ? MessagePort.prototype : null, 'port');
  wrapSend(typeof Worker !== 'undefined' ? Worker.prototype : null, 'worker');
  wrapSend(typeof BroadcastChannel !== 'undefined' ? BroadcastChannel.prototype : null, 'broadcast');
  const received = (e: Event) => { add('page.messages_received'); add('page.clone_bytes_received', bytes((e as MessageEvent).data)); };
  // Receivers: listeners added and onmessage handlers set on ports, workers and channels.
  for (const proto of [typeof MessagePort !== 'undefined' ? MessagePort.prototype : null, typeof Worker !== 'undefined' ? Worker.prototype : null, typeof BroadcastChannel !== 'undefined' ? BroadcastChannel.prototype : null]) {
    if (!proto) continue;
    const addListener = proto.addEventListener;
    const counted = new WeakSet<object>();
    proto.addEventListener = function (this: any, type: string, fn: any, opts?: any) {
      if (type === 'message' && !counted.has(this)) { counted.add(this); addListener.call(this, 'message', received); }
      return addListener.call(this, type, fn, opts);
    };
    const desc = Object.getOwnPropertyDescriptor(proto, 'onmessage');
    if (desc?.set && desc.get) {
      Object.defineProperty(proto, 'onmessage', {
        configurable: true,
        get() { return desc.get!.call(this); },
        set(fn: any) {
          if (!counted.has(this)) { counted.add(this); addListener.call(this, 'message', received); }
          desc.set!.call(this, fn);
        }
      });
    }
  }
  window.addEventListener('message', received, true);
  // WebGL: today's renderer.
  const gl = (typeof WebGL2RenderingContext !== 'undefined' ? WebGL2RenderingContext.prototype : null) as any;
  if (gl) {
    const hook = (name: string, key: string, size?: (args: any[]) => number) => {
      const orig = gl[name];
      if (typeof orig !== 'function') return;
      gl[name] = function (this: unknown, ...args: any[]) {
        add(key);
        if (size) add(`${key}_bytes`, size(args));
        return orig.apply(this, args);
      };
    };
    const pixels = (args: any[]) => {
      for (const a of args) {
        if (ArrayBuffer.isView(a)) return a.byteLength;
        if (a && typeof a === 'object' && 'width' in a && 'height' in a) return (a.videoWidth || a.width) * (a.videoHeight || a.height) * 4;
      }
      return 0;
    };
    for (const n of ['drawArrays', 'drawElements', 'drawArraysInstanced', 'drawElementsInstanced', 'drawRangeElements']) hook(n, 'gl.draw_calls');
    hook('bindFramebuffer', 'gl.framebuffer_binds');
    hook('useProgram', 'gl.program_binds');
    for (const n of ['texImage2D', 'texSubImage2D', 'texImage3D', 'texSubImage3D', 'bufferData', 'bufferSubData']) hook(n, 'gl.uploads', pixels);
    hook('readPixels', 'gl.readbacks', pixels);
    for (const n of ['createTexture', 'createBuffer', 'createFramebuffer', 'createRenderbuffer']) hook(n, 'gl.allocations');
    hook('createProgram', 'gl.programs');
    hook('compileShader', 'gl.shader_compiles');
    hook('linkProgram', 'gl.program_links');
    for (const n of ['uniform1f', 'uniform1i', 'uniform2f', 'uniform3f', 'uniform4f', 'uniform1fv', 'uniform2fv', 'uniform3fv', 'uniform4fv', 'uniformMatrix3fv', 'uniformMatrix4fv']) hook(n, 'gl.uniform_sets');
  }
  w.__pmRig = { counts: () => ({ ...c }) };
}

type Metrics = Record<string, number>;
async function metrics(cdp: CDPSession): Promise<Metrics> {
  const { metrics } = await cdp.send('Performance.getMetrics');
  return Object.fromEntries(metrics.map((m: { name: string; value: number }) => [m.name, m.value]));
}

const diff = (a: Record<string, number>, b: Record<string, number>): Record<string, number> => {
  const out: Record<string, number> = {};
  for (const k of new Set([...Object.keys(a), ...Object.keys(b)])) {
    const d = (b[k] ?? 0) - (a[k] ?? 0);
    if (d) out[k] = d;
  }
  return out;
};

async function engineReport(session: LaunchedApp): Promise<any> {
  return session.app.evaluate(async () => {
    const report = await (globalThis as any).__powermoveNativeEngine?.engine?.report?.('interactive');
    return report?.presenter ?? null;
  });
}

/** One step in the page: perform the step's request and wait for its presentation. */
type StepSpec = { kind: 'time'; t: number } | { kind: 'move'; layer: string; x: number; t: number };
function pageStep(spec: StepSpec): Promise<{ ms: number | null; counts: Record<string, number> }> {
  const PM = (window as any).PM;
  const rig = (window as any).__pmRig;
  return new Promise((resolve) => {
    let done = false;
    let wantVersion = -Infinity;
    const finish = (ms: number | null) => {
      if (done) return;
      done = true;
      off?.();
      resolve({ ms, counts: rig.counts() });
    };
    const t0 = performance.now();
    const off = PM.bus.on('preview:presented', (e: any) => {
      if (Math.abs(Number(e?.time) - spec.t) > 1e-6) return;
      if (spec.kind === 'move' && !(Number(e?.version ?? -1) >= wantVersion)) return;
      finish(performance.now() - t0);
    });
    if (spec.kind === 'time') PM.setTime(spec.t);
    else {
      PM.Edit.apply({ type: 'set_property', target: spec.layer, path: 'position.x', value: spec.x }, { label: 'Move', origin: 'viewer' });
      wantVersion = PM.animVersion?.() ?? 0;
      PM.invalidate?.();
    }
    setTimeout(() => finish(null), 5000);
  });
}

/**
 * The engine's numbers per step, from its stats: `engineComposeMs`, the
 * compose's CPU (planning and encoding); `engineFrameMs`, each frame from
 * the start of its compose until its GPU work finished (the stats'
 * `gpuUsTotal`, which includes the compose: the render thread's time for
 * the frame); `engineFrames`, frames drawn in the step.
 */
type StepRecord = { ms: number | null; mainMs: number; engineComposeMs: number; engineFrameMs: number; engineFrames: number; page: Record<string, number>; work: Record<string, number> };

function dist(xs: number[]): { n: number; p50?: number; p95?: number; p99?: number; max?: number } {
  if (!xs.length) return { n: 0 };
  const s = [...xs].sort((a, b) => a - b);
  const at = (p: number) => +s[Math.min(s.length - 1, Math.floor((s.length - 1) * p + 1e-9))]!.toFixed(3);
  return { n: s.length, p50: at(0.5), p95: at(0.95), p99: at(0.99), max: at(1) };
}

function sum(records: Record<string, number>[]): Record<string, number> {
  const out: Record<string, number> = {};
  for (const r of records) for (const [k, v] of Object.entries(r)) out[k] = (out[k] ?? 0) + v;
  return out;
}

async function runSteps(session: LaunchedApp, mode: Mode, specs: StepSpec[]): Promise<{ steps: StepRecord[]; calibrationMs: number; beginFrame: string }> {
  const { page } = session;
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Performance.enable', { timeDomain: 'timeTicks' });
  // Chromium's deterministic begin frames: refused outside headless Chromium.
  let beginFrame = 'available';
  try {
    await cdp.send('HeadlessExperimental.enable' as any);
  } catch (e) {
    beginFrame = `refused: ${String((e as Error).message ?? e).slice(0, 160)}`;
  }
  // The rig's own round trip on the main thread (a no-op evaluate), subtracted per step.
  const cal: number[] = [];
  for (let i = 0; i < 30; i++) {
    const a = await metrics(cdp);
    await page.evaluate(() => (window as any).__pmRig.counts());
    const b = await metrics(cdp);
    cal.push((b.TaskDuration! - a.TaskDuration!) * 1000);
  }
  const calibrationMs = dist(cal).p50 ?? 0;
  // Warm-up: the first frames build caches on both paths.
  for (const spec of specs.slice(0, 4)) await stage('warm-up frame', page.evaluate(pageStep, spec), 60_000);
  const steps: StepRecord[] = [];
  // $PM_PERFBENCH_RIG_PROFILE=<dir>: a V8 CPU profile of the page over the
  // measured steps (scripts/cpuprofile-summary.mjs summarizes it).
  const profileDir = process.env.PM_PERFBENCH_RIG_PROFILE;
  if (profileDir) {
    await cdp.send('Profiler.enable');
    await cdp.send('Profiler.setSamplingInterval', { interval: 250 });
    await cdp.send('Profiler.start');
  }
  // $PM_PERFBENCH_RIG_TRACE=<dir>: a Chromium trace of the page over the
  // measured steps (layout, style, paint, and what invalidated layout, with
  // stacks), for finding forced layouts.
  const traceDir = process.env.PM_PERFBENCH_RIG_TRACE;
  if (traceDir) {
    await cdp.send('Tracing.start', {
      transferMode: 'ReturnAsStream',
      traceConfig: {
        includedCategories: ['devtools.timeline', 'disabled-by-default-devtools.timeline', 'disabled-by-default-devtools.timeline.invalidationTracking', 'disabled-by-default-devtools.timeline.stack', 'blink.user_timing', 'v8.execute']
      }
    } as any);
  }
  let report = mode === 'engine' ? await engineReport(session) : null;
  let counts = await page.evaluate(() => (window as any).__pmRig.counts());
  for (const spec of specs) {
    const m0 = await metrics(cdp);
    const r = await stage('frame', page.evaluate(pageStep, spec), 30_000);
    const m1 = await metrics(cdp);
    const next = mode === 'engine' ? await engineReport(session) : null;
    steps.push({
      ms: r.ms,
      mainMs: Math.max(0, (m1.TaskDuration! - m0.TaskDuration!) * 1000 - calibrationMs),
      engineComposeMs: next && report ? (next.composeUsTotal - report.composeUsTotal) / 1000 : 0,
      engineFrameMs: next && report ? (next.gpuUsTotal - report.gpuUsTotal) / 1000 : 0,
      engineFrames: next && report ? next.frames - report.frames : 0,
      page: diff(counts, r.counts),
      work: next && report ? diff(report.work ?? {}, next.work ?? {}) : {}
    });
    counts = r.counts;
    report = next;
  }
  if (profileDir) {
    const { profile } = await cdp.send('Profiler.stop');
    const { writeFile, mkdir } = await import('node:fs/promises');
    await mkdir(profileDir, { recursive: true });
    await writeFile(path.join(profileDir, `rig-${mode}-${Date.now()}.cpuprofile`), JSON.stringify(profile));
  }
  if (traceDir) {
    const { writeFile, mkdir } = await import('node:fs/promises');
    const complete = new Promise<string>((resolve) => cdp.once('Tracing.tracingComplete' as any, (e: any) => resolve(e.stream)));
    await cdp.send('Tracing.end');
    const stream = await complete;
    const parts: string[] = [];
    for (;;) {
      const chunk: any = await cdp.send('IO.read' as any, { handle: stream, size: 1 << 20 } as any);
      parts.push(chunk.base64Encoded ? Buffer.from(chunk.data, 'base64').toString('utf8') : chunk.data);
      if (chunk.eof) break;
    }
    await cdp.send('IO.close' as any, { handle: stream } as any);
    await mkdir(traceDir, { recursive: true });
    await writeFile(path.join(traceDir, `rig-${mode}-${Date.now()}.trace.json`), parts.join(''));
  }
  await cdp.detach().catch(() => undefined);
  return { steps, calibrationMs, beginFrame };
}

function summarize(row: string, mode: Mode, run: { steps: StepRecord[]; calibrationMs: number; beginFrame: string }, extra: Record<string, unknown>) {
  const s = run.steps;
  const presented = s.filter((x) => x.ms !== null);
  // The render thread's time is the frame's span from its compose to its
  // GPU completion (`engineFrameMs`). Before 2026-10-23 the rig added the
  // compose CPU to that span a second time (`engineRenderMsDoubleCounted`,
  // kept for comparison with older rows).
  const fits = s.filter((x) => x.ms !== null && x.mainMs <= BUDGET_MS && x.engineFrameMs <= BUDGET_MS);
  const fitsDoubleCounted = s.filter((x) => x.ms !== null && x.mainMs <= BUDGET_MS && x.engineComposeMs + x.engineFrameMs <= BUDGET_MS);
  const totals = sum(s.map((x) => x.page));
  const work = sum(s.map((x) => x.work));
  const per = (o: Record<string, number>) => Object.fromEntries(Object.entries(o).map(([k, v]) => [k, +(v / Math.max(1, s.length)).toFixed(2)]));
  return {
    row,
    mode,
    rateHz: 120,
    budgetMs: +BUDGET_MS.toFixed(3),
    steps: s.length,
    presented: presented.length,
    fitBudget: fits.length,
    overBudget: s.length - fits.length,
    notPresented: s.length - presented.length,
    mainThreadMs: dist(s.map((x) => x.mainMs)),
    engineRenderMs: mode === 'engine' ? dist(s.map((x) => x.engineFrameMs)) : null,
    engineComposeMs: mode === 'engine' ? dist(s.map((x) => x.engineComposeMs)) : null,
    engineFramesPerStep: mode === 'engine' ? dist(s.map((x) => x.engineFrames)) : null,
    engineRenderMsDoubleCounted: mode === 'engine' ? dist(s.map((x) => x.engineComposeMs + x.engineFrameMs)) : null,
    fitBudgetDoubleCounted: mode === 'engine' ? fitsDoubleCounted.length : null,
    stepLatencyMs: dist(presented.map((x) => x.ms!)),
    pagePerFrame: per(totals),
    enginePerFrame: per(work),
    calibrationMs: run.calibrationMs,
    beginFrameControl: run.beginFrame,
    load: os.loadavg()[0],
    ...extra
  };
}

/** `promise`, or a failure naming the stage that did not finish in `ms`. */
async function stage<T>(label: string, promise: Promise<T>, ms: number): Promise<T> {
  let timer: NodeJS.Timeout | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error(`stage timed out after ${ms} ms: ${label}`)), ms); })
    ]);
  } finally {
    clearTimeout(timer);
  }
}

async function record(value: Record<string, unknown>): Promise<void> {
  await appendFile(OUT, `${JSON.stringify({ at: new Date().toISOString(), ...value })}\n`);
  console.log(`[rig] ${JSON.stringify(value)}`);
}

/** Launches with the hooks installed before the app's scripts (an init script and a reload). */
async function launch(mode: Mode): Promise<LaunchedApp> {
  const session = await launchApp({ env: ENV[mode] });
  await session.page.context().addInitScript(installHooks);
  await session.page.reload();
  await session.page.waitForFunction(() => Boolean((window as any).__pmRig && (window as any).PM?.ProjectsScreen));
  return session;
}

/** With the engine on: waits until the viewer's session routes to it and presents; returns the route it settled on otherwise. */
async function waitRoute(page: Page, mode: Mode): Promise<{ route: string; reason: string | null; failures: unknown[] } | null> {
  if (mode !== 'engine') return null;
  const read = () => stage('engine status', page.evaluate(() => {
    const s = (window as any).PM.EngineConnection.status();
    return { route: String(s.route), reason: s.routeReason ?? null, failures: s.failures ?? [], presenting: !!(window as any).PM.Kernel.services.get('viewer').enginePresentation?.presenting };
  }), 20_000);
  const deadline = Date.now() + 120_000;
  let last = await read();
  while (Date.now() < deadline && !(last.route === 'engine' && last.presenting)) {
    await new Promise((r) => setTimeout(r, 500));
    last = await read();
  }
  return { route: last.presenting ? last.route : `${last.route} (not presenting)`, reason: last.reason, failures: last.failures };
}

async function synthetic(session: LaunchedApp, mode: Mode, layers: number, fps: number): Promise<any> {
  await session.openEditor();
  const summary = await session.page.evaluate(buildSyntheticProject, { layers, fps, dur: 10 });
  await session.page.evaluate(() => { const PM = (window as any).PM; PM.pause?.(); PM.setTime(0); });
  const route = await waitRoute(session.page, mode);
  expect(route?.route ?? 'engine', JSON.stringify(route)).toBe('engine');
  return summary;
}

const playbackSpecs = (from: number): StepSpec[] => Array.from({ length: STEPS + 4 }, (_, k) => ({ kind: 'time', t: from + (k + 1) / 120 }));

test.describe('120 Hz rig', () => {
  // `edit`: one layer moved a pixel per frame through PM.Edit.apply (an
  // inspector edit, with the project's provenance), paused.
  for (const scenario of ['playback', 'edit'] as const) {
    test(`synthetic 500 layers: ${scenario}, today and the engine`, async () => {
      test.setTimeout(900_000);
      for (const mode of ['today', 'engine'] as Mode[]) {
        const session = await launch(mode);
        try {
          const summary = await synthetic(session, mode, 500, 120);
          let specs: StepSpec[];
          if (scenario === 'playback') specs = playbackSpecs(0);
          else {
            const layer = await session.page.evaluate(() => {
              const PM = (window as any).PM;
              const solids = PM.proj.layers.filter((l: any) => l.type === 'solid');
              return solids[Math.min(150, solids.length - 1)].id;
            });
            const t = 1;
            await session.page.evaluate((t) => (window as any).PM.setTime(t), t);
            specs = Array.from({ length: STEPS + 4 }, (_, k) => ({ kind: 'move', layer, x: 100 + k, t }));
          }
          const run = await runSteps(session, mode, specs);
          await record(summarize(`synthetic-500-${scenario}`, mode, run, { fixture: summary }));
          expect(run.steps.length).toBe(STEPS + 4);
        } finally {
          await session.close();
        }
      }
    });
  }

  test('private projects from the runtime manifest: playback, today and the engine', async () => {
    const manifest = process.env.PM_PERFBENCH_PRIVATE;
    test.skip(!manifest || !existsSync(manifest), 'no private manifest ($PM_PERFBENCH_PRIVATE)');
    test.setTimeout(7_200_000);
    const list = JSON.parse(await readFile(manifest!, 'utf8')).projects as { label: string; file: string; extensions?: string; from?: number; steps?: number }[];
    for (const p of list) {
      for (const mode of ['today', 'engine'] as Mode[]) {
        const scratch = await mkdtemp(path.join(os.tmpdir(), 'pm-rig-private-'));
        try {
          const copy = path.join(scratch, 'project.pmv');
          await cp(p.file, copy);
          const session = await launch(mode);
          try {
            if (p.extensions) await cp(p.extensions, path.join(session.userData, 'extensions'), { recursive: true });
            if (p.extensions) {
              await session.relaunch();
              await session.page.context().addInitScript(installHooks);
              await session.page.reload();
              await session.page.waitForFunction(() => Boolean((window as any).__pmRig && (window as any).PM?.ProjectsScreen));
            }
            await session.app.evaluate(({ dialog }, filePath) => { dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] }); }, copy);
            const page = session.page;
            let at = 'open';
            try {
              await stage('the open API', page.waitForFunction(() => Boolean((window as any).PM?.openProject && (window as any).PM?.Kernel?.loader)), 120_000);
              await stage('builtins', page.evaluate(async () => { await (window as any).PM.Kernel.loader.builtinsReady; }), 120_000);
              // Not awaited: an open that asks a question (PM.confirm) never resolves by itself.
              void page.evaluate(() => (window as any).PM.openProject()).catch(() => undefined);
              at = 'editor';
              await stage('the editor opens', page.waitForFunction(() => !(window as any).PM.ProjectsScreen?.isOpen && (window as any).PM.proj?.layers?.length > 0, undefined, { timeout: 0 }), 900_000);
              at = 'assets';
              await stage('project assets', page.evaluate(async () => { const PM = (window as any).PM; await PM.projectAssetsReady; PM.pause?.(); }), 900_000);
              at = 'route';
              const route = await waitRoute(page, mode);
              if (route && route.route !== 'engine') {
                await record({ row: `private-${p.label}-playback`, mode, routed: route, steps: 0, load: os.loadavg()[0] });
                continue;
              }
              at = 'frames';
              const from = p.from ?? 0;
              const steps = p.steps ?? STEPS;
              const specs: StepSpec[] = Array.from({ length: steps + 4 }, (_, k) => ({ kind: 'time', t: from + (k + 1) / 120 }));
              // Presented times follow the composition's own frame grid: wait for the frame of each step's time.
              const comp = await page.evaluate(() => { const PM = (window as any).PM; return { fps: PM.comp?.()?.fps ?? PM.proj.fps, layers: PM.proj.layers.length }; });
              const snapped = specs.map((s) => ({ ...s, t: Math.floor(s.t * comp.fps + 1e-9) / comp.fps }));
              // Only refreshes that change the composition frame present a new frame.
              const distinct = snapped.filter((s, i) => i === 0 || s.t !== snapped[i - 1]!.t);
              const run = await runSteps(session, mode, distinct);
              await record(summarize(`private-${p.label}-playback`, mode, run, { fps: comp.fps, layers: comp.layers, refreshes: steps, route }));
            } catch (e) {
              // What the page showed when a stage stalled (a dialog, the route), for the report.
              const state = await stage('diagnostics', page.evaluate(() => {
                const PM = (window as any).PM;
                const dialogs = [...document.querySelectorAll('dialog[open], [role="dialog"], [role="alertdialog"]')].map((d) => (d.textContent ?? '').trim().slice(0, 200));
                let engine: unknown = null;
                try { engine = PM?.EngineConnection?.status?.(); } catch { engine = 'status threw'; }
                return { projectsOpen: !!PM?.ProjectsScreen?.isOpen, layers: PM?.proj?.layers?.length ?? null, dialogs, engine: engine && typeof engine === 'object' ? { route: (engine as any).route, reason: (engine as any).routeReason, failures: (engine as any).failures?.length } : engine };
              }), 15_000).catch((err) => ({ unreadable: String(err) }));
              await record({ row: `private-${p.label}-playback`, mode, failedAt: at, error: String((e as Error).message ?? e).slice(0, 300), state, load: os.loadavg()[0] });
            }
          } finally {
            await session.close();
          }
        } finally {
          await rm(scratch, { recursive: true, force: true });
        }
      }
    }
  });
});
