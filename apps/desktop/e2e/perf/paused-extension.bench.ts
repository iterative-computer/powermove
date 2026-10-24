/** Opt-in paired latency gate. Build first, then run under pm-electron-lock
 * with two SHA256 workers; see docs/engine/extensions-compat.md. */
import { execFileSync } from 'node:child_process';
import { cp, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import type { Page } from 'playwright';
import { expect, launchApp, test } from '../helpers/app';

async function project(page: Page): Promise<number> {
  return page.evaluate(() => {
    const PM = (window as any).PM;
    const p = PM.mkProject({ name: 'Paused extension performance', w: 640, h: 360, fps: 30, dur: 10, bg: '#203040' });
    p.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    const tinted = PM.mkLayer('shape', { name: 'Tinted', from: 0, dur: 10, d: { shape: 'ellipse', w: 240, h: 160, color: '#33CC66', stroke: 0 } }, p);
    tinted.id = 'tinted'; tinted.p['position.x'].v = 200; tinted.p['position.y'].v = 180;
    const off = Array.from({ length: 2500 }, (_, i) => {
      const l = PM.mkLayer('shape', { name: `Off ${i}`, from: 0, dur: 10, d: { shape: 'rect', w: 40 + i % 50, h: 30, color: '#3366CC', stroke: 0 } }, p);
      l.id = `off-${i}`; l.on = false;
      l.p.rotation = PM.P(0, { kf: [PM.KF(0, 0), PM.KF(5, i % 360), PM.KF(10, 0)] });
      l.p['position.x'] = PM.P(0, { kf: [PM.KF(0, i % 640), PM.KF(10, i * 7 % 640)] });
      return l;
    });
    p.layers = [tinted, ...off];
    PM.replaceProject(p); PM.Projects?.put?.(PM.proj);
    return JSON.stringify(PM.proj).length;
  });
}

async function edits(page: Page, engine: boolean) {
  return page.evaluate(async engine => {
    const PM = (window as any).PM;
    let shown: (() => void) | null = null;
    const off = PM.Kernel.api('extension-perf').events.on('frame:rendered', () => {
      const latest = PM.Kernel.services.get('viewer')?.enginePresentation?.latest;
      if (engine && (!latest || latest.revision < PM.EngineDoc.barrier.revision)) return;
      shown?.(); shown = null;
    });
    const samples: Array<{ began: number; at: number; ms: number; revision: number | null; seq: number | null }> = [];
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      for (let i = 0; i < 60; i++) {
        const began = performance.now();
        const presented = new Promise<void>((resolve, reject) => {
          shown = resolve;
          timer = setTimeout(() => reject(new Error(`Edit ${i} did not present within 2000 ms.`)), 2000);
        });
        PM.proj.layers.find((l: any) => l.id === 'tinted').p['position.x'].v = 200 + i * 4;
        PM.touch(); PM.invalidate();
        await presented; clearTimeout(timer);
        const at = performance.now();
        const latest = PM.Kernel.services.get('viewer')?.enginePresentation?.latest;
        samples.push({ began, at, ms: at - began, revision: latest?.revision ?? null, seq: latest?.seq ?? null });
      }
    } finally {
      clearTimeout(timer);
      if (typeof off === 'function') off(); else off?.dispose?.();
    }
    return {
      samples,
      hostOffsetUs: PM.EngineConnection?.status?.()?.hostOffsetUs ?? null,
      stages: PM.EngineRender?.trace?.take() ?? null,
      viewer: await PM.Kernel.services.get('viewer')?.enginePresentation?.takeTrace?.().catch(() => null) ?? null,
      doc: PM.EngineDoc?.stats?.() ?? null,
      raster: PM.RasterCompat?.stats ?? null,
      route: PM.EngineConnection?.status?.()?.route ?? null
    };
  }, engine);
}

function quantiles(samples: number[]) {
  const sorted = [...samples].sort((a, b) => a - b);
  const q = (p: number) => sorted[Math.ceil(p * sorted.length) - 1]!;
  return { p50: q(.5), p95: q(.95), max: q(1) };
}

test('paused edits with an override are no slower than today in alternating paired runs', async () => {
  const output = process.env.PM_EXTENSION_PERF_OUT!;
  await mkdir(output, { recursive: true });
  const head = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
  const runs: Array<{ engine: boolean; override: boolean; samples: number[] }> = [];
  // Two paired runs with opposite orders, one Electron at a time.
  for (const [round, engine] of [true, false, false, true].entries()) {
    const userData = await mkdtemp(path.join(os.tmpdir(), 'powermove-extension-perf-'));
    try {
      await cp(path.join(__dirname, '../fixtures/compat-corpus/shape-tint'), path.join(userData, 'extensions/shape-tint'), { recursive: true });
      const app = await launchApp({ userData, env: engine ? { POWERMOVE_NEW_ENGINE: '1', POWERMOVE_BRIDGE_TRACE: '1' } : { POWERMOVE_NEW_ENGINE: '0' } });
      try {
        await app.openEditor();
        await app.page.waitForFunction(() => (window as any).PM.Kernel.loader.activeIds().includes('shape-tint'));
        const bytes = await project(app.page);
        expect(bytes).toBeGreaterThan(4_000_000);
        if (engine) {
          await expect.poll(() => app.page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 90_000 }).toBe('engine');
          await app.page.evaluate(() => (window as any).PM.EngineConnection.setTrace(true));
        }
        const native = () => app.app.evaluate(async () => (globalThis as any).__powermoveNativeEngine?.engine?.report?.('interactive').catch(() => null));
        for (const override of engine ? [true, false] : [true]) {
          if (!override) await app.page.evaluate(() => (window as any).PM.Kernel.loader.deactivate('shape-tint'));
          await app.page.waitForTimeout(1500);
          if (engine) await native();
          const data = await edits(app.page, engine);
          if (engine) expect(data.route).toBe('engine');
          const samples = data.samples.map(s => s.ms);
          runs.push({ engine, override, samples });
          const report = { head, round, engine, override, documentBytes: bytes, load: os.loadavg(), quantiles: quantiles(samples), data, native: engine ? await native() : null };
          await writeFile(path.join(output, `${round}-${engine ? 'engine' : 'today'}-${override ? 'override' : 'plain'}.json`), JSON.stringify(report));
          console.log(JSON.stringify({ round, engine, override, load: report.load, ...report.quantiles }));
        }
        expect(app.diagnostics.pageErrors).toEqual([]);
      } finally { await app.close(); }
    } finally { await rm(userData, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 }); }
  }
  const measured = (engine: boolean, override = true) => quantiles(runs.filter(r => r.engine === engine && r.override === override).flatMap(r => r.samples));
  const engine = measured(true), today = measured(false), plain = measured(true, false);
  const pass = { p50: engine.p50 <= today.p50, p95: engine.p95 <= today.p95, max: engine.max <= today.max };
  await writeFile(path.join(output, 'comparison.json'), JSON.stringify({ head, order: ['engine', 'today', 'today', 'engine'], engine, today, plain, pass }, null, 2));
  console.log(JSON.stringify({ engine, today, plain, pass }));
  expect(pass).toEqual({ p50: true, p95: true, max: true });
});
