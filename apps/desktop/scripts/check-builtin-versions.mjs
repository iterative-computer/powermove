#!/usr/bin/env bun
import { readdir, readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';
import { parseManifest } from '@powermove/registry/manifest';
import { snapshot } from '@powermove/registry/snapshot';
import { walkDir } from '@powermove/registry/node';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../src/extensions');
const lockfile = join(root, '.builtin-versions.json');

async function main() {
  const update = process.argv.includes('--update');
  if (process.argv.slice(2).some(arg => arg !== '--update')) throw new Error('Usage: bun scripts/check-builtin-versions.mjs [--update]');
  const entries = await readdir(root, { withFileTypes: true });
  const dirs = entries.filter(entry => entry.isDirectory() && !entry.name.startsWith('.')).map(entry => entry.name).sort();
  let previous = {};
  try { previous = JSON.parse(await readFile(lockfile, 'utf8')); }
  catch (error) { if (!update) throw new Error(`${lockfile} is missing or invalid; run check:builtins --update`); }
  const next = {};
  const errors = [];
  for (const id of dirs) {
    const dir = join(root, id);
    const files = await walkDir(dir);
    const raw = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(files.find(file => file.path === 'manifest.json')?.bytes));
    const parsed = parseManifest(raw);
    if (!parsed.ok) throw new Error(`${id}: ${parsed.error}`);
    if (parsed.manifest.id !== id) throw new Error(`${id}: manifest id does not match directory name`);
    const { treeSha } = await snapshot(files);
    const version = parsed.manifest.version;
    next[id] = { version, treeSha };
    if (!update) {
      if (!previous[id]) errors.push(`${id}: missing from .builtin-versions.json; run check:builtins --update`);
      else if (previous[id].treeSha !== treeSha && previous[id].version === version) errors.push(`${id}: tree changed without a version bump; bump the version in manifest.json`);
      else if (previous[id].treeSha !== treeSha || previous[id].version !== version) errors.push(`${id}: version lock is stale; run check:builtins --update`);
    }
  }
  if (update) {
    await writeFile(lockfile, JSON.stringify(next, null, 2) + '\n');
    console.log(`Updated ${lockfile} (${dirs.length} built-ins)`);
  } else if (errors.length) throw new Error(errors.join('\n'));
  else console.log(`Checked ${dirs.length} built-in versions`);
}

main().catch(error => { console.error(error instanceof Error ? error.message : String(error)); process.exitCode = 1; });
