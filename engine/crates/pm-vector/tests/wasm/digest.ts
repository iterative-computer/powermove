/**
 * Native and wasm32 builds of pm-vector must compute bit-identical geometry
 * (section 26, Phase 1 gate). Builds the `digest` example natively and the
 * `digest_wasm` example for wasm32-unknown-unknown, runs both over the same
 * generated cases (src/conformance.rs), and compares the digests.
 *
 *   bun engine/crates/pm-vector/tests/wasm/digest.ts [cases]
 *
 * Needs `source engine/env.sh` (pinned toolchain with the wasm32 target).
 */
import { readFileSync } from 'node:fs';
import path from 'node:path';

const cases = Number(process.argv[2] ?? 2000);
const engine = path.resolve(import.meta.dir, '../../../..');
const cargo = (...args: string[]) => {
  const run = Bun.spawnSync(['nice', '-n', '15', 'cargo', ...args], { cwd: engine, stderr: 'inherit' });
  if (run.exitCode !== 0) throw new Error(`cargo ${args.join(' ')} failed`);
  return run.stdout.toString();
};

const [nativeHash, nativeValues] = cargo('run', '--quiet', '--release', '-p', 'pm-vector', '--features', 'conformance', '--example', 'digest', '--', String(cases)).trim().split(' ');

cargo('build', '--quiet', '--release', '-p', 'pm-vector', '--features', 'conformance', '--example', 'digest_wasm', '--target', 'wasm32-unknown-unknown');
const target = process.env.CARGO_TARGET_DIR ?? path.join(engine, 'target');
const wasmPath = path.join(target, 'wasm32-unknown-unknown/release/examples/digest_wasm.wasm');
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath));
const exports = instance.exports as { pm_vector_digest(n: number): bigint; pm_vector_digest_values(n: number): bigint };
const wasmHash = BigInt.asUintN(64, exports.pm_vector_digest(cases)).toString(16).padStart(16, '0');
const wasmValues = BigInt.asUintN(64, exports.pm_vector_digest_values(cases)).toString();

console.log(`native ${nativeHash} over ${nativeValues} values`);
console.log(`wasm32 ${wasmHash} over ${wasmValues} values`);
if (nativeHash !== wasmHash || nativeValues !== wasmValues) {
  console.error('native and wasm32 geometry differ');
  process.exit(1);
}
console.log(`identical (${cases} cases per family)`);
