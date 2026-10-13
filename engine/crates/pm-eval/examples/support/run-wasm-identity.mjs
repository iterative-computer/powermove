// Loads a wasm identity example module (wasm_identity or
// wasm_layer_identity) and prints its digest of the vectors. A `.gz`
// vectors file is decompressed first.
import { readFileSync } from 'node:fs';
import { gunzipSync } from 'node:zlib';

const [wasmPath, vectorsPath] = process.argv.slice(2);
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {});
const { memory, alloc, digest, count } = instance.exports;
const raw = readFileSync(vectorsPath);
const bytes = vectorsPath.endsWith('.gz') ? gunzipSync(raw) : raw;
const run = (fn) => {
  const ptr = alloc(bytes.length);
  new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes);
  return fn(ptr, bytes.length);
};
const value = BigInt.asUintN(64, run(digest)).toString(16).padStart(16, '0');
const values = BigInt.asUintN(64, run(count));
console.log(`wasm digest ${value} over ${values} values`);
