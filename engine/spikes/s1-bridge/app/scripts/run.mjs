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
const runs = [
  // The S1 gate: 4K 60 fps for 60 s with main under load.
  { name: 'playback-bgra-load', scenario: 'playback', format: 'bgra', load: LOAD, durationMs: seconds * 1000 },
  { name: 'playback-rgbaf16-load', scenario: 'playback', format: 'rgbaf16', load: LOAD, durationMs: seconds * 1000, hdrCanvas: true },
  { name: 'playback-bgra-load-external', scenario: 'playback', format: 'bgra', load: LOAD, durationMs: seconds * 1000, drawPath: 'external', transport: 'persistent' },
  // Tuned transport: one persistent import per slot in the viewer, descriptors engine
  // -> viewer directly (main's JavaScript off the per-frame path).
  { name: 'playback-bgra-load-persistent', scenario: 'playback', format: 'bgra', load: LOAD, durationMs: seconds * 1000, transport: 'persistent' },
  { name: 'playback-rgbaf16-load-persistent', scenario: 'playback', format: 'rgbaf16', load: LOAD, durationMs: seconds * 1000, hdrCanvas: true, transport: 'persistent' },
  { name: 'playback-bgra-load-persistent-redraw', scenario: 'playback', format: 'bgra', load: LOAD, durationMs: seconds * 1000, transport: 'persistent', redraw: 'every-vsync' },
  // Release through the per-use release callback instead of onSubmittedWorkDone.
  { name: 'playback-bgra-load-callback', scenario: 'playback', format: 'bgra', load: LOAD, durationMs: seconds * 1000, releaseMode: 'callback' },
  // Validity check: a small on-screen window instead of off every screen (15 s).
  { name: 'playback-bgra-load-onscreen', scenario: 'playback', format: 'bgra', load: LOAD, durationMs: 15_000, onscreen: true, transport: 'persistent' },
  // Control: the same without main load.
  { name: 'playback-bgra-idle', scenario: 'playback', format: 'bgra', load: false, durationMs: seconds * 1000 },
  // Pointer-to-pixel: today's in-renderer path vs the engine path, main under load.
  { name: 'latency-bgra-load', scenario: 'latency', format: 'bgra', load: LOAD, durationMs: 20_000, extraPasses: 0, poolSize: 8, leadFrames: 4 },
  { name: 'latency-bgra-load-persistent', scenario: 'latency', format: 'bgra', load: LOAD, durationMs: 20_000, extraPasses: 0, poolSize: 8, leadFrames: 4, transport: 'persistent' },
  { name: 'latency-bgra-idle', scenario: 'latency', format: 'bgra', load: false, durationMs: 20_000, extraPasses: 0, poolSize: 8, leadFrames: 4 },
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
