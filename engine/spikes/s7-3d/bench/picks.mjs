// pm-geom3d wasm pick benchmark and parity check in V8 (Node), against the three.js
// references from `bun ref/picks.ts`. Mirrors crates/s7-geom3d/src/bin/s7-picks.rs.
//
//   node bench/picks.mjs <s7_geom3d.wasm> [scene ...]
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { performance } from 'node:perf_hooks';

const root = join(dirname(new URL(import.meta.url).pathname), '..');
const [wasmPath, ...wanted] = process.argv.slice(2);
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), { env: { s7_now: () => performance.now() } });
const w = instance.exports;
const mem = () => new Uint8Array(w.memory.buffer);
const f64 = (ptr, n) => new Float64Array(w.memory.buffer, ptr, n);

function put(bytes) {
  const ptr = w.s7_alloc(bytes.length);
  mem().set(bytes, ptr);
  return ptr;
}
const scratch = w.s7_alloc(16 * 8 * 2);
function putMat(m, slot = 0) {
  f64(scratch + slot * 128, 16).set(m);
  return scratch + slot * 128;
}
const pct = (v, p) => { if (!v.length) return 0; const s = [...v].sort((a, b) => a - b); return s[Math.round((s.length - 1) * p)]; };

const dir = join(root, 'out/picks');
const scenes = wanted.length ? wanted : readdirSync(dir).filter(f => f.endsWith('.json') && !/^(native|wasm)/.test(f)).map(f => f.slice(0, -5)).sort();
const report = [];
for (const id of scenes) {
  const ref = JSON.parse(readFileSync(join(dir, `${id}.json`), 'utf8'));
  const def = JSON.parse(readFileSync(join(root, `out/corpus/scenes/${id}.json`), 'utf8'));
  w.s7_scene_reset();
  const load = [];
  for (const layerId of ref.ids) {
    const object = def.scene.objects.find(o => o.id === layerId);
    const file = def.assets[object.source.assetId].file;
    const bytes = readFileSync(join(root, 'out/corpus', file));
    const ptr = put(bytes);
    const t0 = performance.now();
    const model = w.s7_model_load(ptr, bytes.length, 1);
    const ms = performance.now() - t0;
    w.s7_free(ptr, bytes.length);
    if (model < 0) throw new Error(`${file}: ${new TextDecoder().decode(mem().subarray(w.s7_error_ptr(), w.s7_error_ptr() + w.s7_error_len()))}`);
    load.push({ layer: layerId, load_ms: ms });
    const layer = w.s7_layer_add(model);
    w.s7_layer_lazy(layer, process.env.S7_FULL_SKIN ? 0 : 1);
  }
  let total = 0, agree = 0, maxRel = 0;
  const frames = [];
  for (const frame of ref.frames) {
    const c = frame.camera;
    w.s7_camera(putMat(c.projection, 0), putMat(c.world, 1), c.isOrthographic ? 1 : 0, c.near, c.far);
    frame.layers.forEach((l, i) => { w.s7_layer_transform(i, putMat(l.gltfToWorld, 0)); w.s7_layer_time(i, frame.time); });
    const times = [];
    let first = 0;
    frame.rays.forEach(([x, y], k) => {
      const t0 = performance.now();
      const layer = w.s7_pick(x, y);
      const dt = performance.now() - t0;
      if (k === 0) first = dt; else times.push(dt);
      total++;
      const theirs = frame.hits[k];
      if (layer < 0) { if (theirs === null) agree++; return; }
      if (theirs === null) return;
      const hit = f64(w.s7_hit_ptr(), 8);
      const kind = hit[2] === 0 ? 'mesh' : hit[2] === 1 ? 'points' : 'line';
      const rel = Math.abs(hit[0] - theirs.distance) / Math.max(theirs.distance, 1e-9);
      if (ref.ids[layer] === theirs.layer && kind === theirs.kind) { maxRel = Math.max(maxRel, rel); if (rel < 1e-4) agree++; }
    });
    const update = Math.max(...frame.layers.map((_, i) => w.s7_layer_update_ms(i)));
    frames.push({ time: frame.time, rays: frame.rays.length, first_pick_ms: first, update_ms: update, median_ms: pct(times, 0.5), p95_ms: pct(times, 0.95), max_ms: pct(times, 1) });
  }
  // Stress timing on rays inside the model's screen footprint (the reference hits' NDC
  // bounds), from a fixed pseudo-random sequence: 200 cold picks, each the first after a
  // time change, then 2000 warm picks at one unchanged frame.
  const changes = [], warm = [];
  let hotHits = 0;
  if (ref.frames.length) {
    const frame = ref.frames[0];
    const hit = frame.rays.filter((_, k) => frame.hits[k]);
    const xs = hit.map(r => r[0]), ys = hit.map(r => r[1]);
    const [x0, x1, y0, y1] = xs.length ? [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)] : [-1, 1, -1, 1];
    let seed = 12345;
    const rnd = () => ((seed = (seed * 1103515245 + 12345) >>> 0) / 2 ** 32);
    for (let k = 0; k < 200; k++) {
      frame.layers.forEach((_, i) => w.s7_layer_time(i, 0.013 + k * 0.0371));
      const [x, y] = [x0 + (x1 - x0) * rnd(), y0 + (y1 - y0) * rnd()];
      const t0 = performance.now();
      w.s7_pick(x, y);
      changes.push(performance.now() - t0);
    }
    frame.layers.forEach((_, i) => w.s7_layer_time(i, 0.777));
    w.s7_pick(0, 0);
    for (let k = 0; k < 2000; k++) {
      const [x, y] = [x0 + (x1 - x0) * rnd(), y0 + (y1 - y0) * rnd()];
      const t0 = performance.now();
      if (w.s7_pick(x, y) >= 0) hotHits++;
      warm.push(performance.now() - t0);
    }
  }
  const vertices = ref.ids.reduce((n, _, i) => n + w.s7_layer_vertices(i), 0);
  const entry = { scene: id, vertices, load, rays: total, agree, max_rel_distance: maxRel, frames, time_change_first_pick_ms: { median: pct(changes, 0.5), p95: pct(changes, 0.95), max: pct(changes, 1) }, warm_footprint_pick_ms: { median: pct(warm, 0.5), p95: pct(warm, 0.95), max: pct(warm, 1), hits: hotHits, rays: warm.length } };
  report.push(entry);
  console.log(`${id}: ${vertices} vertices, load ${load.map(l => l.load_ms.toFixed(0)).join('+')} ms, ${agree}/${total} picks agree (max rel distance ${maxRel.toExponential(2)})`);
  for (const f of frames) console.log(`  t=${f.time}: first pick ${f.first_pick_ms.toFixed(2)} ms (update ${f.update_ms.toFixed(2)} ms), median ${f.median_ms.toFixed(4)} ms, p95 ${f.p95_ms.toFixed(4)} ms, max ${f.max_ms.toFixed(4)} ms`);
  const tc = entry.time_change_first_pick_ms, wm = entry.warm_footprint_pick_ms;
  console.log(`  footprint rays, first pick after a time change (200): median ${tc.median.toFixed(3)} ms, p95 ${tc.p95.toFixed(3)} ms, max ${tc.max.toFixed(3)} ms`);
  console.log(`  footprint rays, unchanged frame (2000, ${wm.hits} hits): median ${wm.median.toFixed(4)} ms, p95 ${wm.p95.toFixed(4)} ms, max ${wm.max.toFixed(4)} ms`);
}
writeFileSync(join(dir, `wasm-${wasmPath.includes('simd') ? 'simd' : 'scalar'}${process.env.S7_FULL_SKIN ? '-full' : ''}.json`), JSON.stringify(report, null, 1));
