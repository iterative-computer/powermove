/*
 * The customization moat with "Use New Engine" on, round two (owner
 * amendments 23 and 24; docs/engine/extensions-compat.md):
 *
 * - A raster wrapper assigned to `PM.raster` through `api.host.pm` (the
 *   pre-services pattern) draws on the engine as today.
 * - A raster override at a deep viewer zoom gets today's preview crop
 *   window (the typewriter fixture), presents the requested time, and
 *   keeps advancing at densities 24 and 32.
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
import { TICKS_PER_SECOND } from '../src/shared/frame-rate';
import { previewTextRaster } from '../src/renderer/src/legacy/gl/shape-raster-window';

const ON = { POWERMOVE_NEW_ENGINE: '1' };
/** Side-by-sides go outside the repository (no screenshots in Git). */
const EVIDENCE = path.join(os.tmpdir(), 'ne-extcompat-evidence');

const FIXTURES: Record<string, string> = {
  'legacy-raster-assign': 'fixtures/compat-corpus/legacy-raster-assign',
  'context-watermark': 'fixtures/compat-corpus/context-watermark',
  'crop-recorder': 'fixtures/compat-corpus/crop-recorder',
  'label-substitution': 'fixtures/compat-corpus/label-substitution',
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
    const plain = PM.mkLayer('text', { name: 'Plain', from: 0, dur: 3, d: { ...typed.d, text: 'Plain sample', color: '#3366CC' } }, project);
    plain.id = 'plain';
    delete plain.d.fontAnchorBounds; delete plain.d.sizeAnchorBounds;
    for (const key of ['position.x', 'position.y', 'scale.x', 'scale.y']) plain.p[key].v = typed.p[key].v;
    project.layers = [typed, plain];
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
    // The current built-in UI clamps its menu to 8x. Exercise the engine's
    // 16x view contract explicitly as well (custom viewers may request it).
    if (zoom > 8 && V.enginePresentation) {
      // Keep the custom-viewer request through subsequent playhead updates.
      const update = V.enginePresentation.update.bind(V.enginePresentation);
      V.enginePresentation.update = (params: any, stage: any) => update({ ...params, zoom, pan: V.pan, playEveryFrame: false }, stage);
      V.enginePresentation.update({ zoom, pan: V.pan, resolution: 'full', media: 'originals', playEveryFrame: false });
    }
    PM.invalidate?.('render');
    return { stage: [V._sw, V._sh], shown: V.enginePresentation?.state?.zoom ?? V.shown };
  }, { zoom, x, y, time });
}

