// S5 measurements, shared by the Node runner and the Electron renderer.
//
// Every number is measured, never derived from a target. Per-operation times
// use `now()` around each operation; aggregate times divide a batch by its
// count. Timer resolution is measured and reported with the results.

import { WasmMirror, internAll } from './evaluator.mjs';
import { project884, coverageProject, sampleTimes, shapeStats, rng } from './synth.mjs';

export const TARGETS = {
  evPCachedNs: 1000, // evP < 1 microsecond cached
  propertyPatchMs: 0.2, // patch apply < 0.2 ms, property edits
  structuralPatchMs: 2, // patch apply < 2 ms, structural edits (884 layers)
};

function summarize(samples) {
  const sorted = Float64Array.from(samples).sort();
  const q = (p) => sorted[Math.min(sorted.length - 1, Math.floor(p * sorted.length))];
  const sum = sorted.reduce((a, b) => a + b, 0);
  return { n: sorted.length, mean: sum / sorted.length, p50: q(0.5), p95: q(0.95), p99: q(0.99), max: sorted[sorted.length - 1], min: sorted[0] };
}

export function timerResolution(now) {
  let min = Infinity;
  for (let i = 0; i < 2000; i++) {
    const a = now();
    let b = now();
    while (b === a) b = now();
    min = Math.min(min, b - a);
  }
  return min; // ms
}

/** Median over trials of (elapsed / calls), in ns per call. */
function perCall(now, trials, calls, body) {
  const results = [];
  for (let trial = 0; trial < trials; trial++) {
    const t0 = now();
    body();
    results.push(((now() - t0) * 1e6) / calls);
  }
  results.sort((a, b) => a - b);
  return { medianNs: results[results.length >> 1], minNs: results[0], maxNs: results[results.length - 1], trials, callsPerTrial: calls };
}

function cloneLayer(layer, uid, rand) {
  const copy = JSON.parse(JSON.stringify(layer));
  copy.id = uid();
  copy.name = `${copy.name} copy ${Math.floor(rand() * 1000)}`;
  for (const p of Object.values(copy.p)) for (const k of p.kf) k.i = uid();
  return copy;
}

