import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { expect, it, vi } from 'vitest';
import { makePM } from '../legacy/__tests__/make-pm';
import { setSceneParameterValue } from '../legacy/core/workspace';
import { installEngineDoc } from './install';
import { parseMode } from './flags';

const loadModule = () => WebAssembly.compile(readFileSync(fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url))));

/** Records every batch run: [resident ids, time, start, count]. */
function spyRuns(wasm: any) {
  const runs: Array<[number, number, number, number]> = [];
  const prepared: number[] = [];
  const prepare = wasm.prepareBatch.bind(wasm);
  const spy = vi.spyOn(wasm, 'prepareBatch').mockImplementation((ids: any) => {
    const evaluate = prepare(ids);
    prepared.push(ids.length);
    return (time: number, start = 0, count = ids.length - start) => { runs.push([ids.length, time, start, count]); return evaluate(time, start, count); };
  });
  const evaluated = () => runs.reduce((n, run) => n + run[3], 0);
  return { runs, prepared, evaluated, spy };
}

it('evaluates resident expression channels in runs, exactly, across time, edits, errors and module reset', async () => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history');
  const doc = installEngineDoc(PM, { mode: parseMode('on'), strict: true, loadModule, sink: () => {} });
  PM.proj = PM.mkProject({ dur: 10 });
  PM.proj.layers = Array.from({ length: 120 }, (_, i) => {
    const L = PM.mkLayer('solid');
    L.p.opacity.expr = i % 2 ? 'value + time * 40' : 'time ? param("amp") + layer.p.opacity.v : 0 / 0';
    return L;
  });
  PM.proj.params = { amp: { value: 7 } };
  PM.touch();
  expect(await doc.whenReady).toBe(true);
  const api = doc.api!;
  const { runs, prepared, evaluated } = spyRuns(doc.wasm!);
  const error = vi.spyOn(doc.wasm!, 'expressionError');
  const compare = (time: number) => {
    for (const L of PM.proj.layers) expect(Object.is(api.evP(L, L.p.opacity, time, 'opacity'), PM.evP(L, L.p.opacity, time, 'opacity'))).toBe(true);
  };
  compare(1); // Learn the live addresses.
  compare(1.25); // The resident ids join the members in read order.
  for (const time of [2, 3, 0, -0, 1.5]) {
    runs.length = 0; error.mockClear();
    compare(time);
    // In-order reads: runs of 4, 8, 16, … cover the 120 members exactly once.
    expect(runs.map(r => r[3])).toEqual([4, 8, 16, 32, 60]);
    expect(evaluated()).toBe(120);
    expect(error.mock.calls.length).toBe(time === 0 ? 60 : 0);
    for (let i = 0; i < 120; i++) expect(api.errors.has(PM.proj.layers[i].p.opacity)).toBe(time === 0 && i % 2 === 0);
    compare(time);
    expect(evaluated()).toBe(120);
  }
  PM.proj.params.amp.value = 9;
  PM.touch();
  compare(1.5); // Same time, different revision.
  PM.proj.layers[0].p.opacity.expr = '"changed"';
  PM.touch();
  compare(1.5); compare(2.5); // Nonnumeric answers retain their exact scalar decoding.
  delete PM.proj.layers[0].p.opacity.expr;
  PM.touch();
  compare(2.6); compare(3.6); // Removing one id updates all live member positions.
  expect(prepared.at(-1)).toBe(119);
  PM.proj.layers[0].p.opacity.expr = 'value + time';
  PM.touch([['layers', 0, 'p', 'opacity', 'expr']]);
  compare(2.7); compare(3.7); // Reusing its ID restores the expression slot.
  expect(prepared.at(-1)).toBe(120);
  api.reset();
  compare(2.5); compare(3.5); compare(4);
  doc.wasm!.x.memory.grow(1);
  compare(4.5); // ABI views are reacquired after memory growth.
  expect((doc.stats() as any).native.fallbacks).toEqual({});
});

