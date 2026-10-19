/* Opt in with PM_FIXUPS2_MEASURE=1. Actual facade vs legacy evaluator, same V8. */
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { expect, it } from 'vitest';
import { makePM } from '../legacy/__tests__/make-pm';
import { installEngineDoc } from './install';
import { parseMode } from './flags';

it.runIf(!!process.env.PM_FIXUPS2_MEASURE)('measures root wrappers and genuinely detached scopes', async () => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history');
  const module = await WebAssembly.compile(readFileSync(fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url))));
  const doc = installEngineDoc(PM, { mode: parseMode('on'), strict: true, loadModule: async () => module, sink: () => {} });
  PM.proj = PM.mkProject({ dur: 10 });
  PM.proj.layers = Array.from({ length: 100 }, (_, i) => {
    const L = PM.mkLayer('solid', { name: `L${i}` });
    for (let k = 0; k < 40; k++) L.p[`extra${k}`] = PM.P(k, { kf: Array.from({ length: 5 }, (_, t) => PM.KF(t, t)) });
    return L;
  });
  PM.touch();
  expect(await doc.whenReady).toBe(true);
  const rows = [];
  for (const detached of [false, true]) {
    const comp = detached ? structuredClone({ id: 'detached', fps: PM.proj.fps, layers: PM.proj.layers }) : null;
    PM.scope = [{}];
    PM.curComp = () => comp ?? { id: PM.proj.compId, fps: PM.proj.fps, layers: PM.proj.layers };
    const L = PM.curComp().layers[0];
    const prop = L.p.extra0;
    const count = detached ? 24 : 1_000_000;
    const measure = (fn: () => number, count: number, warm: number) => {
      for (let i = 0; i < warm; i++) fn();
      const start = performance.now();
      let sum = 0;
      for (let i = 0; i < count; i++) sum += fn();
      expect(sum).toBe(count * 1.5);
      return (performance.now() - start) * 1e6 / count;
    };
    // Alternate order over five samples to avoid giving one path only warm runs.
    const native: number[] = [], legacy: number[] = [];
    for (let n = 0; n < 5; n++) {
      const a = () => native.push(measure(() => doc.api!.evP(L, prop, 1.5, 'extra0'), count, detached ? 3 : 100_000));
      const b = () => legacy.push(measure(() => PM.evP(L, prop, 1.5, 'extra0'), 1_000_000, 100_000));
      if (n % 2) { b(); a(); } else { a(); b(); }
    }
    rows.push({ scope: detached ? 'detached' : 'fresh-root-wrapper', nativeNs: native, legacyNs: legacy, stats: doc.wasm!.stats() });
  }
  console.log(JSON.stringify(rows));
  if (process.env.PM_FIXUPS2_EVIDENCE) writeFileSync(process.env.PM_FIXUPS2_EVIDENCE + '/scope.json', JSON.stringify(rows, null, 2));
}, 120_000);
