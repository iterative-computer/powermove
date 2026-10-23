/*
 * The 120 Hz frame rig (docs/engine/perfbench.md, "The 120 Hz rig"): today's
 * viewer (Use New Engine off) and the engine (on), driven through the same
 * deterministic sequence of frames, one per 120 Hz refresh (8.33 ms budget),
 * and the app-level twins of the perfbench benches (scrub 24 seeks, typing
 * 60 keys, opening the large project, exporting 120 frames) on both.
 *
 * Chromium's begin-frame control (HeadlessExperimental.beginFrame with
 * --enable-begin-frame-control) exists only in headless Chromium; Electron's
 * windowed renderer refuses the domain (the rig records the refusal). So the
 * stepped rows step a virtual clock instead: step k asks for the frame of
 * refresh k (a playback step seeks, paused, to k/120 s on a 120 fps
 * composition; an edit step applies one edit) and waits until that frame is
 * GPU-complete and reaches the next animation frame. The old, unequal
 * endpoints are retained as diagnostic columns. Each step records:
 *
 * - the page's main-thread work in the step (CDP Performance.getMetrics
 *   TaskDuration, minus the rig's own calibrated round trip), its forced
 *   layouts and style recalculations (LayoutCount, RecalcStyleCount), and
 *   the main-thread tasks begun between the request and the presentation
 *   (a `toplevel` trace: RunTask events on the page's CrRendererMain);
 * - the engine's render-thread time for the step from its stats: compose
 *   CPU (`composeUsTotal`), and compose start to GPU done (`gpuUsTotal`,
 *   which already contains the compose: `pm-node` `record_frame`), whose
 *   difference is the GPU-only time; and its pm-perf work counters;
 * - today's WebGL GPU time for the step when the context offers
 *   EXT_disjoint_timer_query_webgl2 (a TIME_ELAPSED query from the request to
 *   the presentation; only on projects without shader or effect layers, which
 *   the app times itself);
 * - the page's messages posted and received and their structured-clone bytes
 *   (estimated by walking the payload), and today's WebGL calls (draws,
 *   framebuffer binds, uploads and their bytes, readbacks, programs, shader
 *   compiles) through prototype hooks installed before the app boots;
 * - the step's wall time from the request to the presentation.
 *
 * A step fits the budget when it was presented and every measured stage is
 * within 8.33 ms: the page's main thread, and (engine) the compose CPU and the
 * GPU-only time, or (today) the WebGL GPU time where measured. The engine rows
 * also report `spanFit`: compose start to GPU done within 8.33 ms (no
 * overlap of CPU and GPU across frames). Today's rows without a WebGL timer
 * count the main thread only and say so (`todayGpu`).
 *
 * The `playing` row is real playback (PM.play) for a fixed wall time: frames
 * presented, gaps between presentations, the page's animation-frame rate
 * (the hidden window's display link), and per presented frame the main
 * thread and the engine's render thread.
 *
 * Fixtures: the synthetic project (public, generated in the page), the
 * public e2e clip, and, when $PM_PERFBENCH_PRIVATE names a manifest outside
 * the repository, the projects it lists
 * (`{ "projects": [{ "label", "file", "extensions"?, "from"?, "steps"? }] }`;
 * labels only are reported). Nothing private is written to the repository.
 *
 * Opt in (heavy; one Electron at a time):
 *   PM_PERFBENCH_RIG=1 pm-electron-lock bunx playwright test e2e/perfbench-rig.spec.ts --workers=1
 * Results go to $PM_PERFBENCH_RIG_OUT (JSON lines; default /tmp/powermove-perfbench-rig.jsonl).
 */
import { execFileSync } from 'node:child_process';
import { cp, mkdtemp, readFile, appendFile, rm } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import type { CDPSession, Page } from 'playwright';

import { buildSyntheticProject } from '../src/main/engine/integration/synthetic-project';
import { expect, launchApp, test, type LaunchedApp } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

test.skip(!process.env.PM_PERFBENCH_RIG, 'the 120 Hz rig runs on request (PM_PERFBENCH_RIG=1)');

const OUT = process.env.PM_PERFBENCH_RIG_OUT ?? path.join(os.tmpdir(), 'powermove-perfbench-rig.jsonl');
const STEPS = Number(process.env.PM_PERFBENCH_RIG_STEPS ?? 240);
const BUDGET_MS = 1000 / 120;
/** pm-protocol ticks per second. */
const TICKS = 705_600_000;
const repoRoot = path.resolve(__dirname, '..');
type Mode = 'today' | 'engine';
const ENV: Record<Mode, Record<string, string>> = { today: { POWERMOVE_NEW_ENGINE: '0' }, engine: { POWERMOVE_NEW_ENGINE: '1' } };
const MODES: Mode[] = ['today', 'engine'];

