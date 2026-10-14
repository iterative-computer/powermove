/*
 * The production frame bridge measured with the S1 harness (plan section
 * 18; amendment 5's gate): the engine path is no worse than today's viewer
 * on the same harness, at p50 and p99, idle and under the gate load.
 *
 * Builds the pm-bridge addon with its `harness` feature (synthetic input),
 * bundles src/main/engine/integration/measure-main.ts and measure-page.ts
 * with the real preload and engine process, runs every scenario in its own
 * Electron launch (one at a time, window off every screen, never focused),
 * and analyzes Chromium's PipelineReporter trace as S1 did. Summaries go to
 * docs/engine/bridge-measurements/<run>.json and a table to stdout.
 *
 * The `app-*` runs measure the real app instead (amendment 18): the WebGL
 * viewer drawing a corpus project ("today"), and the engine path presented
 * by the real viewer in the same app with the same project
 * (measure-app-main.ts, measure-app-page.ts). They need the app built
 * (bun run build) and the project at --project (default: the corpus perf
 * copy of the Powermove ad project, kept outside the repository).
 *
 *   node scripts/frame-bridge-measure.mjs [--only run,run] [--seconds 60] [--skip-build] [--tag r2] [--project file.pmv] [--keep]
 *   node scripts/frame-bridge-measure.mjs --only run --tag r2 --from <dir --keep left>   (analysis only)
 *
 * Needs native/pm_node.node (bun run build:engine). Run it under
 * pm-electron-lock: it starts Electron once per run.
 */
import { execFileSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';

import { build } from 'esbuild';

const desktop = path.resolve(import.meta.dirname, '..');
const repo = path.resolve(desktop, '../..');
const engineDir = path.join(repo, 'engine');
const summaries = path.join(repo, 'docs/engine/bridge-measurements');
const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : fallback;
};
const seconds = Number(arg('seconds', '60'));
const only = arg('only', '').split(',').filter(Boolean);
const tag = arg('tag', '');
// Re-analyze raw results a `--keep` run left in this directory instead of
// measuring again (the analysis changed, not the measurement).
const from = arg('from', '') ? path.resolve(arg('from', '')) : null;
const project = path.resolve(arg('project', path.join(os.homedir(), 'Library/Application Support/Powermove Engine Corpus/perf/Powermove ad (perf copy).pmv')));

const TICKS = 705_600_000;
const FPS = 60;

