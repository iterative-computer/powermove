#!/usr/bin/env node
/* Native vs wasm32 identity of pm-wasm's answers (plan section 7,
   determinism; the Phase 1 gate "native and WASM bit-identical").

     node engine/crates/pm-wasm/scripts/identity.mjs [--seeds N] [--layers N]

   Generates seeded random projects (parents, groups, 2.5D, keys of every
   interpolation, springs, expressions with Math and wiggle, effects, masks,
   paths), asks every pm-wasm query over every layer at many times, and the
   fast path for every channel, through the built module in this Node (V8)
   and through the native engine (`cargo run --example replay`). Every reply
   must be byte-identical; fast-path numbers are compared by their bits (NaN
   canonicalized: WebAssembly does not fix NaN payloads). */
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const engineDir = resolve(here, '../../..');
const repo = resolve(engineDir, '..');
const args = process.argv.slice(2);
const option = (name, fallback) => { const i = args.indexOf(name); return i >= 0 ? Number(args[i + 1]) : fallback; };
const SEEDS = option('--seeds', 6);
const LAYERS = option('--layers', 40);
const modulePath = join(repo, 'apps/desktop/src/renderer/public/engine/pm_wasm.wasm');

function rng(seed) {
  return () => {
    seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const CHANNELS = ['position.x', 'position.y', 'anchor.x', 'anchor.y', 'scale.x', 'scale.y', 'rotation', 'opacity', 'skew',
  'position.z', 'rotation.x', 'rotation.y', 'orientation.z', 'scale.z', 'perspective'];
const EXPRESSIONS = ['value + 10', 'time * 30', 'Math.sin(time * 3) * 50 + value', 'wiggle(2, 10)', 'value *', 'Math.atan2(time, 2) * 100',
  'Math.pow(time + 1, 1.7)', 'clamp(time * 100, 0, 50)', 'random() * 10', 'layer.from + t', 'comp.w / 4', 'param("size")', 'loop(value)', 'ease(time, 0, 2, 0, 100)'];

function project(seed) {
  const random = rng(seed);
  let n = 0;
  const id = (p) => `${p}${seed}-${++n}`;
  const key = (t, v) => {
    const k = { t, v, i: id('k') };
    const roll = random();
    if (roll < 0.25) Object.assign(k, { eo: [0.33, 0], ei: [0.67, 1] });
    else if (roll < 0.5) Object.assign(k, { outInterp: 'bezier', inInterp: 'bezier', outEase: { speed: random() * 200 - 100, influence: 10 + random() * 80 }, inEase: { speed: random() * 50, influence: 33.33 } });
    else if (roll < 0.6) Object.assign(k, { outInterp: 'hold', inInterp: 'linear', outEase: { speed: 0, influence: 16.67 }, inEase: { speed: 0, influence: 16.67 } });
    else if (roll < 0.7) Object.assign(k, { spring: { damping: 10 + random() * 20, stiffness: 100 + random() * 200, mass: 1 } });
    else if (roll < 0.8) Object.assign(k, { autoBezier: true, outInterp: 'bezier', inInterp: 'bezier', outEase: { speed: 0, influence: 33.33 }, inEase: { speed: 0, influence: 33.33 } });
    return k;
  };
  const channel = (v) => {
    const p = { v, kf: [], expr: null };
    const roll = random();
    if (roll < 0.35) {
      const count = 2 + Math.floor(random() * 4);
      for (let i = 0; i < count; i++) p.kf.push(key(Math.round((i * 1.3 + random()) * 30) / 30, Math.round(random() * 600 - 300) / (random() < 0.5 ? 1 : 7)));
      p.kf.sort((a, b) => a.t - b.t);
    } else if (roll < 0.5) p.expr = EXPRESSIONS[Math.floor(random() * EXPRESSIONS.length)];
    return p;
  };
  const types = ['solid', 'shape', 'null', 'image', 'video', 'precomp', 'adjustment', 'shader', 'extension', 'group'];
  const layers = [];
  for (let i = 0; i < LAYERS; i++) {
    const type = types[Math.floor(random() * types.length)];
    const p = {};
    for (const c of CHANNELS) if (random() < 0.8 || i % 3 === 0) p[c] = channel(c.startsWith('scale') ? 100 : c === 'opacity' ? 100 : c === 'perspective' ? 50 : Math.round(random() * 400));
    const layer = { id: id('L'), type, name: `L${i}`, from: Math.round(random() * 60) / 30 - 0.5, dur: 0.5 + Math.round(random() * 240) / 30,
      on: random() < 0.95, lock: random() < 0.1, parent: null, p, fx: [], masks: [], d: { w: 50 + Math.round(random() * 500), h: 50 + Math.round(random() * 300), color: '#808080' } };
    if (random() < 0.3) layer.threeD = true;
    if (type === 'shape') {
      layer.d = { shape: random() < 0.5 ? 'rect' : 'ellipse', w: channel(200), h: 120, radius: 12, stroke: 2, color: '#ffffff', points: 5 };
      if (random() < 0.4) layer.d.paths = [{ id: id('g'), name: 'Path', parent: null, p: { x: channel(0), y: channel(0), rotation: channel(0), scaleX: channel(100), scaleY: channel(100), closed: true },
        vertices: [0, 1, 2, 3].map(j => ({ id: id('v'), p: { x: channel(Math.cos(j) * 80), y: channel(Math.sin(j) * 80), inX: channel(0), inY: channel(0), outX: channel(10), outY: channel(0) } })) }];
    }
    if (random() < 0.3) layer.fx.push({ id: id('F'), type: 'blur', on: random() < 0.8, p: { amount: channel(5) } });
    if (random() < 0.2) layer.masks.push({ id: id('M'), shape: 'rect', mode: 'add', on: true, p: { x: channel(0), y: channel(0), w: channel(100), h: channel(80), rotation: channel(0), feather: channel(4) } });
    layers.push(layer);
  }
  layers.forEach((layer, i) => {
    if (i > 0 && random() < 0.35) layer.parent = layers[Math.floor(random() * i)].id;
    if (layer.type !== 'group' && random() < 0.25) {
      const groups = layers.filter(g => g.type === 'group' && g !== layer);
      if (groups.length) layer.group = groups[Math.floor(random() * groups.length)].id;
    }
  });
  return {
    id: `P${seed}`, name: 'Identity', compId: 'C1', revision: 3, assets: {}, params: { size: { value: 42 } },
    comps: {
      C1: { id: 'C1', name: 'Main', w: 1920, h: 1080, fps: seed % 2 ? 30 : 29.97, dur: 8, layers, markers: [], work: [0, 8] },
      C2: { id: 'C2', name: 'Nested', w: 640, h: 360, fps: 24, dur: 3, layers: layers.slice(0, 5).map(l => ({ ...l, id: `${l.id}n`, parent: null, group: undefined })), markers: [], work: [0, 3] },
    },
  };
}

const TIMES = [0, 0.5, 1 / 3, 1.25, 2, 2.0000000001, 3.999, 5.5, 7.9, -0.25, 12];
const LAYER_OPS = ['active', 'localMatrix', 'worldMatrix', 'transformParentMatrix', 'worldOpacity', 'resolveContent', 'local3D', 'parent3D',
  'world3D', 'is3DLayer', 'perspectiveAmount', 'planeMatrix', 'bounds', 'groupBounds', 'groupAncestors', 'groupSpan', 'allProps'];

function requests(doc, random) {
  const out = [];
  for (const comp of ['C1', 'C2']) {
    const layers = doc.comps[comp].layers;
    layers.forEach((layer, index) => {
      for (const t of TIMES) {
        for (const op of LAYER_OPS) out.push({ op, comp, open: 'C1', layer: index, t });
        out.push({ op: 'world3D', comp, open: 'C1', layer: index, t, positioning: true });
        out.push({ op: 'planeContains', comp, open: 'C1', layer: index, t, x: random() * 1920, y: random() * 1080, bounds: { x0: -100, y0: -100, x1: 100, y1: 100 } });
        for (const key of Object.keys(layer.p)) out.push({ op: 'ev', comp, open: 'C1', layer: index, key, t });
        if (layer.d?.paths?.length) { out.push({ op: 'pathValues', comp, open: 'C1', layer: index, index: 0, t }); out.push({ op: 'groupMatrix', comp, open: 'C1', layer: index, index: 0, t }); }
      }
      out.push({ op: 'findProp', comp, open: 'C1', layer: index, key: 'rotation' });
      out.push({ op: 'wouldCycle', comp, open: 'C1', layer: index, parent: layers[Math.floor(random() * layers.length)].id });
      out.push({ op: 'validMatteSource', comp, open: 'C1', layer: index, id: layers[Math.floor(random() * layers.length)].id });
    });
    for (let i = 0; i < 60; i++) out.push({ op: 'pick', comp, open: 'C1', x: random() * 1920, y: random() * 1080, t: random() * 8, includeLocked: random() < 0.5 });
    out.push({ op: 'transformRoots', comp, open: 'C1', ids: layers.map(l => l.id) });
    out.push({ op: 'expandGroups', comp: 'C1', open: 'C1', ids: layers.filter(l => l.type === 'group').map(l => l.id) });
  }
  out.push({ op: 'mul', a: [1.5, 0.25, -0.5, 2, 10, 20], b: [0.1, 0.2, 0.3, 0.4, 0.5, 0.6] });
  out.push({ op: 'countTextUnits', lines: ['Hello world', 'naïve café 👩‍👩‍👧'], unit: 'characters' });
  out.push({ op: 'expressionDiagnostic', source: 'Math.sin(time' });
  return out;
}

function fast(doc) {
  const out = [];
  for (const layer of doc.comps.C1.layers) for (const key of Object.keys(layer.p)) out.push({ address: { comp: 'C1', layer: layer.id, key, via: 'channel' }, times: TIMES });
  return out;
}

const decoder = new TextDecoder();
const encoder = new TextEncoder();
async function wasmReplies(module, cases) {
  const lines = [];
  for (const c of cases) {
    const { exports: x } = await WebAssembly.instantiate(module, { pm_expr: { pow: Math.pow } });
    const put = (text) => { const bytes = encoder.encode(text); const ptr = x.pm_in(bytes.length || 1); new Uint8Array(x.memory.buffer, ptr, bytes.length).set(bytes); return bytes.length; };
    const out = (len) => decoder.decode(new Uint8Array(x.memory.buffer, x.pm_out_ptr(), len));
    if (x.pm_resync(put(c.doc), 1) !== 0) throw new Error('resync failed: ' + out(x.pm_out_len()));
    for (const r of c.requests) lines.push(out(x.pm_query(put(JSON.stringify(r)))));
    const view = new DataView(new ArrayBuffer(8));
    for (const f of c.fast) {
      const id = x.pm_intern(put(JSON.stringify(f.address)));
      if (id < 0) { lines.push('fast unaddressable'); continue; }
      for (const t of f.times) {
        const n = x.pm_ev(id, t);
        view.setFloat64(0, Number.isNaN(n) ? NaN : n);
        lines.push(`fast ${x.pm_kind()} ${view.getBigUint64(0).toString(16).padStart(16, '0')}`);
      }
    }
    lines.push(`case ${lines.length && cases.indexOf(c)} hash ${out(x.pm_hash())}`);
  }
  return lines;
}

const cases = [];
for (let seed = 1; seed <= SEEDS; seed++) {
  const doc = project(seed);
  cases.push({ doc: JSON.stringify(doc), requests: requests(doc, rng(seed * 7919)), fast: fast(doc) });
}
const dir = mkdtempSync(join(tmpdir(), 'pm-wasm-identity-'));
try {
  const file = join(dir, 'requests.json');
  writeFileSync(file, JSON.stringify({ cases }));
  const native = execFileSync('bash', ['-c', `source env.sh && nice -n 15 cargo run --quiet --release -p pm-wasm --example replay -- "${file}"`], { cwd: engineDir, maxBuffer: 1 << 30 }).toString().trimEnd().split('\n');
  const wasm = await wasmReplies(await WebAssembly.compile(readFileSync(modulePath)), cases);
  let differ = 0, numbers = 0, first = null;
  for (let i = 0; i < Math.max(native.length, wasm.length); i++) {
    if (native[i] !== wasm[i]) { differ++; first ??= { line: i, native: native[i]?.slice(0, 300), wasm: wasm[i]?.slice(0, 300) }; }
    numbers += (wasm[i]?.match(/-?\d+(\.\d+)?(e[+-]?\d+)?/g) ?? []).length;
  }
  const hosts = wasm.filter(l => l === '{"host":true}').length, fonts = wasm.filter(l => l === '{"fonts":true}').length, throws = wasm.filter(l => l.startsWith('{"throw"')).length;
  console.log(`pm-wasm identity: ${wasm.length} replies (${native.length} native) over ${SEEDS} projects of ${LAYERS} layers, ~${numbers} numbers; ${differ} differ; ${throws} throws, ${hosts} host-only, ${fonts} need fonts`);
  if (first) console.log('first difference', first);
  process.exitCode = differ || native.length !== wasm.length ? 1 : 0;
} finally {
  rmSync(dir, { recursive: true, force: true });
}