/** Page hooks: WebGL calls, messages and their clone bytes, media seeks. Installed before the app's scripts run. */
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
  // Completed media seeks (today's viewer draws a video from its element): time and when.
  const seeks: { time: number; at: number }[] = [];
  const watched = new WeakSet<HTMLMediaElement>();
  const ct = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'currentTime');
  if (ct?.get && ct.set) {
    Object.defineProperty(HTMLMediaElement.prototype, 'currentTime', {
      configurable: true,
      get() { return ct.get!.call(this); },
      set(value: number) {
        if (!watched.has(this)) {
          watched.add(this);
          this.addEventListener('seeked', () => seeks.push({ time: ct.get!.call(this), at: performance.now() }));
        }
        ct.set!.call(this, value);
      }
    });
  }
  // Test-only endpoint: a WebGL fence after the last draw, then the next
  // page rAF. Poll without rAF so polling does not insert a second refresh.
  const afterWebGL = (complete: (gpuDoneAt: number, shownAt: number) => void) => {
    const gl: WebGL2RenderingContext = w.PM.GL.gl;
    const sync = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0);
    if (!sync) throw new Error('rig: WebGL fenceSync failed');
    let timer: ReturnType<typeof setTimeout> | undefined;
    let raf = 0;
    const cancel = () => { clearTimeout(timer); cancelAnimationFrame(raf); gl.deleteSync(sync); };
    const poll = () => {
      const status = gl.clientWaitSync(sync, 0, 0);
      if (status === gl.WAIT_FAILED) { cancel(); throw new Error('rig: WebGL fence wait failed'); }
      if (status === gl.TIMEOUT_EXPIRED) { timer = setTimeout(poll, 1); return; }
      const gpuDoneAt = performance.now();
      raf = requestAnimationFrame(() => { cancel(); complete(gpuDoneAt, performance.now()); });
    };
    gl.flush();
    timer = setTimeout(poll, 0);
    return cancel;
  };
  // onPresented identifies the drawn frame; onDone(seq) acknowledges the
  // page's WebGPU workDone, including when drawing lives in the worker.
  // Observe new presenters too (open can replace the session). Cold reveal
  // may call onPresented AFTER onDone, so remember completions by sequence.
  const watchEngine = (draw: (frame: any) => void, complete: (frame: any, gpuDoneAt: number) => void) => {
    const presentation = w.PM.Kernel.services.get('viewer').enginePresentation;
    const done = new Map<number, number>();
    const pending = new Map<number, any>();
    let offDone = () => {};
    const attach = () => {
      offDone(); done.clear(); pending.clear();
      if (!presentation.presenter?.onDone) throw new Error('rig: engine draw completion unavailable');
      offDone = presentation.presenter.onDone((seq: number) => {
        const at = performance.now();
        done.set(seq, at);
        const frame = pending.get(seq);
        if (frame) { pending.delete(seq); complete(frame, at); }
        // Playback is bounded, but keep only recent cold-reveal completions.
        if (done.size > 512) done.delete(done.keys().next().value!);
      });
    };
    const start = presentation.startPresenter;
    const wrapped = function (this: any, ...args: any[]) { const result = start.apply(this, args); attach(); return result; };
    presentation.startPresenter = wrapped;
    if (presentation.presenter) attach();
    const off = presentation.onPresented((frame: any) => {
      draw(frame);
      const at = done.get(frame.seq);
      if (at === undefined) pending.set(frame.seq, frame);
      else complete(frame, at);
    });
    return () => { off(); offDone(); presentation.startPresenter = start; };
  };
  w.__pmRig = { counts: () => ({ ...c }), seeks, afterWebGL, watchEngine };
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

/** The engine's render-thread time between two reports (ms). `span`: compose start to GPU done (it contains the compose). */
function engineTime(a: any, b: any): { cpuMs: number; spanMs: number; gpuOnlyMs: number; frames: number } {
  if (!a || !b) return { cpuMs: 0, spanMs: 0, gpuOnlyMs: 0, frames: 0 };
  const cpuMs = (b.composeUsTotal - a.composeUsTotal) / 1000;
  const spanMs = (b.gpuUsTotal - a.gpuUsTotal) / 1000;
  return { cpuMs, spanMs, gpuOnlyMs: Math.max(0, spanMs - cpuMs), frames: (b.frames ?? 0) - (a.frames ?? 0) };
}

/** One step in the page: perform the step's request and wait for its presentation. */
type StepSpec =
  | { k: number; kind: 'time'; t: number }
  | { k: number; kind: 'edit'; command: Record<string, unknown>; t: number }
  | { k: number; kind: 'seek'; t: number; engine: boolean };
type StepArgs = StepSpec & { gpuTime: boolean; mode: Mode };
type PageStepResult = {
  ms: number | null;
  oldEndpointMs: number | null;
  busPresentedMs: number | null;
  drawCallbackMs: number | null;
  gpuCompleteMs: number | null;
  counts: Record<string, number>;
  gpuQuery: number | null;
};
function pageStep(spec: StepArgs): Promise<PageStepResult> {
  const PM = (window as any).PM;
  const rig = (window as any).__pmRig;
  const gl: WebGL2RenderingContext | null = spec.gpuTime ? PM.GL?.gl ?? null : null;
  const ext = gl?.getExtension('EXT_disjoint_timer_query_webgl2') ?? null;
  const queries: WebGLQuery[] = (rig.queries ??= []);
  let query: WebGLQuery | null = null;
  return new Promise((resolve) => {
    let done = false;
    let claimed = false;
    let wantVersion = Infinity;
    let wantRevision = Infinity;
    let ms: number | null = null;
    let oldEndpointMs: number | null = null;
    let busPresentedMs: number | null = null;
    let drawCallbackMs: number | null = null;
    let gpuCompleteMs: number | null = null;
    let queryEnded = false;
    let timer: ReturnType<typeof setTimeout>;
    const offs: Array<() => void> = [];
    const endQuery = () => {
      if (query && gl && ext && !queryEnded) { queryEnded = true; gl.endQuery(ext.TIME_ELAPSED_EXT); queries.push(query); }
    };
    const finish = (timeout = false) => {
      // Keep the old event even if it arrives after the corrected endpoint.
      if (done || (!timeout && (ms === null || oldEndpointMs === null || busPresentedMs === null))) return;
      done = true;
      clearTimeout(timer);
      offs.forEach((off) => off());
      endQuery();
      resolve({ ms, oldEndpointMs, busPresentedMs, drawCallbackMs, gpuCompleteMs, counts: rig.counts(), gpuQuery: query ? queries.length - 1 : null });
    };
    performance.mark(`pmrig:${spec.k}:request`);
    const t0 = performance.now();
    if (gl && ext) {
      query = gl.createQuery();
      if (query) gl.beginQuery(ext.TIME_ELAPSED_EXT, query);
    }
    const shown = (gpuDoneAt: number, shownAt: number) => {
      gpuCompleteMs = gpuDoneAt - t0;
      ms = shownAt - t0;
      performance.mark(`pmrig:${spec.k}:shown`);
      finish();
    };
    const fromSeek = rig.seeks.length;
    const fps = PM.proj?.fps ?? 30;
    const seeked = () => spec.kind !== 'seek' || rig.seeks.slice(fromSeek).some((s: any) => Math.abs(s.time - spec.t) < 0.5 / fps);
    offs.push(PM.bus.on('preview:presented', (e: any) => {
      if (Math.abs(Number(e?.time) - spec.t) > 1e-6) return;
      if (spec.kind === 'edit' && !(Number(e?.version ?? -1) >= wantVersion)) return;
      if (spec.mode === 'today' && !seeked()) return;
      busPresentedMs ??= performance.now() - t0;
      if (!(spec.kind === 'seek' && spec.mode === 'engine')) oldEndpointMs ??= busPresentedMs;
      if (spec.mode === 'today' && !claimed) {
        claimed = true;
        drawCallbackMs = busPresentedMs;
        endQuery();
        // This bus callback runs at GL.render's end, after its final commands.
        offs.push(rig.afterWebGL(shown));
      }
      finish();
    }));
    if (spec.mode === 'engine') {
      const matches = (f: any) => Math.abs(f.pts - Math.round(spec.t * 705_600_000)) <= 1 && !f.standIn
        && (spec.kind !== 'edit' || f.revision >= wantRevision);
      let selected: number | null = null;
      offs.push(rig.watchEngine((f: any) => {
        if (claimed || !matches(f)) return;
        claimed = true; selected = f.seq;
        drawCallbackMs = performance.now() - t0;
        // The previous scrub endpoint bypassed PresentedEvents already.
        if (spec.kind === 'seek') oldEndpointMs = drawCallbackMs;
      }, (f: any, gpuDoneAt: number) => {
        if (f.seq !== selected) return;
        const raf = requestAnimationFrame(() => shown(gpuDoneAt, performance.now()));
        offs.push(() => cancelAnimationFrame(raf));
      }));
    }
    if (spec.kind === 'edit') {
      PM.Edit.apply(spec.command, { label: 'Rig', origin: 'viewer' });
      wantVersion = PM.animVersion?.() ?? 0;
      // Read the revision after the normal edit flush, without forcing one.
      wantRevision = PM.EngineDoc?.barrier?.revision ?? 0;
      PM.invalidate?.();
    } else PM.setTime(spec.t);
    timer = setTimeout(() => finish(true), 5000);
  });
}