const playback = { scenario: 'playback', durationMs: seconds * 1000 };
const latency = { scenario: 'latency', durationMs: 20_000 };
const RUNS = [
  { name: 'playback-today-idle', ...playback, mode: 'today', load: false },
  { name: 'playback-engine-idle', ...playback, mode: 'engine', load: false },
  { name: 'playback-engine-bgra-idle', ...playback, mode: 'engine', load: false, edr: 'off' },
  { name: 'playback-today-load', ...playback, mode: 'today', load: true },
  { name: 'playback-engine-load', ...playback, mode: 'engine', load: true },
  { name: 'playback-engine-bgra-load', ...playback, mode: 'engine', load: true, edr: 'off' },
  // The gate load without main-thread autosave: the app since autosave
  // serialization moved to a persistence worker (amendment 5).
  { name: 'playback-today-load-noautosave', ...playback, mode: 'today', load: true, loadProfile: 'no-autosave' },
  { name: 'playback-engine-load-noautosave', ...playback, mode: 'engine', load: true, loadProfile: 'no-autosave' },
  // Render-ahead 8 frames (pool 12): slack for GPU-process stalls that hold slots.
  { name: 'playback-engine-lead8-load', ...playback, mode: 'engine', load: true, lead: 8 },
  { name: 'latency-today-idle', ...latency, mode: 'today', load: false, inputModes: ['baseline'] },
  { name: 'latency-engine-idle', ...latency, mode: 'engine', load: false, inputModes: ['drag', 'orbit', 'walk'] },
  { name: 'latency-engine-raw-idle', ...latency, mode: 'engine', load: false, rawInput: true, inputModes: ['drag', 'orbit', 'walk'] },
  { name: 'latency-today-load', ...latency, mode: 'today', load: true, inputModes: ['baseline'] },
  { name: 'latency-engine-load', ...latency, mode: 'engine', load: true, inputModes: ['drag', 'orbit', 'walk'] },
  { name: 'latency-engine-raw-load', ...latency, mode: 'engine', load: true, rawInput: true, inputModes: ['drag', 'orbit', 'walk'] },
  // The C′ prototype (amendment 5): playback through a remote Core
  // Animation layer; presentation times from Core Animation. Core Animation
  // only presents a layer whose window is on a display, so C′ runs (and the
  // Chromium runs they are compared with) use a window on the primary
  // display behind the desktop picture (`placement: 'behind'`).
  { name: 'playback-cprime-idle', ...playback, mode: 'engine', load: false, remoteLayer: true, placement: 'behind' },
  { name: 'playback-cprime-load', ...playback, mode: 'engine', load: true, loadProfile: 'no-autosave', remoteLayer: true, placement: 'behind' },
  { name: 'playback-today-idle-behind', ...playback, mode: 'today', load: false, placement: 'behind' },
  { name: 'playback-engine-idle-behind', ...playback, mode: 'engine', load: false, placement: 'behind' },
  // ProMotion (section 15): 25 fps content on the 120 Hz panel, with and
  // without asking for a multiple of the content rate (C′ only: Electron
  // has no refresh-rate request for its compositor).
  { name: 'playback-cprime-25-idle', ...playback, mode: 'engine', load: false, remoteLayer: true, fps: 25, placement: 'behind' },
  { name: 'playback-cprime-25-rate-idle', ...playback, mode: 'engine', load: false, remoteLayer: true, fps: 25, placement: 'behind', env: { POWERMOVE_BRIDGE_REMOTE_RATE: '1' } },
  { name: 'playback-engine-25-idle', ...playback, mode: 'engine', load: false, fps: 25 },
  { name: 'playback-engine-load-noautosave-phaseoff', ...playback, mode: 'engine', load: true, loadProfile: 'no-autosave', env: { POWERMOVE_BRIDGE_PHASE: 'off' } },
  { name: 'playback-engine-idle-phaseoff', ...playback, mode: 'engine', load: false, env: { POWERMOVE_BRIDGE_PHASE: 'off' } },
  // The real app (amendment 18). Load: the gate profile without the
  // main-thread autosave, which the app no longer has.
  { name: 'app-playback-today-idle', ...playback, app: true, mode: 'today', load: false },
  { name: 'app-playback-today-load', ...playback, app: true, mode: 'today', load: true, loadProfile: 'no-autosave' },
  // Engine path before the vsync-phase hint (step 3), then with it.
  { name: 'app-playback-engine-idle-phaseoff', ...playback, app: true, mode: 'engine', load: false, env: { POWERMOVE_BRIDGE_PHASE: 'off' } },
  { name: 'app-playback-engine-load-phaseoff', ...playback, app: true, mode: 'engine', load: true, loadProfile: 'no-autosave', env: { POWERMOVE_BRIDGE_PHASE: 'off' } },
  { name: 'app-playback-engine-idle', ...playback, app: true, mode: 'engine', load: false },
  { name: 'app-playback-engine-load', ...playback, app: true, mode: 'engine', load: true, loadProfile: 'no-autosave' },
  // The engine path drawn on the page's main thread (before the
  // presentation worker), for comparison.
  { name: 'app-playback-engine-idle-main', ...playback, app: true, mode: 'engine', load: false, offMainThread: false },
  { name: 'app-playback-engine-load-main', ...playback, app: true, mode: 'engine', load: true, loadProfile: 'no-autosave', offMainThread: false },
  { name: 'app-latency-engine-idle-main', ...latency, app: true, mode: 'engine', load: false, offMainThread: false, inputModes: ['pan'] },
  { name: 'app-latency-engine-load-main', ...latency, app: true, mode: 'engine', load: true, loadProfile: 'no-autosave', offMainThread: false, inputModes: ['pan'] },
  { name: 'app-playback-engine-load-phase250', ...playback, app: true, mode: 'engine', load: true, loadProfile: 'no-autosave', env: { POWERMOVE_BRIDGE_PHASE: '250' } },
  { name: 'app-playback-engine-load-phase750', ...playback, app: true, mode: 'engine', load: true, loadProfile: 'no-autosave', env: { POWERMOVE_BRIDGE_PHASE: '750' } },
  // The C′ prototype in the real app: playback through the remote layer
  // over the real viewer's stage.
  { name: 'app-playback-cprime-idle', ...playback, app: true, mode: 'engine', load: false, remoteLayer: true, placement: 'behind' },
  { name: 'app-playback-cprime-load', ...playback, app: true, mode: 'engine', load: true, loadProfile: 'no-autosave', remoteLayer: true, placement: 'behind' },
  { name: 'app-playback-today-idle-behind', ...playback, app: true, mode: 'today', load: false, placement: 'behind' },
  { name: 'app-playback-today-load-behind', ...playback, app: true, mode: 'today', load: true, loadProfile: 'no-autosave', placement: 'behind' },
  // Gestures: the engine path pans the view (it draws the test source, not
  // the document, so a layer drag would change nothing it renders); today
  // pans the same way and drags a layer. `-move`: before step 2
  // (pointermove).
  { name: 'app-latency-today-idle', ...latency, app: true, mode: 'today', load: false, inputModes: ['pan', 'drag'] },
  { name: 'app-latency-today-load', ...latency, app: true, mode: 'today', load: true, loadProfile: 'no-autosave', inputModes: ['pan', 'drag'] },
  { name: 'app-latency-engine-idle-move', ...latency, app: true, mode: 'engine', load: false, rawGestures: false, inputModes: ['pan'] },
  { name: 'app-latency-engine-load-move', ...latency, app: true, mode: 'engine', load: true, loadProfile: 'no-autosave', rawGestures: false, inputModes: ['pan'] },
  { name: 'app-latency-engine-idle', ...latency, app: true, mode: 'engine', load: false, inputModes: ['pan'] },
  { name: 'app-latency-engine-load', ...latency, app: true, mode: 'engine', load: true, loadProfile: 'no-autosave', inputModes: ['pan'] }
];
const selected = RUNS.filter((run) => !only.length || only.includes(run.name));

/* ── build ─────────────────────────────────────────────────────────── */

