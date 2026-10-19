/*
 * The production frame bridge measured with the S1 harness (plan section
 * 18; amendment 5's gate): the engine path is no worse than today's viewer
 * on the same harness, at p50 and p99, idle and under the gate load.
 *
 * Builds the pm-bridge addon with its `harness` feature (synthetic input),
 * and for C′ runs the pm-node addon with its `remote-layer` feature,
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
 * Runs need the main display awake and the session unlocked: with the
 * display asleep nothing is scanned out, Core Animation presents no C′
 * drawable, and Chromium's presentation times are its own estimates. A run
 * is skipped when it starts that way (--wait-display waits up to 12 hours
 * instead) and its summary is marked invalid when the state changed during
 * it. --allow-display-asleep measures anyway (harness debugging only).
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

import { rawText } from './esbuild-raw-text.mjs';

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
  // Agent snapshots during playback (section 25 Phase 2 row): 4 frames at
  // 720 px per agent turn, a turn every 2 s, a 24-layer 4K project.
  { name: 'playback-engine-agent-snapshots', ...playback, mode: 'engine', load: false, agentSnapshots: { turnMs: 2000, frames: 4, width: 720, layers: 24 }, env: { POWERMOVE_ENGINE_STATUS_LOG_MS: '500' } },
  { name: 'playback-engine-agent-snapshots-busy', ...playback, mode: 'engine', load: false, agentSnapshots: { turnMs: 0, frames: 4, width: 720, layers: 24 }, env: { POWERMOVE_ENGINE_STATUS_LOG_MS: '500' } },
  // An over-bound pass (contract 13's 2 ms bound): 1500 solids in a
  // full-size 4K precomposition, which renders whole for every band, at
  // 3840 px stills; a 16-row band takes about 6.6 ms of GPU time idle
  // (pm-export measure_heavy_precomp_band_cost). Cold: first met during
  // playback; warm: one agent turn before playback.
  { name: 'playback-engine-agent-snapshots-heavy', ...playback, mode: 'engine', load: false, agentSnapshots: { turnMs: 0, frames: 2, width: 3840, layers: 1500, heavy: 'precomp' }, env: { POWERMOVE_ENGINE_STATUS_LOG_MS: '500' } },
  { name: 'playback-engine-agent-snapshots-heavy-warm', ...playback, mode: 'engine', load: false, agentSnapshots: { turnMs: 0, frames: 2, width: 3840, layers: 1500, heavy: 'precomp', warm: true }, env: { POWERMOVE_ENGINE_STATUS_LOG_MS: '500' } },
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
  // The Chromium path with main asking for the multiple through public API
  // (a display link with a preferred rate; Electron has no request of its own).
  { name: 'playback-engine-25-idle-behind', ...playback, mode: 'engine', load: false, fps: 25, placement: 'behind' },
  { name: 'playback-engine-25-rate-idle', ...playback, mode: 'engine', load: false, fps: 25, placement: 'behind', mainRate: true },
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
  { name: 'app-latency-engine-load', ...latency, app: true, mode: 'engine', load: true, loadProfile: 'no-autosave', inputModes: ['pan'] },
  // The Phase 2 tracer bullet (docs/engine/tracer.md): the app's own engine
  // connection draws a synthetic project of slice features (Use New
  // Engine on); "today" is the WebGL viewer on the same project. Summaries
  // go to docs/engine/tracer-measurements/.
  ...tracerRuns(),
  // Phase 3a (docs/engine/phase3a-gate.md): layer drag on the 884-layer
  // project (the corpus perf copy), today and with Use New Engine on, idle.
  { name: 'p3a-drag-today-884-idle', ...latency, app: true, mode: 'today', load: false, inputModes: ['drag'], dir: 'docs/engine/phase3a-gate-measurements' },
  { name: 'p3a-drag-engine-884-idle', ...latency, app: true, mode: 'tracer', load: false, inputModes: ['drag'], env: { POWERMOVE_ENGINE_FRAME_SOURCE: 'document' }, dir: 'docs/engine/phase3a-gate-measurements' }
];

/** The tracer's runs: layer drag on 500 layers (engine and today, idle and
 *  under load), an inspector edit every 250 ms during playback of 1,000
 *  layers, and cadence at 23.976 and 25 fps. */
