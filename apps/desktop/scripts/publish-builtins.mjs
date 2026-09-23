#!/usr/bin/env bun
import { readdir, readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';
import { publishBuiltins } from './lib/publish-builtins.ts';

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const root = join(desktop, 'src/extensions');

async function main() {
  const args = process.argv.slice(2);
  let dryRun = false;
  let notes;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--dry-run') dryRun = true;
    else if (args[i] === '--notes' && args[i + 1] && !args[i + 1].startsWith('--')) notes = args[++i];
    else throw new Error('Usage: bun scripts/publish-builtins.mjs [--dry-run] [--notes TEXT]');
  }
  const { version } = JSON.parse(await readFile(join(desktop, 'package.json'), 'utf8'));
  const entries = await readdir(root, { withFileTypes: true });
  const dirs = entries.filter(entry => entry.isDirectory() && !entry.name.startsWith('.')).map(entry => join(root, entry.name)).sort();
  const plans = await publishBuiltins({
    fetch: globalThis.fetch, dirs, token: process.env.POWERMOVE_REGISTRY_TOKEN,
    origin: process.env.POWERMOVE_REGISTRY_URL ?? 'https://cloud.trypowermove.com',
    appVersion: version, notes, dryRun
  });
  for (const plan of plans) console.log(`${plan.status} ${plan.coordinate}@${plan.version} treeSha=${plan.treeSha} files=${plan.fileCount} category=${plan.category}`);
  console.log(`${plans.length} built-ins: ${plans.filter(plan => plan.status === 'published').length} published, ${plans.filter(plan => plan.status === 'skipped').length} skipped`);
}

main().catch(error => { console.error(error instanceof Error ? error.message : String(error)); process.exitCode = 1; });
