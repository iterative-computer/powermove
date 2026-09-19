#!/usr/bin/env node
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const distDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../dist');
const [major] = process.versions.node.split('.').map(Number);
if (major < 22) {
  console.error(`Powermove needs Node 22 or newer (found ${process.versions.node}).`);
  process.exit(1);
}

let version = '0.0.0';
try { version = JSON.parse(await readFile(path.join(distDir, 'build.json'), 'utf8')).version; } catch {
  console.error(`The Powermove host is not built (missing ${distDir}). Run \`bun run build\` in packages/cli.`);
  process.exit(1);
}

const { main } = await import(path.join(distDir, 'server.mjs'));
await main(process.argv.slice(2), { distDir, version });
