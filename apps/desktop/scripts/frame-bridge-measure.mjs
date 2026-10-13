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
 *   node scripts/frame-bridge-measure.mjs [--only run,run] [--seconds 60] [--skip-build] [--tag r2]
 *
 * Needs native/pm_node.node (bun run build:engine).
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
  { name: 'latency-today-idle', ...latency, mode: 'today', load: false, inputModes: ['baseline'] },
  { name: 'latency-engine-idle', ...latency, mode: 'engine', load: false, inputModes: ['drag', 'orbit', 'walk'] },
  { name: 'latency-engine-raw-idle', ...latency, mode: 'engine', load: false, rawInput: true, inputModes: ['drag', 'orbit', 'walk'] },
  { name: 'latency-today-load', ...latency, mode: 'today', load: true, inputModes: ['baseline'] },
  { name: 'latency-engine-load', ...latency, mode: 'engine', load: true, inputModes: ['drag', 'orbit', 'walk'] },
  { name: 'latency-engine-raw-load', ...latency, mode: 'engine', load: true, rawInput: true, inputModes: ['drag', 'orbit', 'walk'] }
];

/* ── build ─────────────────────────────────────────────────────────── */

const work = fs.mkdtempSync(path.join(os.tmpdir(), 'pm-bridge-measure-'));
const harnessAddon = path.join(work, 'pm_bridge_harness.node');
if (!process.argv.includes('--skip-build')) {
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
  return { frames: frames.length, toTrace, hostToTrace, frameAt };
}

const mediaAt = (sample, hostUs) => sample.mediaTime + ((hostUs - sample.hostTime) * TICKS * sample.rate.num) / (1e6 * sample.rate.den);
const hostOfMedia = (sample, ticks) => sample.hostTime + ((ticks - sample.mediaTime) * 1e6 * sample.rate.den) / (TICKS * sample.rate.num);

function analyzePlayback(raw) {
  const ctx = traceContext(raw.config.trace, raw.clock.offsetUs);
  const periodUs = 1e6 / raw.display.hz;
  const frameTicks = TICKS / FPS;
  const page = raw.page;
  const line = [...page.clocks].reverse().find((c) => c.state === 'playing' && c.rate.num !== 0);
  if (!line) throw new Error('no playing clock sample');
  const host = (ms) => ms * 1000 + raw.clock.offsetUs;
  const m0 = mediaAt(line, host(raw.window.startMs));
  const m1 = mediaAt(line, host(raw.window.endMs));
  const kFirst = Math.ceil((m0 + 0.1 * TICKS) / frameTicks);
  const kLast = Math.floor((m1 - 0.1 * TICKS) / frameTicks) - 1;
  // Draws of timed content frames: [rafMs, pts].
  const draws = page.mode === 'engine' ? page.engineTrace.draws.filter((d) => d[7] === 1).map((d) => [d[0], d[3]]) : page.today.draws.map((d) => [d[0], d[2]]);
  const byPts = new Map();
  for (const [rafMs, pts] of draws) if (!byPts.has(Math.round(pts / frameTicks))) byPts.set(Math.round(pts / frameTicks), rafMs);
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
    const pf = ctx.frameAt(rafMs);
    if (!pf || !PRESENTED.has(pf.state)) {
      counts.notPresented++;
      per.push(null);
      continue;
    }
    per.push({ k, presentUs: pf.end, dueUs: ctx.hostToTrace(hostOfMedia(line, k * frameTicks)), rafToPresentUs: pf.end - ctx.toTrace(rafMs) });
  }
  const presented = per.filter(Boolean);
  const offsets = presented.map((p) => p.presentUs - p.dueUs);
  const median = pct(offsets, 0.5);
  const jitterMs = offsets.map((o) => (o - median) / 1000);
  const intervalErrMs = [];
  for (let i = 1; i < per.length; i++) if (per[i - 1] && per[i]) intervalErrMs.push((per[i].presentUs - per[i - 1].presentUs) / 1000 - 1000 / FPS);
  const depth = {};
  for (const p of presented) {
    const d = Math.round(p.rafToPresentUs / periodUs);
    depth[d] = (depth[d] ?? 0) + 1;
  }
  return {
    window: { seconds: +((raw.window.endMs - raw.window.startMs) / 1000).toFixed(2), displayHz: raw.display.hz, reporterFrames: ctx.frames },
    frames: { ...counts, drops: counts.notDrawn + counts.notPresented, presented: presented.length, offSlot: jitterMs.filter((j) => Math.abs(j) > periodUs / 2000).length },
    jitterAbsMs: dist(jitterMs.map(Math.abs), 3),
    intervalErrorAbsMs: dist(intervalErrMs.map(Math.abs), 3),
    irregularIntervals: intervalErrMs.filter((e) => Math.abs(e) > periodUs / 2000).length,
    rafToPresentMs: dist(presented.map((p) => p.rafToPresentUs / 1000), 2),
    pipelineDepthRefreshes: depth,
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
    const draws = (page.mode === 'engine' ? page.engineTrace.draws.map((d) => ({ rafMs: d[0], rev: d[6] })) : page.today.draws.map((d) => ({ rafMs: d[0], rev: d[3] }))).sort((a, b) => a.rafMs - b.rafMs);
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
      const pf = ctx.frameAt(d.rafMs);
      if (!pf || !PRESENTED.has(pf.state)) {
        unpresented++;
        continue;
      }
      lat.push((pf.end - ctx.hostToTrace(tsUs)) / 1000);
    }
    out[mode] = { events: run.generated.length, dispatches: page.inputs.length, draws: draws.length, unmatched, unpresented, pointerToPixelMs: dist(lat, 2) };
  }
  return out;
}

