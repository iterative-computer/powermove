// Aggregate repeated S5 runs (scripts/run-all.sh) into pass/fail per target.
//
//   node js/summarize.mjs [results-dir]
//
// For each run it reads node.local.json and electron-<mode>.local.json, takes
// each run's own statistic (median over trials for per-call times, p95 over
// samples for patch times), and reports the median and the worst run. A
// target passes only if it passes in every run. Bit identity compares the
// SHA-256 of every vector file and every reported hash.
import { readFileSync, readdirSync, existsSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const dir = path.resolve(process.argv[2] ?? path.join(root, 'results'));
const runsDir = path.join(dir, 'runs');
const runs = readdirSync(runsDir).filter((d) => /^\d+$/.test(d)).sort((a, b) => a - b);
const load = (run, file) => {
  const p = path.join(runsDir, run, file);
  return existsSync(p) ? JSON.parse(readFileSync(p, 'utf8')) : null;
};
const hosts = {
  node: (run) => load(run, 'node.local.json'),
  'electron-plain': (run) => load(run, 'electron-plain.local.json')?.results ?? null,
  'electron-isolated': (run) => load(run, 'electron-isolated.local.json')?.results ?? null,
};

const kinds = ['reorder', 'insert', 'delete'];
// [name, unit, target or null, getter, lower is better]
const metrics = [
  ['evP cached, all 15,858 channels', 'ns', 1000, (r) => r.ev.evPCached.medianNs],
  ['evP cached, animated channels', 'ns', 1000, (r) => r.ev.evPAnimatedCached.medianNs],
  ['evP new time, animated (curve solve)', 'ns', null, (r) => r.ev.evPAnimatedUncached.medianNs],
  ['ev by numeric id, cached', 'ns', null, (r) => r.ev.evCached.medianNs],
  ['evBatch per value, cached', 'ns', null, (r) => r.ev.batchCachedPerValue.medianNs],
  ['world matrices, 884 layers, new time', 'us', null, (r) => r.ev.worldAllPerFrame.medianNs / 1000],
  ['property edit (position.x + revision), p95', 'ms', 0.2, (r) => r.propertyEdit.positionX.p95],
  ['keyframe value edit, p95', 'ms', 0.2, (r) => r.propertyEdit.keyframeValue.p95],
  ['property edit, mean of 200 back to back', 'ms', 0.2, (r) => r.propertyEdit.batchedMeanMs.median],
  ...kinds.map((k) => [`structural ${k}, id-keyed order patch, p95`, 'ms', 2, (r) => r.structural[k].orderPatch.total.p95]),
  ...kinds.map((k) => [`structural ${k}, history UTF-8 bytes, p95`, 'ms', 2, (r) => r.structural[k].bytesPatch.total.p95]),
  ...kinds.map((k) => [`structural ${k}, JSON string encode + apply, p95`, 'ms', 2, (r) => r.structural[k].arrayPatch.encodeAndApply.p95]),
  ...kinds.map((k) => [`structural ${k}, JSON.stringify + encode + apply, p95`, 'ms', 2, (r) => r.structural[k].arrayPatch.total.p95]),
  ['edit-log append (260 KB array), wasm apply p95', 'ms', null, (r) => r.editLogAppend.wasmApply.p95],
  ['load 884-layer project (1.4 MB JSON)', 'ms', null, (r) => r.loadMs],
  ['Rust heap live, one window steady state', 'MB', null, (r) => r.stages.afterLoad.heapLiveBytes / 1e6],
  ['Rust heap peak during load', 'MB', null, (r) => r.stages.loadPeak.heapPeakBytes / 1e6],
  ['wasm linear memory, steady state', 'MB', null, (r) => r.stages.afterLoad.wasmMemoryBytes / 1e6],
  ['wasm linear memory after the benchmark', 'MB', null, (r) => r.stages.afterBench.wasmMemoryBytes / 1e6],
];

const median = (xs) => { const s = [...xs].sort((a, b) => a - b); return s[s.length >> 1]; };
const fmt = (v, unit) => (v == null ? 'n/a' : unit === 'ns' || unit === 'us' ? v.toFixed(1) : unit === 'MB' ? v.toFixed(2) : v.toFixed(3));

const summary = { runs: runs.length, hosts: {}, correctness: {}, memory: {}, bitIdentity: {}, load: {} };
const lines = [];
for (const [host, get] of Object.entries(hosts)) {
  const results = runs.map(get).filter(Boolean);
  if (!results.length) continue;
  summary.hosts[host] = {};
  lines.push(`\n### ${host} (${results.length} runs)\n`, '| Measurement | Target | Median run | Worst run | Pass |', '|---|---|---|---|---|');
  for (const [name, unit, target, getter] of metrics) {
    const values = results.map(getter);
    const pass = target == null ? null : values.every((v) => v < target);
    const worst = Math.max(...values);
    summary.hosts[host][name] = { unit, target, values, median: median(values), worst, pass };
    lines.push(`| ${name} | ${target == null ? '-' : `< ${target} ${unit}`} | ${fmt(median(values), unit)} ${unit} | ${fmt(worst, unit)} ${unit} | ${pass == null ? '-' : pass ? 'pass' : 'FAIL'} |`);
  }
  summary.correctness[host] = results.map((r) => ({
    mirrorMatchesDocument: r.mirrorMatchesDocument,
    incrementalEqualsFresh: r.incrementalEqualsFresh,
    onlyChangedIdsInvalidated: r.structuralPass.onlyChangedIdsInvalidated,
    propertyWrong: r.propertyEdit.positionX.wrong + r.propertyEdit.keyframeValue.wrong,
    propertyInvalidatedMax: r.propertyEdit.positionX.invalidatedMax,
    growth: r.growth,
    unsupported: r.unsupported.length,
    timerResolutionMs: r.timerResolutionMs,
  }));
  summary.load[host] = results.map((r) => r.loadavg ?? null);
}

// Electron renderer process memory, from main, per run.
summary.memory.electronRenderer = runs.map((run) => {
  const e = load(run, 'electron-plain.local.json');
  if (!e?.memory) return null;
  const kb = (stage) => e.memory[stage]?.rssKB ?? e.memory[stage]?.workingSetKB ?? null;
  return {
    pageLoadedMB: kb('pageLoaded') / 1024, wasmInstantiatedMB: kb('wasmInstantiated') / 1024,
    beforeLoadMB: kb('beforeLoad') / 1024, afterLoadMB: kb('afterLoad') / 1024, afterBenchMB: kb('afterBench') / 1024,
    projectDeltaMB: (kb('afterLoad') - kb('beforeLoad')) / 1024,
    evaluatorDeltaMB: (kb('afterLoad') - kb('pageLoaded')) / 1024,
    jsHeap: e.results?.jsHeap ?? null, loadavg: e.loadavg,
  };
});

// Bit identity: native vector files, the Node wasm vector files, and every
// hash Electron reported.
const sha = (file) => (existsSync(file) ? createHash('sha256').update(readFileSync(file)).digest('hex') : null);
for (const name of ['p884', 'coverage']) {
  const hashes = {
    native: sha(path.join(dir, `vectors-${name}.native.local.bin`)),
    wasmNode: sha(path.join(dir, `vectors-${name}.wasm-node.local.bin`)),
    nativeSystemMath: sha(path.join(dir, `vectors-${name}.native-system.local.bin`)),
  };
  for (const run of runs) {
    for (const host of Object.keys(hosts)) {
      const r = hosts[host](run);
      if (r) hashes[`${host}#${run}`] = r.vectors[name].sha256;
    }
  }
  const { nativeSystemMath, ...rest } = hashes;
  const identical = new Set(Object.values(rest)).size === 1 && rest.native != null;
  summary.bitIdentity[name] = { identical, controlDiffers: nativeSystemMath !== rest.native, hashes };
}
const parity = path.join(dir, 'legacy-parity.local.json');
if (existsSync(parity)) summary.legacyParity = JSON.parse(readFileSync(parity, 'utf8'));

writeFileSync(path.join(dir, 'summary.local.json'), JSON.stringify(summary, null, 2));
console.log(lines.join('\n'));
console.log('\nBit identity:', JSON.stringify(Object.fromEntries(Object.entries(summary.bitIdentity).map(([k, v]) => [k, { identical: v.identical, controlDiffers: v.controlDiffers, sha256: v.hashes.native }]))));
console.log('Correctness:', JSON.stringify(summary.correctness));
console.log('Electron renderer memory:', JSON.stringify(summary.memory.electronRenderer));
if (summary.legacyParity) console.log('Legacy parity:', JSON.stringify(Object.fromEntries(Object.entries(summary.legacyParity.projects).map(([k, v]) => [k, { values: v.values, differing: v.differing, channelDiffs: v.channelDiffs, matrixDiffs: v.matrixDiffs, maxUlps: v.maxUlps }]))));
