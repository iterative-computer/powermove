import { requireEngineDiskSpace } from './helpers/engine-fixups-resources';
import { mkdir, writeFile } from 'node:fs/promises';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import os from 'node:os';
import path from 'node:path';
import { expect, launchApp, repoRoot, test } from './helpers/app';

// Run under pm-electron-lock on a quiet machine. Correctness has separate
// committed vector and dual/audit gates; timings never relax their results.
test('pairs hundreds of frame expressions against today in the same V8', async () => {
  test.skip(!process.env.PM_FIXUPS3_MEASURE, 'opt-in paired measurement');
  test.setTimeout(900_000);
  const deadline = Date.now() + 30_000;
  while (os.loadavg()[0]! > 8 && Date.now() < deadline) await new Promise(resolve => setTimeout(resolve, 5_000));
  expect(os.loadavg()[0], 'machine must be quiet before measuring').toBeLessThanOrEqual(8);
  requireEngineDiskSpace();
  const app = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await app.page.waitForFunction(() => (window as any).PM?.EngineDoc?.ready);
    const result = await app.page.evaluate(() => {
      const PM = (window as any).PM, doc = PM.EngineDoc, api = doc.api;
      const original = PM.proj, scope = PM.scope;
      const sources = ['value + time * 40', 'param("amp") + comp.layers.length + time', 'layer.p.opacity.v + t * 13', 'Math.sin(time) * 10 + idx + value'];
      PM.scope = [];
      PM.proj = PM.mkProject({ dur: 100 });
      PM.proj.params = { amp: { value: 7 } };
      PM.proj.layers = Array.from({ length: 100 }, (_, i) => {
        const layer = PM.mkLayer('solid');
        for (let k = 0; k < 4; k++) layer.p[`expr${k}`] = { v: i + k, kf: [], expr: sources[k] };
        return layer;
      });
      PM.touch();
      const channels = PM.proj.layers.flatMap((layer: any) => sources.map((_, k) => ({ layer, prop: layer.p[`expr${k}`], key: `expr${k}` })));
      const native = (c: any, t: number) => api.evP(c.layer, c.prop, t, c.key);
      const today = (c: any, t: number) => api.legacy.evP(c.layer, c.prop, t, c.key);
      const measure = (fn: typeof native, count: number, single = false) => {
        let sum = 0;
        const started = performance.now();
        for (let frame = 0; frame < count; frame++) {
          const time = (frame % 1024) / 30;
          if (single) sum += fn(channels[0], time);
          else for (const c of channels) sum += fn(c, time);
        }
        const ms = performance.now() - started;
        return { nsPerCall: ms * 1e6 / count / (single ? 1 : channels.length), usPerFrame: ms * 1000 / count, sum };
      };
      try {
        for (const time of [0, -0, .001, .5, 1, 17 / 30, 9.999]) for (const c of channels) {
          const a = native(c, time), b = today(c, time);
          if (!Object.is(a, b)) throw new Error(`expression parity ${c.key} at ${time}: ${a} vs ${b}`);
        }
        measure(native, 400); measure(today, 400);
        const rows = [];
        for (let pair = 0; pair < 5; pair++) {
          let engine, legacy;
          if (pair % 2) { legacy = measure(today, 2000); engine = measure(native, 2000); }
          else { engine = measure(native, 2000); legacy = measure(today, 2000); }
          if (!Object.is(engine.sum, legacy.sum)) throw new Error('paired frame sums differ');
          rows.push({ pair, engine, today: legacy });
        }
        const ids = channels.map((c: any) => api.props.get(c.prop).id);
        const diagnostic = [];
        for (let source = 0; source < 4; source++) {
          const selected = ids.filter((_: number, i: number) => i % 4 === source);
          const start = performance.now();
          for (let frame = 0; frame < 2000; frame++) doc.wasm.evBatch(selected, frame / 30);
          diagnostic.push({ source: sources[source], wasmNsPerCall: (performance.now() - start) * 1e6 / 2000 / selected.length });
        }
        // Measure a separate one-channel fixture for extension scalar calls.
        PM.proj.layers = [PM.proj.layers[0]];
        for (let k = 1; k < 4; k++) delete PM.proj.layers[0].p[`expr${k}`];
        PM.touch(); api.reset(); native(channels[0], 0);
        const scalar = [];
        for (let pair = 0; pair < 5; pair++) {
          let engine, legacy;
          if (pair % 2) { legacy = measure(today, 200_000, true); engine = measure(native, 200_000, true); }
          else { engine = measure(native, 200_000, true); legacy = measure(today, 200_000, true); }
          if (!Object.is(engine.sum, legacy.sum)) throw new Error('paired scalar sums differ');
          scalar.push({ pair, engine, today: legacy });
        }
        return { channels: channels.length, framesPerPair: 2000, rows, scalar, diagnostic, stats: doc.stats(), heap: doc.wasm.heap() };
      } finally { PM.proj = original; PM.scope = scope; PM.touch(); }
    });
    const loadAfter = os.loadavg();
    const moduleSha256 = createHash("sha256").update(readFileSync(path.join(repoRoot, "out/renderer/engine/pm_wasm.wasm"))).digest("hex");
    const output = { ...result, loadAfter, moduleSha256 };
    if (process.env.PM_FIXUPS3_EVIDENCE) {
      await mkdir(process.env.PM_FIXUPS3_EVIDENCE, { recursive: true });
      await writeFile(path.join(process.env.PM_FIXUPS3_EVIDENCE, 'expressions.json'), JSON.stringify(output, null, 2));
    }
    console.log(JSON.stringify(output));
    const median = (xs: number[]) => xs.sort((a, b) => a - b)[Math.floor(xs.length / 2)]!;
    expect(loadAfter[0], 'machine stayed quiet').toBeLessThanOrEqual(8);
    expect(median(result.rows.map(r => r.engine.usPerFrame))).toBeLessThanOrEqual(median(result.rows.map(r => r.today.usPerFrame)));
  } finally { await app.close(); }
});
