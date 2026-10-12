/* Native vs WASM layout identity (plan section 12: native and WASM layout
   identical). Runs the level 1 layout compiled to wasm32-unknown-unknown
   (examples/layout_wasm.rs) over the metrics-and-shaping subsets and the
   recorded registry answers, then compares its JSON output byte for byte
   with the native run of the same inputs (replay-subsets-output.json) and
   value for value with the native CoreText run (native-output.json).

   Inputs come from `pm-text-corpus export` (scripts/check.sh runs it all).
   Usage: bun scripts/wasm-identity.mjs <export dir> <module.wasm>
   V8: ELECTRON_RUN_AS_NODE=1 <electron> scripts/wasm-identity.mjs <dir> <wasm> */
import { gzipSync } from 'node:zlib';
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const [dir, wasmPath] = process.argv.slice(2);
if (!dir || !wasmPath) throw new Error('usage: wasm-identity.mjs <export dir> <module.wasm>');
const subsets = join(dir, 'subsets');

/* Module size without custom sections (names, debug info), as shipped. */
function strippedSize(bytes) {
  let size = 8, i = 8;
  const leb = () => { let v = 0, s = 0, b; do { b = bytes[i++]; v |= (b & 0x7f) << s; s += 7; } while (b & 0x80); return v; };
  const kept = [bytes.subarray(0, 8)];
  while (i < bytes.length) {
    const start = i, id = bytes[i++], len = leb(), end = i + len;
    if (id !== 0) { size += end - start; kept.push(bytes.subarray(start, end)); }
    i = end;
  }
  return { size, gzip: gzipSync(Buffer.concat(kept), { level: 9 }).length };
}

const module = readFileSync(wasmPath);
const started = performance.now();
const { instance } = await WebAssembly.instantiate(module, {});
const x = instance.exports;
const put = (bytes) => {
  const p = x.pm_alloc(bytes.length);
  new Uint8Array(x.memory.buffer, p, bytes.length).set(bytes);
  return [p, bytes.length];
};
const enc = new TextEncoder();
let fontBytes = 0;
for (const name of readdirSync(subsets).filter(n => n.endsWith('.bin')).sort()) {
  const data = readFileSync(join(subsets, name));
  fontBytes += data.length;
  const [kp, kl] = put(enc.encode(name.slice(0, -4)));
  const [dp, dl] = put(data);
  x.pm_add_font(kp, kl, dp, dl);
}
const loaded = performance.now();
const [ap, al] = put(readFileSync(join(dir, 'answers.json')));
const [cp, cl] = put(readFileSync(join(dir, 'corpus.json')));
const status = x.pm_run(ap, al, cp, cl);
if (status !== 0) throw new Error(`pm_run failed with status ${status}`);
const laidOut = performance.now();
const out = new Uint8Array(x.memory.buffer, x.pm_out_ptr(), x.pm_out_len()).slice();

const runtime = globalThis.Bun ? `Bun ${Bun.version} (JavaScriptCore)` : `Node ${process.versions.node} (V8 ${process.versions.v8})${process.versions.electron ? `, Electron ${process.versions.electron}` : ''}`;
writeFileSync(join(dir, `wasm-output-${globalThis.Bun ? 'jsc' : 'v8'}.json`), out);
const nativeSubsets = readFileSync(join(dir, 'replay-subsets-output.json'));
const bytesIdentical = Buffer.compare(Buffer.from(out), nativeSubsets) === 0;
const wasm = JSON.parse(new TextDecoder().decode(out));
const native = JSON.parse(readFileSync(join(dir, 'native-output.json'), 'utf8'));
let identical = 0;
const differing = [];
for (let i = 0; i < Math.max(wasm.length, native.length); i++) {
  if (JSON.stringify(wasm[i]) === JSON.stringify(native[i])) identical++;
  else if (differing.length < 10) differing.push(wasm[i]?.id ?? native[i]?.id ?? String(i));
}
const stripped = strippedSize(module);
const result = {
  runtime,
  cases: wasm.length,
  moduleBytes: module.length,
  strippedBytes: stripped.size,
  strippedGzipBytes: stripped.gzip,
  subsetFontBytes: fontBytes,
  misses: x.pm_misses(),
  pending: x.pm_pending(),
  bytesIdenticalToNativeSubsetRun: bytesIdentical,
  identicalToNativeCoreTextRun: identical,
  differing,
  loadSeconds: (loaded - started) / 1000,
  layoutSeconds: (laidOut - loaded) / 1000,
};
writeFileSync(join(dir, `wasm-identity-${globalThis.Bun ? 'jsc' : 'v8'}.json`), JSON.stringify(result, null, 2));
console.log(JSON.stringify(result, null, 2));
if (!bytesIdentical || identical !== native.length || wasm.length !== native.length || result.misses !== 0 || result.pending !== 0) process.exit(1);
