/*
 * The customization moat with "Use New Engine" on (owner amendments 23 and
 * 24; docs/engine/extensions-compat.md). Extensions written against today's
 * API keep working unchanged on the engine:
 *
 * - The typewriter fixture (e2e/fixtures/typewriter-effect), an apiVersion
 *   1 extension that wraps the `raster` service: its typed prefix and cursor
 *   are what the engine draws in snapshots and viewer frames, at the layer's
 *   own time and density (a scaled, animated layer), next to an unaffected
 *   text layer the engine draws itself, and disposing it restores the plain
 *   layer. Compared with today's compositor in the same window.
 *
 * Run serially, one Electron at a time:
 *   pm-electron-lock bunx playwright test e2e/engine-extension-compat.spec.ts --workers=1
 */
import { execFileSync } from 'node:child_process';
import { cp, mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import type { ElectronApplication, Page } from 'playwright';

import { expect, test } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

const ON = { POWERMOVE_NEW_ENGINE: '1' };

/** A 480×270 composition with a typewriter text (scaled 150%, progress
 *  keyed 0 → 100 over 2 s) and a plain text, over #203040. */
async function typewriterProject(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Typewriter on the engine', w: 480, h: 270, fps: 30, dur: 3, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    const typed = PM.mkLayer('text', { name: 'Typed', from: 0, dur: 3, d: { text: 'Hello engine', font: 'Helvetica', size: 40, weight: 700, color: '#FFFFFF' } }, project);
    typed.id = 'typed';
    typed.p['position.x'].v = 240; typed.p['position.y'].v = 90;
    typed.p['scale.x'].v = 150; typed.p['scale.y'].v = 150;
    const fx = PM.mkEffect('typewriter');
    fx.p.progress = PM.P(0, { kf: [PM.KF(0, 0), PM.KF(2, 100)] });
    fx.p.cursor.v = true;
    typed.fx = [fx];
    const plain = PM.mkLayer('text', { name: 'Plain', from: 0, dur: 3, d: { text: 'Unaffected', font: 'Helvetica', size: 32, weight: 400, color: '#FFCC00' } }, project);
    plain.id = 'plain';
    plain.p['position.x'].v = 240; plain.p['position.y'].v = 200;
    project.layers = [typed, plain];
    PM.replaceProject(project);
    PM.Projects?.put?.(PM.proj);
  });
}

/** The next save panel answers `file`. */
async function stubSavePanel(app: ElectronApplication, file: string): Promise<void> {
  await app.evaluate(({ dialog }, target) => {
    dialog.showSaveDialog = (async () => ({ canceled: false, filePath: target })) as typeof dialog.showSaveDialog;
  }, file);
}

/** A PNG's pixels as 8-bit RGBA (through our FFmpeg). */
function pngRgba(file: string): Buffer {
  return execFileSync(ffmpegBinary, ['-v', 'error', '-i', file, '-f', 'rawvideo', '-pix_fmt', 'rgba', '-'], { maxBuffer: 64 * 1024 * 1024 });
}