/** The viewer's frame as pixels: the canvas showing it (the engine's presentation, or today's compositor canvas). */
async function viewerPixels(page: Page, engine: boolean): Promise<{ width: number; height: number; data: Buffer; png: string }> {
  const result = await page.evaluate(async (engine) => {
    const PM = (window as any).PM;
    const settle = async () => { for (let i = 0; i < 2; i++) await new Promise((r) => requestAnimationFrame(r)); await new Promise((r) => setTimeout(r, 600)); };
    await settle();
    const source = engine
      ? (Array.from(document.querySelectorAll('canvas#engine-present')) as HTMLCanvasElement[]).find((c) => c.style.visibility === 'visible' && c.width > 0)!
      : (document.querySelector('canvas#gl') as HTMLCanvasElement | null) ?? PM.GL.canvas as HTMLCanvasElement;
    const c = document.createElement('canvas');
    c.width = source.width; c.height = source.height;
    const c2 = c.getContext('2d')!;
    // Read the exact surface the presenter selected, preserved before its
    // pool slot was released. The WebGPU swapchain clears after compositing.
    // At the custom 16x request the built-in's legacy bound canvas still has
    // its 8x layout, so that canvas is not the engine presentation's viewport.
    const tag = engine ? PM.Kernel.services.get('viewer').enginePresentation.latest : null;
    const pixels = engine ? PM.CompatContext.state?.copies.get(`${tag.revision}:${tag.viewSeq}:${tag.seq}`) : source;
    if (!pixels) throw new Error('The presented engine surface was not preserved');
    if (pixels.width !== c.width || pixels.height !== c.height) throw new Error(`Presented surface ${pixels.width}x${pixels.height} differs from canvas ${c.width}x${c.height}`);
    c2.drawImage(pixels, 0, 0, c.width, c.height);
    const bytes = c2.getImageData(0, 0, c.width, c.height).data;
    let raw = ''; for (let i = 0; i < bytes.length; i += 32768) raw += String.fromCharCode(...bytes.subarray(i, i + 32768));
    return { width: c.width, height: c.height, rgba: btoa(raw), png: c.toDataURL('image/png') };
  }, engine);
  return { width: result.width, height: result.height, data: Buffer.from(result.rgba, 'base64'), png: result.png };
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

  test('a wrapper that substitutes text keeps its own raster geometry and pixels', async ({ session }) => {
    await install(session.userData, ['label-substitution']);
    await session.relaunch(); await session.openEditor();
    await waitActive(session.page, ['label-substitution']);
    const rows = await session.page.evaluate(async () => {
      const PM = (window as any).PM, api = PM.Kernel.api('label-probe');
      const p = PM.mkProject({ w: 480, h: 270, dur: 2, fps: 30, bg: '#203040' });
      const layer = PM.mkLayer('text', { from: 0, dur: 2, d: { text: 'MESSAGE', font: 'Helvetica', size: 36, color: '#FFFFFF' } }, p);
      layer.id = 'dynamic-label'; p.layers = [layer]; PM.replaceProject(p); PM.Projects.put(PM.proj);
      const geometry = (r: any) => [r.w, r.h, r.anchorX, r.anchorY, r.selection, r.cv.width, r.cv.height];
      const pixels = (c: HTMLCanvasElement) => c.getContext('2d')!.getImageData(0, 0, c.width, c.height).data;
      const rows = [];
      for (const time of [.2, 1.2]) {
        PM.pause(); PM.setTime(time, { force: true });
        await PM.prepareFrame?.(time); await PM.HostMedia?.flushBindings?.();
        const live = PM.L('dynamic-label');
        const changed = { ...live, id: 'detached-label', d: { ...api.anim.resolveContent(live, time), text: time < 1 ? 'I' : 'Wide label' } };
        const reference = PM.__rasterBase(changed, 1, time);
        const sync = api.render.raster(live, 1, time);
        const asyncSurface = await api.render.rasterAsync(live, 1, time);
        const engine = api.render.renderFrameTo(time, 480, 270, { mblur: false });
        const today = PM.renderFrameTo(time, 480, 270, { mblur: false });
        const a = pixels(engine), b = pixels(today);
        let max = 0, over1 = 0, values = 0;
        for (let i = 0; i < a.length; i++) if (i % 4 !== 3) { const d = Math.abs(a[i]! - b[i]!); max = Math.max(max, d); over1 += Number(d > 1); values++; }
        rows.push({ time, sync: geometry(sync), async: geometry(asyncSurface), today: geometry(reference), max, over1, values, enginePng: engine.toDataURL(), todayPng: today.toDataURL() });
      }
      return rows;
    });
    await mkdir(EVIDENCE, { recursive: true });
    for (const row of rows) {
      await writeFile(path.join(EVIDENCE, `substituted-label-${row.time}-engine.png`), Buffer.from(row.enginePng.split(',')[1]!, 'base64'));
      await writeFile(path.join(EVIDENCE, `substituted-label-${row.time}-today.png`), Buffer.from(row.todayPng.split(',')[1]!, 'base64'));
    }
    console.log(JSON.stringify({ substitutedLabel: rows.map(({ enginePng, todayPng, ...row }) => row) }));
    for (const row of rows) {
      expect(row.sync).toEqual(row.today); expect(row.async).toEqual(row.today);
      expect(row.max).toBeLessThanOrEqual(4); expect(row.over1).toBeLessThanOrEqual(row.values * .001);
    }
  });

  test('a raster override advances at zoom 8 and 16 and receives today\'s planned crop windows', async ({ session }) => {
    test.setTimeout(180_000);
    const ids = ['typewriter-effect', 'crop-recorder'];
    await install(session.userData, ids);
    await session.relaunch();
    await session.openEditor();
    const page = session.page;
    await waitActive(page, ids);
    await bigTypewriterProject(page);
    await page.evaluate(() => { (window as any).__installCropRecorder(); void (window as any).PM.Kernel.api('zoom-probe').render.gl.context; });
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    // A hidden legacy window does not enter preview-viewport mode. Gate
    // the engine on its own against the legacy crop function (the same
    // function ported by pm_render::compat_crop), without showing/focusing
    // an ordinary window on the owner's desktop.
    await mkdir(EVIDENCE, { recursive: true });
    const presentedAt = async (zoom: number, time: number) => {
      try {
        await expect.poll(() => page.evaluate(() => {
          const view = (window as any).PM.Kernel.services.get('viewer').enginePresentation;
          return [view.latest?.pts, view.latest?.viewSeq === Number(view.state.viewSeq)];
        })).toEqual([Math.round(time * TICKS_PER_SECOND), true]);
      } catch (error) {
        const state = await page.evaluate(() => {
          const PM = (window as any).PM, view = PM.Kernel.services.get('viewer').enginePresentation;
          return { time: PM.time, latest: view.latest, view: view.state, presenter: view.stats(), status: PM.EngineConnection.status(), crops: (window as any).__crops };
        });
        console.log(JSON.stringify({ staleZoom: zoom, requestedTime: time, state }));
        const stale = await viewerPixels(page, true);
        await writeFile(path.join(EVIDENCE, `zoom-${zoom}-stale-at-${time}.png`), Buffer.from(stale.png.split(',')[1]!, 'base64'));
        throw error;
      }
    };
    for (const zoom of [8, 16]) {
      const before = await page.evaluate(() => (window as any).PM.EngineConnection.status().frames);
      await page.evaluate(() => { (window as any).__crops.length = 0; });
      // Keep the glyph bodies inside the much smaller 16x field of view.
      // Centring on the baseline's top at y=120 legitimately clips every
      // visible glyph at t=.2 (the i dot and cursor are outside its x range).
      await zoomViewer(page, zoom, 200, 145, .6);
      await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().frames), { timeout: 20_000 }).toBeGreaterThan(before);
      await expect.poll(() => page.evaluate(() => (window as any).__crops.filter((c: any) => c.layer === 'typed' && c.scale > 16).length)).toBeGreaterThan(0);
      await viewerPixels(page, true); // let setZoom's intermediate pan state settle
      await page.evaluate(() => { (window as any).__crops.length = 0; const PM = (window as any).PM; PM.setTime(.2, { force: true }); });
      await expect.poll(() => page.evaluate(() => (window as any).__crops.filter((c: any) => c.layer === 'plain' && c.scale > 16).length)).toBeGreaterThan(0);
      await presentedAt(zoom, .2);
      const first = await viewerPixels(page, true);
      await writeFile(path.join(EVIDENCE, `zoom-${zoom}-time-0.2.png`), Buffer.from(first.png.split(',')[1]!, 'base64'));
      const plan = await page.evaluate(() => {
        const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer');
        const state = viewer.enginePresentation.state;
        const crops = (window as any).__crops.filter((c: any) => c.layer === 'plain' && c.scale > 16);
        const typed = (window as any).__crops.filter((c: any) => c.layer === 'typed' && c.scale > 16);
        return { state, latest: viewer.enginePresentation.latest, crops, typed, world: PM.worldMatrix(PM.L('plain'), PM.time),
          geometry: PM.textRasterGeometry(PM.L('plain'), crops[crops.length - 1].scale, PM.time),
          size: [PM.proj.w, PM.proj.h], failures: PM.EngineConnection.status().failures };
      });
      console.log(JSON.stringify({ zoomPlan: plan }));
      expect(first.data.some((v, i) => i % 4 !== 3 && v > 180), 'the presented frame contains text pixels').toBe(true);
      const { state, size, world } = plan;
      expect(state.zoom, 'the requested engine zoom was reached').toBe(zoom);
      expect(plan.crops.every((c: any) => c.scale === Math.min(32, zoom * 3))).toBe(true);
      const resolution = state.resolution === 'half' ? .5 : state.resolution === 'quarter' ? .25 : 1;
      const full = size.map((n: number) => Math.max(1, n * state.zoom * state.devicePixelRatio));
      // Rust f64::round (unlike Math.round for negative half-integers).
      const round = (n: number) => Math.sign(n) * Math.floor(Math.abs(n) + .5);
      const output = full.map((n: number) => Math.min(16384, Math.max(1, round(n * resolution))));
      const origin = full.map((n: number, i: number) => round(((i ? state.size.height : state.size.width) - n) / 2 + state.pan[i] * state.devicePixelRatio));
      const lo = origin.map((n: number, i: number) => Math.min(output[i]!, Math.max(0, Math.floor(-n * resolution))));
      const hi = origin.map((n: number, i: number) => Math.min(output[i]!, Math.max(0, Math.ceil(((i ? state.size.height : state.size.width) - n) * resolution))));
      const sx = output[0]! / size[0], sy = output[1]! / size[1];
      const visible = [world[0] * sx, world[1] * sy, world[2] * sx, world[3] * sy, world[4] * sx - lo[0]!, world[5] * sy - lo[1]!];
      const crop = previewTextRaster(plan.geometry, visible, hi[0]! - lo[0]!, hi[1]! - lo[1]!) ?? null;
      expect(crop, `zoom ${zoom}: a cropped source is exercised`).not.toBeNull();
      for (const called of plan.crops) expect(called.crop, `zoom ${zoom}, density ${called.scale}`).toEqual(crop);
      // Today's typewriter definition is a renderable effect, so it keeps the full source.
      for (const called of plan.typed) expect(called.crop).toBeNull();
      expect(plan.failures).toEqual([]);
      const frameBeforeSeek = await page.evaluate(() => (window as any).PM.EngineConnection.status().frames);
      await page.evaluate(() => { const PM = (window as any).PM; PM.setTime(1.7, { force: true }); });
      await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().frames)).toBeGreaterThan(frameBeforeSeek);
      await presentedAt(zoom, 1.7);
      const second = await viewerPixels(page, true);
      await writeFile(path.join(EVIDENCE, `zoom-${zoom}-time-0.2.png`), Buffer.from(first.png.split(',')[1]!, 'base64'));
      await writeFile(path.join(EVIDENCE, `zoom-${zoom}-time-1.7.png`), Buffer.from(second.png.split(',')[1]!, 'base64'));
      expect(first.data.some((v, i) => v !== second.data[i]), `zoom ${zoom}: the typewriter advances after seeking`).toBe(true);
      console.log(JSON.stringify({ zoom, frames: [before, frameBeforeSeek, await page.evaluate(() => (window as any).PM.EngineConnection.status().frames)], scales: [...new Set(plan.crops.map((c: any) => c.scale))], crop }));
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('render.raster at any scale returns today\'s surface and pixels synchronously and never blanks the viewer', async ({ session }) => {
    test.setTimeout(180_000);
    await session.openEditor();
    await assignProject(session.page);
    await expect.poll(() => session.page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    const stage = () => session.page.evaluate(() => ({
      status: document.querySelector('[data-engine-status]')?.textContent ?? '',
      shown: (Array.from(document.querySelectorAll('canvas#engine-present')) as HTMLCanvasElement[]).some((c) => c.width > 0 && getComputedStyle(c).visibility !== 'hidden'),
      failures: (window as any).PM.EngineConnection.status().failures as string[]
    }));
    await expect.poll(async () => (await stage()).shown, { timeout: 30_000 }).toBe(true);
    const before = await stage();
    const results = await session.page.evaluate(() => {
      const PM = (window as any).PM, api = PM.Kernel.api('corpus');
      // Today's `PM.raster` has no upper bound on scale: the density is
      // min(scale, 8192 / w, 8192 / h). A small box reaches scale 50
      // without the cap; a thin bar meets the 8192-pixel edge at any scale
      // past 8192 / 208 without a 200 MB canvas.
      const box = PM.mkLayer('shape', { d: { shape: 'rect', w: 16, h: 12, color: '#33CC66', stroke: 0 } }, PM.proj);
      const bar = PM.mkLayer('shape', { d: { shape: 'rect', w: 200, h: 2, color: '#3366CC', stroke: 0 } }, PM.proj);
      const fields = (r: any) => r && [r.w, r.h, r.anchorX, r.anchorY, r.cv.width, r.cv.height, r.selection];
      const bytes = (r: any) => r.cv.width && r.cv.height ? r.cv.getContext('2d').getImageData(0, 0, r.cv.width, r.cv.height).data : new Uint8ClampedArray(0);
      const result = [];
      for (const [layer, scale] of [[box, 16.25], [box, 24], [box, 32], [box, 33], [box, 40], [box, 50], [bar, 1000], [bar, Infinity], [box, 0.005], [box, 0], [box, -1]] as Array<[any, number]>) {
        const today = PM.__rasterBase(layer, scale, 0);
        const started = performance.now();
        const engine = api.render.raster(layer, scale, 0);
        const ms = performance.now() - started;
        let max = 0, todayInk = 0, engineInk = 0;
        if (today?.cv && engine?.cv && today.cv.width === engine.cv.width && today.cv.height === engine.cv.height) {
          const a = bytes(today), b = bytes(engine);
          for (let i = 0; i < a.length; i++) max = Math.max(max, Math.abs(a[i] - b[i]));
          for (let i = 3; i < a.length; i += 4) { todayInk += Number(a[i] > 0); engineInk += Number(b[i] > 0); }
        } else max = Infinity;
        result.push({ layer: layer === box ? 'box' : 'bar', scale: String(scale), sync: !!engine && !(engine instanceof Promise), today: fields(today), engine: fields(engine), max, todayInk, engineInk, ms: Math.round(ms) });
      }
      return result;
    });
    console.log(JSON.stringify({ anyScaleRasters: results }));
    for (const result of results) {
      expect(result.sync, `${result.layer} at ${result.scale}`).toBe(true);
      expect(result.engine, `${result.layer} at ${result.scale}`).toEqual(result.today);
      expect(result.max, `${result.layer} at ${result.scale}: pixels`).toBe(0);
    }
    // No request failed, and the viewer kept its frame on the stage.
    const after = await stage();
    expect(after.failures).toEqual(before.failures);
    expect(after.status).toBe('');
    expect(after.shown).toBe(true);
    expect(session.diagnostics.pageErrors).toEqual([]);
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
