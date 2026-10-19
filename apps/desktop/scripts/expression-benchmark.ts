import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import os from 'node:os';
import { install as easing } from '../src/renderer/src/legacy/core/easing';
import { install as model } from '../src/renderer/src/legacy/core/model';
import { install as anim } from '../src/renderer/src/legacy/core/anim';
import { install as selection } from '../src/renderer/src/legacy/core/selection';
import { install as history } from '../src/renderer/src/legacy/core/history';
import { install as editing } from '../src/renderer/src/legacy/core/editing';
import { setSceneParameterValue } from '../src/renderer/src/legacy/core/workspace';
import { installEngineDoc } from '../src/renderer/src/engine-doc/install';
import { parseMode } from '../src/renderer/src/engine-doc/flags';
import { ELECTRON_ONLY_SUITES, EXPRESSION_SUITES, frameChannels } from '../e2e/helpers/expression-suites';

const PROFILE = !!process.env.PM_BENCH_PROFILE;
// The original warm/edit/cold fixture. The frame suite below adds the other
// expression styles and a sequential-layers read pattern.
const sources = EXPRESSION_SUITES.numeric!;
const quantile = (xs: number[], q: number) => [...xs].sort((a, b) => a - b)[Math.min(xs.length - 1, Math.floor(xs.length * q))]!;
const stats = (xs: number[]) => ({ median: quantile(xs, .5), p95: quantile(xs, .95), min: Math.min(...xs), max: Math.max(...xs) });

