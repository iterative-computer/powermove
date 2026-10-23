/*
 * Main-process integration test for the native engine host: builds the
 * pm-node addon (unless --skip-build), bundles the test's Electron main
 * and the engine utility process entry, and runs them in Electron with no
 * window. See src/main/engine/integration/electron-main.ts for what it
 * checks. Exit code 0 on pass.
 *
 *   node scripts/engine-host-integration.mjs [--skip-build] [--json out.json]
 *   PM_IT_ELECTRON_LAUNCHER=/path/to/pm-electron-lock wraps only Electron,
 *   leaving the optional addon build and bundling outside the lock.
 */
import { execFileSync, spawn } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';

import { build } from 'esbuild';

import { rawText } from './esbuild-raw-text.mjs';

const desktop = path.resolve(import.meta.dirname, '..');
const args = process.argv.slice(2);
const jsonOut = args.includes('--json') ? args[args.indexOf('--json') + 1] : null;

if (!args.includes('--skip-build')) {
  execFileSync(process.execPath, [path.join(desktop, 'scripts/build-engine-addon.mjs')], { stdio: 'inherit' });
}

const out = mkdtempSync(path.join(os.tmpdir(), 'pm-engine-host-it-'));
try {
  await build({
    entryPoints: {
      main: path.join(desktop, 'src/main/engine/integration/electron-main.ts'),
      'engine-process': path.join(desktop, 'src/main/engine/engine-process.ts')
    },
    outdir: out,
    bundle: true,
    platform: 'node',
    format: 'cjs',
    target: 'node22',
    external: ['electron'],
    logLevel: 'warning',
    plugins: [rawText]
  });
  const electron = createRequire(path.join(desktop, 'package.json'))('electron');
  const env = {
    ...process.env,
    PM_IT_ENGINE_PROCESS: path.join(out, 'engine-process.js'),
    PM_IT_MEDIA_FIXTURE: path.join(desktop, 'e2e', 'fixtures', 'h264-aac.mp4'),
    POWERMOVE_ENGINE_ADDON: path.join(desktop, 'native', 'pm_node.node'),
    // Main's host clock is the engines' clock through pm-bridge (built with pm-node).
    POWERMOVE_PM_BRIDGE: path.join(desktop, 'native', 'pm_bridge.node'),
    ELECTRON_DISABLE_SECURITY_WARNINGS: '1'
  };
  delete env.ELECTRON_RUN_AS_NODE;
  const launcher = process.env.PM_IT_ELECTRON_LAUNCHER;
  const child = spawn(launcher ?? electron, launcher ? [electron, path.join(out, 'main.js')] : [path.join(out, 'main.js')], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '';
  let stderr = '';
  child.stdout.on('data', (chunk) => { stdout += chunk; });
  child.stderr.on('data', (chunk) => { stderr += chunk; });
  const [code, signal] = await new Promise((resolve) => child.on('exit', (c, s) => resolve([c, s])));
  const report = stdout.split('\n').find((line) => line.startsWith('{'));
  if (report) {
    const parsed = JSON.parse(report);
    for (const step of parsed.steps) process.stdout.write(`  ${String(step.ms).padStart(8)} ms  ${step.name}\n`);
    if (jsonOut) writeFileSync(jsonOut, `${JSON.stringify(parsed, null, 2)}\n`);
    if (code !== 0) process.stdout.write(`${parsed.log.join('\n')}\n`);
  }
  const verdict = stdout.split('\n').find((line) => line.startsWith('ENGINE-HOST-IT')) ?? `ENGINE-HOST-IT FAIL electron exited ${code ?? signal}`;
  process.stdout.write(`${verdict}\n`);
  if (code !== 0) process.stderr.write(`${stdout.split('\n').slice(-20).join('\n')}\n${stderr.split('\n').slice(-40).join('\n')}\n`);
  process.exitCode = code === 0 && verdict.includes('PASS') ? 0 : 1;
} finally {
  rmSync(out, { recursive: true, force: true });
}
