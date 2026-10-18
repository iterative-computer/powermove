#!/usr/bin/env node
/* The memory row's `pm-wasm` part (plan section 25: "`pm-wasm` budget with the
   3D corpus"; docs/engine/phase3a-gate.md). Loads every 3D project of the
   newest v1 corpus capture (and the real project for scale) into its own
   instance of the built module, as a window's mirror holds it, asks every
   layer query at every captured sample time plus a pick grid, and reports
   the module's own counters: the Rust heap live and peak (`pm_heap_live`,
   `pm_heap_peak`, a counting allocator) and linear memory (never shrinks),
   against S5's proposed per-window budget (live ≤ 8 MB, linear high water
   ≤ 32 MB; docs/engine/spikes/S5.md). pm-wasm holds no 3D meshes or BVHs
   yet (3D picks and bounds stay on three.js until Phase 3c), so this is the
   mirror's share for 3D documents.

     node engine/crates/pm-cli/scripts/wasm-budget-3d.mjs [capture dir]

   Prints one JSON line per project and a summary, with the number of
   queries that answered an error (counted per op). */
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../../../..');
const modulePath = join(repo, 'apps/desktop/src/renderer/public/engine/pm_wasm.wasm');
const root = join(homedir(), 'Library/Application Support/Powermove Engine Corpus');
const capture = process.argv[2] ?? join(root, readdirSync(root).filter((d) => d.startsWith('v1-')).sort().pop());
const OPS = ['active', 'localMatrix', 'worldMatrix', 'transformParentMatrix', 'worldOpacity', 'resolveContent', 'local3D', 'parent3D'];
const LIVE_BUDGET = 8e6;
const LINEAR_BUDGET = 32e6;

const module = await WebAssembly.compile(readFileSync(modulePath));
const encoder = new TextEncoder();
const decoder = new TextDecoder();

const dirs = [];
const groups = join(capture, 'projects');
for (const g of ['3d', 'layers']) {
  for (const p of readdirSync(join(groups, g))) {
    if (g === 'layers' && !['obj-model', 'studio-cube'].includes(p)) continue;
    dirs.push([`${g}/${p}`, join(groups, g, p)]);
  }
}
for (const p of existsSync(join(capture, 'real')) ? readdirSync(join(capture, 'real')) : []) dirs.push([`real/${p}`, join(capture, 'real', p)]);

const rows = [];
for (const [name, dir] of dirs) {
  if (!existsSync(join(dir, 'loaded.json'))) continue;
  const text = readFileSync(join(dir, 'loaded.json'), 'utf8');
  const record = JSON.parse(readFileSync(join(dir, 'project.json'), 'utf8'));
  const doc = JSON.parse(text);
  const { exports: x } = await WebAssembly.instantiate(module, { pm_expr: { pow: Math.pow } });
  const put = (s) => { const bytes = encoder.encode(s); const ptr = x.pm_in(bytes.length || 1); new Uint8Array(x.memory.buffer, ptr, bytes.length).set(bytes); return bytes.length; };
  const out = (len) => decoder.decode(new Uint8Array(x.memory.buffer, x.pm_out_ptr(), len));
  if (x.pm_resync(put(text), 1) !== 0) throw new Error(`${name}: resync failed: ${out(x.pm_out_len())}`);
  x.pm_in_release?.();
  const loadedLive = x.pm_heap_live();
  const comps = [[doc.compId ?? doc.id, doc.layers ?? []], ...Object.entries(doc.comps ?? {}).map(([id, c]) => [id, c.layers ?? []])];
  const open = doc.compId ?? doc.id;
  const times = (record.samples ?? []).map((s) => s.time);
  let queries = 0;
  // Every reply is read: an error reply (`{"error":…}`) is counted, so a
  // failing query cannot pass for a cheap one and understate memory.
  const errors = {};
  const query = (request) => {
    const text = out(x.pm_query(put(JSON.stringify(request))));
    queries++;
    if (text.startsWith('{"error"')) errors[request.op] = (errors[request.op] ?? 0) + 1;
  };
  for (const [comp, layers] of comps) {
    layers.forEach((layer, index) => {
      for (const t of times) {
        for (const op of OPS) query({ op, comp, open, layer: index, t });
        query({ op: 'world3D', comp, open, layer: index, t, positioning: true });
        for (const key of Object.keys(layer.p ?? {})) query({ op: 'ev', comp, open, layer: index, key, t });
      }
    });
    for (let i = 0; i < 64; i++) {
      query({ op: 'pick', comp, open, x: (i % 8) * (doc.w ?? 1920) / 8, y: Math.floor(i / 8) * (doc.h ?? 1080) / 8, t: times[i % Math.max(1, times.length)] ?? 0, includeLocked: false });
    }
  }
  const row = {
    project: name,
    layers: comps.reduce((a, [, l]) => a + l.length, 0),
    documentBytes: text.length,
    queries,
    queryErrors: Object.values(errors).reduce((a, n) => a + n, 0),
    queryErrorsByOp: errors,
    heapLiveAfterLoad: loadedLive,
    heapLive: x.pm_heap_live(),
    heapPeak: x.pm_heap_peak(),
    linearBytes: x.memory.buffer.byteLength
  };
  rows.push(row);
  console.log(JSON.stringify(row));
}
const threeD = rows.filter((r) => !r.project.startsWith('real/'));
const max = (k, list = threeD) => Math.max(...list.map((r) => r[k]));
console.log(JSON.stringify({
  capture: capture.replace(homedir(), '~'),
  projects3d: threeD.length,
  heapLiveMax: max('heapLive'),
  heapPeakMax: max('heapPeak'),
  linearMax: max('linearBytes'),
  budget: { heapLive: LIVE_BUDGET, linear: LINEAR_BUDGET },
  queryErrors: rows.reduce((a, r) => a + r.queryErrors, 0),
  within: max('heapLive') <= LIVE_BUDGET && max('linearBytes') <= LINEAR_BUDGET,
  real: rows.find((r) => r.project.startsWith('real/')) ?? null
}));