function tracerRuns() {
  const doc = { POWERMOVE_ENGINE_FRAME_SOURCE: 'document' };
  const s500 = { layers: 500, images: 2 };
  const s1000 = { layers: 1000, images: 2, dur: 90 };
  return [
    { name: 'tracer-drag-today-500-idle', ...latency, app: true, tracer: true, mode: 'today', load: false, synthetic: s500, inputModes: ['drag'] },
    { name: 'tracer-drag-engine-500-idle', ...latency, app: true, tracer: true, mode: 'tracer', load: false, synthetic: s500, inputModes: ['drag'], env: doc },
    { name: 'tracer-drag-today-500-load', ...latency, app: true, tracer: true, mode: 'today', load: true, loadProfile: 'no-autosave', synthetic: s500, inputModes: ['drag'] },
    { name: 'tracer-drag-engine-500-load', ...latency, app: true, tracer: true, mode: 'tracer', load: true, loadProfile: 'no-autosave', synthetic: s500, inputModes: ['drag'], env: doc },
    { name: 'tracer-playback-engine-1000', ...playback, app: true, tracer: true, mode: 'tracer', load: false, synthetic: s1000, startS: 0, env: doc },
    { name: 'tracer-edit-playback-engine-1000', ...playback, app: true, tracer: true, mode: 'tracer', load: false, synthetic: s1000, edits: { everyMs: 250 }, startS: 0, env: doc },
    // The same edits with Use New Engine off (docs/engine/perf.md: the page's per-edit cost on the legacy path).
    { name: 'tracer-edit-playback-today-1000', ...playback, app: true, tracer: true, mode: 'today', load: false, synthetic: s1000, edits: { everyMs: 250 }, startS: 0 },
    { name: 'tracer-playback-engine-25', ...playback, app: true, tracer: true, mode: 'tracer', load: false, synthetic: { ...s500, fps: 25, dur: 90 }, startS: 0, fps: 25, env: doc },
    { name: 'tracer-playback-engine-23976', ...playback, app: true, tracer: true, mode: 'tracer', load: false, synthetic: { ...s500, fpsRational: [24000, 1001], dur: 90 }, startS: 0, fps: 24000 / 1001, env: doc },
    { name: 'tracer-playback-engine-30-load', ...playback, app: true, tracer: true, mode: 'tracer', load: true, loadProfile: 'no-autosave', synthetic: { ...s500, dur: 90 }, startS: 0, env: doc },
    // A/V sync and cadence at each rate, idle and under the gate load.
    { name: 'tracer-playback-engine-30', ...playback, app: true, tracer: true, mode: 'tracer', load: false, synthetic: { ...s500, dur: 90 }, startS: 0, env: doc },
    { name: 'tracer-playback-engine-25-load', ...playback, app: true, tracer: true, mode: 'tracer', load: true, loadProfile: 'no-autosave', synthetic: { ...s500, fps: 25, dur: 90 }, startS: 0, fps: 25, env: doc },
    { name: 'tracer-playback-engine-23976-load', ...playback, app: true, tracer: true, mode: 'tracer', load: true, loadProfile: 'no-autosave', synthetic: { ...s500, fpsRational: [24000, 1001], dur: 90 }, startS: 0, fps: 24000 / 1001, env: doc },
    // Today's WebGL viewer on the same projects: the cadence bar is today's
    // irregular intervals or fewer at each rate (amendment 22, perf.md).
    { name: 'tracer-playback-today-30', ...playback, app: true, tracer: true, mode: 'today', load: false, synthetic: { ...s500, dur: 90 }, startS: 0 },
    { name: 'tracer-playback-today-25', ...playback, app: true, tracer: true, mode: 'today', load: false, synthetic: { ...s500, fps: 25, dur: 90 }, startS: 0, fps: 25 },
    { name: 'tracer-playback-today-23976', ...playback, app: true, tracer: true, mode: 'today', load: false, synthetic: { ...s500, fpsRational: [24000, 1001], dur: 90 }, startS: 0, fps: 24000 / 1001 },
    { name: 'tracer-playback-today-30-load', ...playback, app: true, tracer: true, mode: 'today', load: true, loadProfile: 'no-autosave', synthetic: { ...s500, dur: 90 }, startS: 0 },
    { name: 'tracer-playback-today-25-load', ...playback, app: true, tracer: true, mode: 'today', load: true, loadProfile: 'no-autosave', synthetic: { ...s500, fps: 25, dur: 90 }, startS: 0, fps: 25 },
    { name: 'tracer-playback-today-23976-load', ...playback, app: true, tracer: true, mode: 'today', load: true, loadProfile: 'no-autosave', synthetic: { ...s500, fpsRational: [24000, 1001], dur: 90 }, startS: 0, fps: 24000 / 1001 }
  ];
}
const selected = RUNS.filter((run) => !only.length || only.includes(run.name));