it('pays only for the channels a frame reads: sequential layers, deleted layers, motion-blur samples', async () => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history');
  const doc = installEngineDoc(PM, { mode: parseMode('on'), strict: true, loadModule, sink: () => {} });
  PM.proj = PM.mkProject({ dur: 100 });
  PM.proj.params = { amp: { value: 7 } };
  const sources = ['clamp(value + time * 3, 0, 50)', 'time < 2 ? param("amp") : value', 'value + (idx % 2 ? time : -time)', 'pingpong(time, 3) * value'];
  PM.proj.layers = Array.from({ length: 100 }, (_, i) => {
    const L = PM.mkLayer('solid');
    sources.forEach((expr, k) => { L.p[`e${k}`] = { v: i + k, kf: [], expr }; });
    return L;
  });
  PM.touch();
  expect(await doc.whenReady).toBe(true);
  const api = doc.api!;
  const { runs, evaluated } = spyRuns(doc.wasm!);
  const read = (layers: any[], time: number) => {
    let reads = 0;
    for (const L of layers) for (let k = 0; k < 4; k++) {
      reads += 1;
      expect(Object.is(api.evP(L, L.p[`e${k}`], time, `e${k}`), PM.evP(L, L.p[`e${k}`], time, `e${k}`))).toBe(true);
    }
    return reads;
  };
  read(PM.proj.layers, 0.5); read(PM.proj.layers, 0.75); // Every channel resident.
  // A window of 10 sequential layers per frame, as when most are inactive.
  let reads = 0;
  runs.length = 0;
  for (let frame = 0; frame < 120; frame++) {
    const first = Math.floor(frame / 4) % 91;
    reads += read(PM.proj.layers.slice(first, first + 10), 1 + frame / 30);
  }
  expect(evaluated()).toBeLessThanOrEqual(reads * 1.25);
  // A deleted layer (history still holds it) leaves the members once no frame reads it.
  const removed = PM.proj.layers.splice(0, 50);
  PM.touch();
  for (let frame = 0; frame < 140; frame++) read(PM.proj.layers, 6 + frame / 30);
  runs.length = 0;
  reads = read(PM.proj.layers, 20);
  expect(evaluated()).toBe(reads);
  expect(removed.length).toBe(50);
  // Motion blur: each layer is also sampled at ten nearby times between frames.
  runs.length = 0; reads = 0;
  for (let frame = 0; frame < 6; frame++) {
    const T = 30 + frame / 30;
    for (const L of PM.proj.layers.slice(0, 20)) {
      reads += read([L], T);
      for (let s = 0; s < 10; s++) reads += read([L], T + (s - 4.5) / 600);
    }
  }
  expect(evaluated()).toBeLessThanOrEqual(reads * 2);
  expect((doc.stats() as any).native.fallbacks).toEqual({});
});

it('retains live addresses across leaf revisions and invalidates them on structural changes', async () => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history');
  PM.proj = PM.mkProject({ dur: 10 });
  PM.proj.params = { amp: { value: 7 } };
  PM.proj.layers = Array.from({ length: 24 }, () => {
    const layer = PM.mkLayer('solid');
    layer.p.opacity.expr = 'param("amp") + time + value';
    return layer;
  });
  const doc = installEngineDoc(PM, { mode: parseMode('on'), strict: true, loadModule, sink: () => {} });
  expect(await doc.whenReady).toBe(true);
  const api = doc.api!, wasm = doc.wasm!;
  const intern = vi.spyOn(wasm, 'intern');
  const { runs, evaluated, spy: batches } = spyRuns(wasm);
  const compare = (time: number) => {
    for (const layer of PM.proj.layers.slice(0, 24)) {
      expect(Object.is(api.evP(layer, layer.p.opacity, time, 'opacity'), PM.evP(layer, layer.p.opacity, time, 'opacity'))).toBe(true);
    }
  };
  compare(1); compare(1.5);
  const initialInterns = intern.mock.calls.length;
  expect(initialInterns).toBe(24);
  runs.length = 0;
  const structuralDiffs = doc.barrier!.stats.structuralDiffs;
  setSceneParameterValue(PM, PM.proj.params.amp, 9, 'amp'); // Existing projects may lack param.name.
  compare(1);
  expect(doc.barrier!.stats.structuralDiffs).toBe(structuralDiffs);
  expect(intern).toHaveBeenCalledTimes(initialInterns);
  expect(evaluated()).toBe(24);
  runs.length = 0;
  const first = PM.proj.layers[0];
  PM.doc.write([['layers', 0, 'p', 'opacity', 'expr']], () => { first.p.opacity.expr = 'param("amp") + time * 41 + value'; });
  compare(2);
  expect(intern).toHaveBeenCalledTimes(initialInterns);
  expect(evaluated()).toBe(24);
  expect(runs.every(r => r[0] === 24 && r[1] === 2)).toBe(true);
  const stableInterns = intern.mock.calls.length;
  const extra = PM.mkLayer('solid');
  PM.doc.write([['layers']], () => { PM.proj.layers.push(extra); });
  compare(2.5);
  expect(intern).toHaveBeenCalledTimes(stableInterns);
  PM.doc.write([['layers']], () => { PM.proj.layers.pop(); });
  compare(2.75);
  expect(intern).toHaveBeenCalledTimes(stableInterns);
  // A held channel that is no longer at its path must never inherit the
  // address of the replacement channel.
  const held = first.p.opacity;
  PM.doc.write([['layers', 0, 'p', 'opacity']], () => {
    first.p.opacity = { ...held, expr: 'time + 100' };
  });
  expect(Object.is(api.evP(first, first.p.opacity, 3, 'opacity'), PM.evP(first, first.p.opacity, 3, 'opacity'))).toBe(true);
  expect(intern.mock.calls.length).toBeGreaterThan(initialInterns);
  // Shape changes and duplicate ids require the full address check again.
  const duplicate = PM.mkLayer('solid');
  duplicate.id = first.id;
  duplicate.p.opacity.expr = 'time + 200';
  PM.doc.write([['layers']], () => { PM.proj.layers.unshift(duplicate); });
  expect(Object.is(api.evP(first, first.p.opacity, 4, 'opacity'), PM.evP(first, first.p.opacity, 4, 'opacity'))).toBe(true);
  expect(Object.is(api.evP(duplicate, duplicate.p.opacity, 4, 'opacity'), PM.evP(duplicate, duplicate.p.opacity, 4, 'opacity'))).toBe(true);
  expect(doc.check('leaf-rebind-test')?.ok).toBe(true);
  batches.mockRestore();
});

