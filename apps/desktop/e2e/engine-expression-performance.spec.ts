import { requireEngineDiskSpace } from './helpers/engine-fixups-resources';
import { mkdir, writeFile } from 'node:fs/promises';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import os from 'node:os';
import path from 'node:path';
import { expect, launchApp, repoRoot, test } from './helpers/app';
import { EXPRESSION_SUITES } from './helpers/expression-suites';

// Run under pm-electron-lock on a quiet machine. Correctness has separate
// committed vector and dual/audit gates; timings never relax their results.
test('pairs every expression style and read pattern against today in the same V8', async () => {
  test.skip(!process.env.PM_FIXUPS3_MEASURE, 'opt-in paired measurement');
  test.setTimeout(900_000);
  const deadline = Date.now() + 30_000;
  while (os.loadavg()[0]! > 8 && Date.now() < deadline) await new Promise(resolve => setTimeout(resolve, 5_000));
  expect(os.loadavg()[0], 'machine must be quiet before measuring').toBeLessThanOrEqual(8);
  requireEngineDiskSpace();
  const app = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await app.page.waitForFunction(() => (window as any).PM?.EngineDoc?.ready);
    const result = await app.page.evaluate((suites: Record<string, string[]>) => {
      const PM = (window as any).PM, doc = PM.EngineDoc, api = doc.api;
      const original = PM.proj, scope = PM.scope;
      const native = (c: any, t: number) => api.evP(c.layer, c.prop, t, c.key);
      const today = (c: any, t: number) => api.legacy.evP(c.layer, c.prop, t, c.key);
      const fixture = (sources: string[], keyed: boolean) => {
        PM.scope = [];
        PM.proj = PM.mkProject({ dur: 100 });
        PM.proj.params = { amp: { value: 7 } };
        PM.proj.layers = Array.from({ length: 100 }, (_, i) => {
          const layer = PM.mkLayer('solid');
          for (let k = 0; k < 4; k++) {
            const kf = keyed ? [{ t: 0, v: i + k }, { t: 3, v: i + k + 30 }] : [];
            layer.p[`expr${k}`] = { v: i + k, kf, expr: sources[k] };
          }
          return layer;
        });
        PM.touch(); api.reset();
        return PM.proj.layers.flatMap((layer: any) => sources.map((_, k) => ({ layer, prop: layer.p[`expr${k}`], key: `expr${k}` })));
      };
      // The same read patterns as e2e/helpers/expression-suites.ts.
      const range = (pattern: string, time: number, count: number) => {
        if (pattern === 'all') return [0, count];
        const start = (Math.round(time * 30) % 91) * 4;
        return [start, start + 40];
      };
      const measure = (channels: any[], pattern: string, fn: typeof native, count: number, single = false) => {
        let sum = 0;
        const started = performance.now();
        for (let frame = 0; frame < count; frame++) {
          const time = (frame % 1024) / 30;
          if (single) { sum += fn(channels[0], time); continue; }
          const [first, end] = range(pattern, time, channels.length);
          for (let i = first; i < end; i++) sum += fn(channels[i], time);
        }
        const ms = performance.now() - started;
        return { usPerFrame: ms * 1000 / count, nsPerCall: ms * 1e6 / count / (single ? 1 : channels.length), sum };
      };
      const quantile = (xs: number[], q: number) => [...xs].sort((a, b) => a - b)[Math.min(xs.length - 1, Math.floor(xs.length * q))]!;
      try {
        const rows = [];
        for (const [name, sources] of Object.entries(suites)) for (const keyed of [false, true]) for (const pattern of ['all', 'window']) {
          const channels = fixture(sources, keyed);
          for (const time of [0, -0, .001, .5, 1, 17 / 30, 2.5, 9.999]) for (const c of channels) {
            const a = native(c, time), b = today(c, time);
            if (!Object.is(a, b)) throw new Error(`expression parity ${name} ${c.key} at ${time}: ${a} vs ${b}`);
          }
          measure(channels, pattern, native, 400); measure(channels, pattern, today, 400);
          const pairs = [];
          for (let pair = 0; pair < 5; pair++) {
            let engine, legacy;
            if (pair % 2) { legacy = measure(channels, pattern, today, 1000); engine = measure(channels, pattern, native, 1000); }
            else { engine = measure(channels, pattern, native, 1000); legacy = measure(channels, pattern, today, 1000); }
            if (!Object.is(engine.sum, legacy.sum)) throw new Error(`paired frame sums differ in ${name}`);
            pairs.push({ pair, engine, today: legacy });
          }
          const engineUs = pairs.map(p => p.engine.usPerFrame), todayUs = pairs.map(p => p.today.usPerFrame);
          rows.push({ suite: name, base: keyed ? 'keyed' : 'static', pattern, pairs,
            engineUs: { median: quantile(engineUs, .5), max: Math.max(...engineUs) }, todayUs: { median: quantile(todayUs, .5), max: Math.max(...todayUs) } });
        }
        // A one-channel fixture for extension scalar calls.
        const channels = fixture(suites.numeric!, false);
        PM.proj.layers = [PM.proj.layers[0]];
        for (let k = 1; k < 4; k++) delete PM.proj.layers[0].p[`expr${k}`];
        PM.touch(); api.reset(); native(channels[0], 0);
        const scalar = [];
        for (let pair = 0; pair < 5; pair++) {
          let engine, legacy;
          if (pair % 2) { legacy = measure(channels, 'all', today, 200_000, true); engine = measure(channels, 'all', native, 200_000, true); }
          else { engine = measure(channels, 'all', native, 200_000, true); legacy = measure(channels, 'all', today, 200_000, true); }
          if (!Object.is(engine.sum, legacy.sum)) throw new Error('paired scalar sums differ');
          scalar.push({ pair, engine, today: legacy });
        }
        return { channels: 400, framesPerPair: 1000, rows, scalar, stats: doc.stats(), heap: doc.wasm.heap() };
      } finally { PM.proj = original; PM.scope = scope; PM.touch(); }
    }, EXPRESSION_SUITES);
    const loadAfter = os.loadavg();
    const moduleSha256 = createHash("sha256").update(readFileSync(path.join(repoRoot, "out/renderer/engine/pm_wasm.wasm"))).digest("hex");
    const output = { ...result, loadAfter, moduleSha256 };
    if (process.env.PM_FIXUPS3_EVIDENCE) {
      await mkdir(process.env.PM_FIXUPS3_EVIDENCE, { recursive: true });
      await writeFile(path.join(process.env.PM_FIXUPS3_EVIDENCE, 'expressions.json'), JSON.stringify(output, null, 2));
    }
    console.log(JSON.stringify(output));
    expect(loadAfter[0], 'machine stayed quiet').toBeLessThanOrEqual(8);
    for (const row of result.rows) {
      expect(row.engineUs.median, `${row.suite} ${row.base} ${row.pattern} median`).toBeLessThanOrEqual(row.todayUs.median);
    }
  } finally { await app.close(); }
});