const work = fs.mkdtempSync(path.join(os.tmpdir(), 'pm-bridge-measure-'));
const harnessAddon = path.join(work, 'pm_bridge_harness.node');
if (!process.argv.includes('--skip-build') && !from) {
  execFileSync('/bin/sh', ['-c', '. ./env.sh && exec nice -n 15 cargo build --release --locked -p pm-bridge --lib --features harness'], { cwd: engineDir, stdio: 'inherit' });
}
fs.copyFileSync(path.join(engineDir, 'target/release/libpm_bridge.dylib'), harnessAddon);
const common = { bundle: true, logLevel: 'warning', alias: { powermove: path.join(desktop, 'src/renderer/src/kernel/api.ts') } };
await build({
  ...common,
  entryPoints: { main: path.join(desktop, 'src/main/engine/integration/measure-main.ts'), 'engine-process': path.join(desktop, 'src/main/engine/engine-process.ts') },
  outdir: work,
  platform: 'node',
  format: 'cjs',
  target: 'node22',
  external: ['electron']
});
await build({ ...common, entryPoints: { preload: path.join(desktop, 'src/preload/index.ts') }, outdir: work, platform: 'browser', format: 'cjs', target: 'es2023', external: ['electron'] });
await build({ ...common, entryPoints: { page: path.join(desktop, 'src/main/engine/integration/measure-page.ts') }, outdir: work, platform: 'browser', format: 'iife', target: 'es2023' });
if (selected.some((run) => run.app)) {
  await build({ ...common, entryPoints: { 'app-main': path.join(desktop, 'src/main/engine/integration/measure-app-main.ts') }, outdir: work, platform: 'node', format: 'cjs', target: 'node22', external: ['electron'] });
  await build({ ...common, entryPoints: { 'app-page': path.join(desktop, 'src/main/engine/integration/measure-app-page.ts') }, outdir: work, platform: 'browser', format: 'iife', target: 'es2023' });
  if (!fs.existsSync(path.join(desktop, 'out/main/index.js'))) throw new Error('the app is not built: bun run build');
  if (!fs.existsSync(project)) throw new Error(`the corpus project is missing: ${project}`);
}
fs.writeFileSync(
  path.join(work, 'measure.html'),
  '<!doctype html><meta charset="utf-8"><title>frame bridge measurement</title><body style="margin:0;background:#000;overflow:hidden">' +
    '<div id="stage" style="position:absolute;inset:0"><canvas id="overlay" style="position:absolute;inset:0;pointer-events:none"></canvas></div><script src="page.js"></script></body>\n'
);

/* ── statistics ────────────────────────────────────────────────────── */

const pct = (xs, p) => {
  if (!xs.length) return null;
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.floor((s.length - 1) * p + 1e-9))];
};
const dist = (xs, digits = 2) => {
  if (!xs.length) return { n: 0 };
  const r = (v) => (v == null ? null : +v.toFixed(digits));
  return { n: xs.length, p50: r(pct(xs, 0.5)), p95: r(pct(xs, 0.95)), p99: r(pct(xs, 0.99)), max: r(pct(xs, 1)) };
};
const PRESENTED = new Set(['STATE_PRESENTED_ALL', 'STATE_PRESENTED_PARTIAL']);