test.describe('Use New Engine: existing extensions', () => {
  test.use({ desktopLaunchOptions: { env: ON } });

  test('a raster override (the typewriter fixture) is drawn by the engine as today, and disposing it restores the layer', async ({ session }, testInfo) => {
    await cp(path.join(__dirname, 'fixtures/typewriter-effect'), path.join(session.userData, 'extensions/typewriter-effect'), { recursive: true });
    await session.relaunch();
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => (window as any).PM?.Kernel?.loader?.activeIds?.().includes('typewriter-effect') && Boolean((window as any).PM?.EngineRender));
    await typewriterProject(page);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    // The base raster an override wraps is today's surface: the whole
    // canvas with today's box and anchor (a scaled text layer).
    const geometry = await page.evaluate(() => {
      const PM = (window as any).PM;
      const layer = PM.proj.layers.find((l: any) => l.id === 'typed');
      const pick = (r: any) => r && { w: r.w, h: r.h, cw: r.cv.width, ch: r.cv.height, anchorX: r.anchorX, anchorY: r.anchorY };
      const engine = pick(PM.RasterCompat.base([layer, 1.5, 2.5]));
      const today = pick(PM.__rasterBase(layer, 1.5, 2.5));
      return { engine, today };
    });
    console.log(JSON.stringify(geometry));
    expect(geometry.engine).toEqual(geometry.today);
    const result = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('compat-probe');
      const W = 480, H = 270;
      const pixels = (c: HTMLCanvasElement) => c.getContext('2d')!.getImageData(0, 0, c.width, c.height).data;
      /** White ink in the typed layer's band and yellow ink in the plain one's. */
      const ink = (d: Uint8ClampedArray) => {
        let typed = 0, plain = 0;
        for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
          const i = (y * W + x) * 4;
          if (y < 150 && d[i]! > 180 && d[i + 1]! > 180 && d[i + 2]! > 180) typed++;
          if (y >= 170 && d[i]! > 180 && d[i + 1]! > 150 && d[i + 2]! < 90) plain++;
        }
        return { typed, plain };
      };
      const diff = (a: Uint8ClampedArray, b: Uint8ClampedArray, from: number, to: number) => {
        let max = 0, over = 0;
        for (let y = from; y < to; y++) for (let x = 0; x < W; x++) {
          for (let c = 0; c < 3; c++) {
            const i = (y * W + x) * 4 + c;
            const d = Math.abs(a[i]! - b[i]!);
            if (d > max) max = d;
            if (d > 8) over++;
          }
        }
        return { max, over };
      };
      const frames: Record<string, { engine: Uint8ClampedArray; today: Uint8ClampedArray }> = {};
      const contact = document.createElement('canvas'); contact.width = W * 2; contact.height = H * 4;
      const cx = contact.getContext('2d')!;
      let row = 0;
      for (const [name, t] of [['empty', 0], ['partial', 0.8], ['full', 2.5]] as const) {
        // apiVersion 1 synchronous API: the engine's frame, with the wrapper's answers.
        const engine = api.render.renderFrameTo(t, W, H, { mblur: false });
        // Today's compositor in the same window (scaffolding until the router is removed).
        const today = PM.renderFrameTo(t, W, H, { mblur: false });
        frames[name] = { engine: pixels(engine), today: pixels(today) };
        cx.drawImage(engine, 0, row * H); cx.drawImage(today, W, row * H);
        row++;
      }
      const stats = { ...PM.EngineRender.compatStats };
      // Viewer frames ask too: play a little and let the engine present.
      PM.setTime(1.2);
      await new Promise((r) => setTimeout(r, 1500));
      const viewerAsked = PM.EngineRender.compatStats.needs - stats.needs;
      // Disposal: the plain layer comes back everywhere.
      const plainBefore = pixels(api.render.renderFrameTo(2.5, W, H, { mblur: false }));
      await PM.Kernel.loader.deactivate('typewriter-effect');
      const restored = api.render.renderFrameTo(2.5, W, H, { mblur: false });
      cx.drawImage(restored, 0, row * H);
      const restoredPixels = pixels(restored);
      const producers = PM.EngineRender.compatStats;
      return {
        ink: Object.fromEntries(Object.entries(frames).map(([k, v]) => [k, { engine: ink(v.engine), today: ink(v.today) }])),
        diffs: Object.fromEntries(Object.entries(frames).map(([k, v]) => [k, { typed: diff(v.engine, v.today, 0, 150), plain: diff(v.engine, v.today, 170, 270) }])),
        stats, viewerAsked,
        restoredInk: ink(restoredPixels),
        fullInk: ink(plainBefore),
        restoredDiffersFromTyped: diff(restoredPixels, plainBefore, 0, 150).over > 0,
        producers,
        contact: contact.toDataURL()
      };
    });
    const contactPath = testInfo.outputPath('typewriter-engine-vs-today.png');
    await writeFile(contactPath, Buffer.from(result.contact.split(',')[1]!, 'base64'));
    await testInfo.attach('typewriter-engine-vs-today', { path: contactPath, contentType: 'image/png' });
    console.log(JSON.stringify({ ink: result.ink, diffs: result.diffs, stats: result.stats, viewerAsked: result.viewerAsked }));
    // The wrapper's answers reached the engine: rasters for the typed layer,
    // pass-throughs for the plain one.
    expect(result.stats.rasters).toBeGreaterThanOrEqual(2);
    expect(result.stats.natives).toBeGreaterThanOrEqual(3);
    // Progress 0: no typed ink (a cursor may show); partial < full; the plain layer unchanged.
    expect(result.ink.partial.engine.typed).toBeGreaterThan(result.ink.empty.engine.typed);
    expect(result.ink.full.engine.typed).toBeGreaterThan(result.ink.partial.engine.typed);
    for (const name of ['empty', 'partial', 'full']) {
      expect(result.ink[name].engine.plain).toBeGreaterThan(100);
      // The wrapper's raster is today's: the typed band matches the
      // compositor's within one code value.
      expect(result.diffs[name].typed.max).toBeLessThanOrEqual(1);
    }
    // Viewer frames asked the producer for the frames they drew.
    expect(result.viewerAsked).toBeGreaterThan(0);
    // Disposed: the typed layer is the plain text again (full, no cursor), the plain layer as before.
    expect(result.restoredInk.plain).toBe(result.fullInk.plain);
    expect(result.restoredInk.typed).toBeGreaterThan(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('an export of a project with a raster override draws the override, frame by frame', async ({ session }, testInfo) => {
    test.setTimeout(240_000);
    await cp(path.join(__dirname, 'fixtures/typewriter-effect'), path.join(session.userData, 'extensions/typewriter-effect'), { recursive: true });
    await session.relaunch();
    await session.openEditor();
    const { app, page } = session;
    await page.waitForFunction(() => (window as any).PM?.Kernel?.loader?.activeIds?.().includes('typewriter-effect') && Boolean((window as any).PM?.EngineRender));
    await typewriterProject(page);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineRender.producing), { timeout: 10_000 }).toEqual(['rasterOverride']);
    const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-compat-export-'));
    try {
      const folder = path.join(root, 'Frames');
      await stubSavePanel(app, folder);
      const run = await page.evaluate(async () => {
        const PM = (window as any).PM;
        const before = PM.EngineRender.compatStats.exportAsks;
        const result = await PM.Export.run({ format: 'png', scale: 1, fps: 30, range: 'all', quality: 'high', mblur: false, audio: false, alpha: false });
        return { result, exportAsks: PM.EngineRender.compatStats.exportAsks - before };
      });
      console.log(JSON.stringify(run));
      expect(run.result).toEqual({ cancelled: false });
      // The job engine drew it, asking this window for the override's layers
      // (two text layers, 90 frames).
      expect(run.exportAsks).toBeGreaterThanOrEqual(90);
      const files = (await readdir(folder)).filter((f) => f.endsWith('.png')).sort();
      expect(files.length).toBe(90);
      // Frame 24 (0.8 s) and frame 75 (2.5 s): today's compositor in the same window.
      const today = await page.evaluate(() => {
        const PM = (window as any).PM;
        return [0.8, 2.5].map((t) => Array.from(PM.renderFrameTo(t, 480, 270, { mblur: false }).getContext('2d').getImageData(0, 0, 480, 270).data as Uint8ClampedArray));
      });
      const diffs = [24, 75].map((frame, i) => {
        const exported = pngRgba(path.join(folder, files[frame]!));
        let typedMax = 0, typedInk = 0, todayInk = 0;
        for (let y = 0; y < 150; y++) for (let x = 0; x < 480; x++) {
          const p = (y * 480 + x) * 4;
          for (let c = 0; c < 3; c++) typedMax = Math.max(typedMax, Math.abs(exported[p + c]! - today[i]![p + c]!));
          if (exported[p]! > 180 && exported[p + 1]! > 180 && exported[p + 2]! > 180) typedInk++;
          if (today[i]![p]! > 180 && today[i]![p + 1]! > 180 && today[i]![p + 2]! > 180) todayInk++;
        }
        return { frame, typedMax, typedInk, todayInk };
      });
      console.log(JSON.stringify(diffs));
      await testInfo.attach('export-frame-24', { path: path.join(folder, files[24]!), contentType: 'image/png' });
      for (const d of diffs) {
        expect(d.typedInk).toBe(d.todayInk);
        expect(d.typedMax).toBeLessThanOrEqual(2);
      }
      // The partial frame has less typed ink than the full one.
      expect(diffs[0]!.typedInk).toBeLessThan(diffs[1]!.typedInk);
      expect(session.diagnostics.pageErrors).toEqual([]);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});