it('preserves independent frame buffers when callers alternate composition scopes', async () => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history');
  const doc = installEngineDoc(PM, { mode: parseMode('on'), strict: true, loadModule, sink: () => {} });
  PM.proj = PM.mkProject({ dur: 10 });
  const a = PM.mkLayer('solid'), b = PM.mkLayer('solid');
  a.p.opacity.expr = 'time + 2'; b.p.opacity.expr = 'time + 7';
  PM.proj.layers = [a];
  PM.proj.comps = { other: { id: 'other', fps: 30, layers: [b] } };
  PM.touch();
  expect(await doc.whenReady).toBe(true);
  for (const t of [0, 1, 2, 1, -0]) {
    for (const [L, scope] of [[a, PM.proj], [b, PM.proj.comps.other], [a, PM.proj]] as const) {
      PM.scope = [scope]; PM.curComp = () => scope;
      expect(doc.api!.evP(L, L.p.opacity, t, 'opacity')).toBe(PM.evP(L, L.p.opacity, t, 'opacity'));
    }
  }
});

it('restores prepared batch ids after a larger batch and after ABI memory growth', async () => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim');
  PM.proj = PM.mkProject({ dur: 10 });
  const layer = PM.mkLayer('solid');
  layer.p.opacity.expr = 'time + value';
  layer.p.skew.expr = 'time * 2 + value';
  PM.proj.layers = [layer];
  const doc = installEngineDoc(PM, { mode: parseMode('on'), strict: true, loadModule, sink: () => {} });
  expect(await doc.whenReady).toBe(true);
  const wasm = doc.wasm!;
  const id = (key: string) => wasm.intern({ comp: `@root:${PM.proj.compId}`, layer: layer.id, key, via: 'channel' });
  const opacity = id('opacity'), skew = id('skew');
  const a = wasm.prepareBatch([opacity]), b = wasm.prepareBatch([skew, opacity]);
  const expected = (key: string, t: number) => PM.evP(layer, layer.p[key], t, key);
  for (const t of [0, 1, -0, 3]) {
    expect([...a(t).values]).toEqual([expected('opacity', t)]);
    expect([...b(t).values]).toEqual([expected('skew', t), expected('opacity', t)]);
    wasm.evBatch([skew], t); // The generic ABI user overwrites its shared ids.
    wasm.x.memory.grow(1);
    expect([...a(t).values]).toEqual([expected('opacity', t)]);
  }
});
