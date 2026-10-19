import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { expect, it, vi } from 'vitest';
import { makePM } from '../legacy/__tests__/make-pm';
import { installEngineDoc } from './install';
import { parseMode } from './flags';

const loadModule = () => WebAssembly.compile(readFileSync(fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url))));

it('batches resident expression channels exactly, across time, edits, errors and module reset', async () => {
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
  const batch = vi.spyOn(doc.wasm!, 'evBatch');
  const error = vi.spyOn(doc.wasm!, 'expressionError');
  const compare = (time: number) => {
    for (const L of PM.proj.layers) expect(Object.is(api.evP(L, L.p.opacity, time, 'opacity'), PM.evP(L, L.p.opacity, time, 'opacity'))).toBe(true);
  };
  compare(1); // Learn the live addresses.
  for (const time of [2, 3, 0, -0, 1.5]) {
    batch.mockClear(); error.mockClear();
    compare(time);
    expect(batch).toHaveBeenCalledTimes(1);
    expect(batch.mock.calls[0]![0].length).toBe(120);
    expect(error.mock.calls.length).toBe(time === 0 ? 60 : 0);
    for (let i = 0; i < 120; i++) expect(api.errors.has(PM.proj.layers[i].p.opacity)).toBe(time === 0 && i % 2 === 0);
    compare(time);
    expect(batch).toHaveBeenCalledTimes(1);
  }
  PM.proj.params.amp.value = 9;
  PM.touch();
  compare(1.5); // Same time, different revision.
  PM.proj.layers[0].p.opacity.expr = '"changed"';
  PM.touch();
  compare(1.5); compare(2.5); // Nonnumeric answers retain their exact scalar decoding.
  api.reset();
  compare(2.5); compare(3.5);
  doc.wasm!.x.memory.grow(1);
  compare(4.5); // ABI views are reacquired after memory growth.
  expect((doc.stats() as any).native.fallbacks).toEqual({});
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