/** Today's WebGL query results (ms; null where unavailable or disjoint), after the GPU has finished them. */
async function gpuQueryResults(page: Page): Promise<{ available: boolean; ms: (number | null)[] }> {
  return page.evaluate(async () => {
    const PM = (window as any).PM;
    const rig = (window as any).__pmRig;
    const gl: WebGL2RenderingContext | null = PM.GL?.gl ?? null;
    const ext = gl?.getExtension('EXT_disjoint_timer_query_webgl2') ?? null;
    const queries: WebGLQuery[] = rig.queries ?? [];
    rig.queries = [];
    if (!gl || !ext) return { available: false, ms: [] };
    const deadline = performance.now() + 3000;
    while (performance.now() < deadline && !queries.every((q) => gl.getQueryParameter(q, gl.QUERY_RESULT_AVAILABLE))) {
      await new Promise((r) => requestAnimationFrame(r));
    }
    const disjoint = gl.getParameter(ext.GPU_DISJOINT_EXT);
    const ms = queries.map((q) => (!disjoint && gl.getQueryParameter(q, gl.QUERY_RESULT_AVAILABLE) ? gl.getQueryParameter(q, gl.QUERY_RESULT) / 1e6 : null));
    for (const q of queries) gl.deleteQuery(q);
    return { available: true, ms };
  });
}

/** A `toplevel` trace of the run: main-thread tasks between each step's request and presentation marks. */
async function startTrace(cdp: CDPSession): Promise<() => Promise<Map<number, number>>> {
  const events: any[] = [];
  const onData = (e: { value: any[] }) => { for (const v of e.value) events.push(v); };
  cdp.on('Tracing.dataCollected', onData);
  await cdp.send('Tracing.start', { traceConfig: { includedCategories: ['toplevel', 'blink.user_timing', '__metadata'], recordMode: 'recordAsMuchAsPossible' }, transferMode: 'ReportEvents' } as any);
  return async () => {
    const complete = new Promise<void>((r) => cdp.once('Tracing.tracingComplete', () => r()));
    await cdp.send('Tracing.end');
    await complete;
    cdp.off('Tracing.dataCollected', onData);
    const marks = events.filter((e) => typeof e.name === 'string' && e.name.startsWith('pmrig:'));
    const pid = marks[0]?.pid;
    const main = events.find((e) => e.ph === 'M' && e.name === 'thread_name' && e.pid === pid && e.args?.name === 'CrRendererMain')?.tid;
    const tasks = events.filter((e) => e.pid === pid && e.tid === main && e.ph === 'X' && (e.name === 'RunTask' || e.name === 'ThreadControllerImpl::RunTask')).map((e) => e.ts as number).sort((a, b) => a - b);
    const at = new Map<string, number>();
    for (const m of marks) at.set(m.name, m.ts);
    const out = new Map<number, number>();
    for (const name of at.keys()) {
      const match = /^pmrig:(\d+):request$/.exec(name);
      if (!match) continue;
      const k = Number(match[1]);
      const from = at.get(name)!;
      const to = at.get(`pmrig:${k}:shown`);
      if (to === undefined) continue;
      out.set(k, tasks.filter((ts) => ts >= from && ts <= to).length);
    }
    return out;
  };
}

type StepRecord = {
  ms: number | null;
  oldEndpointMs: number | null;
  busPresentedMs: number | null;
  drawCallbackMs: number | null;
  gpuCompleteMs: number | null;
  mainMs: number;
  layouts: number;
  styleRecalcs: number;
  tasks: number | null;
  engineCpuMs: number;
  engineSpanMs: number;
  engineGpuOnlyMs: number;
  engineFrames: number;
  webglGpuMs: number | null;
  page: Record<string, number>;
  work: Record<string, number>;
};

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

type Run = { steps: StepRecord[]; loadAtStart: number[]; calibrationMs: number; beginFrame: string; webglTimer: string };