function traceContext(tracePath, pingOffsetUs) {
  const data = JSON.parse(fs.readFileSync(tracePath, 'utf8'));
  const events = Array.isArray(data) ? data : data.traceEvents;
  const origin = events.find((e) => e.name === 'pm-origin' && e.cat === 'blink.user_timing' && e.args?.data?.startTime != null);
  if (!origin) throw new Error('no pm-origin mark in the trace');
  const traceOffsetUs = origin.ts - origin.args.data.startTime * 1000;
  const open = new Map();
  const frames = [];
  for (const e of events.filter((x) => x.name === 'PipelineReporter' && x.pid === origin.pid).sort((a, b) => a.ts - b.ts)) {
    const k = e.id2?.local ?? e.id;
    if (e.ph === 'b') {
      if (!open.has(k)) open.set(k, []);
      open.get(k).push(e);
    } else if (e.ph === 'e') {
      const b = open.get(k)?.shift();
      if (b) frames.push({ begin: b.ts, end: e.ts, state: b.args?.frame_reporter?.state ?? b.args?.chrome_frame_reporter?.state });
    }
  }
  frames.sort((a, b) => a.begin - b.begin);
  const begins = frames.map((f) => f.begin);
  const toTrace = (ms) => traceOffsetUs + ms * 1000;
  const hostToTrace = (us) => us - pingOffsetUs + traceOffsetUs;
  const frameAt = (rafMs) => {
    const t = toTrace(rafMs);
    let lo = 0;
    let hi = begins.length - 1;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (begins[mid] < t) lo = mid + 1;
      else hi = mid;
    }
    let best = null;
    for (const i of [lo - 1, lo, lo + 1]) {
      const f = frames[i];
      if (f && Math.abs(f.begin - t) <= 400 && (!best || Math.abs(f.begin - t) < Math.abs(best.begin - t))) best = f;
    }
    return best;
  };
  // viz's side (Graphics.Pipeline, recorded since the worker canvas runs):
  // compositor frames received per frame sink, and display frames from
  // DrawAndSwap's begin to its end (presentation feedback, the same time
  // PipelineReporter ends at for a page's own frames).
  const receives = new Map();
  for (const e of events) {
    const a = e.name === 'Graphics.Pipeline' && e.ph === 'X' ? e.args?.chrome_graphics_pipeline : null;
    if (a?.step !== 'STEP_RECEIVE_COMPOSITOR_FRAME' || !a.frame_sink_id) continue;
    const k = `${a.frame_sink_id.frame_sink_client_id}:${a.frame_sink_id.frame_sink_id}`;
    if (!receives.has(k)) receives.set(k, []);
    receives.get(k).push(e.ts);
  }
  for (const list of receives.values()) list.sort((a, b) => a - b);
  const swaps = [];
  const openSwaps = new Map();
  for (const e of events.filter((x) => x.name === 'Graphics.Pipeline.DrawAndSwap' && (x.ph === 'b' || x.ph === 'e')).sort((a, b) => a.ts - b.ts)) {
    const k = `${e.pid}:${e.id2?.local ?? e.id}`;
    if (e.ph === 'b') {
      const swap = { begin: e.ts, end: null };
      openSwaps.set(k, swap);
      swaps.push(swap);
    } else if (openSwaps.has(k)) {
      openSwaps.get(k).end = e.ts;
      openSwaps.delete(k);
    }
  }
  const firstAtOrAfter = (xs, t, key = (x) => x) => {
    let lo = 0;
    let hi = xs.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (key(xs[mid]) < t) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  };
  /**
   * The frame sink that carries these submits (ms, page clock): a worker
   * canvas's own sink receives one compositor frame per submit, shortly
   * after it. The page's sink also receives a frame right after most of
   * them during playback (its compositor runs on the same BeginFrames) but
   * many more besides, so among sinks that received a frame within 12 ms
   * of at least 90% of the submits, the one whose frame count is closest
   * to the number of submits.
   *
   * A worker's `performance.timeOrigin` differs from the page's by a
   * fraction of a millisecond beyond what the presenter corrects (measured:
   * its frames reached viz 0.1 to 0.6 ms before their page-clock submit
   * time), so a submit matches a receive up to `SUBMIT_SLACK_US` earlier.
   */
  const SUBMIT_SLACK_US = 2_000;
  const sinkFor = (submitsMs) => {
    let best = null;
    for (const [k, list] of receives) {
      const delays = [];
      for (const ms of submitsMs) {
        const t = toTrace(ms) - SUBMIT_SLACK_US;
        const i = firstAtOrAfter(list, t);
        if (i < list.length && list[i] - t <= 12_000) delays.push(list[i] - t);
      }
      if (delays.length < submitsMs.length * 0.9) continue;
      const excess = Math.abs(list.length - submitsMs.length) / submitsMs.length;
      if (!best || excess < best.excess) best = { k, excess };
    }
    return best?.k ?? null;
  };
  /**
   * A worker canvas's frame submitted at `submitMs`: received by viz on
   * `sink`, then shown by the first display frame drawn after it unless a
   * newer frame of the same sink was received before that draw.
   */
  const vizFrame = (sink, rafMs, submitMs) => {
    const list = receives.get(sink) ?? [];
    const t = toTrace(submitMs) - SUBMIT_SLACK_US;
    const i = firstAtOrAfter(list, t);
    if (i >= list.length || list[i] - t > 50_000) return { begin: toTrace(rafMs), end: null, state: 'VIZ_NOT_RECEIVED' };
    const received = list[i];
    const swap = swaps[firstAtOrAfter(swaps, received, (s) => s.begin)];
    if (!swap?.end) return { begin: toTrace(rafMs), end: null, state: 'VIZ_NOT_DRAWN' };
    if (i + 1 < list.length && list[i + 1] <= swap.begin) return { begin: toTrace(rafMs), end: null, state: 'VIZ_SUPERSEDED' };
    return { begin: toTrace(rafMs), end: swap.end, state: 'STATE_PRESENTED_ALL' };
  };
  return { frames: frames.length, toTrace, hostToTrace, frameAt, sinkFor, vizFrame };
}

/**
 * The main-process load events a missing frame due at host µs `due` is
 * attributed to: those whose main-thread span started before its due time
 * and ended no earlier than 50 ms before it (S1's attribution, which also
 * required the start to be within 300 ms of the due time; a live resize
 * lasts 1.3 s, so that rule called every drop in its last second "none").
 */
function nearLoad(raw, due) {
  const near = (raw.load?.events ?? []).filter(([, u, d]) => u <= due && u + d * 1000 >= due - 50_000).map((e) => e[0]);
  return near.length ? [...new Set(near)].sort().join('+') : 'none';
}

const mediaAt = (sample, hostUs) => sample.mediaTime + ((hostUs - sample.hostTime) * TICKS * sample.rate.num) / (1e6 * sample.rate.den);
const hostOfMedia = (sample, ticks) => sample.hostTime + ((ticks - sample.mediaTime) * 1e6 * sample.rate.den) / (TICKS * sample.rate.num);

/**
 * Content frame timing for one run: the frames expected in the recording
 * window, the animation frame that drew each, and each one's due time
 * (host µs). Engine frames are due on the engine's clock line; today's
 * stand-in on the same line; the real WebGL viewer on its own playback line
 * (`PM.time = base + (rAF − origin) / 1000`, recorded per draw as the
 * offset `rAF − PM.time × 1000`).
 */
