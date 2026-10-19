/* Opt in with PM_FIXUPS2_MEASURE=1. Actual facade vs legacy evaluator, same V8. */
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { expect, it } from 'vitest';
import { makePM } from '../legacy/__tests__/make-pm';
import { rootComp } from '../legacy/core/compositions';
import { installEngineDoc } from './install';
import { parseMode } from './flags';

it.runIf(!!process.env.PM_FIXUPS2_MEASURE)('measures root wrappers and genuinely detached scopes', async () => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history');
  const module = await WebAssembly.compile(readFileSync(fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url))));
  const doc = installEngineDoc(PM, { mode: parseMode('on'), strict: true, loadModule: async () => module, sink: () => {} });
  PM.proj = PM.mkProject({ dur: 10 });
  PM.proj.params = { amp: { value: 7 } };
  PM.proj.layers = Array.from({ length: 100 }, (_, i) => {
    const L = PM.mkLayer('solid', { name: `L${i}` });
    for (let k = 0; k < 40; k++) L.p[`extra${k}`] = PM.P(k, { kf: Array.from({ length: 5 }, (_, t) => PM.KF(t, t)) });
    // An expression that reads the scope object (`comp`) and `param()`.
    L.p.expr0 = PM.P(0);
    L.p.expr0.expr = 'param("amp") + (comp.params ? 1000 : 0) + comp.layers.length + time';
    return L;
  });
  PM.touch();
  expect(await doc.whenReady).toBe(true);
  const L = PM.proj.layers[0];
  const keyed = L.p.extra0, expr = L.p.expr0;
  const held = rootComp(PM.proj);
  const detached = structuredClone({ id: 'detached', fps: PM.proj.fps, layers: PM.proj.layers });
  const cases: Array<{ scope: string; curComp: () => any; layer: () => any; prop: (L: any) => any; key: string; count: number }> = [
    // PM.Comps.get(openId): a new rootComp(PM.proj) on every call.
    { scope: 'fresh-root-wrapper keyed', curComp: () => rootComp(PM.proj), layer: () => L, prop: l => l.p.extra0, key: 'extra0', count: 1_000_000 },
    { scope: 'held-root-wrapper keyed', curComp: () => held, layer: () => L, prop: l => l.p.extra0, key: 'extra0', count: 1_000_000 },
    { scope: 'held-root-wrapper expression', curComp: () => held, layer: () => L, prop: l => l.p.expr0, key: 'expr0', count: 200_000 },
    { scope: 'no scope expression', curComp: () => PM.proj, layer: () => L, prop: l => l.p.expr0, key: 'expr0', count: 200_000 },
    { scope: 'fresh-root-wrapper expression', curComp: () => rootComp(PM.proj), layer: () => L, prop: l => l.p.expr0, key: 'expr0', count: 200_000 },
    // Over the open layers but not a pristine rootComp: a keyed channel cannot read it.
    { scope: 'fresh-partial-wrapper keyed', curComp: () => ({ id: PM.proj.compId, fps: PM.proj.fps, layers: PM.proj.layers }), layer: () => L, prop: l => l.p.extra0, key: 'extra0', count: 1_000_000 },
    { scope: 'detached keyed', curComp: () => detached, layer: () => detached.layers[0], prop: l => l.p.extra0, key: 'extra0', count: 10_000 },
    { scope: 'detached expression', curComp: () => detached, layer: () => detached.layers[0], prop: l => l.p.expr0, key: 'expr0', count: 2_000 },
    // A detached composition object made anew on every PM.curComp() call (same layers).
    { scope: 'fresh detached object keyed', curComp: () => ({ ...detached }), layer: () => detached.layers[0], prop: l => l.p.extra0, key: 'extra0', count: 200 },
    { scope: 'fresh detached object expression', curComp: () => ({ ...detached }), layer: () => detached.layers[0], prop: l => l.p.expr0, key: 'expr0', count: 200 },
  ];
  const rows = [];
  for (const c of cases.filter(c => !process.env.PM_FIXUPS2_CASE || c.scope.includes(process.env.PM_FIXUPS2_CASE))) {
    PM.scope = [{}];
    PM.curComp = c.curComp;
    const layer = c.layer(), prop = c.prop(layer);
    const expected = PM.evP(layer, prop, 1.5, c.key);
    expect(doc.api!.evP(layer, prop, 1.5, c.key)).toBe(expected);
    const measure = (fn: () => number, count: number, warm: number) => {
      for (let i = 0; i < warm; i++) fn();
      const start = performance.now();
      let sum = 0;
      for (let i = 0; i < count; i++) sum += fn();
      expect(sum).toBe(count * expected);
      return (performance.now() - start) * 1e6 / count;
    };
    // Alternate order over five samples to avoid giving one path only warm runs.
    const native: number[] = [], legacy: number[] = [];
    const warm = Math.min(100_000, c.count / 10);
    for (let n = 0; n < 5; n++) {
      const a = () => native.push(measure(() => doc.api!.evP(layer, prop, 1.5, c.key), c.count, warm));
      const b = () => legacy.push(measure(() => PM.evP(layer, prop, 1.5, c.key), Math.max(c.count, 200_000), 100_000));
      if (n % 2) { b(); a(); } else { a(); b(); }
    }
    const median = (xs: number[]) => [...xs].sort((x, y) => x - y)[2]!;
    rows.push({ scope: c.scope, nativeMedianNs: median(native), legacyMedianNs: median(legacy), nativeNs: native, legacyNs: legacy });
  }
  // An expression at a new time on every call (no memo on either side).
  for (const [scope, curComp] of [['held-root-wrapper expression, varying time', () => held], ['no scope expression, varying time', () => PM.proj]] as const) {
    PM.scope = [{}];
    PM.curComp = curComp;
    const times = Array.from({ length: 1024 }, (_, i) => i / 128);
    const run = (fn: (t: number) => number, count: number) => {
      const start = performance.now();
      let sum = 0;
      for (let i = 0; i < count; i++) sum += fn(times[i & 1023]!);
      return { ns: (performance.now() - start) * 1e6 / count, sum };
    };
    const native: number[] = [], legacy: number[] = [];
    for (let n = 0; n < 5; n++) {
      const a = () => { const r = run(t => doc.api!.evP(L, expr, t, 'expr0'), 200_000); native.push(r.ns); return r.sum; };
      const b = () => { const r = run(t => PM.evP(L, expr, t, 'expr0'), 200_000); legacy.push(r.ns); return r.sum; };
      const [x, y] = n % 2 ? [b(), a()] : [a(), b()];
      expect(x).toBe(y);
    }
    const median = (xs: number[]) => [...xs].sort((x, y) => x - y)[2]!;
    rows.push({ scope, nativeMedianNs: median(native), legacyMedianNs: median(legacy), nativeNs: native, legacyNs: legacy });
  }
  rows.push({ stats: doc.wasm!.stats(), fallbacks: (doc.stats() as any).native.fallbacks });
  console.log(JSON.stringify(rows));
  if (process.env.PM_FIXUPS2_EVIDENCE) writeFileSync(process.env.PM_FIXUPS2_EVIDENCE + '/scope.json', JSON.stringify(rows, null, 2));
}, 300_000);
