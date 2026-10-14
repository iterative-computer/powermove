/*
 * S5's measurements through the real write path in Node (V8), on S5's
 * 884-layer synthetic project: the identity registry's evP, property and
 * structural edits through history and the barrier into pm-wasm, and memory.
 * Opt in: PM_WASM_BENCH=1 vitest run src/renderer/src/engine-doc/bench.test.ts
 * (the in-app numbers come from scripts/pm-wasm-bench.mjs).
 */
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { expect, it } from 'vitest';

import { makePM } from '../legacy/__tests__/make-pm';
import { parseMode } from './flags';
import { installEngineDoc } from './install';
// @ts-expect-error: the spike's generator is plain JavaScript.
import { project884 } from '../../../../../../engine/spikes/s5-wasm/js/synth.mjs';

const MODULE = fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url));

it.skipIf(!process.env.PM_WASM_BENCH || !existsSync(MODULE))('S5 targets through the write path (Node)', async () => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  const doc = installEngineDoc(PM, { mode: parseMode('on'), loadModule: () => WebAssembly.compile(readFileSync(MODULE)), sink: () => {} });
  const synthetic = project884();
  PM.proj = { ...synthetic, id: 'P-bench', compId: 'C-bench', compName: 'Bench', comps: {}, assets: {}, markers: [], work: [0, 60] };
  expect(await doc.whenReady).toBe(true);
  const { barrier, wasm, api } = doc as any;
  const quantile = (xs: number[], q: number) => [...xs].sort((a, b) => a - b)[Math.min(xs.length - 1, Math.floor(q * xs.length))]!;
  const stats = (xs: number[]) => ({ p50: quantile(xs, 0.5), p95: quantile(xs, 0.95), mean: xs.reduce((a, b) => a + b, 0) / xs.length });
  const applies: number[] = [];
  const apply = wasm.apply.bind(wasm);
  wasm.apply = (body: Uint8Array, base: number, rev: number) => { const t0 = performance.now(); const r = apply(body, base, rev); applies.push(performance.now() - t0); return r; };
  const flushes: number[] = [];
  const flush = () => { const t0 = performance.now(); barrier.flush(); flushes.push(performance.now() - t0); };

  const t0 = performance.now(); barrier.resync(); const loadMs = performance.now() - t0;
  const layers = PM.proj.layers;
  const channels: Array<[any, any, string]> = [];
  for (const L of layers) for (const key of Object.keys(L.p || {})) channels.push([L, L.p[key], key]);
  const animated = channels.filter(([, p]) => p.kf?.length);
  const timeEvP = (list: typeof channels, passes: number) => {
    for (const [L, p, k] of list) api.evP(L, p, 1.5, k);
    const t = performance.now();
    for (let i = 0; i < passes; i++) for (const [L, p, k] of list) api.evP(L, p, 1.5, k);
    return (performance.now() - t) * 1e6 / (passes * list.length);
  };
  const evPAllNs = timeEvP(channels, 20), evPAnimatedNs = timeEvP(animated, 200);

  applies.length = 0; flushes.length = 0;
  const target = layers[442];
  for (let i = 0; i < 300; i++) { PM.Edit.apply({ type: 'set_property', target: target.id, path: 'position.x', value: 100 + i }); flush(); }
  const property = { apply: stats(applies), flush: stats(flushes) };
  const structural: Record<string, unknown> = {};
  for (const kind of ['reorder', 'insert', 'delete']) {
    applies.length = 0; flushes.length = 0;
    for (let i = 0; i < 60; i++) {
      PM.hist.do(kind, () => {
        const list = PM.proj.layers;
        if (kind === 'reorder') { const [moved] = list.splice(7, 1); list.splice(400, 0, moved); }
        else if (kind === 'insert') PM.addLayer(PM.mkLayer('solid', { name: `Bench ${i}` }), 300);
        else { const at = list.findIndex((l: any) => l.name?.startsWith('Bench ')); if (at >= 0) list.splice(at, 1); }
      });
      flush();
    }
    structural[kind] = { apply: stats(applies), flush: stats(flushes) };
  }
  // A structural diff that finds nothing (what a touch() without paths, or a
  // query inside a plain transaction, costs on this project).
  flushes.length = 0;
  for (let i = 0; i < 50; i++) { barrier.touchAll(); flush(); }
  const emptyStructuralDiff = stats(flushes);
  // evP inside a plain transaction (one structural diff per synchronous run).
  PM.hist.begin('Bench');
  const inTransactionNs = timeEvP(animated, 20);
  PM.hist.cancel();
  const check = doc.check('bench')!;
  const heap = wasm.heap();
  console.log(JSON.stringify({
    layers: layers.length, channels: channels.length, animated: animated.length, loadMs,
    evP: { allCachedNs: evPAllNs, animatedCachedNs: evPAnimatedNs, inPlainTransactionNs: inTransactionNs }, property, structural, emptyStructuralDiff,
    consistency: { ok: check.ok, ms: check.ms }, memory: { heapLiveMB: heap.live / 1e6, heapPeakMB: heap.peak / 1e6, linearMB: heap.memory / 1e6 },
  }, null, 2));
  expect(check.ok).toBe(true);
}, 120_000);