function contentFrames(raw, fps) {
  const page = raw.page;
  const rec = raw.recordWindow ?? raw.window;
  const frameTicks = TICKS / fps;
  if (page.mode === 'today' && page.real) {
    const draws = page.today.draws;
    if (!draws.length) throw new Error('the real viewer drew nothing');
    const offsetMs = pct(draws.map((d) => d[5]), 0.5);
    const frameMs = 1000 / fps;
    const byK = new Map();
    for (const d of draws) if (!byK.has(d[1])) byK.set(d[1], d[0]);
    return {
      kFirst: Math.ceil((rec.startMs + 100 - offsetMs) / frameMs),
      kLast: Math.floor((rec.endMs - 100 - offsetMs) / frameMs) - 1,
      rafOf: (k) => byK.get(k),
      dueHostUs: (k) => (offsetMs + k * frameMs) * 1000 + raw.clock.offsetUs,
      line: { realOffsetMs: offsetMs, offsetSpreadMs: +(pct(draws.map((d) => d[5]), 0.99) - pct(draws.map((d) => d[5]), 0.01)).toFixed(3) }
    };
  }
  const line = [...page.clocks].reverse().find((c) => c.state === 'playing' && c.rate.num !== 0);
  if (!line) throw new Error('no playing clock sample');
  const host = (ms) => ms * 1000 + raw.clock.offsetUs;
  const m0 = mediaAt(line, host(rec.startMs));
  const m1 = mediaAt(line, host(rec.endMs));
  // Draws of timed content frames: [rafMs, pts].
  const draws = page.mode === 'engine' ? page.engineTrace.draws.filter((d) => d[7] === 1).map((d) => [d[0], d[3], d[5]]) : page.today.draws.map((d) => [d[0], d[2], null]);
  const byPts = new Map();
  const submits = new Map();
  for (const [rafMs, pts, submitMs] of draws) {
    const k = Math.round(pts / frameTicks);
    if (byPts.has(k)) continue;
    byPts.set(k, rafMs);
    submits.set(k, submitMs);
  }
  return {
    kFirst: Math.ceil((m0 + 0.1 * TICKS) / frameTicks),
    kLast: Math.floor((m1 - 0.1 * TICKS) / frameTicks) - 1,
    rafOf: (k) => byPts.get(k),
    submitOf: (k) => submits.get(k),
    dueHostUs: (k) => hostOfMedia(line, k * frameTicks),
    line
  };
}

function analyzePlayback(raw) {
  const ctx = traceContext(raw.config.trace, raw.clock.offsetUs);
  const periodUs = 1e6 / raw.display.hz;
  const page = raw.page;
  const fps = page.fps ?? FPS;
  const rec = raw.recordWindow ?? raw.window;
  const { kFirst, kLast, rafOf, submitOf, dueHostUs, line } = contentFrames(raw, fps);
  const byPts = { get: rafOf };
  // A presentation worker's canvas has no PipelineReporter: its frames are
  // timed on viz's side (traceContext.vizFrame).
  const offMain = page.mode === 'engine' && !!page.presenter?.offMainThread;
  const sink = offMain ? ctx.sinkFor(page.engineTrace.draws.map((d) => d[5])) : null;
  if (offMain && !sink) throw new Error('no viz frame sink matches the worker canvas (was graphics.pipeline traced?)');
  const presentedFrame = (k, rafMs) => (offMain ? ctx.vizFrame(sink, rafMs, submitOf(k)) : ctx.frameAt(rafMs));
  const hostOfK = dueHostUs;
  const per = [];
  const counts = { expected: 0, notDrawn: 0, notPresented: 0 };
  for (let k = kFirst; k <= kLast; k++) {
    counts.expected++;
    const rafMs = byPts.get(k);
    if (rafMs == null) {
      counts.notDrawn++;
      per.push(null);
      continue;
    }
    const pf = presentedFrame(k, rafMs);
    if (!pf || !PRESENTED.has(pf.state)) {
      counts.notPresented++;
      per.push(null);
      continue;
    }
    per.push({ k, presentUs: pf.end, dueUs: ctx.hostToTrace(hostOfK(k)), rafToPresentUs: pf.end - ctx.toTrace(rafMs) });
  }
  const missing = [];
  for (let k = kFirst, i = 0; k <= kLast; k++, i++) {
    if (per[i]) continue;
    const rafMs = byPts.get(k);
    const state = rafMs == null ? 'not-drawn' : (presentedFrame(k, rafMs)?.state ?? 'no-reporter');
    missing.push({ k, state, near: nearLoad(raw, hostOfK(k)) });
  }
  const presented = per.filter(Boolean);
  const offsets = presented.map((p) => p.presentUs - p.dueUs);
  const median = pct(offsets, 0.5);
  const jitterMs = offsets.map((o) => (o - median) / 1000);
  const intervalErrMs = [];
  for (let i = 1; i < per.length; i++) if (per[i - 1] && per[i]) intervalErrMs.push((per[i].presentUs - per[i - 1].presentUs) / 1000 - 1000 / fps);
  const depth = {};
  for (const p of presented) {
    const d = Math.round(p.rafToPresentUs / periodUs);
    depth[d] = (depth[d] ?? 0) + 1;
  }
  return {
    window: { seconds: +((rec.endMs - rec.startMs) / 1000).toFixed(2), displayHz: raw.display.hz, contentFps: fps, reporterFrames: ctx.frames },
    presentation: offMain
      ? { thread: 'worker', timing: 'viz', sink }
      : {
          thread: 'main',
          timing: 'PipelineReporter',
          // The viz timing applied to the page's own frames, against
          // PipelineReporter: how far apart the two put the same frame.
          vizCheck: page.mode === 'engine' ? (() => {
            const pageSink = ctx.sinkFor(page.engineTrace.draws.map((d) => d[5]));
            const diffs = [];
            for (let k = kFirst; k <= kLast; k++) {
              const rafMs = rafOf(k);
              if (rafMs == null) continue;
              const a = ctx.frameAt(rafMs);
              const b = pageSink ? ctx.vizFrame(pageSink, rafMs, submitOf(k)) : null;
              if (a && b?.end && PRESENTED.has(a.state)) diffs.push((b.end - a.end) / 1000);
            }
            return { sink: pageSink, vizMinusReporterMs: dist(diffs, 2) };
          })() : undefined
        },
    ...(page.real ? { app: { project: page.project, stage: page.stage, quality: page.quality, line: page.mode === 'today' ? line : undefined, qualityDuringRun: page.mode === 'today' ? dist(page.today.draws.map((d) => d[6]), 2) : undefined } } : {}),
    frames: { ...counts, drops: counts.notDrawn + counts.notPresented, presented: presented.length, offSlot: jitterMs.filter((j) => Math.abs(j) > periodUs / 2000).length },
    jitterAbsMs: dist(jitterMs.map(Math.abs), 3),
    intervalErrorAbsMs: dist(intervalErrMs.map(Math.abs), 3),
    irregularIntervals: intervalErrMs.filter((e) => Math.abs(e) > periodUs / 2000).length,
    rafToPresentMs: dist(presented.map((p) => p.rafToPresentUs / 1000), 2),
    // Presentation minus due time on the clock line (the A/V offset the
    // jitter numbers remove; section 15 keeps A/V sync within ±10 ms).
    dueToPresentMs: dist(offsets.map((o) => o / 1000), 2),
    ...(page.mode === 'engine'
      ? (() => {
          const t = page.engineTrace;
          // Renderer time of each draw (rAF to submit), and when each timed
          // frame arrived relative to its due time (negative: early; the
          // key says "lead" but the value is arrival minus due).
          const drawMs = t.draws.map((d) => d[5] - d[0]);
          const leadMs = t.arrivals.filter((a) => a[4] === 1).map((a) => (hostOfMedia(line, a[1]) - raw.clock.offsetUs) / 1000 - a[2]);
          const gaps = [];
          for (let i = 1; i < t.rafs.length; i++) gaps.push(t.rafs[i] - t.rafs[i - 1]);
          return { drawMainThreadMs: dist(drawMs, 3), arrivalLeadMs: dist(leadMs.map((x) => -x), 2), rafGapMs: dist(gaps, 2), rafGapsOver25Ms: gaps.filter((g) => g > 25).length };
        })()
      : (() => {
          const gaps = [];
          const r = page.today.rafs;
          for (let i = 1; i < r.length; i++) gaps.push(r[i] - r[i - 1]);
          return { rafGapMs: dist(gaps, 2), rafGapsOver25Ms: gaps.filter((g) => g > 25).length };
        })()),
    missing,
    pipelineDepthRefreshes: depth,
    presenter: page.presenter,
    receiver: page.receiver
  };
}

