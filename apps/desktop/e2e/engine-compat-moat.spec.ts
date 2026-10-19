/*
 * The customization moat with "Use New Engine" on, round two (owner
 * amendments 23 and 24; docs/engine/extensions-compat.md):
 *
 * - A raster wrapper assigned to `PM.raster` through `api.host.pm` (the
 *   pre-services pattern) draws on the engine as today.
 * - A raster override at a deep viewer zoom gets today's preview crop
 *   window (the typewriter fixture), and the engine's viewer frame equals
 *   today's.
 * - What an extension draws into `render.gl.context` after each frame is in
 *   a WebM export (today's `renderInto` exports), not in an MP4 one, and
 *   `drawImage(context.canvas)` reads the frame and the drawing as today.
 *
 * Run serially, one Electron at a time:
 *   pm-electron-lock bunx playwright test e2e/engine-compat-moat.spec.ts --workers=1
 */
import { execFileSync } from 'node:child_process';
import { cp, mkdir, mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import type { ElectronApplication, Page } from 'playwright';

import { expect, launchApp, test } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

const ON = { POWERMOVE_NEW_ENGINE: '1' };
/** Side-by-sides go outside the repository (no screenshots in Git). */
const EVIDENCE = path.join(os.tmpdir(), 'ne-extcompat-evidence');

const FIXTURES: Record<string, string> = {
  'legacy-raster-assign': 'fixtures/compat-corpus/legacy-raster-assign',
  'context-watermark': 'fixtures/compat-corpus/context-watermark',
  'crop-recorder': 'fixtures/compat-corpus/crop-recorder',
  'typewriter-effect': 'fixtures/typewriter-effect'
};

async function install(userData: string, ids: string[]): Promise<void> {
  for (const id of ids) await cp(path.join(__dirname, FIXTURES[id]!), path.join(userData, 'extensions', id), { recursive: true });
}

async function waitActive(page: Page, ids: string[]): Promise<void> {
  await page.waitForFunction((ids) => ids.every((id) => (window as any).PM?.Kernel?.loader?.activeIds?.().includes(id)), ids, { timeout: 30_000 });
}

/** Today's app: a fresh profile with `ids` installed and "Use New Engine" off. */
async function todayApp(ids: string[]) {
  const userData = await mkdtemp(path.join(os.tmpdir(), 'powermove-today-'));
  await install(userData, ids);
  const app = await launchApp({ userData, env: {} });
  await app.openEditor();
  await waitActive(app.page, ids);
  return app;
}

/** Section 25's render measure over two RGBA buffers (color channels). */
function measure(a: ArrayLike<number>, b: ArrayLike<number>): { max: number; over1: number; values: number } {
  let max = 0, over1 = 0, values = 0;
  for (let i = 0; i < Math.min(a.length, b.length); i++) {
    if (i % 4 === 3) continue;
    const d = Math.abs(a[i]! - b[i]!);
    values++;
    if (d > max) max = d;
    if (d > 1) over1++;
  }
  return { max, over1, values };
}
const withinSection25 = (m: { max: number; over1: number; values: number }) => m.max <= 4 && m.over1 <= m.values * 0.001;

/** A 480×270 project: a red-tinted ellipse, a shifted text, a plain rectangle. */
async function assignProject(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'PM.raster assignment', w: 480, h: 270, fps: 30, dur: 2, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    const tinted = PM.mkLayer('shape', { name: 'Tinted', from: 0, dur: 2, d: { shape: 'ellipse', w: 120, h: 80, color: '#33CC66', stroke: 0 } }, project);
    tinted.id = 'tinted'; tinted.p['position.x'].v = 110; tinted.p['position.y'].v = 90;
    const shifted = PM.mkLayer('text', { name: 'Shifted', from: 0, dur: 2, d: { text: 'Shifted', font: 'Helvetica', size: 36, weight: 700, color: '#FFFFFF' } }, project);
    shifted.id = 'shifted'; shifted.p['position.x'].v = 330; shifted.p['position.y'].v = 90;
    const plain = PM.mkLayer('shape', { name: 'Plain', from: 0, dur: 2, d: { shape: 'rect', w: 80, h: 60, color: '#3366CC', stroke: 0 } }, project);
    plain.id = 'plain'; plain.p['position.x'].v = 240; plain.p['position.y'].v = 200;
    project.layers = [tinted, shifted, plain];
    PM.replaceProject(project);
    PM.Projects?.put?.(PM.proj);
  });
}