/* ── run ───────────────────────────────────────────────────────────── */

fs.mkdirSync(summaries, { recursive: true });
const electron = createRequire(path.join(desktop, 'package.json'))('electron');
const results = {};
try {
  for (const run of RUNS) {
    if (only.length && !only.includes(run.name)) continue;
    const name = tag ? `${run.name}-${tag}` : run.name;
    const out = path.join(work, `${name}.json`);
    const trace = path.join(work, `${name}.trace.json`);
    const env = {
      ...process.env,
      PM_IT_DESKTOP: desktop,
      PM_IT_OUT: work,
      PM_MEASURE: JSON.stringify({ ...run, out, trace }),
      POWERMOVE_PM_BRIDGE: harnessAddon,
      POWERMOVE_ENGINE_FRAME_SOURCE: 'test-pattern',
      ELECTRON_DISABLE_SECURITY_WARNINGS: '1'
    };
    delete env.ELECTRON_RUN_AS_NODE;
    process.stdout.write(`\n== ${name}\n`);
    const t0 = Date.now();
    const r = spawnSync(electron, [path.join(work, 'main.js')], { env, encoding: 'utf8', timeout: run.durationMs * (run.inputModes?.length ?? 1) * 3 + 120_000 });
    const verdict = (r.stdout ?? '').split('\n').find((l) => l.startsWith('MEASURE')) ?? `MEASURE FAIL exit ${r.status}`;
    process.stdout.write(`   ${verdict} after ${((Date.now() - t0) / 1000).toFixed(1)} s\n`);
    if (!fs.existsSync(out)) continue;
    const raw = JSON.parse(fs.readFileSync(out, 'utf8'));
    let summary;
    try {
      summary = {
        run: name,
        config: { scenario: run.scenario, mode: run.mode, load: run.load, edr: run.edr ?? 'auto', rawInput: !!run.rawInput, durationMs: run.durationMs, inputModes: run.inputModes },
        versions: raw.versions,
        display: raw.display,
        machineLoad: raw.machineLoad,
        clock: raw.clock,
        load: raw.load ? { ...raw.load, events: undefined, eventCount: raw.load.events?.length } : null,
        bridge: raw.bridge,
        ...(run.scenario === 'playback' ? { playback: analyzePlayback(raw) } : { latency: analyzeLatency(raw) })
      };
    } catch (error) {
      process.stdout.write(`   analysis failed: ${error.message}\n${(raw.log ?? []).slice(-15).join('\n')}\n`);
      continue;
    }
    fs.writeFileSync(path.join(summaries, `${name}.json`), `${JSON.stringify(summary, null, 2)}\n`);
    results[name] = summary;
    if (summary.playback) {
      const p = summary.playback;
      process.stdout.write(`   drops ${p.frames.drops}/${p.frames.expected}  |jitter| p50 ${p.jitterAbsMs.p50} p99 ${p.jitterAbsMs.p99} ms  irregular ${p.irregularIntervals}  rAF→present p50 ${p.rafToPresentMs.p50}\n`);
    } else {
      for (const [mode, l] of Object.entries(summary.latency)) process.stdout.write(`   ${mode}: pointer-to-pixel p50 ${l.pointerToPixelMs.p50} p99 ${l.pointerToPixelMs.p99} ms (${l.pointerToPixelMs.n} of ${l.events}, unmatched ${l.unmatched}, unpresented ${l.unpresented})\n`);
    }
  }
} finally {
  fs.rmSync(work, { recursive: true, force: true });
}