export async function runAll({ bindings, memory, now, platform, sha256, gc = () => {}, onStage = async () => {} }) {
  const results = { platform, targets: TARGETS, stages: {} };
  // Linear memory only grows; the counting allocator in s5-wasm reports what
  // is live on the Rust heap and the peak since the last reset.
  const heap = () => ({ wasmMemoryBytes: memory.buffer.byteLength, heapLiveBytes: bindings.heap_live_bytes(), heapPeakBytes: bindings.heap_peak_bytes() });
  const memStage = (name) => { results.stages[name] = heap(); };
  results.timerResolutionMs = timerResolution(now);

  // ── load ───────────────────────────────────────────────────────────────
  const project = project884();
  results.shape = shapeStats(project);
  const projectJson = JSON.stringify(project);
  gc();
  memStage('beforeLoad');
  await onStage('beforeLoad');
  bindings.heap_reset_peak();
  const mirror = new WasmMirror(bindings, memory);
  let t0 = now();
  mirror.load(projectJson);
  results.loadMs = now() - t0;
  memStage('loadPeak');
  mirror.releaseInput();
  results.unsupported = mirror.unsupported();
  const { ids, refs } = internAll(mirror, project);
  results.handles = ids.length;
  // Steady state for one window: every handle interned and cached at one
  // time, world matrices computed once, the load buffer released.
  mirror.evBatch(ids, 10);
  mirror.worldAll(10);
  gc();
  memStage('afterLoad');
  results.stages.afterLoad.rawBytes = mirror.rawBytes();
  results.stages.afterLoad.documentJsonBytes = projectJson.length;
  await onStage('afterLoad');

  // ── evP ────────────────────────────────────────────────────────────────
  const T = 10;
  for (let i = 0; i < ids.length; i++) mirror.ev(ids[i], T); // warm every cache at T
  const e = mirror.e;
  const n = ids.length;
  const ROUNDS = 40;
  let sink = 0;
  const ev = {};
  ev.rawBoundaryCached = perCall(now, 9, n * ROUNDS, () => {
    for (let r = 0; r < ROUNDS; r++) for (let i = 0; i < n; i++) sink += e.ev(ids[i], T);
  });
  ev.evCached = perCall(now, 9, n * ROUNDS, () => {
    for (let r = 0; r < ROUNDS; r++) for (let i = 0; i < n; i++) sink += mirror.ev(ids[i], T);
  });
  for (let i = 0; i < refs.length; i++) mirror.evP(refs[i][0], refs[i][1], T, refs[i][2]); // fill the identity registry
  ev.evPCached = perCall(now, 9, n * ROUNDS, () => {
    for (let r = 0; r < ROUNDS; r++) for (let i = 0; i < refs.length; i++) { const x = refs[i]; sink += mirror.evP(x[0], x[1], T, x[2]); }
  });
  const animated = refs.filter((x) => x[1].kf.length);
  const animatedIds = Uint32Array.from(animated.map((x) => mirror.intern(x[0].id, x[2])));
  const AR = 4000;
  ev.evPAnimatedCached = perCall(now, 9, animated.length * AR, () => {
    for (let r = 0; r < AR; r++) for (let i = 0; i < animated.length; i++) { const x = animated[i]; sink += mirror.evP(x[0], x[1], T, x[2]); }
  });
  // Uncached: a new time on every call, so every animated call solves its curve.
  let tick = 0;
  ev.evPAnimatedUncached = perCall(now, 9, animated.length * AR, () => {
    for (let r = 0; r < AR; r++) for (let i = 0; i < animated.length; i++) { const x = animated[i]; sink += mirror.evP(x[0], x[1], 5 + (tick++ % 100000) * 1e-4, x[2]); }
  });
  ev.evPStaticNewTime = perCall(now, 9, n * 4, () => {
    for (let r = 0; r < 4; r++) for (let i = 0; i < refs.length; i++) { const x = refs[i]; sink += mirror.evP(x[0], x[1], 20 + (tick++ % 100000) * 1e-4, x[2]); }
  });
  mirror.evBatch(ids, T);
  ev.batchCachedPerValue = perCall(now, 9, n * 20, () => { for (let r = 0; r < 20; r++) sink += mirror.evBatch(ids, T)[r]; });
  ev.batchNewTimePerValue = perCall(now, 9, n * 20, () => { for (let r = 0; r < 20; r++) sink += mirror.evBatch(ids, 30 + (tick++) * 1e-3)[r]; });
  ev.worldAllPerFrame = perCall(now, 9, 20, () => { for (let r = 0; r < 20; r++) sink += mirror.worldAll(30 + (tick++) * 1e-3)[r]; });
  ev.worldAllPerFrame.layers = mirror.layerCount();
  results.ev = ev;
  results.evPass = ev.evPCached.medianNs < TARGETS.evPCachedNs && ev.evPAnimatedCached.medianNs < TARGETS.evPCachedNs;
  results.sink = Number.isFinite(sink);

  // ── property edits ─────────────────────────────────────────────────────
  const rand = rng(2026);
  let revision = 2;
  const editable = project.layers.map((l, i) => [l, i]).filter(([l]) => l.p['position.x']);
  const propertyEdit = { samples: [], invalidated: summarize([0]), wrong: 0 };
  const inv = [];
  const EDITS = 3000;
  for (let k = 0; k < EDITS; k++) {
    const [layer, index] = editable[Math.floor(rand() * editable.length)];
    const id = mirror.intern(layer.id, 'position.x');
    mirror.ev(id, T); // cached before the edit
    const value = Math.round(rand() * 1e6) / 1000;
    layer.p['position.x'].v = value; // the renderer's own document
    const patches = [
      { path: ['layers', index, 'p', 'position.x', 'v'], exists: true, value },
      { path: ['revision'], exists: true, value: revision },
    ];
    project.revision = revision++;
    t0 = now();
    mirror.apply(patches);
    propertyEdit.samples.push(now() - t0);
    inv.push(mirror.stats().invalidated);
    const expected = layer.p['position.x'].kf.length ? null : value;
    if (expected !== null && mirror.ev(id, T) !== expected) propertyEdit.wrong++;
  }
  // Keyframe value edits on animated channels.
  const keyEdit = { samples: [], wrong: 0 };
  const animatedRefs = project.layers.flatMap((l, i) => Object.entries(l.p).filter(([, p]) => p.kf.length).map(([k]) => [l, i, k]));
  for (let k = 0; k < EDITS; k++) {
    const [layer, index, channel] = animatedRefs[Math.floor(rand() * animatedRefs.length)];
    const keyIndex = Math.floor(rand() * layer.p[channel].kf.length);
    const value = Math.round(rand() * 1e5) / 100;
    layer.p[channel].kf[keyIndex].v = value;
    t0 = now();
    mirror.apply([{ path: ['layers', index, 'p', channel, 'kf', keyIndex, 'v'], exists: true, value }]);
    keyEdit.samples.push(now() - t0);
    // At the key's own time the channel answers the key's value. (from + t) - from
    // can differ from t in the last bits, so allow 1e-9 relative.
    const id = mirror.intern(layer.id, channel);
    const got = mirror.ev(id, layer.from + layer.p[channel].kf[keyIndex].t);
    if (!(Math.abs(got - value) <= 1e-9 * Math.max(1, Math.abs(value)))) keyEdit.wrong++;
  }
  // Batched: 200 edits applied back to back, timed together (timer-resolution independent).
  const batched = [];
  for (let trial = 0; trial < 9; trial++) {
    const list = [];
    for (let k = 0; k < 200; k++) {
      const [layer, index] = editable[Math.floor(rand() * editable.length)];
      const value = Math.round(rand() * 1e6) / 1000;
      layer.p['position.x'].v = value;
      list.push([{ path: ['layers', index, 'p', 'position.x', 'v'], exists: true, value }, { path: ['revision'], exists: true, value: revision }]);
      project.revision = revision++;
    }
    t0 = now();
    for (const patches of list) mirror.apply(patches);
    batched.push((now() - t0) / list.length);
  }
  batched.sort((a, b) => a - b);
  results.propertyEdit = {
    positionX: { ...summarize(propertyEdit.samples), wrong: propertyEdit.wrong, invalidatedMax: Math.max(...inv), invalidatedMin: Math.min(...inv) },
    keyframeValue: { ...summarize(keyEdit.samples), wrong: keyEdit.wrong },
    batchedMeanMs: { median: batched[4], min: batched[0], max: batched[8] },
  };
  results.propertyPass = results.propertyEdit.positionX.p95 < TARGETS.propertyPatchMs
    && results.propertyEdit.keyframeValue.p95 < TARGETS.propertyPatchMs
    && results.propertyEdit.batchedMeanMs.median < TARGETS.propertyPatchMs
    && propertyEdit.wrong === 0 && keyEdit.wrong === 0;

  // ── structural edits ───────────────────────────────────────────────────
  let counter = 0;
  const uid = () => `s5new${(counter++).toString(36)}`;
  const kinds = ['reorder', 'insert', 'delete'];
  const withChannels = () => { for (;;) { const l = project.layers[Math.floor(rand() * project.layers.length)]; if (Object.keys(l.p).length === 18) return l; } };
  const structural = {};
  for (const kind of kinds) structural[kind] = { stringify: [], encode: [], apply: [], total: [], parsed: [], invalidated: [], kept: [], orderTotal: [], orderParsed: [], orderInvalidated: [] };
  const evaluateAll = () => mirror.evBatch(ids, T); // all handles cached before each edit
  const cacheLayer = (layer) => { for (const key of Object.keys(layer.p)) mirror.ev(mirror.intern(layer.id, key), T); };
  const orderPatch = (layers, changed) => [{ path: ['layers'], order: layers.map((l) => l.id), layers: changed }];
  const ROUNDS_S = 60;
  for (let r = 0; r < ROUNDS_S; r++) {
    for (const kind of kinds) {
      for (const encoding of ['array', 'order']) {
        const layers = project.layers;
        let changed = [];
        let inserted = null;
        if (kind === 'reorder') {
          const from = Math.floor(rand() * layers.length);
          const [moved] = layers.splice(from, 1);
          layers.splice(Math.floor(rand() * layers.length), 0, moved);
        } else if (kind === 'insert') {
          inserted = cloneLayer(withChannels(), uid, rand);
          layers.splice(Math.floor(rand() * layers.length), 0, inserted);
          changed = [inserted];
        } else {
          // Setup (untimed): insert a layer, then delete it in the timed patch.
          const victim = cloneLayer(withChannels(), uid, rand);
          layers.splice(Math.floor(rand() * layers.length), 0, victim);
          mirror.apply(orderPatch(layers, [victim]));
          layers.splice(layers.indexOf(victim), 1);
          cacheLayer(victim);
        }
        evaluateAll();
        if (inserted) cacheLayer(inserted); // handles waiting for the new id (answer NaN until it exists)
        const s = structural[kind];
        if (encoding === 'array') {
          const a = now();
          const json = JSON.stringify(layers);
          const b = now();
          const len = mirror.encode([{ path: ['layers'], exists: true, json }]);
          const c = now();
          mirror.applyEncoded(len);
          const d = now();
          s.stringify.push(b - a); s.encode.push(c - b); s.apply.push(d - c); s.total.push(d - a);
          const st = mirror.stats();
          s.parsed.push(st.parsed); s.invalidated.push(st.invalidated); s.kept.push(st.kept);
        } else {
          const a = now();
          mirror.apply(orderPatch(layers, changed));
          const d = now();
          s.orderTotal.push(d - a);
          const st = mirror.stats();
          s.orderParsed.push(st.parsed); s.orderInvalidated.push(st.invalidated);
        }
        if (inserted) {
          // Cleanup (untimed): remove the inserted layer again.
          layers.splice(layers.indexOf(inserted), 1);
          mirror.apply(orderPatch(layers, []));
        }
        if (inserted && !Number.isNaN(mirror.ev(mirror.intern(inserted.id, 'position.x'), T))) {
          throw new Error('removed layer still answers');
        }
      }
    }
  }
  // The patched mirror must equal the renderer's document, and answer exactly
  // what a mirror loaded from scratch answers.
  const mirrored = JSON.parse(mirror.toJSON());
  results.mirrorMatchesDocument = JSON.stringify(mirrored) === JSON.stringify(project);
  {
    const fresh = new WasmMirror(bindings, memory);
    fresh.load(project);
    const a = internAll(mirror, project).ids;
    const b = internAll(fresh, project).ids;
    let compared = 0, differing = 0;
    for (const t of [0, 3.3, 10, 17.25, 41]) {
      const x = mirror.evBatch(a, t).slice();
      const y = fresh.evBatch(b, t);
      for (let i = 0; i < x.length; i++) { compared++; if (!Object.is(x[i], y[i])) differing++; }
      const wx = mirror.worldAll(t).slice();
      const wy = fresh.worldAll(t);
      for (let i = 0; i < wx.length; i++) { compared++; if (!Object.is(wx[i], wy[i])) differing++; }
    }
    results.incrementalEqualsFresh = { compared, differing };
    fresh.free();
  }
  results.structural = {};
  for (const kind of kinds) {
    const s = structural[kind];
    results.structural[kind] = {
      arrayPatch: {
        stringify: summarize(s.stringify), encode: summarize(s.encode), wasmApply: summarize(s.apply),
        encodeAndApply: summarize(s.encode.map((x, i) => x + s.apply[i])), total: summarize(s.total),
        parsed: summarize(s.parsed), invalidated: summarize(s.invalidated), kept: summarize(s.kept),
      },
      orderPatch: { total: summarize(s.orderTotal), parsed: summarize(s.orderParsed), invalidated: summarize(s.orderInvalidated) },
    };
  }
  const st = results.structural;
  results.structuralPass = {
    // The mirror's own cost: patch bytes already serialized by the history.
    arrayEncodeAndApplyP95: kinds.every((k) => st[k].arrayPatch.encodeAndApply.p95 < TARGETS.structuralPatchMs),
    // Including JSON.stringify of the whole 884-layer array in JS.
    arrayTotalP95: kinds.every((k) => st[k].arrayPatch.total.p95 < TARGETS.structuralPatchMs),
    // Id-keyed order patch, end to end in JS.
    orderTotalP95: kinds.every((k) => st[k].orderPatch.total.p95 < TARGETS.structuralPatchMs),
    // Reorder: nothing parsed, nothing invalidated. Insert: one layer parsed,
    // only the 18 handles waiting for its id invalidated. Delete: only the
    // deleted layer's 18 handles invalidated.
    onlyChangedIdsInvalidated: [st.reorder.arrayPatch, st.reorder.orderPatch].every((x) => x.invalidated.max === 0 && x.parsed.max === 0)
      && [st.insert.arrayPatch, st.insert.orderPatch].every((x) => x.invalidated.min === 18 && x.invalidated.max === 18 && x.parsed.min === 1 && x.parsed.max === 1)
      && [st.delete.arrayPatch, st.delete.orderPatch].every((x) => x.invalidated.min === 18 && x.invalidated.max === 18 && x.parsed.max === 0),
  };

  // Edit-log append: the top-level `edits` array replaced with one more entry.
  // Edit.mutate's record() appends to it, and diffPatches replaces the whole array.
  const editLog = { stringify: [], encode: [], apply: [] };
  for (let r = 0; r < 60; r++) {
    project.edits.push({ id: uid(), at: 1700000000000 + r, source: 'user', label: 'Move', commands: [] });
    const a = now();
    const json = JSON.stringify(project.edits);
    const b = now();
    const len = mirror.encode([{ path: ['edits'], exists: true, json }]);
    const c = now();
    mirror.applyEncoded(len);
    const d = now();
    editLog.stringify.push(b - a); editLog.encode.push(c - b); editLog.apply.push(d - c);
  }
  results.editLogAppend = {
    bytes: JSON.stringify(project.edits).length, stringify: summarize(editLog.stringify),
    encode: summarize(editLog.encode), wasmApply: summarize(editLog.apply),
  };

  gc();
  memStage('afterBench');
  results.viewRefreshes = mirror.viewRefreshes;
  await onStage('afterBench');

  // ── memory growth: views must survive a grow ─────────────────────────────
  const before = mirror.evBatch(ids.subarray(0, 32), T).slice();
  const grownFrom = memory.buffer.byteLength;
  mirror.reserveInput(memory.buffer.byteLength + 8 * 1024 * 1024); // forces memory.grow
  const after = mirror.evBatch(ids.subarray(0, 32), T);
  results.growth = {
    grownFromBytes: grownFrom, grownToBytes: memory.buffer.byteLength,
    viewsReacquired: mirror.viewRefreshes, sameResults: before.every((v, i) => Object.is(v, after[i])),
  };

  // ── vectors for bit-identity ─────────────────────────────────────────────
  results.vectors = {};
  const vectorBytes = {};
  for (const [name, doc] of [['p884', project884()], ['coverage', coverageProject()]]) {
    const m = new WasmMirror(bindings, memory);
    m.load(doc);
    const { ids: vids } = internAll(m, doc);
    const times = sampleTimes();
    const per = vids.length + m.layerCount() * 6;
    const out = new Float64Array(per * times.length);
    times.forEach((t, k) => {
      out.set(m.evBatch(vids, t), k * per);
      out.set(m.worldAll(t), k * per + vids.length);
    });
    const bytes = new Uint8Array(out.buffer);
    results.vectors[name] = { values: out.length, sha256: await sha256(bytes), unsupported: m.unsupported().length };
    vectorBytes[name] = bytes;
    m.free();
  }
  return { results, vectorBytes };
}
