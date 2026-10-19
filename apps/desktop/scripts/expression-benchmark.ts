import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import os from 'node:os';
import { install as easing } from '../src/renderer/src/legacy/core/easing';
import { install as model } from '../src/renderer/src/legacy/core/model';
import { install as anim } from '../src/renderer/src/legacy/core/anim';
import { installEngineDoc } from '../src/renderer/src/engine-doc/install';
import { parseMode } from '../src/renderer/src/engine-doc/flags';

const sources = ['value + time * 40', 'param("amp") + comp.layers.length + time', 'layer.p.opacity.v + t * 13', 'Math.sin(time) * 10 + idx + value'];
const quantile = (xs: number[], q: number) => [...xs].sort((a, b) => a - b)[Math.min(xs.length - 1, Math.floor(xs.length * q))]!;
const stats = (xs: number[]) => ({ median: quantile(xs, .5), p95: quantile(xs, .95), min: Math.min(...xs), max: Math.max(...xs) });

// Only non-evaluation host services are inert. Model construction, composition
// lookup, expression compilation/evaluation, write barrier and native facade are
// the production modules; no expression helper or numerical operation is stubbed.
async function fixture(module: WebAssembly.Module, native: boolean, scalar = false) {
  let id = 0;
  const listeners = new Map<string, Set<(...args: any[]) => void>>();
  const PM: any = { uid: (prefix: string) => `${prefix}-${++id}`, clamp: (v: number, a: number, b: number) => Math.max(a, Math.min(b, v)),
    round: (v: number, places = 0) => Number(v.toFixed(places)), snapF: (t: number, fps: number) => Math.round(t * fps) / fps,
    invalidate() {}, toast() {}, assets: { map: new Map(), get: () => null }, FX: {},
    bus: { on(name: string, fn: (...args: any[]) => void) { const group = listeners.get(name) ?? new Set(); group.add(fn); listeners.set(name, group); return () => group.delete(fn); }, emit(name: string, ...args: any[]) { for (const fn of listeners.get(name) ?? []) fn(...args); } } };
  easing(PM); model(PM); anim(PM);
  PM.scope = [];
  PM.proj = PM.mkProject({ dur: 100 });
  PM.proj.params = { amp: { value: 7 } };
  PM.proj.layers = Array.from({ length: scalar ? 1 : 100 }, (_, i) => {
    const layer = PM.mkLayer('solid');
    for (let k = 0; k < (scalar ? 1 : 4); k++) layer.p[`expr${k}`] = { v: i + k, kf: [], expr: sources[k] };
    return layer;
  });
  const doc = native ? installEngineDoc(PM, { mode: parseMode('on'), strict: true, loadModule: async () => module, sink: () => {} }) : null;
  if (doc && !await doc.whenReady) throw new Error('Production pm-wasm failed to load');
  PM.touch();
  const channels = PM.proj.layers.flatMap((layer: any) => sources.slice(0, scalar ? 1 : 4).map((_, k) => ({ layer, prop: layer.p[`expr${k}`], key: `expr${k}` })));
  const ev = native ? (c: any, t: number) => doc!.api!.evP(c.layer, c.prop, t, c.key) : (c: any, t: number) => PM.evP(c.layer, c.prop, t, c.key);
  const frame = (t: number) => { let sum = 0; for (const c of channels) sum += ev(c, t); return sum; };
  return { PM, doc, channels, ev, frame };
}