async function runSteps(session: LaunchedApp, mode: Mode, specs: StepSpec[], options: { warmup?: number; gpuTime?: boolean } = {}): Promise<Run> {
  const loadAtStart = os.loadavg();
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
  // Today's WebGL timer: only where the app does not time layers itself (shader and effect layers).
  const gpuTime = mode === 'today' && options.gpuTime !== false && await page.evaluate(() => {
    const PM = (window as any).PM;
    const timed = (layers: any[]): boolean => layers.some((l) => l.type === 'shader' || (l.fx?.length ?? 0) > 0);
    const comps = [PM.proj, ...Object.values<any>(PM.proj.comps ?? {})];
    return !comps.some((c) => timed(c?.layers ?? [])) && Boolean(PM.GL?.gl?.getExtension('EXT_disjoint_timer_query_webgl2'));
  });
  const webglTimer = mode === 'engine' ? 'n/a' : gpuTime ? 'EXT_disjoint_timer_query_webgl2' : 'not measured (no timer extension, or the project has shader or effect layers)';
  // Warm-up: the first frames build caches on both paths.
  const warm = options.warmup ?? 4;
  for (const spec of specs.slice(0, warm)) await stage('warm-up frame', page.evaluate(pageStep, { ...spec, k: -1 - spec.k, gpuTime: false, mode }), 60_000);
  const measured = specs.slice(warm);
  const steps: StepRecord[] = [];
  // $PM_PERFBENCH_RIG_PROFILE=<dir>: a V8 CPU profile of the page over the
  // measured steps (scripts/cpuprofile-summary.mjs summarizes it).
  const profileDir = process.env.PM_PERFBENCH_RIG_PROFILE;
  if (profileDir) {
    await cdp.send('Profiler.enable');
    await cdp.send('Profiler.setSamplingInterval', { interval: 250 });
    await cdp.send('Profiler.start');
  }
  const endTrace = await startTrace(cdp);
  const queryIndex: (number | null)[] = [];
  let report = mode === 'engine' ? await engineReport(session) : null;
  let counts = await page.evaluate(() => (window as any).__pmRig.counts());
  for (const spec of measured) {
    const m0 = await metrics(cdp);
    const r = await stage('frame', page.evaluate(pageStep, { ...spec, gpuTime, mode }), 30_000);
    const m1 = await metrics(cdp);
    const next = mode === 'engine' ? await engineReport(session) : null;
    const e = engineTime(report, next);
    queryIndex.push(r.gpuQuery);
    steps.push({
      ms: r.ms,
      oldEndpointMs: r.oldEndpointMs,
      busPresentedMs: r.busPresentedMs,
      drawCallbackMs: r.drawCallbackMs,
      gpuCompleteMs: r.gpuCompleteMs,
      mainMs: Math.max(0, (m1.TaskDuration! - m0.TaskDuration!) * 1000 - calibrationMs),
      layouts: (m1.LayoutCount ?? 0) - (m0.LayoutCount ?? 0),
      styleRecalcs: (m1.RecalcStyleCount ?? 0) - (m0.RecalcStyleCount ?? 0),
      tasks: null,
      engineCpuMs: e.cpuMs,
      engineSpanMs: e.spanMs,
      engineGpuOnlyMs: e.gpuOnlyMs,
      engineFrames: e.frames,
      webglGpuMs: null,
      page: diff(counts, r.counts),
      work: next && report ? diff(report.work ?? {}, next.work ?? {}) : {}
    });
    counts = r.counts;
    report = next;
  }
  const tasks = await endTrace().catch(() => new Map<number, number>());
  measured.forEach((spec, i) => { steps[i]!.tasks = tasks.get(spec.k) ?? null; });
  if (gpuTime) {
    const { ms } = await gpuQueryResults(page);
    queryIndex.forEach((q, i) => { steps[i]!.webglGpuMs = q === null ? null : ms[q] ?? null; });
  }
  if (profileDir) {
    const { profile } = await cdp.send('Profiler.stop');
    const { writeFile, mkdir } = await import('node:fs/promises');
    await mkdir(profileDir, { recursive: true });
    await writeFile(path.join(profileDir, `rig-${mode}-${Date.now()}.cpuprofile`), JSON.stringify(profile));
  }
  await cdp.detach().catch(() => undefined);
  return { steps, loadAtStart, calibrationMs, beginFrame, webglTimer };
}