// Only non-evaluation host services are inert. Model construction, composition
// lookup, expression compilation/evaluation, write barrier and native facade are
// the production modules; no expression helper or numerical operation is stubbed.
async function fixture(module: WebAssembly.Module, native: boolean, scalar = false,
  options: { sources?: string[]; keyed?: boolean; pattern?: 'all' | 'window' } = {}) {
  const fixtureSources = options.sources ?? sources;
  let id = 0;
  const listeners = new Map<string, Set<(...args: any[]) => void>>();
  const PM: any = { uid: (prefix: string) => `${prefix}-${++id}`, clamp: (v: number, a: number, b: number) => Math.max(a, Math.min(b, v)),
    round: (v: number, places = 0) => Number(v.toFixed(places)), snapF: (t: number, fps: number) => Math.round(t * fps) / fps,
    invalidate() {}, toast() {}, assets: { map: new Map(), get: () => null }, FX: {},
    bus: { on(name: string, fn: (...args: any[]) => void) { const group = listeners.get(name) ?? new Set(); group.add(fn); listeners.set(name, group); return () => group.delete(fn); }, emit(name: string, ...args: any[]) { for (const fn of listeners.get(name) ?? []) fn(...args); } } };
  easing(PM); model(PM); selection(PM); anim(PM); history(PM); editing(PM);
  PM.scope = [];
  PM.proj = PM.mkProject({ dur: 100 });
  PM.proj.params = { amp: { name: 'amp', value: 7 } };
  PM.proj.layers = Array.from({ length: scalar ? 1 : 100 }, (_, i) => {
    const layer = PM.mkLayer('solid');
    for (let k = 0; k < (scalar ? 1 : 4); k++) {
      const kf = options.keyed ? [{ t: 0, v: i + k }, { t: 3, v: i + k + 30 }] : [];
      layer.p[`expr${k}`] = { v: i + k, kf, expr: fixtureSources[k] };
    }
    return layer;
  });
  const doc = native ? installEngineDoc(PM, { mode: parseMode('on'), strict: true, loadModule: async () => module, sink: () => {} }) : null;
  if (doc && !await doc.whenReady) throw new Error('Production pm-wasm failed to load');
  PM.touch();
  const profile = { flushUs: 0, batchUs: 0, frameUs: 0, evaluated: 0 };
  if (doc) {
    // Counts ids evaluated by frame runs (an integer add per run, not per id).
    const prepare = doc.wasm!.prepareBatch.bind(doc.wasm!);
    doc.wasm!.prepareBatch = ids => {
      const evaluate = prepare(ids);
      return (time, start = 0, count = ids.length - start) => { profile.evaluated += count; return evaluate(time, start, count); };
    };
  }
  if (doc && PROFILE) {
    const flush = doc.barrier!.flush.bind(doc.barrier!);
    doc.barrier!.flush = () => {
      if (!doc.barrier!.pending) return flush();
      const start = performance.now(); try { return flush(); } finally { profile.flushUs += (performance.now() - start) * 1000; }
    };
    const prepare = doc.wasm!.prepareBatch.bind(doc.wasm!);
    doc.wasm!.prepareBatch = ids => {
      const evaluate = prepare(ids);
      return (time, start, count) => { const begin = performance.now(); try { return evaluate(time, start, count); } finally { profile.batchUs += (performance.now() - begin) * 1000; } };
    };
  }
  const channels = PM.proj.layers.flatMap((layer: any) => fixtureSources.slice(0, scalar ? 1 : 4).map((_, k) => ({ layer, prop: layer.p[`expr${k}`], key: `expr${k}` })));
  const ev = native ? (c: any, t: number) => doc!.api!.evP(c.layer, c.prop, t, c.key) : (c: any, t: number) => PM.evP(c.layer, c.prop, t, c.key);
  const pattern = options.pattern ?? 'all';
  const evaluateFrame = (t: number) => {
    const [first, end] = frameChannels(pattern, t, channels.length);
    let sum = 0;
    for (let i = first; i < end; i++) sum += ev(channels[i], t);
    return sum;
  };
  const frame = PROFILE ? (t: number) => { const start = performance.now(); try { return evaluateFrame(t); } finally { profile.frameUs += (performance.now() - start) * 1000; } } : evaluateFrame;
  return { PM, doc, channels, ev, frame, profile };
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
  // PM_BENCH_SECTIONS=suite,edits,… runs a subset while iterating; the
  // recorded results run every section (a partial run never passes).
  const sections = new Set((process.env.PM_BENCH_SECTIONS ?? 'frames,scalar,cold,edits,suite').split(','));
  const partial = sections.size < 5;
  const want = (name: string) => sections.has(name);
  const skipped = { rows: [], engineUs: stats([0]), todayUs: stats([0]) };
  const frames = want('frames') ? pairs(engine, today, frameCount) : skipped;
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
  for (let pair = 0; pair < (want('frames') ? 8 : 0); pair++) {
    let a, b;
    if (pair % 2) { b = sampleFrames(today); a = sampleFrames(engine); }
    else { a = sampleFrames(engine); b = sampleFrames(today); }
    if (!Object.is(a.sum, b.sum)) throw new Error('Frame latency sums differ');
    frameSamples.engine.push(...a.samples); frameSamples.today.push(...b.samples);
  }
  const latency = want('frames') ? { engineUs: stats(frameSamples.engine), todayUs: stats(frameSamples.today), framesPerSide: frameSamples.engine.length } : null;
  const scalar = want('scalar') ? pairs(await fixture(module, true, true), await fixture(module, false, true), 100_000) : skipped;
  const cold = [];
  for (let pair = 0; pair < (want('cold') ? 9 : 0); pair++) {
    const a = await fixture(module, true), b = await fixture(module, false);
    const once = (f: typeof engine) => { const start = performance.now(); const sum = f.frame(.5); return { usPerFrame: (performance.now() - start) * 1000, sum }; };
    const runPair = (fn: typeof once) => {
      let engine, today;
      if (pair % 2) { today = fn(b); engine = fn(a); } else { engine = fn(a); today = fn(b); }
      if (!Object.is(engine.sum, today.sum)) throw new Error('Cold/edit sums differ');
      return { pair, engine, today };
    };
    cold.push(runPair(once));
  }
  const summarize = (rows: typeof cold) => ({ rows, engineUs: stats(rows.map(r => r.engine.usPerFrame)), todayUs: stats(rows.map(r => r.today.usPerFrame)) });
  // Each step includes the production writer and the next 400-channel frame.
  // The generated control and this harness share the same setter in workspace.ts.
  // Source and structural edits use Edit.apply's scoped history transaction.
  const edits: Record<string, { rows: unknown[]; engineUs: ReturnType<typeof stats>; todayUs: ReturnType<typeof stats>;
    effects: { structuralDiffs: number; batches: number; ops: number; interned: number }; stepsPerPair: number }> = {};
  const stepsPerPair = Number(process.env.PM_BENCH_EDIT_STEPS ?? 40);
  if (!Number.isSafeInteger(stepsPerPair) || stepsPerPair < 2) throw new Error('PM_BENCH_EDIT_STEPS must be an integer >= 2');
  for (const kind of want('edits') ? ['parameterDrag', 'sourceEdit', 'layerAdd', 'layerRemove'] as const : []) {
    const rows = [];
    for (let pair = 0; pair < 9; pair++) {
      const a = await fixture(module, true), b = await fixture(module, false);
      a.frame(.25); b.frame(.25);
      const command = (f: typeof engine, operation: 'add' | 'remove') => {
        const request = operation === 'add'
          ? { type: 'add_layer', id: 'benchmark-added', layerType: 'solid', name: 'Benchmark layer', select: false }
          : { type: 'delete_layers', targets: ['benchmark-added'] };
        const result = f.PM.Edit.apply(request, { origin: 'interface' });
        if (!result.ok) throw new Error(`Layer ${operation} failed: ${result.message}`);
      };
      const step = (f: typeof engine, index: number) => {
        if (kind === 'parameterDrag') {
          setSceneParameterValue(f.PM, f.PM.proj.params.amp, index % 2 ? 9 : 7);
        } else if (kind === 'sourceEdit') {
          const result = f.PM.Edit.apply({ type: 'set_expression', target: f.channels[0].layer.id,
            path: 'expr0', expression: index % 2 ? 'value + time * 41' : sources[0] }, { origin: 'interface' });
          if (!result.ok) throw new Error(`Source edit failed: ${result.message}`);
        } else command(f, kind === 'layerAdd' ? 'add' : 'remove');
        return f.frame((index + 1) / 30);
      };
      // Warm the writer/revalidation path as well as the untouched frame.
      // Keep its real history/provenance, identically on both sides.
      const warmEdits = (f: typeof engine) => {
        for (let index = 0; index < 40; index++) {
          if (kind === 'layerAdd' && index > 0) { command(f, 'remove'); f.frame(index / 30); }
          if (kind === 'layerRemove') { command(f, 'add'); f.frame(index / 30); }
          step(f, index);
        }
        if (kind === 'layerAdd') { command(f, 'remove'); f.frame(0); }
      };
      if (pair % 2) { warmEdits(b); warmEdits(a); } else { warmEdits(a); warmEdits(b); }
      const runSteps = (f: typeof engine) => {
        const samples = [];
        const answers = [];
        const effects = { structuralDiffs: 0, batches: 0, ops: 0, interned: 0 };
        const profile = { flushUs: 0, batchUs: 0, frameUs: 0, evaluated: 0 };
  if (doc) {
    // Counts ids evaluated by frame runs (an integer add per run, not per id).
    const prepare = doc.wasm!.prepareBatch.bind(doc.wasm!);
    doc.wasm!.prepareBatch = ids => {
      const evaluate = prepare(ids);
      return (time, start = 0, count = ids.length - start) => { profile.evaluated += count; return evaluate(time, start, count); };
    };
  }
        const plansBefore = f.doc?.wasm?.stats();
        let sum = 0;
        for (let index = 0; index < stepsPerPair; index++) {
          if (kind === 'layerAdd' && index > 0) { command(f, 'remove'); f.frame(index / 30); }
          if (kind === 'layerRemove') { command(f, 'add'); f.frame(index / 30); }
          const barrier = f.doc?.barrier?.stats, native = f.doc?.api?.stats;
          const before = { structuralDiffs: barrier?.structuralDiffs ?? 0, batches: barrier?.batches ?? 0,
            ops: barrier?.ops ?? 0, interned: native?.interned ?? 0 };
          const profilingBefore = { ...f.profile };
          const started = performance.now();
          const answer = step(f, index);
          answers.push(answer);
          sum += answer;
          samples.push((performance.now() - started) * 1000);
          for (const key of ['flushUs', 'batchUs', 'frameUs'] as const) profile[key] += f.profile[key] - profilingBefore[key];
          for (const key of ['structuralDiffs', 'batches', 'ops'] as const) effects[key] += (barrier?.[key] ?? 0) - before[key];
          effects.interned += (native?.interned ?? 0) - before.interned;
        }
        const plansAfter = f.doc?.wasm?.stats();
        const planDiagnostics = Object.fromEntries((['evalPlans', 'evalReused', 'evalMemoHits'] as const)
          .map(key => [key, (plansAfter?.[key] ?? 0) - (plansBefore?.[key] ?? 0)]));
        return { samples, answers, effects, planDiagnostics, profile, sum };
      };
      const nativeFirst = pair % 2 === 0;
      const resultA = nativeFirst ? runSteps(a) : runSteps(b);
      const resultB = nativeFirst ? runSteps(b) : runSteps(a);
      const engineResult = nativeFirst ? resultA : resultB, todayResult = nativeFirst ? resultB : resultA;
      if (!Object.is(engineResult.sum, todayResult.sum)) throw new Error(`${kind} frame sums differ at pair ${pair}`);
      for (let index = 0; index < stepsPerPair; index++) {
        if (!Object.is(engineResult.answers[index], todayResult.answers[index])) throw new Error(`${kind} frame parity at pair ${pair}, step ${index}`);
      }
      for (let i = 0; i < a.channels.length; i++) {
        const left = a.ev(a.channels[i], stepsPerPair / 30), right = b.ev(b.channels[i], stepsPerPair / 30);
        if (!Object.is(left, right)) throw new Error(`${kind} parity at channel ${i}, pair ${pair}`);
      }
      rows.push({ pair, engine: { usPerFrame: stats(engineResult.samples).median, sum: engineResult.sum },
        today: { usPerFrame: stats(todayResult.samples).median, sum: todayResult.sum },
        engineProfile: engineResult.profile, engineEffects: engineResult.effects, todayEffects: todayResult.effects,
        planDiagnostics: engineResult.planDiagnostics,
        engineSamples: engineResult.samples, todaySamples: todayResult.samples });
    }
    edits[kind] = { rows, engineUs: stats(rows.flatMap(r => r.engineSamples)), todayUs: stats(rows.flatMap(r => r.todaySamples)),
      effects: rows.reduce((total, row) => {
        for (const key of ['structuralDiffs', 'batches', 'ops', 'interned'] as const) total[key] += row.engineEffects[key];
        return total;
      }, { structuralDiffs: 0, batches: 0, ops: 0, interned: 0 }), stepsPerPair };
  }
  // Every expression style, static and keyframed bases, all channels and a
  // sequential-layers window, each paired against today exactly.
  const suiteFrames = Number(process.env.PM_BENCH_SUITE_FRAMES ?? 2000);
  const suite: Array<{ suite: string; base: string; pattern: string; engineUs: ReturnType<typeof stats>; todayUs: ReturnType<typeof stats>; evaluated?: number; pass: boolean }> = [];
  for (const [name, set] of want('suite') ? Object.entries(EXPRESSION_SUITES) : []) {
    if (ELECTRON_ONLY_SUITES.has(name)) continue;
    for (const keyed of [false, true]) for (const pattern of ['all', 'window'] as const) {
      const a = await fixture(module, true, false, { sources: set, keyed, pattern });
      const b = await fixture(module, false, false, { sources: set, keyed, pattern });
      for (const time of [0, -0, .001, .5, 1, 17 / 30, 2.5, 9.999]) for (let i = 0; i < a.channels.length; i++) {
        const x = a.ev(a.channels[i], time), y = b.ev(b.channels[i], time);
        if (!Object.is(x, y)) throw new Error(`Parity in ${name} (${keyed ? 'keyed' : 'static'}) at channel ${i}, time ${time}: ${x} vs ${y}`);
      }
      // Ids evaluated by frame runs per frame, against the channels read.
      for (let frame = 0; frame < 200; frame++) a.frame(frame / 30);
      const before = a.profile.evaluated;
      for (let frame = 200; frame < 400; frame++) a.frame(frame / 30);
      const perFrame = (a.profile.evaluated - before) / 200;
      const row = pairs(a, b, suiteFrames);
      suite.push({ suite: name, base: keyed ? 'keyed' : 'static', pattern, engineUs: row.engineUs, todayUs: row.todayUs,
        evaluated: perFrame, pass: row.engineUs.median <= row.todayUs.median && row.engineUs.p95 <= row.todayUs.p95 });
      console.error(`suite ${name} ${keyed ? 'keyed' : 'static'} ${pattern}: native ${row.engineUs.median.toFixed(2)}/${row.engineUs.p95.toFixed(2)} us, today ${row.todayUs.median.toFixed(2)}/${row.todayUs.p95.toFixed(2)} us, ${perFrame} wasm evaluations per frame`);
    }
  }
  const loadAfter = os.loadavg();
  const cold400 = summarize(cold);
  const cold400Pass = cold.length > 0 && cold400.engineUs.max <= 8000;
  const editAndFramePass = Object.values(edits).every(row => row.engineUs.median <= row.todayUs.median && row.engineUs.p95 <= row.todayUs.p95);
  const result = { runtime: process.versions, nice: os.getPriority(), loadBefore, loadAfter, wasmSha256: createHash('sha256').update(bytes).digest('hex'), channels: 400, framesPerPair: frameCount, warmupFramesPerSide: Math.min(frameCount, 1000),
    fixture: sources, frames, latency, scalar, cold400: { ...cold400, pass: cold400Pass }, editWarmupStepsPerSide: 40, edits, suite, native: engine.doc!.stats(),
    targets: { cold400MaxUs: 8000, editAndFrame: 'native median and p95 <= legacy per edit kind', suite: 'native median and p95 <= legacy per suite, base and read pattern' },
    authority: 'Node 24 V8 is not the shipping Electron 44 V8 (Math.sin differs in the last bit for some inputs); Electron measurements are authoritative',
    profiled: PROFILE, sections: [...sections],
    pass: !PROFILE && !partial && frames.engineUs.median <= frames.todayUs.median && cold400Pass && editAndFramePass && suite.every(row => row.pass) && loadAfter[0]! <= 8 };
  if (process.env.PM_BENCH_OUT) writeFileSync(process.env.PM_BENCH_OUT, JSON.stringify(result, null, 2) + '\n');
  console.log(JSON.stringify(result, null, 2));
  if (!result.pass) process.exitCode = 1;
}
