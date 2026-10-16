/*
 * The frame bridge's Electron integration test (plan section 18; section 28
 * requires it to pass on every Electron upgrade): builds the pm-node and
 * pm-bridge addons (unless --skip-build), bundles the test's main, the
 * engine utility process entry, the real preload, and the test page, and
 * runs them in Electron with one window shown without activation off every
 * screen. See src/main/engine/integration/bridge-main.ts for what it
 * checks. It runs twice: with frames drawn on the page's main thread, and
 * with the viewer's presentation worker (present-worker.ts; frames read back
 * in the worker, bridge-worker.ts). Exit code 0 when both pass.
 *
 *   node scripts/frame-bridge-integration.mjs [--skip-build] [--json out.json] [--only main|worker]
 */
import { execFileSync, spawn } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';

import { build } from 'esbuild';

const desktop = path.resolve(import.meta.dirname, '..');
const args = process.argv.slice(2);
const jsonOut = args.includes('--json') ? args[args.indexOf('--json') + 1] : null;

if (!args.includes('--skip-build')) {
  execFileSync(process.execPath, [path.join(desktop, 'scripts/build-engine-addon.mjs')], { stdio: 'inherit' });
}

const out = mkdtempSync(path.join(os.tmpdir(), 'pm-frame-bridge-it-'));
try {
  const common = { bundle: true, target: 'es2023', logLevel: 'warning', alias: { powermove: path.join(desktop, 'src/renderer/src/kernel/api.ts') } };
  await build({
    ...common,
    entryPoints: {
      main: path.join(desktop, 'src/main/engine/integration/bridge-main.ts'),
      'engine-process': path.join(desktop, 'src/main/engine/engine-process.ts')
    },
    outdir: out,
    platform: 'node',
    format: 'cjs',
    target: 'node22',
    external: ['electron']
  });
  // The sandboxed preload may require only electron.
  await build({ ...common, entryPoints: { preload: path.join(desktop, 'src/preload/index.ts') }, outdir: out, platform: 'browser', format: 'cjs', external: ['electron'] });
  await build({ ...common, entryPoints: { page: path.join(desktop, 'src/main/engine/integration/bridge-page.ts'), 'bridge-worker': path.join(desktop, 'src/main/engine/integration/bridge-worker.ts') }, outdir: out, platform: 'browser', format: 'iife' });
  writeFileSync(
    path.join(out, 'page.html'),
    '<!doctype html><meta charset="utf-8"><title>frame bridge test</title>' +
      '<body style="margin:0;background:#000;overflow:hidden">' +
      '<div id="stage" style="position:absolute;inset:0"><canvas id="overlay" style="position:absolute;inset:0;pointer-events:none"></canvas></div>' +
      '<script src="page.js"></script></body>\n'
  );
  const electron = createRequire(path.join(desktop, 'package.json'))('electron');
  const only = args.includes('--only') ? args[args.indexOf('--only') + 1] : null;
  const reports = {};
  let failed = false;
  for (const present of ['main', 'worker'].filter((m) => !only || m === only)) {
    process.stdout.write(`\n== frames drawn ${present === 'worker' ? 'by the presentation worker' : "on the page's main thread"}\n`);
    const env = {
      ...process.env,
      PM_IT_DESKTOP: desktop,
      PM_IT_OUT: out,
      PM_IT_PRESENT: present,
      POWERMOVE_ENGINE_FRAME_SOURCE: 'test-pattern',
      ELECTRON_DISABLE_SECURITY_WARNINGS: '1'
    };
    delete env.ELECTRON_RUN_AS_NODE;
    const child = spawn(electron, [path.join(out, 'main.js')], { env, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => { stdout += chunk; });
    child.stderr.on('data', (chunk) => { stderr += chunk; });
    const [code, signal] = await new Promise((resolve) => child.on('exit', (c, s) => resolve([c, s])));
    const report = stdout.split('\n').find((line) => line.startsWith('{'));
    if (report) {
      const parsed = JSON.parse(report);
      reports[present] = parsed;
      process.stdout.write(`Electron ${parsed.versions.electron} (Chrome ${parsed.versions.chrome})\n`);
      for (const step of parsed.steps) process.stdout.write(`  ${String(step.ms).padStart(8)} ms  ${step.name}${step.detail ? `  ${JSON.stringify(step.detail)}` : ''}\n`);
      if (code !== 0) process.stdout.write(`${parsed.log.join('\n')}\n`);
    }
    const verdict = stdout.split('\n').find((line) => line.startsWith('FRAME-BRIDGE-IT')) ?? `FRAME-BRIDGE-IT FAIL electron exited ${code ?? signal}`;
    process.stdout.write(`${verdict} (${present})\n`);
    if (code !== 0) process.stderr.write(`${stdout.split('\n').slice(-20).join('\n')}\n${stderr.split('\n').slice(-40).join('\n')}\n`);
    if (!(code === 0 && verdict.includes('PASS'))) failed = true;
  }
  if (jsonOut) writeFileSync(jsonOut, `${JSON.stringify(reports, null, 2)}\n`);
  process.stdout.write(`${failed ? 'FRAME-BRIDGE-IT FAIL' : 'FRAME-BRIDGE-IT PASS'}\n`);
  process.exitCode = failed ? 1 : 0;
} finally {
  rmSync(out, { recursive: true, force: true });
}