/**
 * The C′ prototype's playback: the engine asked Core Animation to show
 * each timed frame at its due time (`presentDrawable:atTime:`) and Core
 * Animation reported when it did (`presentedTime`, 0 for a drawable it
 * never showed). Jitter is presentation minus due time, median removed, as
 * for the Chromium paths; an interval is irregular when it is more than half
 * a refresh off the frame duration.
 */
function analyzeRemote(raw, jsonl, ticksText = '') {
  const page = raw.page;
  const fps = page.fps ?? FPS;
  const frameTicks = TICKS / fps;
  const periodUs = 1e6 / raw.display.hz;
  const rec = raw.recordWindow ?? raw.window;
  const line = [...page.clocks].reverse().find((c) => c.state === 'playing' && c.rate.num !== 0);
  if (!line) throw new Error('no playing clock sample');
  const records = jsonl.split('\n').filter(Boolean).map((l) => JSON.parse(l));
  const host = (ms) => ms * 1000 + raw.clock.offsetUs;
  const kFirst = Math.ceil((mediaAt(line, host(rec.startMs)) + 0.1 * TICKS) / frameTicks);
  const kLast = Math.floor((mediaAt(line, host(rec.endMs)) - 0.1 * TICKS) / frameTicks) - 1;
  // [seq, asked, presented, committed, drawable wait µs] (the last two from
  // the run that added them on).
  const byAsked = new Map(records.map(([seq, asked, presented, committed, waitUs]) => [Math.round(asked / 100), { seq, asked, presented, committed, waitUs }]));
  const per = [];
  const missing = [];
  const counts = { expected: 0, notAsked: 0, notPresented: 0 };
  for (let k = kFirst; k <= kLast; k++) {
    counts.expected++;
    const due = hostOfMedia(line, k * frameTicks);
    const r = byAsked.get(Math.round(due / 100)) ?? byAsked.get(Math.round(due / 100) - 1) ?? byAsked.get(Math.round(due / 100) + 1);
    if (!r) {
      counts.notAsked++;
      per.push(null);
      missing.push({ k, state: 'not-asked', near: nearLoad(raw, due) });
    } else if (r.presented == null) {
      counts.notPresented++;
      per.push(null);
      missing.push({ k, state: 'not-presented', near: nearLoad(raw, due) });
    } else per.push({ k, presentUs: r.presented, dueUs: due, committed: r.committed, waitUs: r.waitUs });
  }
  const presented = per.filter(Boolean);
  const offsets = presented.map((p) => p.presentUs - p.dueUs);
  const median = pct(offsets, 0.5);
  const jitterMs = offsets.map((o) => (o - median) / 1000);
  const intervalErrMs = [];
  for (let i = 1; i < per.length; i++) if (per[i - 1] && per[i]) intervalErrMs.push((per[i].presentUs - per[i - 1].presentUs) / 1000 - 1000 / fps);
  return {
    path: 'cprime',
    window: { seconds: +((rec.endMs - rec.startMs) / 1000).toFixed(2), displayHz: raw.display.hz, contentFps: fps, records: records.length },
    frames: { ...counts, drops: counts.notAsked + counts.notPresented, presented: presented.length, offSlot: jitterMs.filter((j) => Math.abs(j) > periodUs / 2000).length },
    jitterAbsMs: dist(jitterMs.map(Math.abs), 3),
    intervalErrorAbsMs: dist(intervalErrMs.map(Math.abs), 3),
    irregularIntervals: intervalErrMs.filter((e) => Math.abs(e) > periodUs / 2000).length,
    dueToPresentMs: dist(offsets.map((o) => o / 1000), 2),
    // Commit lead: how long before its due time each frame's copy reached
    // Core Animation. Late frames (more than half a refresh after the
    // median) committed well ahead were held up by the WindowServer.
    ...(presented.some((p) => p.committed)
      ? (() => {
          const late = presented.filter((p) => p.presentUs - p.dueUs - median > periodUs / 2);
          const lead = (p) => (p.dueUs - p.committed) / 1000;
          return {
            commitLeadMs: dist(presented.map(lead), 2),
            drawableWaitMs: dist(presented.map((p) => p.waitUs / 1000), 2),
            lateFrames: { n: late.length, commitLeadMs: dist(late.map(lead), 2), drawableWaitMs: dist(late.map((p) => p.waitUs / 1000), 2) }
          };
        })()
      : {}),
    // Frames the layer skipped to drain Core Animation's queue after a stall
    // (Pacing in remote_layer.rs): sequence numbers missing from the records.
    skippedSeqs: (() => {
      let n = 0;
      for (let i = 1; i < records.length; i++) if (records[i][0] > records[i - 1][0] + 1) n += records[i][0] - records[i - 1][0] - 1;
      return n;
    })(),
    missing,
    remoteLayers: raw.remoteLayers ?? null,
    // With the refresh-rate request: the refresh intervals the request's
    // display link saw (ms, 0.5 ms bins), i.e. the rate the panel ran at.
    refreshIntervalsMs: (() => {
      const ticks = ticksText.split('\n').filter(Boolean).map(Number);
      if (ticks.length < 2) return null;
      const bins = {};
      for (let i = 1; i < ticks.length; i++) {
        const ms = Math.round(((ticks[i] - ticks[i - 1]) / 1000) * 2) / 2;
        bins[ms] = (bins[ms] ?? 0) + 1;
      }
      return bins;
    })(),
    engineStatus: raw.engineStatus ?? null,
    presenter: page.presenter,
    receiver: page.receiver
  };
}

