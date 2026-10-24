import { requestJobEngine } from './helpers/job-engine';
import { mkdir, mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { expect, test, type ElectronApplication, type Page } from '@playwright/test';

import { launchApp } from './helpers/app';

/* Section 25's 3b gate "Export, every format at cutover: absolute fps per
   codec and machine, at least today's re-measured baseline". Exports one
   1080p project through today's exporter and through the job engine (Use New
   Engine on), format by format, and prints wall-clock fps from Export to
   done. A measurement, not a check: it runs only with PM_EXPORT_BASELINE=1,
   serially under the Electron lock. */

const FORMATS = ['h264', 'prores-4444', 'webm-vp9', 'png'] as const;
const FPS = 24;
const SECONDS = Number(process.env.PM_EXPORT_BASELINE_SECONDS) || 6;

async function engineReady(app: ElectronApplication): Promise<void> {
  await requestJobEngine(app);
  await expect.poll(() => app.evaluate(() => {
    const status = (globalThis as { __powermoveNativeEngine?: { engine: { status(): Record<string, { state: string }> } | null } }).__powermoveNativeEngine?.engine?.status();
    return status ? `${status['interactive']!.state}/${status['job']!.state}` : 'missing';
  }), { timeout: 30_000 }).toBe('ready/ready');
}

async function project(page: Page): Promise<void> {
  await page.evaluate(({ fps, seconds }) => {
    const PM = (window as any).PM;
    PM.pause();
    PM.assets.clear();
    PM.proj = PM.mkProject({ name: 'Baseline', w: 1920, h: 1080, fps, dur: seconds, bg: '#182439' });
    PM.proj.work = [0, seconds];
    PM.time = 0;
    PM.sel.layers = [];
    // Moving, rotating, translucent solids (layers both exporters render),
    // so no frame repeats.
    const colors = ['#e83a5e', '#3fcf8e', '#4d7cff', '#f5b941', '#b05cff', '#ffffff'];
    PM.proj.layers = colors.map((color, i) => {
      const layer = PM.mkLayer('solid', {
        name: `Solid ${i}`,
        p: { 'position.x': 300 + i * 120, 'position.y': 200 + i * 130, opacity: 90 - i * 10 },
        d: { color, w: 900 - i * 120, h: 600 - i * 80 }
      });
      layer.p['position.x'].kf = [PM.KF(0, 300 + i * 120), PM.KF(seconds, 1600 - i * 120)];
      layer.p.rotation.kf = [PM.KF(0, 0), PM.KF(seconds, 90 + i * 45)];
      for (const axis of ['scale.x', 'scale.y']) layer.p[axis].kf = [PM.KF(0, 80), PM.KF(seconds, 120)];
      layer.dur = seconds;
      return layer;
    });
    PM.ProjectIndex.invalidate();
    PM.touch();
    PM.bus.emit('project');
    PM.bus.emit('layers');
  }, { fps: FPS, seconds: SECONDS });
}

async function measure(engine: boolean, root: string): Promise<Record<string, number>> {
  const session = await launchApp(engine ? { env: { POWERMOVE_NEW_ENGINE: '1' } } : {});
  const { app, page } = session;
  const fps: Record<string, number> = {};
  try {
    if (engine) await engineReady(app);
    await session.openEditor();
    await project(page);
    if (engine) {
      await page.evaluate(() => {
        const w = window as any;
        w.__routed = [];
        w.__powermoveTestBridge.nativeEngine.onExportStatus((s: any) => { if (s.routed) w.__routed.push(s.error); });
      });
    }
    for (const format of FORMATS) {
      const output = path.join(root, `${engine ? 'engine' : 'today'}-${format}${format === 'png' ? '' : format === 'webm-vp9' ? '.webm' : format === 'h264' ? '.mp4' : '.mov'}`);
      // Today's PNG sequence asks for an existing folder; the job engine's
      // save panel names a folder it creates.
      if (format === 'png' && !engine) await mkdir(output);
      await app.evaluate(({ dialog }, filePath) => {
        dialog.showSaveDialog = (async () => ({ canceled: false, filePath })) as typeof dialog.showSaveDialog;
        dialog.showOpenDialog = (async () => ({ canceled: false, filePaths: [filePath] })) as typeof dialog.showOpenDialog;
      }, output);
      const seconds = await page.evaluate(async ({ format, rate }) => {
        const PM = (window as any).PM;
        const start = performance.now();
        const result = await PM.Export.run({ format, scale: 1, fps: rate, range: 'all', quality: 'high', mblur: false, alpha: false, audio: false });
        if (result?.cancelled || result?.error) throw new Error(`${format}: ${result.error ?? 'cancelled'}`);
        while (PM.Export.busy) await new Promise((resolve) => setTimeout(resolve, 20));
        return (performance.now() - start) / 1000;
      }, { format, rate: FPS });
      // Every export here must have rendered in the job engine, not been
      // routed back to today's exporter.
      if (engine) expect(await page.evaluate(() => (window as any).__routed)).toEqual([]);
      fps[format] = (FPS * SECONDS) / seconds;
      console.log(`MEASURE export baseline ${engine ? 'job engine' : "today's exporter"} ${format} 1920x1080, ${FPS * SECONDS} frames: ${fps[format]!.toFixed(1)} fps (${seconds.toFixed(2)} s)`);
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
  return fps;
}

test('export throughput: the job engine against today\'s exporter, format by format', async () => {
  test.skip(process.env.PM_EXPORT_BASELINE !== '1', 'measurement: set PM_EXPORT_BASELINE=1');
  test.setTimeout(600_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-export-baseline-'));
  try {
    const today = await measure(false, root);
    const engine = await measure(true, root);
    for (const format of FORMATS) {
      console.log(`MEASURE export baseline ratio ${format}: job engine ${(engine[format]! / today[format]!).toFixed(2)}× today's exporter`);
    }
  } finally {
    if (process.env.PM_EXPORT_BASELINE_KEEP === '1') console.log(`kept ${root}`);
    else await rm(root, { recursive: true, force: true });
  }
});
