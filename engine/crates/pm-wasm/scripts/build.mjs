#!/usr/bin/env node
/* Builds pm-wasm for the renderer and the headless document host.

     node engine/crates/pm-wasm/scripts/build.mjs [--out <file>] [--debug]

   Runs `cargo build --release --target wasm32-unknown-unknown -p
   pm-wasm-module` (the cdylib over pm-wasm, so the release profile's thin
   LTO applies: docs/engine/release-build.md) with the pinned toolchain
   (engine/env.sh), drops the custom sections
   (DWARF line tables, names) that the release profile keeps for native
   builds, and writes the module to
   apps/desktop/src/renderer/public/engine/pm_wasm.wasm (gitignored), which
   the renderer serves at `engine/pm_wasm.wasm` and the headless host reads
   from out/renderer/engine/. With --debug the custom sections are kept. */
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';

const here = dirname(fileURLToPath(import.meta.url));
const engineDir = resolve(here, '../../..');
const repo = resolve(engineDir, '..');
const args = process.argv.slice(2);
const outIndex = args.indexOf('--out');
const out = outIndex >= 0 ? resolve(args[outIndex + 1]) : join(repo, 'apps/desktop/src/renderer/public/engine/pm_wasm.wasm');
const debug = args.includes('--debug');

execFileSync('bash', ['-c', 'source env.sh && nice -n 15 cargo build --release --target wasm32-unknown-unknown -p pm-wasm-module'], {
  cwd: engineDir,
  stdio: 'inherit'
});
const built = readFileSync(join(engineDir, 'target/wasm32-unknown-unknown/release/pm_wasm_module.wasm'));

/** The module without custom sections (id 0). */
export function strip(bytes) {
  const kept = [bytes.subarray(0, 8)];
  let i = 8;
  const leb = () => {
    let value = 0, shift = 0, byte;
    do { byte = bytes[i++]; value += (byte & 0x7f) * 2 ** shift; shift += 7; } while (byte & 0x80);
    return value;
  };
  while (i < bytes.length) {
    const start = i, id = bytes[i++], len = leb(), end = i + len;
    if (id !== 0) kept.push(bytes.subarray(start, end));
    i = end;
  }
  return Buffer.concat(kept);
}

const module = debug ? built : strip(built);
new WebAssembly.Module(module); // validates
mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, module);
console.log(`pm-wasm: ${module.length} bytes (${gzipSync(module, { level: 9 }).length} gzipped) -> ${out}`);