export async function run(wasmPath: string) {
  if (os.getPriority() !== 0) throw new Error(`Benchmark requires nice 0, got ${os.getPriority()}`);
  const deadline = Date.now() + 30_000;
  while (os.loadavg()[0]! > 8 && Date.now() < deadline) await new Promise(resolve => setTimeout(resolve, 1000));
  if (os.loadavg()[0]! > 8) throw new Error('Machine load exceeds 8; retry on a quiet machine');
  const bytes = readFileSync(wasmPath), module = await WebAssembly.compile(bytes);
  const loadBefore = os.loadavg();
  const engine = await fixture(module, true), today = await fixture(module, false);
  for (const time of [0, -0, .001, .5, 1, 17 / 30, 9.999]) for (let i = 0; i < engine.channels.length; i++) {
    const a = engine.ev(engine.channels[i], time), b = today.ev(today.channels[i], time);
    if (!Object.is(a, b)) throw new Error(`Parity at channel ${i}, time ${time}: ${a} vs ${b}`);
  }
  const measure = (f: typeof engine, count: number) => {
    let sum = 0;
    const started = performance.now();
    for (let frame = 0; frame < count; frame++) sum += f.frame((frame % 1024) / 30);
    return { usPerFrame: (performance.now() - started) * 1000 / count, sum };
  };
  const pairs = (a: typeof engine, b: typeof today, count: number) => {
    measure(a, Math.min(count, 1000)); measure(b, Math.min(count, 1000));
    const rows = [];
    for (let pair = 0; pair < 9; pair++) {
      let engine, today;
      if (pair % 2) { today = measure(b, count); engine = measure(a, count); }
      else { engine = measure(a, count); today = measure(b, count); }
      if (!Object.is(engine.sum, today.sum)) throw new Error('Paired sums differ');
      rows.push({ pair, engine, today });
    }
    return { rows, engineUs: stats(rows.map(r => r.engine.usPerFrame)), todayUs: stats(rows.map(r => r.today.usPerFrame)) };
  };
  const frameCount = Number(process.env.PM_BENCH_FRAMES ?? 4000);
  if (!Number.isSafeInteger(frameCount) || frameCount < 1) throw new Error('PM_BENCH_FRAMES must be a positive integer');
  const frames = pairs(engine, today, frameCount);
  const sampleFrames = (f: typeof engine) => {
    const samples = [];
    let sum = 0;
    for (let frame = 0; frame < 250; frame++) {
      const started = performance.now();
      sum += f.frame((frame % 1024) / 30);
      samples.push((performance.now() - started) * 1000);
    }
    return { samples, sum };
  };
  const frameSamples = { engine: [] as number[], today: [] as number[] };
  for (let pair = 0; pair < 8; pair++) {
    let a, b;
    if (pair % 2) { b = sampleFrames(today); a = sampleFrames(engine); }
    else { a = sampleFrames(engine); b = sampleFrames(today); }
    if (!Object.is(a.sum, b.sum)) throw new Error('Frame latency sums differ');
    frameSamples.engine.push(...a.samples); frameSamples.today.push(...b.samples);
  }
  const latency = { engineUs: stats(frameSamples.engine), todayUs: stats(frameSamples.today), framesPerSide: frameSamples.engine.length };
  const scalar = pairs(await fixture(module, true, true), await fixture(module, false, true), 100_000);
  const cold = [], rebind = [];
  for (let pair = 0; pair < 9; pair++) {
    const a = await fixture(module, true), b = await fixture(module, false);
    const once = (f: typeof engine) => { const start = performance.now(); const sum = f.frame(.5); return { usPerFrame: (performance.now() - start) * 1000, sum }; };
    const edit = (f: typeof engine) => {
      const start = performance.now();
      f.PM.proj.params.amp.value = 9;
      f.channels[0].prop.expr = 'value + time * 41';
      f.PM.touch();
      const sum = f.frame(.5);
      return { usPerFrame: (performance.now() - start) * 1000, sum };
    };
    const runPair = (fn: typeof once) => {
      let engine, today;
      if (pair % 2) { today = fn(b); engine = fn(a); } else { engine = fn(a); today = fn(b); }
      if (!Object.is(engine.sum, today.sum)) throw new Error('Cold/edit sums differ');
      return { pair, engine, today };
    };
    cold.push(runPair(once)); rebind.push(runPair(edit));
    for (let i = 0; i < a.channels.length; i++) {
      if (!Object.is(a.ev(a.channels[i], .5), b.ev(b.channels[i], .5))) throw new Error(`Edit/rebind parity at ${i}`);
    }
  }
  const summarize = (rows: typeof cold) => ({ rows, engineUs: stats(rows.map(r => r.engine.usPerFrame)), todayUs: stats(rows.map(r => r.today.usPerFrame)) });
  const loadAfter = os.loadavg();
  const result = { runtime: process.versions, nice: os.getPriority(), loadBefore, loadAfter, wasmSha256: createHash('sha256').update(bytes).digest('hex'), channels: 400, framesPerPair: frameCount, warmupFramesPerSide: Math.min(frameCount, 1000),
    fixture: sources, frames, latency, scalar, cold: summarize(cold), editRebind: summarize(rebind), native: engine.doc!.stats(),
    pass: frames.engineUs.median <= frames.todayUs.median && loadAfter[0]! <= 8 };
  if (process.env.PM_BENCH_OUT) writeFileSync(process.env.PM_BENCH_OUT, JSON.stringify(result, null, 2) + '\n');
  console.log(JSON.stringify(result, null, 2));
  if (!result.pass) process.exitCode = 1;
}
