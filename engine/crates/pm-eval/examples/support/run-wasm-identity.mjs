// Loads the wasm_identity example module and prints its digest of the
// vectors (see examples/wasm_identity.rs).
import { readFileSync } from 'node:fs';

const [wasmPath, vectorsPath] = process.argv.slice(2);
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {});
const { memory, alloc, digest, count } = instance.exports;
const run = (fn) => {
  const bytes = readFileSync(vectorsPath);
  const ptr = alloc(bytes.length);
  new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes);
  return fn(ptr, bytes.length);
};
const value = BigInt.asUintN(64, run(digest)).toString(16).padStart(16, '0');
const values = BigInt.asUintN(64, run(count));
console.log(`wasm digest ${value} over ${values} values`);