/** The typewriter project of engine-extension-compat.spec.ts, its text larger. */
async function bigTypewriterProject(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Typewriter, zoomed', w: 480, h: 270, fps: 30, dur: 3, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    const typed = PM.mkLayer('text', { name: 'Typed', from: 0, dur: 3, d: { text: 'Hello engine', font: 'Helvetica', size: 40, weight: 700, color: '#FFFFFF' } }, project);
    typed.id = 'typed';
    typed.p['position.x'].v = 240; typed.p['position.y'].v = 120;
    typed.p['scale.x'].v = 150; typed.p['scale.y'].v = 150;
    const fx = PM.mkEffect('typewriter');
    fx.p.progress = PM.P(0, { kf: [PM.KF(0, 0), PM.KF(2, 100)] });
    fx.p.cursor.v = true;
    typed.fx = [fx];
    project.layers = [typed];
    PM.replaceProject(project);
    PM.Projects?.put?.(PM.proj);
  });
}

/** Puts the viewer at `zoom`, centred on composition point (x, y), paused at `time`. */
async function zoomViewer(page: Page, zoom: number, x: number, y: number, time: number): Promise<{ stage: number[]; shown: number }> {
  return page.evaluate(({ zoom, x, y, time }) => {
    const PM = (window as any).PM, V = PM.Kernel.services.get('viewer');
    PM.pause?.();
    PM.setTime(time, { force: true });
    V.fit = false;
    V.setZoom(zoom);
    V.pan = [(PM.proj.w / 2 - x) * zoom, (PM.proj.h / 2 - y) * zoom];
    V.layout();
    PM.invalidate?.('render');
    return { stage: [V._sw, V._sh], shown: V.shown };
  }, { zoom, x, y, time });
}

/** The viewer's frame as pixels: the canvas showing it (the engine's presentation, or today's compositor canvas). */
async function viewerPixels(page: Page, engine: boolean): Promise<{ width: number; height: number; data: number[]; png: string }> {
  return page.evaluate(async (engine) => {
    const PM = (window as any).PM;
    const settle = async () => { for (let i = 0; i < 40; i++) await new Promise((r) => requestAnimationFrame(r)); await new Promise((r) => setTimeout(r, 600)); };
    await settle();
    const source = engine
      ? (Array.from(document.querySelectorAll('canvas#engine-present')) as HTMLCanvasElement[]).find((c) => c.style.visibility === 'visible' && c.width > 0)!
      : (document.querySelector('canvas#gl') as HTMLCanvasElement | null) ?? PM.GL.canvas as HTMLCanvasElement;
    const c = document.createElement('canvas');
    c.width = source.width; c.height = source.height;
    const c2 = c.getContext('2d')!;
    c2.drawImage(source, 0, 0);
    return { width: c.width, height: c.height, data: Array.from(c2.getImageData(0, 0, c.width, c.height).data), png: c.toDataURL('image/png') };
  }, engine);
}

/** An exported video's frames as 8-bit RGBA (through our FFmpeg). */
function videoFrames(file: string, width: number, height: number): Buffer[] {
  const raw = execFileSync(ffmpegBinary, ['-v', 'error', '-i', file, '-f', 'rawvideo', '-pix_fmt', 'rgba', '-'], { maxBuffer: 1024 * 1024 * 1024 });
  const size = width * height * 4;
  const out: Buffer[] = [];
  for (let at = 0; at + size <= raw.length; at += size) out.push(raw.subarray(at, at + size));
  return out;
}

/** The next save panel answers `file`. */
async function stubSavePanel(app: ElectronApplication, file: string): Promise<void> {
  await app.evaluate(({ dialog }, target) => {
    dialog.showSaveDialog = (async () => ({ canceled: false, filePath: target })) as typeof dialog.showSaveDialog;
    dialog.showOpenDialog = (async () => ({ canceled: false, filePaths: [target] })) as typeof dialog.showOpenDialog;
  }, file);
}

/** A small, still project for the watermark exports (a 320×180 composition). */
async function watermarkProject(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Watermark', w: 320, h: 180, fps: 30, dur: 1, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    const box = PM.mkLayer('shape', { name: 'Box', from: 0, dur: 1, d: { shape: 'rect', w: 120, h: 80, color: '#33CC66', stroke: 0 } }, project);
    box.p['position.x'].v = 200; box.p['position.y'].v = 110;
    project.layers = [box];
    PM.replaceProject(project);
    PM.Projects?.put?.(PM.proj);
  });
}

