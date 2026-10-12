/* Native vs WASM layout identity (plan section 12).
   Runs the level 1 layout compiled to wasm32-unknown-unknown over the
   metrics-and-shaping subsets and the recorded registry answers, then
   compares its JSON output byte for byte with the native run of the same
   inputs (out/replay-subsets-output.json) and value for value with the
   native CoreText run (out/native-output.json).

   Prerequisites: s4-compare, then s4-compare --replay --subsets, then
   cargo build --release -p s4-wasm --target wasm32-unknown-unknown.
   Usage: bun scripts/wasm-identity.mjs [--root out]
   V8 (Electron's Node): ELECTRON_RUN_AS_NODE=1 <electron> scripts/wasm-identity.mjs */
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, process.argv.includes('--root') ? process.argv[process.argv.indexOf('--root') + 1] : 'out');
const wasmPath = join(root, 'target', 'wasm32-unknown-unknown', 'release', 's4_wasm.wasm');
const subsets = join(outDir, 'subsets');

const started = performance.now();
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {});
const x = instance.exports;
const put = (bytes) => {
  const p = x.s4_alloc(bytes.length);
  new Uint8Array(x.memory.buffer, p, bytes.length).set(bytes);
  return [p, bytes.length];
};
const enc = new TextEncoder();
let fontBytes = 0;
for (const name of readdirSync(subsets).filter(n => n.endsWith('.bin')).sort()) {
  const data = readFileSync(join(subsets, name));
  fontBytes += data.length;
  const [np, nl] = put(enc.encode(name));
  const [dp, dl] = put(data);
  x.s4_add_font(np, nl, dp, dl);
}
const loaded = performance.now();
const [lp, ll] = put(readFileSync(join(subsets, 'resolver-log.json')));
const [cp, cl] = put(readFileSync(join(outDir, 'corpus.json')));
const status = x.s4_run(lp, ll, cp, cl);
if (status !== 0) throw new Error(`s4_run failed with status ${status}`);
const laidOut = performance.now();
const out = new Uint8Array(x.memory.buffer, x.s4_out_ptr(), x.s4_out_len()).slice();
writeFileSync(join(outDir, 'wasm-output.json'), out);

const nativeSubsets = readFileSync(join(outDir, 'replay-subsets-output.json'));
const bytesIdentical = Buffer.compare(Buffer.from(out), nativeSubsets) === 0;
const wasm = JSON.parse(new TextDecoder().decode(out));
const native = JSON.parse(readFileSync(join(outDir, 'native-output.json'), 'utf8'));
let identical = 0;
const differing = [];
for (let i = 0; i < Math.max(wasm.length, native.length); i++) {
  if (JSON.stringify(wasm[i]) === JSON.stringify(native[i])) identical++;
  else if (differing.length < 10) differing.push(wasm[i]?.id ?? native[i]?.id ?? String(i));
}
const result = {
  cases: wasm.length,
  moduleBytes: readFileSync(wasmPath).length,
  subsetFontBytes: fontBytes,
  misses: x.s4_misses(),
  bytesIdenticalToNativeSubsetRun: bytesIdentical,
  identicalToNativeCoreTextRun: identical,
  differing,
  loadSeconds: (loaded - started) / 1000,
  layoutSeconds: (laidOut - loaded) / 1000,
  runtime: globalThis.Bun ? `Bun ${Bun.version} (JavaScriptCore)` : `Node ${process.versions.node} (V8 ${process.versions.v8})${process.versions.electron ? `, Electron ${process.versions.electron}` : ''}`,
};
writeFileSync(join(outDir, `wasm-identity-${globalThis.Bun ? 'jsc' : 'v8'}.json`), JSON.stringify(result, null, 2));
console.log(JSON.stringify(result, null, 2));
if (!bytesIdentical || identical !== native.length || wasm.length !== native.length || result.misses !== 0) process.exit(1);
