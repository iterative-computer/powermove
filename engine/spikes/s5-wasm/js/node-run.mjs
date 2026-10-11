// Run the S5 measurements in Node: node js/node-run.mjs
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { runAll } from './bench.mjs';
import { project884, coverageProject, sampleTimes } from './synth.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const bindings = await import(path.join(root, 'pkg/s5_wasm.js'));
const exports = bindings.initSync({ module: readFileSync(path.join(root, 'pkg/s5_wasm_bg.wasm')) });
const gc = globalThis.gc ?? (() => {});
const rss = {};
const { results, vectorBytes } = await runAll({
  bindings,
  memory: exports.memory,
  now: () => performance.now(),
  platform: { runtime: 'node', version: process.version, v8: process.versions.v8, arch: process.arch },
  sha256: async (bytes) => createHash('sha256').update(bytes).digest('hex'),
  gc,
  onStage: async (name) => { rss[name] = process.memoryUsage(); },
});
results.nodeMemory = rss;
mkdirSync(path.join(root, 'results'), { recursive: true });
// Inputs for the native half of the bit-identity check.
writeFileSync(path.join(root, 'results/p884.local.json'), JSON.stringify(project884()));
writeFileSync(path.join(root, 'results/coverage.local.json'), JSON.stringify(coverageProject()));
writeFileSync(path.join(root, 'results/times.local.json'), JSON.stringify(sampleTimes()));
for (const [name, bytes] of Object.entries(vectorBytes)) writeFileSync(path.join(root, `results/vectors-${name}.wasm-node.local.bin`), bytes);
writeFileSync(path.join(root, 'results/node.local.json'), JSON.stringify(results, null, 2));
console.log(JSON.stringify(results, null, 2));