function analyzeLatency(raw) {
  const out = {};
  for (const [mode, run] of Object.entries(raw.latency)) {
    const ctx = traceContext(raw.config.trace, raw.clock.offsetUs);
    const page = run.page;
    const revOf = new Map();
    for (const [rev, seqs] of page.inputs) for (const s of seqs) if (!revOf.has(s)) revOf.set(s, rev);
    const draws = (page.mode === 'engine' ? page.engineTrace.draws.map((d) => ({ rafMs: d[0], rev: d[6], submitMs: d[5] })) : page.today.draws.map((d) => ({ rafMs: d[0], rev: d[3], submitMs: null }))).sort((a, b) => a.rafMs - b.rafMs);
    const offMain = page.mode === 'engine' && !!page.presenter?.offMainThread;
    const sink = offMain ? ctx.sinkFor(draws.map((d) => d.submitMs)) : null;
    const lat = [];
    let unmatched = 0;
    let unpresented = 0;
    for (const [seq, tsUs] of run.generated) {
      const rev = revOf.get(seq);
      const d = rev == null ? null : draws.find((x) => x.rev >= rev);
      if (!d) {
        unmatched++;
        continue;
      }
      const pf = offMain ? ctx.vizFrame(sink, d.rafMs, d.submitMs) : ctx.frameAt(d.rafMs);
      if (!pf || !PRESENTED.has(pf.state)) {
        unpresented++;
        continue;
      }
      lat.push((pf.end - ctx.hostToTrace(tsUs)) / 1000);
    }
    out[mode] = { events: run.generated.length, dispatches: page.inputs.length, draws: draws.length, unmatched, unpresented, presentation: offMain ? { thread: 'worker', timing: 'viz', sink } : { thread: 'main', timing: 'PipelineReporter' }, pointerToPixelMs: dist(lat, 2) };
  }
  return out;
}

/* ── run ───────────────────────────────────────────────────────────── */

