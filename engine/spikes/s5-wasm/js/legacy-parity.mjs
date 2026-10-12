// Legacy parity for spike S5: the TypeScript evaluator the engine replaces
// (legacy/core/anim.ts, temporal-bridge.ts, layer-groups.ts), unmodified,
// against the same projects and times, compared value by value with the wasm
// vectors written by `node js/node-run.mjs`. Bundled by Bun (TypeScript) and
// run in Node so the JavaScript engine is V8, as in the editor:
//
//   bun build js/legacy-parity.mjs --target=node --outfile target/legacy-parity.mjs
//   node target/legacy-parity.mjs .
//
// Bit-identity with the legacy code is not an S5 criterion (the criterion is
// native vs wasm); this shows the mirror ports the arithmetic exactly and
// reports any transcendental-function differences between the JS engine's
// Math and the libm crate.
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

import { install as installEasing } from '../../../../apps/desktop/src/renderer/src/legacy/core/easing.ts';
import { install as installAnim } from '../../../../apps/desktop/src/renderer/src/legacy/core/anim.ts';
import { installLayerGroups } from '../../../../apps/desktop/src/renderer/src/legacy/core/layer-groups.ts';

// The spike directory: the first argument, or this file's parent when unbundled.
const root = path.resolve(process.argv[2] ?? path.join(path.dirname(fileURLToPath(import.meta.url)), '..'));

function legacyPM(project) {
  const PM = {};
  PM.clamp = (v, a, b) => (v < a ? a : v > b ? b : v); // legacy/core/util.ts:174
  installEasing(PM);
  installAnim(PM);
  installLayerGroups(PM);
  PM.proj = project;
  PM.curComp = () => PM.proj;
  PM.L = (id) => PM.proj.layers.find((layer) => layer.id === id) || null;
  return PM;
}

function ulps(a, b) {
  if (Object.is(a, b)) return 0;
  if (Number.isNaN(a) || Number.isNaN(b)) return Infinity;
  const buf = new DataView(new ArrayBuffer(16));
  buf.setFloat64(0, a); buf.setFloat64(8, b);
  const ia = buf.getBigInt64(0), ib = buf.getBigInt64(8);
  const ord = (x) => (x < 0n ? -(x & 0x7fffffffffffffffn) : x);
  const d = ord(ia) - ord(ib);
  return Number(d < 0n ? -d : d);
}

const times = JSON.parse(readFileSync(path.join(root, 'results/times.local.json'), 'utf8'));
const report = { runtime: process.versions.bun ? `bun ${process.versions.bun}` : `node ${process.version} (V8 ${process.versions.v8})`, times: times.length, projects: {} };
let allEqual = true;
for (const name of ['p884', 'coverage']) {
  const project = JSON.parse(readFileSync(path.join(root, `results/${name}.local.json`), 'utf8'));
  const wasmBytes = readFileSync(path.join(root, `results/vectors-${name}.wasm-node.local.bin`));
  const wasm = new Float64Array(wasmBytes.buffer, wasmBytes.byteOffset, wasmBytes.byteLength / 8);
  const PM = legacyPM(project);
  const refs = [];
  for (const layer of project.layers) for (const key of Object.keys(layer.p || {})) refs.push([layer, key]);
  const per = refs.length + project.layers.length * 6;
  if (per * times.length !== wasm.length) throw new Error(`${name}: vector length ${wasm.length}, expected ${per * times.length}`);
  const legacy = new Float64Array(wasm.length);
  times.forEach((T, k) => {
    PM.beginEval(T);
    let at = k * per;
    for (const [layer, key] of refs) legacy[at++] = PM.ev(layer, key, T);
    for (const layer of project.layers) { legacy.set(PM.worldMatrix(layer, T), at); at += 6; }
  });
  let differing = 0, maxUlps = 0, channelDiffs = 0, matrixDiffs = 0;
  const examples = [];
  for (let i = 0; i < wasm.length; i++) {
    if (Object.is(wasm[i], legacy[i])) continue;
    differing++;
    const isChannel = i % per < refs.length;
    if (isChannel) channelDiffs++; else matrixDiffs++;
    const u = ulps(wasm[i], legacy[i]);
    maxUlps = Math.max(maxUlps, u);
    if (examples.length < 8) {
      const k = Math.floor(i / per), j = i % per;
      const where = j < refs.length ? `${refs[j][0].id}.${refs[j][1]}` : `world[${Math.floor((j - refs.length) / 6)}][${(j - refs.length) % 6}]`;
      examples.push({ t: times[k], where, wasm: wasm[i], legacy: legacy[i], ulps: u });
    }
  }
  allEqual &&= differing === 0;
  report.projects[name] = { values: wasm.length, differing, channelDiffs, matrixDiffs, maxUlps, examples };
}
report.bitIdenticalToLegacy = allEqual;
writeFileSync(path.join(root, 'results/legacy-parity.local.json'), JSON.stringify(report, null, 2));
console.log(JSON.stringify(report, null, 2));