/** The mean color of a rectangle of an RGBA frame. */
function mean(frame: Buffer, width: number, x0: number, y0: number, w: number, h: number): number[] {
  const sum = [0, 0, 0];
  for (let y = y0; y < y0 + h; y++) for (let x = x0; x < x0 + w; x++) for (let c = 0; c < 3; c++) sum[c] += frame[(y * width + x) * 4 + c]!;
  return sum.map((v) => Math.round(v / (w * h)));
}

test.describe('Use New Engine: the customization moat, round two', () => {
  test.use({ desktopLaunchOptions: { env: ON } });

  test('a raster wrapper assigned to PM.raster through host.pm draws on the engine as today', async ({ session }, testInfo) => {
    await install(session.userData, ['legacy-raster-assign']);
    await session.relaunch();
    await session.openEditor();
    const { page } = session;
    await waitActive(page, ['legacy-raster-assign']);
    await assignProject(page);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    const result = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('compat-moat');
      const pixels = (c: HTMLCanvasElement) => Array.from(c.getContext('2d')!.getImageData(0, 0, c.width, c.height).data);
      const before = { ...PM.EngineRender.compatStats };
      // The engine's frame (apiVersion 1 synchronous API) and today's
      // compositor in the same window, which calls PM.raster as it always did.
      const engine = api.render.renderFrameTo(0.5, 480, 270, { mblur: false });
      const today = PM.renderFrameTo(0.5, 480, 270, { mblur: false });
      const contact = document.createElement('canvas'); contact.width = 960; contact.height = 270;
      contact.getContext('2d')!.drawImage(engine, 0, 0); contact.getContext('2d')!.drawImage(today, 480, 0);
      // The viewer's frames ask the producer too.
      PM.setTime(0.8, { force: true });
      await new Promise((r) => setTimeout(r, 1200));
      const after = PM.EngineRender.compatStats;
      return { engine: pixels(engine), today: pixels(today), wrapped: PM.RasterCompat.wrapped(), asked: after.needs - before.needs, rasters: after.rasters - before.rasters, contact: contact.toDataURL() };
    });
    await mkdir(EVIDENCE, { recursive: true });
    const evidence = path.join(EVIDENCE, 'legacy-raster-assign-engine-left-today-right.png');
    await writeFile(evidence, Buffer.from(result.contact.split(',')[1]!, 'base64'));
    await testInfo.attach('engine-vs-today', { path: evidence, contentType: 'image/png' });
    const at = (d: number[], x: number, y: number) => d.slice((y * 480 + x) * 4, (y * 480 + x) * 4 + 3);
    const whole = measure(result.engine, result.today);
    // The shifted text's band and the tinted ellipse, apart.
    const band = (from: number, to: number) => measure(result.engine.slice(from * 480 * 4, to * 480 * 4), result.today.slice(from * 480 * 4, to * 480 * 4));
    console.log(JSON.stringify({ whole, top: band(40, 140), bottom: band(160, 240), asked: result.asked, rasters: result.rasters, tint: at(result.engine, 110, 90), evidence }));
    expect(result.wrapped).toBe(true);
    // The assigned wrapper's red ellipse, in the engine's frame and today's.
    expect(at(result.engine, 110, 90)).toEqual([232, 32, 42]);
    expect(at(result.today, 110, 90)).toEqual([232, 32, 42]);
    expect(result.asked).toBeGreaterThan(0);
    expect(withinSection25(band(160, 240)), 'the plain rectangle').toBe(true);
    expect(withinSection25(whole), `the whole frame vs today (${evidence})`).toBe(true);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('a raster override at a deep viewer zoom gets today\'s crop window, and the viewer shows today\'s frame', async ({ session }, testInfo) => {
    test.setTimeout(240_000);
    const ids = ['typewriter-effect', 'crop-recorder'];
    const ZOOM = 16, TIME = 1.2;
    const run = async (page: Page, engine: boolean) => {
      await waitActive(page, ids);
      await bigTypewriterProject(page);
      await page.evaluate(() => (window as any).__installCropRecorder());
      if (engine) await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
      // The calls of the zoomed viewer's frames only (a raster density
      // above the fitted view's).
      await page.evaluate(() => { (window as any).__crops.length = 0; });
      const view = await zoomViewer(page, ZOOM, 200, 120, TIME);
      const frame = await viewerPixels(page, engine);
      const crops = await page.evaluate(() => (window as any).__crops.filter((c: any) => c.layer === 'typed' && c.scale > 4));
      const state = await page.evaluate(() => {
        const PM = (window as any).PM, V = PM.Kernel.services.get('viewer');
        const all = (window as any).__crops as Array<{ layer: string; scale: number; crop: unknown }>;
        return { zoom: V.zoom, fit: V.fit, shown: V.shown, viewport: PM.GL?.previewViewport ?? null, quality: PM.quality, scales: [...new Set(all.map((c) => `${c.layer}@${c.scale}${c.crop ? '+crop' : ''}`))], time: PM.time, presenting: !!V.enginePresentation?.presenting };
      });
      console.log(JSON.stringify({ engine, state }));
      const errors = await page.evaluate(() => (window as any).PM.EngineRender?.compatStats ?? null);
      return { view, frame, crops, errors };
    };
    await install(session.userData, ids);
    await session.relaunch();
    await session.openEditor();
    const engine = await run(session.page, true);
    expect(session.diagnostics.pageErrors).toEqual([]);
    await session.close();
    const off = await todayApp(ids);
    let today: Awaited<ReturnType<typeof run>>;
    try {
      today = await run(off.page, false);
      expect(off.diagnostics.pageErrors).toEqual([]);
    } finally {
      await off.close();
      await rm(off.userData, { recursive: true, force: true });
    }
    const key = (c: any) => JSON.stringify([c.scale, c.crop]);
    const engineCrops = [...new Set(engine.crops.map(key))].sort();
    const todayCrops = [...new Set(today.crops.map(key))].sort();
    await mkdir(EVIDENCE, { recursive: true });
    const evidence = { engine: path.join(EVIDENCE, 'typewriter-zoom16-engine.png'), today: path.join(EVIDENCE, 'typewriter-zoom16-today.png') };
    await writeFile(evidence.engine, Buffer.from(engine.frame.png.split(',')[1]!, 'base64'));
    await writeFile(evidence.today, Buffer.from(today.frame.png.split(',')[1]!, 'base64'));
    await testInfo.attach('engine', { path: evidence.engine, contentType: 'image/png' });
    await testInfo.attach('today', { path: evidence.today, contentType: 'image/png' });
    const m = measure(engine.frame.data, today.frame.data);
    console.log(JSON.stringify({ view: [engine.view, today.view], size: [[engine.frame.width, engine.frame.height], [today.frame.width, today.frame.height]], engineCrops, todayCrops, measure: m, evidence }));
    // Today's compositor asked for a window of the typed layer's raster at this zoom.
    expect(todayCrops.some((k) => !k.endsWith(',null]'))).toBe(true);
    // The engine called the override with the same density and windows.
    expect(engineCrops).toEqual(todayCrops);
    // And the viewer shows today's frame.
    expect([engine.frame.width, engine.frame.height]).toEqual([today.frame.width, today.frame.height]);
    expect(withinSection25(m), `viewer frame vs today (${evidence.engine} / ${evidence.today})`).toBe(true);
  });

  test('a drawing into render.gl.context is in a WebM export as today, not in an MP4 one, and context.canvas reads it', async ({ session }) => {
    test.setTimeout(300_000);
    const ids = ['context-watermark'];
    const W = 320, H = 180;
    const exportAs = async (app: ElectronApplication, page: Page, format: string, file: string) => {
      await stubSavePanel(app, file);
      return page.evaluate(async (format) => {
        const PM = (window as any).PM;
        const before = PM.EngineRender?.compatStats?.exportAsks ?? 0;
        const result = await PM.Export.run({ format, scale: 1, fps: 30, range: 'all', quality: 'high', mblur: false, audio: false, alpha: false });
        return { result, exportAsks: (PM.EngineRender?.compatStats?.exportAsks ?? 0) - before };
      }, format);
    };
    const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-watermark-'));
    try {
      // The engine.
      await install(session.userData, ids);
      await session.relaunch();
      await session.openEditor();
      await waitActive(session.page, ids);
      await watermarkProject(session.page);
      await expect.poll(() => session.page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
      await session.page.evaluate(() => new Promise((r) => setTimeout(r, 1000)));
      // A fresh frame for the reads (the playhead moves one frame).
      await session.page.evaluate(() => { const PM = (window as any).PM; PM.setTime(Math.round(PM.time * 30 + 1) / 30); });
      await session.page.evaluate(() => new Promise((r) => setTimeout(r, 800)));
      const engineReads = await session.page.evaluate(() => (window as any).__watermark.reads.slice(-2));
      const engineStats = await session.page.evaluate(() => ({ ...(window as any).PM.CompatContext.stats, sources: (window as any).PM.EngineRender.exportSources('webm') }));
      console.log(JSON.stringify({ engineStats }));
      const engineWebm = await exportAs(session.app, session.page, 'webm', path.join(root, 'engine.webm'));
      const engineMp4 = await exportAs(session.app, session.page, 'mp4', path.join(root, 'engine.mp4'));
      expect(session.diagnostics.pageErrors).toEqual([]);
      await session.close();
      // Today.
      const off = await todayApp(ids);
      let todayReads: unknown, todayWebm: unknown, todayMp4: unknown;
      try {
        await watermarkProject(off.page);
        await off.page.evaluate(() => new Promise((r) => setTimeout(r, 1000)));
        await off.page.evaluate(() => { const PM = (window as any).PM; PM.setTime(Math.round(PM.time * 30 + 1) / 30); });
        await off.page.evaluate(() => new Promise((r) => setTimeout(r, 800)));
        todayReads = await off.page.evaluate(() => (window as any).__watermark.reads.slice(-2));
        todayWebm = await exportAs(off.app, off.page, 'webm', path.join(root, 'today.webm'));
        todayMp4 = await exportAs(off.app, off.page, 'mp4', path.join(root, 'today.mp4'));
      } finally {
        await off.close();
        await rm(off.userData, { recursive: true, force: true });
      }
      const files = await readdir(root);
      const frames = Object.fromEntries(['engine.webm', 'today.webm', 'engine.mp4', 'today.mp4'].map((f) => [f, files.includes(f) ? videoFrames(path.join(root, f), W, H) : []]));
      const marks = Object.fromEntries(Object.entries(frames).map(([f, list]) => [f, list.length ? [mean(list[0]!, W, 12, 12, 40, 16), mean(list[list.length - 1]!, W, 12, 12, 40, 16)] : null]));
      const box = Object.fromEntries(Object.entries(frames).map(([f, list]) => [f, list.length ? mean(list[0]!, W, 160, 90, 40, 40) : null]));
      console.log(JSON.stringify({ engineWebm, engineMp4, todayWebm, todayMp4, counts: Object.fromEntries(Object.entries(frames).map(([f, l]) => [f, l.length])), marks, box, engineReads, todayReads }));
      expect((engineWebm as any).result).toEqual({ cancelled: false });
      expect((todayWebm as any).result).toEqual({ cancelled: false });
      // Every WebM frame went through the window's compatibility context.
      expect((engineWebm as any).exportAsks).toBeGreaterThanOrEqual(30);
      expect((engineMp4 as any).exportAsks).toBe(0);
      expect(frames['engine.webm']!.length).toBe(frames['today.webm']!.length);
      const red = (c: number[] | undefined) => !!c && c[0]! > 200 && c[1]! < 70 && c[2]! < 80;
      // The shipped app (470eeaf3) encodes WebM frames from the compositor's
      // canvas after frame:rendered (exportWebCodecs, renderInto), so the
      // mark is in its WebM exports, not its MP4 ones (exportNative reads
      // GL.renderToPixels). This branch's flag-off exporter passes
      // `exporting: true` (unreleased aad31a41) and drops it: reported, not
      // the reference.
      for (const c of marks['engine.webm']!) expect(red(c), 'engine.webm: the mark, as the shipped app').toBe(true);
      for (const f of ['engine.mp4', 'today.mp4']) if (marks[f]) for (const c of marks[f]!) expect(red(c), `${f}: no mark`).toBe(false);
      // The rest of the frame is the composition in both (within the codecs' error).
      for (const f of Object.keys(box)) if (box[f]) expect(Math.max(...box[f]!.map((v, i) => Math.abs(v - [0x33, 0xCC, 0x66][i]!))), `${f}: the box`).toBeLessThanOrEqual(12);
      // drawImage(context.canvas) reads the mark and the frame under it, as today.
      const pick = (reads: any) => reads?.[reads.length - 1];
      const e = pick(engineReads), t = pick(todayReads);
      expect(e?.mark?.slice(0, 3)).toEqual(t?.mark?.slice(0, 3));
      expect(Math.max(...(e?.center ?? [999]).slice(0, 3).map((v: number, i: number) => Math.abs(v - t.center[i])))).toBeLessThanOrEqual(1);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});