function summarize(row: string, mode: Mode, run: Run, extra: Record<string, unknown>) {
  const s = run.steps;
  const presented = s.filter((x) => x.ms !== null);
  const within = (ms: number | null) => ms === null || ms <= BUDGET_MS;
  const fits = s.filter((x) => x.ms !== null && x.mainMs <= BUDGET_MS && (mode === 'engine'
    ? x.engineCpuMs <= BUDGET_MS && x.engineGpuOnlyMs <= BUDGET_MS
    : within(x.webglGpuMs)));
  const spanFits = mode === 'engine' ? s.filter((x) => x.ms !== null && x.mainMs <= BUDGET_MS && x.engineSpanMs <= BUDGET_MS).length : null;
  const totals = sum(s.map((x) => x.page));
  const work = sum(s.map((x) => x.work));
  const per = (o: Record<string, number>) => Object.fromEntries(Object.entries(o).map(([k, v]) => [k, +(v / Math.max(1, s.length)).toFixed(2)]));
  const webgl = s.map((x) => x.webglGpuMs).filter((x): x is number => x !== null);
  const tasks = s.map((x) => x.tasks).filter((x): x is number => x !== null);
  return {
    row,
    mode,
    rateHz: 120,
    budgetMs: +BUDGET_MS.toFixed(3),
    steps: s.length,
    presented: presented.length,
    fitBudget: fits.length,
    fitCriterion: mode === 'engine' ? 'presented; page main, engine compose CPU, engine GPU-only each <= budget' : run.webglTimer.startsWith('EXT') ? 'presented; page main and WebGL GPU each <= budget' : 'presented; page main <= budget (WebGL GPU time not measured)',
    spanFit: spanFits,
    overBudget: s.length - fits.length,
    notPresented: s.length - presented.length,
    mainThreadMs: dist(s.map((x) => x.mainMs)),
    mainThreadTasks: dist(tasks),
    layoutsPerStep: dist(s.map((x) => x.layouts)),
    styleRecalcsPerStep: dist(s.map((x) => x.styleRecalcs)),
    engineCpuMs: mode === 'engine' ? dist(s.map((x) => x.engineCpuMs)) : null,
    engineGpuOnlyMs: mode === 'engine' ? dist(s.map((x) => x.engineGpuOnlyMs)) : null,
    engineSpanMs: mode === 'engine' ? dist(s.map((x) => x.engineSpanMs)) : null,
    engineFramesPerStep: mode === 'engine' ? dist(s.map((x) => x.engineFrames)) : null,
    webglGpuMs: mode === 'today' ? (webgl.length ? dist(webgl) : run.webglTimer) : null,
    latencyEndpoint: 'frame GPU-complete, then next page requestAnimationFrame (compositor opportunity; not physical scanout)',
    stepLatencyMs: dist(presented.map((x) => x.ms!)),
    oldEndpointMs: dist(s.flatMap((x) => x.oldEndpointMs === null ? [] : [x.oldEndpointMs])),
    oldEndpoint: mode === 'engine' && row.startsWith('scrub') ? 'direct onPresented (no bus hold)' : 'preview:presented bus event (unequal between paths)',
    busPresentedMs: dist(s.flatMap((x) => x.busPresentedMs === null ? [] : [x.busPresentedMs])),
    drawCallbackMs: dist(s.flatMap((x) => x.drawCallbackMs === null ? [] : [x.drawCallbackMs])),
    gpuCompleteMs: dist(s.flatMap((x) => x.gpuCompleteMs === null ? [] : [x.gpuCompleteMs])),
    pagePerFrame: per(totals),
    enginePerFrame: per(work),
    loadAtStart: run.loadAtStart,
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
  await appendFile(OUT, `${JSON.stringify({ at: new Date().toISOString(), loadAverage: os.loadavg(), ...value })}\n`);
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

async function expectEngine(page: Page, mode: Mode): Promise<void> {
  const route = await waitRoute(page, mode);
  expect(route?.route ?? 'engine', JSON.stringify(route)).toBe('engine');
}

async function synthetic(session: LaunchedApp, mode: Mode, layers: number, fps: number, dur = 10): Promise<any> {
  await session.openEditor();
  const summary = await session.page.evaluate(buildSyntheticProject, { layers, fps, dur });
  await session.page.evaluate(() => { const PM = (window as any).PM; PM.pause?.(); PM.setTime(0); });
  await expectEngine(session.page, mode);
  return summary;
}

const playbackSpecs = (from: number): StepSpec[] => Array.from({ length: STEPS + 4 }, (_, k) => ({ k, kind: 'time', t: from + (k + 1) / 120 }));

/** Main-thread time and engine time over an action that is not a frame step (open, export). */
async function measureAction<T>(session: LaunchedApp, mode: Mode, action: () => Promise<T>): Promise<{ value: T; wallMs: number; mainMs: number; layouts: number; engine: ReturnType<typeof engineTime> }> {
  const cdp = await session.page.context().newCDPSession(session.page);
  await cdp.send('Performance.enable', { timeDomain: 'timeTicks' });
  const r0 = mode === 'engine' ? await engineReport(session) : null;
  const m0 = await metrics(cdp);
  const start = Date.now();
  const value = await action();
  const wallMs = Date.now() - start;
  const m1 = await metrics(cdp);
  const r1 = mode === 'engine' ? await engineReport(session) : null;
  await cdp.detach().catch(() => undefined);
  return { value, wallMs, mainMs: (m1.TaskDuration! - m0.TaskDuration!) * 1000, layouts: (m1.LayoutCount ?? 0) - (m0.LayoutCount ?? 0), engine: engineTime(r0, r1) };
}

test.describe('120 Hz rig', () => {
  // `playback`: paused seeks, one per refresh (the stepped twin of
  // playback_500; the playing path is the `playing` row). `edit`: one layer
  // moved a pixel per frame through PM.Edit.apply (an inspector edit, with
  // the project's provenance), paused.
  for (const scenario of ['playback', 'edit'] as const) {
    test(`synthetic 500 layers: ${scenario}, today and the engine`, async () => {
      test.setTimeout(900_000);
      for (const mode of MODES) {
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
            specs = Array.from({ length: STEPS + 4 }, (_, k) => ({ k, kind: 'edit', command: { type: 'set_property', target: layer, path: 'position.x', value: 100 + k }, t }));
          }
          const run = await runSteps(session, mode, specs);
          await record(summarize(`synthetic-500-${scenario === 'playback' ? 'paused-seeks' : 'edit'}`, mode, run, { fixture: summary }));
          expect(run.steps.length).toBe(STEPS);
          expect(run.steps.every((s) => s.ms !== null && s.oldEndpointMs !== null && s.gpuCompleteMs !== null && s.ms >= s.gpuCompleteMs)).toBe(true);
        } finally {
          await session.close();
        }
      }
    });
  }

  test('synthetic 500 layers: playing (PM.play), today and the engine', async () => {
    test.setTimeout(900_000);
    const seconds = 3;
    for (const mode of MODES) {
      const session = await launch(mode);
      try {
        const summary = await synthetic(session, mode, 500, 120);
        // Warm the playing path once.
        await session.page.evaluate(async () => { const PM = (window as any).PM; PM.setTime(0); PM.play(); await new Promise((r) => setTimeout(r, 1000)); PM.pause(); PM.setTime(0); });
        await new Promise((r) => setTimeout(r, 500));
        const counts0 = await session.page.evaluate(() => (window as any).__pmRig.counts());
        const m = await measureAction(session, mode, () => session.page.evaluate(async ({ ms, mode }) => {
          const PM = (window as any).PM;
          const rig = (window as any).__pmRig;
          const at: number[] = [];
          const oldAt: number[] = [];
          const times = new Set<number>();
          const offs: Array<() => void> = [];
          let directCallbacks = 0;
          let gpuCompletions = 0;
          let coalesced = 0;
          let emissionPushes = 0;
          let run = true;
          const shown = (_gpuDoneAt: number, shownAt: number) => { if (run) at.push(shownAt); };
          const events = PM.EnginePresentedEvents;
          const emitted0 = events?.emitted ?? 0;
          const pendingAtStart = Boolean(events?.pending);
          const push = events?.push;
          if (mode === 'engine') {
            // Count replacements of a pending bus emission, rather than
            // attributing every rendered-minus-emitted frame to merging.
            events.push = function (...args: any[]) { emissionPushes++; if (this.pending) coalesced++; return push.apply(this, args); };
            offs.push(() => { events.push = push; });
            offs.push(rig.watchEngine(() => { directCallbacks++; }, (_f: any, gpuDoneAt: number) => {
              gpuCompletions++;
              const raf = requestAnimationFrame(() => shown(gpuDoneAt, performance.now()));
              offs.push(() => cancelAnimationFrame(raf));
            }));
          }
          offs.push(PM.bus.on('preview:presented', (e: any) => {
            oldAt.push(performance.now()); times.add(Number(e?.time));
            if (mode === 'today') offs.push(rig.afterWebGL(shown));
          }));
          let rafs = 0;
          const tick = () => { if (!run) return; rafs++; requestAnimationFrame(tick); };
          requestAnimationFrame(tick);
          const start = performance.now();
          PM.play();
          await new Promise((r) => setTimeout(r, ms));
          PM.pause();
          run = false;
          const emissions = (events?.emitted ?? 0) - emitted0;
          const pendingAtEnd = Boolean(events?.pending);
          offs.forEach((off) => off());
          const elapsed = performance.now() - start;
          return { at, oldAt, distinct: times.size, rafs, elapsed, directCallbacks, gpuCompletions, coalesced, emissionPushes, emissions, pendingAtStart, pendingAtEnd };
        }, { ms: seconds * 1000, mode }));
        const counts1 = await session.page.evaluate(() => (window as any).__pmRig.counts());
        const p = m.value;
        if (mode === 'engine') {
          expect(p.emissionPushes + Number(p.pendingAtStart)).toBe(p.emissions + p.coalesced + Number(p.pendingAtEnd));
        }
        const gaps = p.at.slice(1).map((t, i) => t - p.at[i]!);
        const presented = p.at.length;
        const expected = Math.floor((p.elapsed / 1000) * 120);
        await record({
          row: 'synthetic-500-playing',
          mode,
          rateHz: 120,
          seconds: +(p.elapsed / 1000).toFixed(3),
          expectedFrames: expected,
          presented,
          presentedEndpoint: 'GPU-complete, then next page rAF',
          oldBusPresented: p.oldAt.length,
          oldBusGapMs: dist(p.oldAt.slice(1).map((t, i) => t - p.oldAt[i]!)),
          engineDirectCallbacks: mode === 'engine' ? p.directCallbacks : null,
          engineDrawCompletions: mode === 'engine' ? p.gpuCompletions : null,
          engineBusEmissions: mode === 'engine' ? p.emissions : null,
          engineEmissionPushes: mode === 'engine' ? p.emissionPushes : null,
          engineCoalescedEmissions: mode === 'engine' ? p.coalesced : null,
          enginePendingAtStart: mode === 'engine' ? p.pendingAtStart : null,
          enginePendingAtEnd: mode === 'engine' ? p.pendingAtEnd : null,
          distinctTimes: p.distinct,
          pageAnimationFramesPerSecond: +(p.rafs / (p.elapsed / 1000)).toFixed(1),
          presentGapMs: dist(gaps),
          gapsOverBudget: gaps.filter((g) => g > BUDGET_MS * 1.5).length,
          mainThreadMsPerPresented: +(m.mainMs / Math.max(1, presented)).toFixed(3),
          layoutsPerPresented: +(m.layouts / Math.max(1, presented)).toFixed(2),
          engineCpuMsPerFrame: mode === 'engine' ? +(m.engine.cpuMs / Math.max(1, m.engine.frames)).toFixed(3) : null,
          engineGpuOnlyMsPerFrame: mode === 'engine' ? +(m.engine.gpuOnlyMs / Math.max(1, m.engine.frames)).toFixed(3) : null,
          engineSpanMsPerFrame: mode === 'engine' ? +(m.engine.spanMs / Math.max(1, m.engine.frames)).toFixed(3) : null,
          engineFrames: mode === 'engine' ? m.engine.frames : null,
          pagePerPresented: Object.fromEntries(Object.entries(diff(counts0, counts1)).map(([k, v]) => [k, +(v / Math.max(1, presented)).toFixed(2)])),
          load: os.loadavg()[0],
          fixture: summary
        });
      } finally {
        await session.close();
      }
    }
  });

  test('scrub 24 seeks on the public clip, today and the engine', async () => {
    test.setTimeout(900_000);
    const video = Array.from(await readFile(path.join(repoRoot, 'e2e/fixtures/h264-aac.mp4')));
    for (const mode of MODES) {
      const session = await launch(mode);
      try {
        await session.openEditor();
        // scrub_24's document: the clip fitted to 1920 × 1080 at 90% under a sliding solid, 30 fps, 2 s.
        await session.page.evaluate(async (video) => {
          const PM = (window as any).PM;
          const project = PM.mkProject({ name: 'Rig scrub', w: 1920, h: 1080, fps: 30, dur: 2, bg: '#000000' });
          PM.replaceProject(project);
          const asset = await PM.assets.add(new File([new Uint8Array(video)], 'clip.mp4', { type: 'video/mp4' }), { newAsset: true, silent: true });
          const clip = PM.mkLayer('video', { name: 'Clip', d: { asset: asset.id, fit: 'contain', w: 1920, h: 1080, embeddedAudio: false } }, PM.proj);
          clip.p['position.x'].v = 960; clip.p['position.y'].v = 540; clip.p.opacity.v = 90;
          const bar = PM.mkLayer('solid', { name: 'Bar', d: { color: '#E8553A', w: 400, h: 120 } }, PM.proj);
          bar.p['position.y'].v = 880;
          bar.p['position.x'].kf = [PM.KF(0, 200, 'easeInOut'), PM.KF(2, 1300, 'easeInOut')];
          PM.proj.layers = [bar, clip];
          PM.touch();
          PM.pause?.();
          PM.setTime(0, { force: true });
        }, video);
        await session.page.evaluate(async () => { await (window as any).PM.projectAssetsReady; });
        await expectEngine(session.page, mode);
        // Every 1/24 of 0..1.9 s in a scattered order (7 is coprime with 24), on the 30 fps grid; one warm-up seek.
        const at = (k: number) => Math.floor(((((k * 7) % 24) * 1.9) / 24 + 1 / 60) * 30 + 1e-9) / 30;
        const specs: StepSpec[] = [{ k: 0, kind: 'seek', t: 58 / 30, engine: mode === 'engine' }, ...Array.from({ length: 24 }, (_, i) => ({ k: i + 1, kind: 'seek' as const, t: at(i), engine: mode === 'engine' }))];
        const run = await runSteps(session, mode, specs, { warmup: 1 });
        await record(summarize('scrub-24-public-clip', mode, run, { clip: 'h264-aac.mp4 (320x180, 30 fps)', exact: mode === 'engine' ? 'the exact frame (stand-ins skipped)' : "the element's completed seek, then a presented frame" }));
        expect(run.steps.length).toBe(24);
        expect(run.steps.every((s) => s.ms !== null && s.oldEndpointMs !== null && s.gpuCompleteMs !== null && s.ms >= s.gpuCompleteMs)).toBe(true);
      } finally {
        await session.close();
      }
    }
  });

  test('typing 60 keys into a 96 px title, today and the engine', async () => {
    test.setTimeout(900_000);
    const sentence = 'The quick brown fox jumps over the lazy dog while editors type captions';
    for (const mode of MODES) {
      const session = await launch(mode);
      try {
        await session.openEditor();
        const id = await session.page.evaluate(() => {
          const PM = (window as any).PM;
          const project = PM.mkProject({ name: 'Rig typing', w: 1920, h: 1080, fps: 30, dur: 10, bg: '#101418' });
          PM.replaceProject(project);
          const back = PM.mkLayer('solid', { name: 'Back', d: { color: '#202436', w: 1920, h: 1080 } }, PM.proj);
          back.p['position.x'].v = 960; back.p['position.y'].v = 540;
          const title = PM.mkLayer('text', { name: 'Title', d: { text: 'T', font: 'SF Pro Display', weight: 600, size: 96, tracking: -2, leading: 1.1, color: '#FFFFFF', align: 'center' } }, PM.proj);
          title.p['position.x'].v = 960; title.p['position.y'].v = 540;
          PM.proj.layers = [title, back];
          PM.touch();
          PM.pause?.();
          PM.setTime(0, { force: true });
          return title.id;
        });
        await expectEngine(session.page, mode);
        // typing_60's keys: each a revision of the title's text (the document path; the canvas text tool's session is not driven).
        let typed = 'T';
        const specs: StepSpec[] = Array.from({ length: 63 }, (_, i) => {
          typed += sentence[(i + 1) % sentence.length];
          return { k: i, kind: 'edit', command: { type: 'set_content', target: id, patch: { text: typed } }, t: 0 };
        });
        const run = await runSteps(session, mode, specs, { warmup: 3 });
        await record(summarize('typing-60-keys', mode, run, { font: 'SF Pro Display 600, 96 px', path: 'PM.Edit.apply set_content per key' }));
        expect(run.steps.length).toBe(60);
        expect(run.steps.every((s) => s.ms !== null && s.oldEndpointMs !== null && s.gpuCompleteMs !== null && s.ms >= s.gpuCompleteMs)).toBe(true);
      } finally {
        await session.close();
      }
    }
  });

  test('opening the 2,000-layer synthetic project, today and the engine', async () => {
    test.setTimeout(1_800_000);
    const scratch = await mkdtemp(path.join(os.tmpdir(), 'pm-rig-open-'));
    const file = path.join(scratch, 'large.pmv');
    const name = 'Synthetic 2000 layers';
    try {
      // The file, written once by today's app (the format is the same on both paths).
      let summary: unknown = null;
      {
        const session = await launch('today');
        try {
          await session.openEditor();
          summary = await session.page.evaluate(buildSyntheticProject, { layers: 2000, fps: 30, dur: 10 });
          await session.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, file);
          expect(await stage('save', session.page.evaluate(() => (window as any).PM.saveProject({ saveAs: true })), 300_000)).toBe(true);
        } finally {
          await session.close();
        }
      }
      for (const mode of MODES) {
        const opens: any[] = [];
        // Each open in a fresh app with the editor on a small project (no
        // session of the file to switch back to).
        for (let i = 0; i < 3; i++) {
          const session = await launch(mode);
          try {
            await session.openEditor();
            await session.app.evaluate(({ dialog }, filePath) => { dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] }); }, file);
            await new Promise((r) => setTimeout(r, 1500));
            const m = await measureAction(session, mode, () => stage('open to first frame', session.page.evaluate(async ({ name, mode }) => {
              const PM = (window as any).PM;
              const rig = (window as any).__pmRig;
              const t0 = performance.now();
              let opened: number | null = null;
              let since: number | null = null;
              let sinceRevision: number | null = null;
              let oldFirstFrameMs: number | null = null;
              let firstFrameMs: number | null = null;
              let gpuCompleteMs: number | null = null;
              let selected: number | null = null;
              let claimed = false;
              const offs: Array<() => void> = [];
              const first = new Promise<void>((resolve) => {
                const finish = () => { if (firstFrameMs !== null && oldFirstFrameMs !== null) resolve(); };
                const shown = (gpuDoneAt: number, shownAt: number) => { gpuCompleteMs = gpuDoneAt - t0; firstFrameMs = shownAt - t0; finish(); };
                const documentReady = () => {
                  if (PM.proj?.name !== name) return false;
                  since ??= Number(PM.animVersion?.() ?? 0);
                  sinceRevision ??= Number(PM.EngineDoc?.barrier?.revision ?? 0);
                  return true;
                };
                if (mode === 'engine') offs.push(rig.watchEngine((frame: any) => {
                  if (claimed || !documentReady() || frame.standIn || frame.revision < sinceRevision!) return;
                  claimed = true; selected = frame.seq;
                }, (frame: any, at: number) => {
                  if (frame.seq !== selected) return;
                  const raf = requestAnimationFrame(() => shown(at, performance.now()));
                  offs.push(() => cancelAnimationFrame(raf));
                }));
                offs.push(PM.bus.on('preview:presented', (e: any) => {
                  if (!documentReady() || e?.project?.name !== name || !(Number(e?.version ?? -1) >= since!)) return;
                  oldFirstFrameMs ??= performance.now() - t0;
                  if (mode === 'today' && !claimed) { claimed = true; offs.push(rig.afterWebGL(shown)); }
                  finish();
                }));
              });
              try {
                void PM.openProject().then(() => { opened = performance.now() - t0; });
                await first;
                return { firstFrameMs, oldFirstFrameMs, gpuCompleteMs, openResolvedMs: opened, layers: PM.proj.layers.length };
              } finally { offs.forEach((off) => off()); }
            }, { name, mode }), 300_000));
            const route = await waitRoute(session.page, mode);
            opens.push({ ...m.value, wallMs: m.wallMs, mainMs: +m.mainMs.toFixed(1), layouts: m.layouts, engineCpuMs: mode === 'engine' ? +m.engine.cpuMs.toFixed(2) : null, engineSpanMs: mode === 'engine' ? +m.engine.spanMs.toFixed(2) : null, route: route?.route ?? null });
          } finally {
            await session.close();
          }
        }
        await record({ row: 'open-large-2000', mode, opens, firstFrameMs: dist(opens.map((o) => o.firstFrameMs)), oldFirstFrameMs: dist(opens.map((o) => o.oldFirstFrameMs)), firstFrameEndpoint: 'GPU-complete, then next page rAF', mainMs: dist(opens.map((o) => o.mainMs)), load: os.loadavg()[0], fixture: summary });
        if (mode === 'engine') expect(opens.every((o) => o.route === 'engine')).toBe(true);
      }
    } finally {
      await rm(scratch, { recursive: true, force: true });
    }
  });

  test('exporting 120 frames of the 500-layer project to H.264 1080p, today and the engine', async () => {
    test.setTimeout(1_800_000);
    const scratch = await mkdtemp(path.join(os.tmpdir(), 'pm-rig-export-'));
    const ffprobe = ffmpegBinary.replace(/ffmpeg$/, 'ffprobe');
    try {
      for (const mode of MODES) {
        const session = await launch(mode);
        try {
          const summary = await synthetic(session, mode, 500, 30, 4);
          if (mode === 'engine') {
            await expect.poll(() => session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine?.engine?.status()?.job?.state ?? 'missing'), { timeout: 60_000 }).toBe('ready');
          }
          const file = path.join(scratch, `export-${mode}.mp4`);
          await session.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, file);
          const m = await measureAction(session, mode, () => stage('export', session.page.evaluate(async () => (window as any).PM.Export.run({ format: 'mp4', scale: 1, fps: 30, range: 'all', quality: 'high', mblur: false, audio: false })), 1_200_000));
          const probe = JSON.parse(execFileSync(ffprobe, ['-v', 'error', '-count_frames', '-show_entries', 'format_tags=encoder:stream=codec_name,width,height,nb_read_frames', '-of', 'json', file], { encoding: 'utf8' }));
          const frames = Number(probe.streams?.[0]?.nb_read_frames ?? 0);
          await record({
            row: 'export-120-h264-1080p',
            mode,
            result: m.value,
            frames,
            stream: probe.streams?.[0] ?? null,
            // The job engine's MP4 is muxed by AVAssetWriter, today's exporter's by FFmpeg (Lavf).
            muxer: String(probe.format?.tags?.encoder ?? '').startsWith('Lavf') ? "today's exporter (FFmpeg)" : 'job engine (AVAssetWriter)',
            wallMs: m.wallMs,
            msPerFrame: +(m.wallMs / Math.max(1, frames)).toFixed(2),
            mainThreadMs: +m.mainMs.toFixed(1),
            mainThreadMsPerFrame: +(m.mainMs / Math.max(1, frames)).toFixed(2),
            load: os.loadavg()[0],
            fixture: summary
          });
          expect(frames).toBe(120);
        } finally {
          await session.close();
        }
      }
    } finally {
      await rm(scratch, { recursive: true, force: true });
    }
  });

  test('private projects from the runtime manifest: paused seeks, today and the engine', async () => {
    const manifest = process.env.PM_PERFBENCH_PRIVATE;
    test.skip(!manifest || !existsSync(manifest), 'no private manifest ($PM_PERFBENCH_PRIVATE)');
    test.setTimeout(7_200_000);
    const list = JSON.parse(await readFile(manifest!, 'utf8')).projects as { label: string; file: string; extensions?: string; from?: number; steps?: number }[];
    for (const p of list) {
      for (const mode of MODES) {
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
                await record({ row: `private-${p.label}-paused-seeks`, mode, routed: route, steps: 0, load: os.loadavg()[0] });
                continue;
              }
              at = 'frames';
              const from = p.from ?? 0;
              const steps = p.steps ?? STEPS;
              // Presented times follow the composition's own frame grid: wait for the frame of each step's time.
              const comp = await page.evaluate(() => { const PM = (window as any).PM; return { fps: PM.comp?.()?.fps ?? PM.proj.fps, layers: PM.proj.layers.length }; });
              const snapped = Array.from({ length: steps + 4 }, (_, k) => Math.floor((from + (k + 1) / 120) * comp.fps + 1e-9) / comp.fps);
              // Only refreshes that change the composition frame present a new frame.
              const distinct = snapped.filter((t, i) => i === 0 || t !== snapped[i - 1]);
              const specs: StepSpec[] = distinct.map((t, k) => ({ k, kind: 'time', t }));
              const run = await runSteps(session, mode, specs);
              await record(summarize(`private-${p.label}-paused-seeks`, mode, run, { fps: comp.fps, layers: comp.layers, refreshes: steps, route }));
            } catch (e) {
              // What the page showed when a stage stalled (a dialog, the route), for the report.
              const state = await stage('diagnostics', page.evaluate(() => {
                const PM = (window as any).PM;
                const dialogs = [...document.querySelectorAll('dialog[open], [role="dialog"], [role="alertdialog"]')].map((d) => (d.textContent ?? '').trim().slice(0, 200));
                let engine: unknown = null;
                try { engine = PM?.EngineConnection?.status?.(); } catch { engine = 'status threw'; }
                return { projectsOpen: !!PM?.ProjectsScreen?.isOpen, layers: PM?.proj?.layers?.length ?? null, dialogs, engine: engine && typeof engine === 'object' ? { route: (engine as any).route, reason: (engine as any).routeReason, failures: (engine as any).failures?.length } : engine };
              }), 15_000).catch((err) => ({ unreadable: String(err) }));
              await record({ row: `private-${p.label}-paused-seeks`, mode, failedAt: at, error: String((e as Error).message ?? e).slice(0, 300), state, load: os.loadavg()[0] });
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
