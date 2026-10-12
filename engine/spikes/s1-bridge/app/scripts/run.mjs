// Runs the S1 measurement matrix one Electron launch at a time and writes a summary
// per run (results/<name>.json) plus raw logs and traces (results/raw/, ignored).
//
//   node app/scripts/run.mjs [--only name,name] [--seconds 60]
//
// Uses Electron 44 from apps/desktop/node_modules. The window is shown inactive and
// moved off every screen, so nothing appears on the display and focus never moves.

import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const app = path.resolve(here, '..');
const spike = path.resolve(app, '..');
const repo = path.resolve(spike, '../../..');
const electron = createRequire(path.join(repo, 'apps/desktop/package.json'))('electron');
const results = path.join(spike, 'results');
const rawDir = path.join(results, 'raw');
fs.mkdirSync(rawDir, { recursive: true });

const arg = (name, dflt) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : dflt;
};
const seconds = Number(arg('seconds', '60'));
const only = arg('only', '')
  .split(',')
  .filter(Boolean);

const LOAD = { longTask: true, autosave: true, agent: true, resize: true };
// Stress, not the gate: an autosave of a 56 MB document (about the size of the largest
// undo history on the reference machine), about 1.2 s of main-thread work every 2 s.
const HEAVY = { ...LOAD, historyEntries: 4000 };
const pb = { scenario: 'playback', durationMs: seconds * 1000 };
const lat = { scenario: 'latency', durationMs: 20_000, extraPasses: 0, poolSize: 8, leadFrames: 4 };
const runs = [
  // The S1 gate as the plan writes it: 4K 60 fps for 60 s with main under load, every
  // frame forwarded by main with sendSharedTexture (section 18 step 3).
  { name: 'playback-bgra-load', ...pb, format: 'bgra', load: LOAD },
  { name: 'playback-rgbaf16-load', ...pb, format: 'rgbaf16', load: LOAD, hdrCanvas: true },
  // Tuning the per-frame path: a jitter buffer deep enough to ride out main's stalls.
  { name: 'playback-bgra-load-deep', ...pb, format: 'bgra', load: LOAD, leadFrames: 18, poolSize: 22 },
  // Release through the per-use release callback instead of onSubmittedWorkDone.
  { name: 'playback-bgra-load-callback', ...pb, format: 'bgra', load: LOAD, releaseMode: 'callback' },
  // Tuned transport: one persistent import per slot in the viewer, descriptors engine
  // -> viewer directly (main's JavaScript off the per-frame path).
  { name: 'playback-bgra-load-persistent', ...pb, format: 'bgra', load: LOAD, transport: 'persistent' },
  { name: 'playback-rgbaf16-load-persistent', ...pb, format: 'rgbaf16', load: LOAD, hdrCanvas: true, transport: 'persistent' },
  { name: 'playback-bgra-load-external', ...pb, format: 'bgra', load: LOAD, drawPath: 'external', transport: 'persistent' },
  { name: 'playback-bgra-load-persistent-redraw', ...pb, format: 'bgra', load: LOAD, transport: 'persistent', redraw: 'every-vsync' },
  { name: 'playback-bgra-heavy-persistent', ...pb, format: 'bgra', load: HEAVY, transport: 'persistent' },
  // Controls: the viewer draws its own content on the same schedule (no surface read),
  // so whatever remains is Chromium's compositor; and the plan path without load.
  { name: 'playback-bgra-load-control', ...pb, format: 'bgra', load: LOAD, transport: 'persistent', drawPath: 'control' },
  { name: 'playback-bgra-idle-control', ...pb, format: 'bgra', load: false, transport: 'persistent', drawPath: 'control' },
  { name: 'playback-bgra-idle', ...pb, format: 'bgra', load: false },
  // Validity check: a small on-screen window instead of off every screen (15 s).
  { name: 'playback-bgra-load-onscreen', ...pb, durationMs: 15_000, format: 'bgra', load: LOAD, onscreen: true, transport: 'persistent' },
  { name: 'playback-bgra-load-onscreen-control', ...pb, durationMs: 15_000, format: 'bgra', load: LOAD, onscreen: true, transport: 'persistent', drawPath: 'control' },
  // Pointer-to-pixel: today's in-renderer path ('baseline') vs the engine path for
  // layer drag, 3D orbit, and walk, main under load and idle.
  { name: 'latency-bgra-load', ...lat, format: 'bgra', load: LOAD },
  { name: 'latency-bgra-load-persistent', ...lat, format: 'bgra', load: LOAD, transport: 'persistent' },
  { name: 'latency-bgra-idle', ...lat, format: 'bgra', load: false },
  { name: 'latency-bgra-idle-persistent', ...lat, format: 'bgra', load: false, transport: 'persistent' },
  // Image quality: capturePage, banding, values above 1.0.
  { name: 'quality-bgra', scenario: 'quality', format: 'bgra', load: false },
  { name: 'quality-rgbaf16', scenario: 'quality', format: 'rgbaf16', load: false, hdrCanvas: true, edrOnscreen: true }
];

for (const run of runs) {
  if (only.length && !only.includes(run.name)) continue;
  const { name, ...config } = run;
  const out = path.join(rawDir, `${name}.json`);
  const env = { ...process.env, S1_CONFIG: JSON.stringify({ ...config, out }), S1_ALLOW_UNSIGNED_DEV: '1' };
  delete env.ELECTRON_RUN_AS_NODE;
  process.stdout.write(`\n== ${name}\n`);
  const t0 = Date.now();
  const r = spawnSync(electron, [app], { env, stdio: ['ignore', 'inherit', 'inherit'], timeout: (config.durationMs ?? 60_000) * 6 + 180_000 });
  process.stdout.write(`   exit ${r.status} after ${((Date.now() - t0) / 1000).toFixed(1)} s\n`);
  if (!fs.existsSync(out)) {
    process.stdout.write(`   no result for ${name}\n`);
    continue;
  }
  const a = spawnSync(process.execPath, [path.join(here, 'analyze.mjs'), out, '--out', path.join(results, `${name}.json`)], { stdio: 'inherit' });
  if (a.status !== 0) process.stdout.write(`   analysis failed for ${name}\n`);
}
