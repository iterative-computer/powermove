/**
 * Native and wasm32 builds of pm-eval must answer bit-identically (plan
 * section 7, determinism; Phase 1 gate). Builds the identity examples
 * natively and for wasm32-unknown-unknown, runs both over the evaluation
 * vectors (`eval-vectors.json`) and the layer and caption vectors
 * (`layer-vectors.json.gz`, `caption-vectors.json.gz`, or the files given),
 * and fails unless every pair of digests agrees.
 *
 *   source engine/env.sh
 *   bun engine/crates/pm-eval/examples/support/check-wasm-identity.ts [layer-vectors.json.gz ...]
 */
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { gunzipSync } from 'node:zlib';

const engine = path.resolve(import.meta.dir, '../../../..');
const vectors = path.join(engine, 'crates/pm-eval/tests/vectors');
const target = process.env.CARGO_TARGET_DIR ?? path.join(engine, 'target');
const cargo = (...args: string[]) => {
  const run = Bun.spawnSync(['nice', '-n', '15', 'cargo', ...args], { cwd: engine, stderr: 'inherit' });
  if (run.exitCode !== 0) throw new Error(`cargo ${args.join(' ')} failed`);
  return run.stdout.toString();
};

type Exports = { memory: WebAssembly.Memory; alloc(n: number): number; digest(p: number, n: number): bigint; count(p: number, n: number): bigint };
async function wasm(example: string, file: string) {
  cargo('build', '--quiet', '--release', '-p', 'pm-eval', '--example', example, '--target', 'wasm32-unknown-unknown');
  const { instance } = await WebAssembly.instantiate(readFileSync(path.join(target, `wasm32-unknown-unknown/release/examples/${example}.wasm`)), {});
  const x = instance.exports as unknown as Exports;
  const raw = readFileSync(file);
  const bytes = file.endsWith('.gz') ? gunzipSync(raw) : raw;
  const run = (fn: (p: number, n: number) => bigint) => {
    const ptr = x.alloc(bytes.length);
    new Uint8Array(x.memory.buffer, ptr, bytes.length).set(bytes);
    return BigInt.asUintN(64, fn(ptr, bytes.length));
  };
  return { hash: run(x.digest).toString(16).padStart(16, '0'), count: run(x.count).toString() };
}

function native(example: string, file: string) {
  const line = cargo('run', '--quiet', '--release', '-p', 'pm-eval', '--example', example, '--', file).trim();
  const m = /digest ([0-9a-f]{16}) over (\d+)/.exec(line);
  if (!m) throw new Error(`unexpected output: ${line}`);
  return { hash: m[1]!, count: m[2]! };
}

const checks: [string, string, string][] = [
  ['identity_native', 'wasm_identity', path.join(vectors, 'eval-vectors.json')],
  ...(process.argv.length > 2 ? process.argv.slice(2).map(f => path.resolve(f)) : [path.join(vectors, 'layer-vectors.json.gz')])
    .concat(process.argv.length > 2 ? [] : [path.join(vectors, 'caption-vectors.json.gz')])
    .map((f): [string, string, string] => ['layer_identity_native', 'wasm_layer_identity', f]),
];
let failed = false;
for (const [nativeExample, wasmExample, file] of checks) {
  const n = native(nativeExample, file), w = await wasm(wasmExample, file);
  const ok = n.hash === w.hash && n.count === w.count;
  failed ||= !ok;
  console.log(`${ok ? 'same' : 'DIFFERENT'} ${path.basename(file)}: native ${n.hash} over ${n.count}, wasm32 ${w.hash} over ${w.count}`);
}
if (failed) process.exit(1);