/* ── build ─────────────────────────────────────────────────────────── */

const work = fs.mkdtempSync(path.join(os.tmpdir(), 'pm-bridge-measure-'));
const harnessAddon = path.join(work, 'pm_bridge_harness.node');
if (!process.argv.includes('--skip-build') && !from) {
  execFileSync('/bin/sh', ['-c', '. ./env.sh && exec nice -n 15 cargo build --release --locked -p pm-bridge --lib --features harness'], { cwd: engineDir, stdio: 'inherit' });
}
fs.copyFileSync(path.join(engineDir, 'target/release/libpm_bridge.dylib'), harnessAddon);
// The C′ prototype is compiled into pm-node only with its `remote-layer`
// feature (never in release builds): its runs load an addon built with it.
const remoteAddon = path.join(work, 'pm_node_remote_layer.node');
if (selected.some((run) => run.remoteLayer) && !from) {
  if (!process.argv.includes('--skip-build')) {
    execFileSync('/bin/sh', ['-c', '. ./env.sh && exec nice -n 15 cargo build --release --locked -p pm-node --features node,remote-layer'], { cwd: engineDir, stdio: 'inherit' });
  }
  fs.copyFileSync(path.join(engineDir, 'target/release/libpm_node.dylib'), remoteAddon);
}
const harness = createRequire(import.meta.url)(harnessAddon);
const allowAsleep = process.argv.includes('--allow-display-asleep');
const waitDisplay = process.argv.includes('--wait-display');
/** Whether the panel shows what a run measures: display awake, session unlocked. */
const displayUsable = (state) => !state.asleep && !state.locked;
const sleepMs = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const common = { bundle: true, logLevel: 'warning', alias: { powermove: path.join(desktop, 'src/renderer/src/kernel/api.ts') }, plugins: [rawText] };
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
  if (selected.some((run) => run.app && !run.synthetic) && !fs.existsSync(project)) throw new Error(`the corpus project is missing: ${project}`);
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
  const traceToHost = (us) => us - traceOffsetUs + pingOffsetUs;
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
    return { begin: toTrace(rafMs), end: swap.end, received, swapBegin: swap.begin, state: 'STATE_PRESENTED_ALL' };
  };
  // viz's BeginFrames (frame times, trace µs): which refresh drew and showed a frame.
  const beginFrames = events.filter((e) => e.name === 'DisplayScheduler::BeginFrame' && e.args?.args?.frame_time_us != null).map((e) => e.args.args.frame_time_us).sort((a, b) => a - b);
  /** The index of the BeginFrame whose interval contains trace µs `t`. */
  const beginFrameAt = (t) => firstAtOrAfter(beginFrames, t + 1) - 1;
  return { frames: frames.length, toTrace, hostToTrace, traceToHost, frameAt, sinkFor, vizFrame, beginFrames, beginFrameAt };
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
  const draws = page.mode === 'engine' || page.mode === 'tracer' ? page.engineTrace.draws.filter((d) => d[7] === 1).map((d) => [d[0], d[3], d[5]]) : page.today.draws.map((d) => [d[0], d[2], null]);
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
  const engineLike = page.mode === 'engine' || page.mode === 'tracer';
  const offMain = engineLike && !!page.presenter?.offMainThread;
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
    per.push({ k, presentUs: pf.end, dueUs: ctx.hostToTrace(hostOfK(k)), rafToPresentUs: pf.end - ctx.toTrace(rafMs), pf, rafUs: ctx.toTrace(rafMs), submitUs: submitOf?.(k) != null ? ctx.toTrace(submitOf(k)) : null });
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
          vizCheck: engineLike ? (() => {
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
    // A/V sync (section 15): each frame's presentation against the audio
    // clock's line in effect at that instant (the newest clock sample made
    // before it), rather than the run's last sample: the engine re-anchors
    // its line to the audio device every 100 ms, and an output glitch moves
    // it, so one line for the whole run charges such a jump to every frame
    // before it.
    ...(line && page.clocks?.length && line.hostTime != null ? { avSync: avSync(raw, presented, ctx, fps) } : {}),
    ...(offMain ? { vizHopsByDepth: vizHopsByDepth(presented, periodUs), vizLags: vizLags(per, ctx, fps, periodUs) } : {}),
    ...(page.edits?.length ? { edits: analyzeEdits(page, ctx, sink, offMain) } : {}),
    ...(engineLike
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
 * Presentation minus the frame's start on the audio clock's line in effect
 * at the presentation (ms): signed, absolute, and the share within ±10 ms.
 */
function avSync(raw, presented, ctx, fps) {
  const samples = raw.page.clocks.filter((c) => c.state === 'playing' && c.rate.num !== 0).sort((a, b) => a.hostTime - b.hostTime);
  const frameTicks = TICKS / fps;
  const out = [];
  /** Frames outside ±10 ms, by the main-process load event near them. */
  const outsideNear = {};
  /** Every frame's signed offset, by the load event near it (diagnostics). */
  const nearOffsets = {};
  for (const p of presented) {
    const host = ctx.traceToHost(p.presentUs);
    let lo = 0;
    let hi = samples.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (samples[mid].hostTime <= host) lo = mid + 1;
      else hi = mid;
    }
    const sample = samples[Math.max(0, lo - 1)];
    const offset = (host - hostOfMedia(sample, p.k * frameTicks)) / 1000;
    out.push(offset);
    const near = nearLoad(raw, host);
    (nearOffsets[near] ??= []).push(offset);
    if (Math.abs(offset) > 10) outsideNear[near] = (outsideNear[near] ?? 0) + 1;
  }
  return {
    outsideNear,
    offsetNearMs: Object.fromEntries(Object.entries(nearOffsets).map(([k, xs]) => [k, { ...dist(xs, 2), p5: +pct(xs, 0.05).toFixed(2) }])),
    offsetMs: dist(out, 2),
    absOffsetMs: dist(out.map(Math.abs), 2),
    p5Ms: out.length ? +pct(out, 0.05).toFixed(2) : null,
    within10Ms: out.filter((o) => Math.abs(o) <= 10).length,
    n: out.length
  };
}

/**
 * Where a worker canvas's timed frames spend the time from their animation
 * frame to the panel, grouped by how many refreshes that took (ms after the
 * animation frame's timestamp): the draw's submit (page clock), viz receiving the
 * canvas's compositor frame, the start and end of viz's DrawAndSwap that
 * showed it.
 */
function vizHopsByDepth(presented, periodUs) {
  const groups = {};
  for (const p of presented) {
    if (p.pf?.received == null) continue;
    const depth = Math.round(p.rafToPresentUs / periodUs);
    (groups[depth] ??= []).push(p);
  }
  return Object.fromEntries(
    Object.entries(groups).map(([depth, ps]) => [
      depth,
      {
        n: ps.length,
        submitted: dist(ps.filter((p) => p.submitUs != null).map((p) => (p.submitUs - p.rafUs) / 1000), 2),
        received: dist(ps.map((p) => (p.pf.received - p.rafUs) / 1000), 2),
        drawBegin: dist(ps.map((p) => (p.pf.swapBegin - p.rafUs) / 1000), 2),
        presented: dist(ps.map((p) => (p.pf.end - p.rafUs) / 1000), 2)
      }
    ])
  );
}

/**
 * Why a worker canvas's frames reach the panel at different refreshes,
 * counted on viz's BeginFrames: `draw` is how many BeginFrames after the
 * animation frame that picked it viz drew the frame (0: the canvas's
 * compositor frame arrived before viz drew for that BeginFrame; 1: just
 * after, so it waited a refresh), `present` how many BeginFrames after that
 * draw it was presented. An irregular interval is attributed to the lag
 * that changed between its two frames.
 */
function vizLags(per, ctx, fps, periodUs) {
  if (!ctx.beginFrames.length) return null;
  const lag = (p) => {
    if (!p?.pf?.swapBegin) return null;
    const picked = firstAtOrAfterSorted(ctx.beginFrames, p.rafUs - 2000);
    const drawn = ctx.beginFrameAt(p.pf.swapBegin);
    const shown = ctx.beginFrameAt(p.pf.end);
    return { draw: drawn - picked, present: shown - drawn };
  };
  const counts = { draw: {}, present: {} };
  const irregular = { drawChanged: 0, presentChanged: 0, both: 0, neither: 0 };
  let prev = null;
  for (const p of per) {
    const l = p ? lag(p) : null;
    if (l) {
      counts.draw[l.draw] = (counts.draw[l.draw] ?? 0) + 1;
      counts.present[l.present] = (counts.present[l.present] ?? 0) + 1;
    }
    if (l && prev?.l && p.k === prev.p.k + 1) {
      const err = (p.presentUs - prev.p.presentUs) / 1000 - 1000 / fps;
      if (Math.abs(err) > periodUs / 2000) {
        const d = l.draw !== prev.l.draw;
        const q = l.present !== prev.l.present;
        irregular[d && q ? 'both' : d ? 'drawChanged' : q ? 'presentChanged' : 'neither']++;
      }
    }
    prev = l ? { p, l } : null;
  }
  return { counts, irregular };
}

function firstAtOrAfterSorted(xs, t) {
  let lo = 0;
  let hi = xs.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (xs[mid] < t) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/**
 * Inspector edits during playback (tracer): how long the renderer held its
 * main thread to apply each one and hand it to the engine, and from the
 * edit to the presentation of the first engine frame whose revision
 * includes it (viz timing for the worker's canvas).
 */
function analyzeEdits(page, ctx, sink, offMain) {
  // Use New Engine off: no engine frames, only the page's cost of each edit.
  if (!page.engineTrace) return { n: page.edits.length, applyMs: dist(page.edits.map(([, t0, t1]) => t1 - t0), 3) };
  const draws = page.engineTrace.draws.map((d) => ({ rafMs: d[0], rev: d[2], submitMs: d[5] })).sort((a, b) => a.rafMs - b.rafMs);
  const applyMs = [];
  const toPresentMs = [];
  let unpresented = 0;
  for (const [rev, t0, t1] of page.edits) {
    applyMs.push(t1 - t0);
    const d = draws.find((x) => x.rev >= rev && x.rafMs >= t0);
    const pf = d ? (offMain ? ctx.vizFrame(sink, d.rafMs, d.submitMs) : ctx.frameAt(d.rafMs)) : null;
    if (pf && PRESENTED.has(pf.state)) toPresentMs.push((pf.end - ctx.toTrace(t0)) / 1000);
    else unpresented++;
  }
  return { n: page.edits.length, applyMs: dist(applyMs, 3), editToPresentMs: dist(toPresentMs, 2), unpresented };
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
    // Engine frames echo the view sequence (pans) or, on the tracer, the
    // document revision (layer drags).
    const engineLike = page.mode === 'engine' || page.mode === 'tracer';
    const draws = (engineLike ? page.engineTrace.draws.map((d) => ({ rafMs: d[0], rev: page.mode === 'tracer' ? d[2] : d[6], submitMs: d[5], arrivedMs: d[4], seq: d[1] })) : page.today.draws.map((d) => ({ rafMs: d[0], rev: d[3], submitMs: null, arrivedMs: null }))).sort((a, b) => a.rafMs - b.rafMs);
    // When the page handled each input (ms, page clock): the first dispatch
    // that carried it.
    const handledOf = new Map();
    for (const [, seqs, , handledMs] of page.inputs) for (const s of seqs) if (!handledOf.has(s)) handledOf.set(s, handledMs);
    const offMain = engineLike && !!page.presenter?.offMainThread;
    const sink = offMain ? ctx.sinkFor(draws.map((d) => d.submitMs)) : null;
    const lat = [];
    // Every matched event, an unpresented first frame counted up to the
    // first later frame that did reach the display (it carries the input
    // too): `pointerToPixelMs` leaves those events out, which flatters a
    // path that drops frames during gestures.
    const latAll = [];
    // Where the time goes, per presented input (ms): input → handled on the
    // page; then for the engine, handled → the descriptor's arrival on the
    // page (view state out, engine render, descriptor back), arrival → the
    // draw's submit (import, transfer, and the worker's draw), submit →
    // viz received the canvas's frame, received → the display frame that
    // showed it ended (presentation), split at the start of viz's
    // DrawAndSwap that drew it; for today, handled → the animation
    // frame that drew it, and that frame → presentation.
    const hops = { inputToHandled: [], handledToArrival: [], arrivalToSubmit: [], submitToReceived: [], receivedToPresent: [], receivedToDraw: [], drawToPresent: [], handledToRaf: [], rafToPresent: [], handledToRenderStart: [], renderStartToGpuDone: [], gpuDoneToSent: [], sentToArrival: [], handledToPosted: [], postedToApplied: [], appliedToPass: [], passToRenderStart: [], composeCpu: [] };
    // The engine's side of handled → arrival, by descriptor seq (host µs → page ms).
    const engineBySeq = new Map((run.engineFrames ?? []).map((f) => [f.seq, f]));
    const hostMs = (us) => (us - raw.clock.offsetUs) / 1000;
    let unmatched = 0;
    let unpresented = 0;
    const presentedOf = (d) => {
      const pf = offMain ? ctx.vizFrame(sink, d.rafMs, d.submitMs) : ctx.frameAt(d.rafMs);
      return pf && PRESENTED.has(pf.state) ? pf : null;
    };
    for (const [seq, tsUs] of run.generated) {
      const rev = revOf.get(seq);
      const i = rev == null ? -1 : draws.findIndex((x) => x.rev >= rev);
      if (i < 0) {
        unmatched++;
        continue;
      }
      const pf = presentedOf(draws[i]);
      if (pf) {
        lat.push((pf.end - ctx.hostToTrace(tsUs)) / 1000);
        latAll.push(lat[lat.length - 1]);
        const d = draws[i];
        const handled = handledOf.get(seq);
        if (handled != null) {
          const handledUs = ctx.toTrace(handled);
          hops.inputToHandled.push((handledUs - ctx.hostToTrace(tsUs)) / 1000);
          if (engineLike && d.arrivedMs != null) {
            hops.handledToArrival.push(d.arrivedMs - handled);
            hops.arrivalToSubmit.push(d.submitMs - d.arrivedMs);
            const e = engineBySeq.get(d.seq);
            if (e) {
              hops.handledToRenderStart.push(hostMs(e.start) - handled);
              hops.renderStartToGpuDone.push((e.gpu_done - e.start) / 1000);
              hops.gpuDoneToSent.push((e.sent - e.gpu_done) / 1000);
              hops.sentToArrival.push(d.arrivedMs - hostMs(e.sent));
              // The engine's side of handled → render start, when the
              // engine's trace has the revision (pm-node FrameTrace):
              // handled → the presenter has the batch (page flush, port,
              // engine thread), → the render thread applied it, → the pass
              // that rendered it began (the thread was busy), → render start
              // (the pass's document step).
              const r = (e.revisions ?? []).find((x) => x[0] === rev);
              if (r) {
                hops.handledToPosted.push(hostMs(r[1]) - handled);
                hops.postedToApplied.push((r[2] - r[1]) / 1000);
                if (e.work_start) {
                  hops.appliedToPass.push((e.work_start - r[2]) / 1000);
                  hops.passToRenderStart.push((e.start - e.work_start) / 1000);
                }
              }
              if (e.cpu_us != null) hops.composeCpu.push(e.cpu_us / 1000);
            }
            if (pf.received != null) {
              hops.submitToReceived.push((pf.received - ctx.toTrace(d.submitMs)) / 1000);
              hops.receivedToPresent.push((pf.end - pf.received) / 1000);
              hops.receivedToDraw.push((pf.swapBegin - pf.received) / 1000);
              hops.drawToPresent.push((pf.end - pf.swapBegin) / 1000);
            }
          } else if (!engineLike) {
            hops.handledToRaf.push(d.rafMs - handled);
            hops.rafToPresent.push((pf.end - ctx.toTrace(d.rafMs)) / 1000);
          }
        }
        continue;
      }
      unpresented++;
      for (let j = i + 1; j < draws.length; j++) {
        const later = presentedOf(draws[j]);
        if (later) {
          latAll.push((later.end - ctx.hostToTrace(tsUs)) / 1000);
          break;
        }
      }
    }
    out[mode] = { events: run.generated.length, dispatches: page.inputs.length, draws: draws.length, unmatched, unpresented, presentation: offMain ? { thread: 'worker', timing: 'viz', sink } : { thread: 'main', timing: 'PipelineReporter' }, pointerToPixelMs: dist(lat, 2), pointerToPixelWithDropsMs: dist(latAll, 2), hopsMs: Object.fromEntries(Object.entries(hops).filter(([, xs]) => xs.length).map(([k, xs]) => [k, dist(xs, 2)])) };
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
    if (run.remoteLayer) Object.assign(env, { POWERMOVE_ENGINE_ADDON: remoteAddon, POWERMOVE_BRIDGE_REMOTE_LAYER: '1', POWERMOVE_BRIDGE_REMOTE_TRACE: remoteTrace, POWERMOVE_BRIDGE_LEAD: String(run.lead ?? 2) });
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
        POWERMOVE_NEW_ENGINE: run.mode === 'engine' || run.mode === 'tracer' ? '1' : '0',
        // The engine's per-frame trace (render start, GPU done, sent), for
        // the hops (PM_MEASURE_ENGINE_TRACE=0 leaves it off).
        POWERMOVE_BRIDGE_TRACE: process.env.PM_MEASURE_ENGINE_TRACE === '0' ? '0' : '1',
        // The engine's test source at the project's rate (the Powermove ad is 30 fps).
        POWERMOVE_BRIDGE_TEST_FPS: String(run.fps ?? 30),
        CODEX_BINARY: path.join(desktop, 'src/main/codex/__fixtures__/fake-codex-app-server.sh'),
        POWERMOVE_FAKE_CHATGPT_STATUS: 'connected'
      });
      args = ['-r', path.join(work, 'app-main.js'), desktop];
    }
    process.stdout.write(`\n== ${name}\n`);
    let displayBefore = harness.displayState();
    if (!from && !allowAsleep && !displayUsable(displayBefore) && waitDisplay) {
      process.stdout.write(`   waiting for the display (asleep ${displayBefore.asleep}, locked ${displayBefore.locked})\n`);
      const until = Date.now() + 12 * 3600_000;
      while (!displayUsable(displayBefore) && Date.now() < until) {
        await sleepMs(30_000);
        displayBefore = harness.displayState();
      }
      // Let the panel and the compositor settle after a wake.
      if (displayUsable(displayBefore)) await sleepMs(20_000);
    }
    if (!from && !allowAsleep && !displayUsable(displayBefore)) {
      process.stdout.write(`   skipped: the main display is ${displayBefore.asleep ? 'asleep' : 'awake'} and the session ${displayBefore.locked ? 'locked' : 'unlocked'}\n`);
      if (profile) fs.rmSync(profile, { recursive: true, force: true });
      continue;
    }
    const t0 = Date.now();
    const r = from
      ? { stdout: fs.existsSync(out) && !JSON.parse(fs.readFileSync(out, 'utf8')).error ? 'MEASURE DONE (kept)' : 'MEASURE FAIL (kept)', status: 0 }
      : spawnSync(electron, args, { env, encoding: 'utf8', timeout: run.durationMs * (run.inputModes?.length ?? 1) * 3 + (run.app ? 1_200_000 : 240_000) });
    const displayAfter = from ? displayBefore : harness.displayState();
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
        config: { scenario: run.scenario, app: !!run.app, mainRate: run.mainRate || undefined, remoteLayer: !!run.remoteLayer, placement: run.placement ?? 'offscreen', env: run.env, mode: run.mode, load: run.load, loadProfile: run.load ? (run.loadProfile ?? 'gate') : null, edr: run.edr ?? 'auto', rawInput: !!run.rawInput, rawGestures: run.app && run.mode !== 'today' ? run.rawGestures !== false : undefined, offMainThread: run.app && run.mode !== 'today' ? run.offMainThread !== false : undefined, fps: run.fps ?? (run.app ? 30 : 60), lead: run.lead ?? 4, durationMs: run.durationMs, inputModes: run.inputModes, synthetic: run.synthetic, edits: run.edits },
        ...(raw.synthetic ? { synthetic: raw.synthetic } : {}),
        versions: raw.versions,
        display: raw.display,
        ...(from ? {} : { displayState: { before: displayBefore, after: displayAfter } }),
        ...(!from && !(displayUsable(displayBefore) && displayUsable(displayAfter)) ? { invalid: 'the main display was asleep or the session locked during the run' } : {}),
        machineLoad: raw.machineLoad,
        ...(raw.project ? { project: raw.project } : {}),
        clock: raw.clock,
        load: raw.load ? { ...raw.load, events: undefined, eventCount: raw.load.events?.length } : null,
        bridge: raw.bridge,
        ...(raw.agentWarmup ? { agentWarmup: raw.agentWarmup } : {}),
        ...(raw.agentSnapshots ? { agentSnapshots: { ...raw.agentSnapshots, queuedMs: undefined, queuedMsMax: Math.max(0, ...(raw.agentSnapshots.queuedMs ?? [])), jobEngineLog: (raw.log ?? []).filter((l) => /job engine/.test(l)).slice(-20) } } : {}),
        ...(raw.mainRate ? { mainRate: raw.mainRate } : {}),
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
    const dir = run.dir ? path.join(repo, run.dir) : run.tracer ? path.join(repo, 'docs/engine/tracer-measurements') : summaries;
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, `${name}.json`), `${JSON.stringify(summary, null, 2)}\n`);
    results[name] = summary;
    if (summary.playback) {
      const p = summary.playback;
      process.stdout.write(`   drops ${p.frames.drops}/${p.frames.expected}  |jitter| p50 ${p.jitterAbsMs.p50} p99 ${p.jitterAbsMs.p99} ms  irregular ${p.irregularIntervals}  ${p.rafToPresentMs ? `rAF→present p50 ${p.rafToPresentMs.p50}` : `due→present p50 ${p.dueToPresentMs?.p50}`}\n`);
      const a = summary.agentSnapshots;
      if (a) process.stdout.write(`   agent snapshots: ${a.batches} batches, ${a.frames} frames (${a.sizes.join(', ')}), batch p50 ${a.batchMsP50} p95 ${a.batchMsP95} max ${a.batchMsMax} ms, failures ${a.failureCount}, routed ${a.routedCount}, playback signals ${JSON.stringify(a.playbackSignals.map(([, on]) => on))}\n`);
      if (a?.jobsAtEnd) process.stdout.write(`   job gate at the end of the window: ${JSON.stringify(a.jobsAtEnd.gate)}\n`);
    } else {
      for (const [mode, l] of Object.entries(summary.latency)) process.stdout.write(`   ${mode}: pointer-to-pixel p50 ${l.pointerToPixelMs.p50} p99 ${l.pointerToPixelMs.p99} ms (${l.pointerToPixelMs.n} of ${l.events}, unmatched ${l.unmatched}, unpresented ${l.unpresented}; with them p50 ${l.pointerToPixelWithDropsMs.p50} p99 ${l.pointerToPixelWithDropsMs.p99} ms)\n`);
      for (const [mode, l] of Object.entries(summary.latency)) if (l.hopsMs) process.stdout.write(`   ${mode} hops p50/p99: ${Object.entries(l.hopsMs ?? {}).map(([k, d]) => `${k} ${d.p50}/${d.p99}`).join(', ')}\n`);
    }
  }
} finally {
  // --keep leaves the raw results, traces, and logs for debugging.
  if (process.argv.includes('--keep')) process.stdout.write(`raw results kept in ${work}\n`);
  else fs.rmSync(work, { recursive: true, force: true });
}