fs.mkdirSync(summaries, { recursive: true });
const electron = createRequire(path.join(desktop, 'package.json'))('electron');
const results = {};
try {
  for (const run of selected) {
    const name = tag ? `${run.name}-${tag}` : run.name;
    const out = path.join(from ?? work, `${name}.json`);
    const trace = path.join(from ?? work, `${name}.trace.json`);
    const env = {
      ...process.env,
      PM_IT_DESKTOP: desktop,
      PM_IT_OUT: work,
      PM_MEASURE: JSON.stringify({ ...run, out, trace }),
      POWERMOVE_PM_BRIDGE: harnessAddon,
      POWERMOVE_ENGINE_FRAME_SOURCE: 'test-pattern',
      ...(run.lead ? { POWERMOVE_BRIDGE_LEAD: String(run.lead) } : {}),
      ...(run.env ?? {}),
      ELECTRON_DISABLE_SECURITY_WARNINGS: '1'
    };
    delete env.ELECTRON_RUN_AS_NODE;
    const remoteTrace = path.join(from ?? work, `${name}.remote.jsonl`);
    if (run.fps && !run.app) env.POWERMOVE_BRIDGE_TEST_FPS = String(run.fps);
    if (run.remoteLayer) Object.assign(env, { POWERMOVE_BRIDGE_REMOTE_LAYER: '1', POWERMOVE_BRIDGE_REMOTE_TRACE: remoteTrace, POWERMOVE_BRIDGE_LEAD: String(run.lead ?? 2) });
    let args = [path.join(work, 'main.js')];
    let profile = null;
    if (run.app) {
      // The real app as a hidden background test launch with its own profile.
      profile = fs.mkdtempSync(path.join(os.tmpdir(), 'pm-measure-app-'));
      fs.writeFileSync(path.join(profile, 'onboarding-v1.json'), '{"version":1}\n', { mode: 0o600 });
      Object.assign(env, {
        PM_MEASURE_APP: JSON.stringify({ ...run, project, startS: run.startS ?? 1, pageScript: path.join(work, 'app-page.js'), out, trace }),
        POWERMOVE_BACKGROUND_TEST: '1',
        POWERMOVE_USER_DATA: profile,
        POWERMOVE_DEVTOOLS: '0',
        POWERMOVE_NEW_ENGINE: run.mode === 'engine' ? '1' : '0',
        // The engine's test source at the project's rate (the Powermove ad is 30 fps).
        POWERMOVE_BRIDGE_TEST_FPS: String(run.fps ?? 30),
        CODEX_BINARY: path.join(desktop, 'src/main/codex/__fixtures__/fake-codex-app-server.sh'),
        POWERMOVE_FAKE_CHATGPT_STATUS: 'connected'
      });
      args = ['-r', path.join(work, 'app-main.js'), desktop];
    }
    process.stdout.write(`\n== ${name}\n`);
    const t0 = Date.now();
    const r = from
      ? { stdout: fs.existsSync(out) && !JSON.parse(fs.readFileSync(out, 'utf8')).error ? 'MEASURE DONE (kept)' : 'MEASURE FAIL (kept)', status: 0 }
      : spawnSync(electron, args, { env, encoding: 'utf8', timeout: run.durationMs * (run.inputModes?.length ?? 1) * 3 + (run.app ? 1_200_000 : 240_000) });
    if (profile) fs.rmSync(profile, { recursive: true, force: true });
    const verdict = (r.stdout ?? '').split('\n').find((l) => l.startsWith('MEASURE')) ?? `MEASURE FAIL exit ${r.status}`;
    process.stdout.write(`   ${verdict} after ${((Date.now() - t0) / 1000).toFixed(1)} s\n`);
    if (!fs.existsSync(out) || !verdict.startsWith('MEASURE DONE')) {
      if (fs.existsSync(out)) process.stdout.write(`${(JSON.parse(fs.readFileSync(out, 'utf8')).log ?? []).slice(-15).join('\n')}\n`);
      continue;
    }
    const raw = JSON.parse(fs.readFileSync(out, 'utf8'));
    if (run.fps && !run.app && raw.page) raw.page.fps ??= run.fps;
    let summary;
    try {
      summary = {
        run: name,
        config: { scenario: run.scenario, app: !!run.app, remoteLayer: !!run.remoteLayer, placement: run.placement ?? 'offscreen', env: run.env, mode: run.mode, load: run.load, loadProfile: run.load ? (run.loadProfile ?? 'gate') : null, edr: run.edr ?? 'auto', rawInput: !!run.rawInput, rawGestures: run.app && run.mode === 'engine' ? run.rawGestures !== false : undefined, offMainThread: run.app && run.mode === 'engine' ? run.offMainThread !== false : undefined, fps: run.fps ?? (run.app ? 30 : 60), lead: run.lead ?? 4, durationMs: run.durationMs, inputModes: run.inputModes },
        versions: raw.versions,
        display: raw.display,
        machineLoad: raw.machineLoad,
        ...(raw.project ? { project: raw.project } : {}),
        clock: raw.clock,
        load: raw.load ? { ...raw.load, events: undefined, eventCount: raw.load.events?.length } : null,
        bridge: raw.bridge,
        ...(run.scenario === 'playback'
          ? run.remoteLayer
            ? { playback: analyzeRemote(raw, fs.existsSync(remoteTrace) ? fs.readFileSync(remoteTrace, 'utf8') : '', fs.existsSync(`${remoteTrace}.ticks`) ? fs.readFileSync(`${remoteTrace}.ticks`, 'utf8') : '') }
            : { playback: analyzePlayback(raw) }
          : { latency: analyzeLatency(raw) })
      };
    } catch (error) {
      process.stdout.write(`   analysis failed: ${error.message}\n${(raw.log ?? []).slice(-15).join('\n')}\n`);
      continue;
    }
    fs.writeFileSync(path.join(summaries, `${name}.json`), `${JSON.stringify(summary, null, 2)}\n`);
    results[name] = summary;
    if (summary.playback) {
      const p = summary.playback;
      process.stdout.write(`   drops ${p.frames.drops}/${p.frames.expected}  |jitter| p50 ${p.jitterAbsMs.p50} p99 ${p.jitterAbsMs.p99} ms  irregular ${p.irregularIntervals}  ${p.rafToPresentMs ? `rAF→present p50 ${p.rafToPresentMs.p50}` : `due→present p50 ${p.dueToPresentMs?.p50}`}\n`);
    } else {
      for (const [mode, l] of Object.entries(summary.latency)) process.stdout.write(`   ${mode}: pointer-to-pixel p50 ${l.pointerToPixelMs.p50} p99 ${l.pointerToPixelMs.p99} ms (${l.pointerToPixelMs.n} of ${l.events}, unmatched ${l.unmatched}, unpresented ${l.unpresented})\n`);
    }
  }
} finally {
  // --keep leaves the raw results, traces, and logs for debugging.
  if (process.argv.includes('--keep')) process.stdout.write(`raw results kept in ${work}\n`);
  else fs.rmSync(work, { recursive: true, force: true });
}
