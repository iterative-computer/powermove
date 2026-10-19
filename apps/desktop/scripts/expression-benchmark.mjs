#!/usr/bin/env node
// Build the real renderer modules for Node; neither Electron nor browser shims
// participate. Keep the temporary bundle outside this public repository.
import { build } from 'esbuild';
import { execFileSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';
const volume = process.platform === 'darwin' ? '/System/Volumes/Data' : '/';
const disk = execFileSync('df', ['-k', volume], { encoding: 'utf8' });
const availableKiB = Number(disk.trim().split('\n').at(-1).trim().split(/\s+/)[3]);
if (!(availableKiB >= 40 * 1024 * 1024)) throw new Error(`Need 40 GiB free before running: ${disk}`);
const temporary = await mkdtemp(join(tmpdir(), 'pm-expression-benchmark-'));
try {
  const outfile = join(temporary, 'benchmark.mjs');
  await build({ entryPoints: [fileURLToPath(new URL('./expression-benchmark.ts', import.meta.url))], outfile, bundle: true, platform: 'node', format: 'esm', define: { 'import.meta.env': '{}' }, logLevel: 'warning' });
  const { run } = await import(pathToFileURL(outfile).href);
  await run(fileURLToPath(new URL('../src/renderer/public/engine/pm_wasm.wasm', import.meta.url)));
} catch (error) { console.error(error); process.exitCode = 1; }
finally { await rm(temporary, { recursive: true, force: true }); }
// The production document host schedules idle checks; this standalone run ends.
process.exit(process.exitCode ?? 0);
