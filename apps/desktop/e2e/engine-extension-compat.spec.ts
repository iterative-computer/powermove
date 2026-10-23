/*
 * The customization moat with "Use New Engine" on (owner amendments 23 and
 * 24; docs/engine/extensions-compat.md). Extensions written against today's
 * API keep working unchanged on the engine:
 *
 * - The typewriter fixture (e2e/fixtures/compat-typewriter), an apiVersion
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
import { cp, mkdir, mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import type { ElectronApplication, Page } from 'playwright';

import { expect, launchApp, test } from './helpers/app';
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

/** A 320×180 composition with a rotating ellipse the shape-tint fixture
 *  tints, and a plain rectangle, over #203040. */
async function shapeTintProject(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Shape tint export', w: 320, h: 180, fps: 30, dur: 3, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    const tinted = PM.mkLayer('shape', { name: 'Tinted', from: 0, dur: 3, d: { shape: 'ellipse', w: 120, h: 80, color: '#33CC66', stroke: 0 } }, project);
    tinted.id = 'tinted';
    tinted.p['position.x'].v = 100; tinted.p['position.y'].v = 90;
    tinted.p['rotation'] = PM.P(0, { kf: [PM.KF(0, 0), PM.KF(3, 90)] });
    const plain = PM.mkLayer('shape', { name: 'Plain', from: 0, dur: 3, d: { shape: 'rect', w: 80, h: 60, color: '#3366CC', stroke: 0 } }, project);
    plain.id = 'plain';
    plain.p['position.x'].v = 230; plain.p['position.y'].v = 90;
    project.layers = [tinted, plain];
    PM.replaceProject(project);
    PM.Projects?.put?.(PM.proj);
  });
}

/** A 640×360 composition for playback cadence: a typewriter text whose
 *  progress is keyed 0 → 100 over the whole 3 s, so the typed prefix (the
 *  detached layer the fixture rasterizes through the service) changes
 *  every frame, and a plain text, over #203040. */
async function typewriterCadenceProject(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Typewriter cadence', w: 640, h: 360, fps: 30, dur: 3, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    const typed = PM.mkLayer('text', { name: 'Typed', from: 0, dur: 3, d: { text: 'Every frame reveals another letter while the engine prepares the next changing text base asynchronously, ahead of the playhead.', font: 'Helvetica', size: 20, weight: 700, color: '#FFFFFF' } }, project);
    typed.id = 'typed';
    typed.p['position.x'].v = 320; typed.p['position.y'].v = 140;
    const fx = PM.mkEffect('typewriter');
    fx.p.progress = PM.P(0, { kf: [PM.KF(0, 0), PM.KF(3, 100)] });
    fx.p.cursor.v = true;
    typed.fx = [fx];
    const plain = PM.mkLayer('text', { name: 'Plain', from: 0, dur: 3, d: { text: 'Unaffected', font: 'Helvetica', size: 32, weight: 400, color: '#FFCC00' } }, project);
    plain.id = 'plain';
    plain.p['position.x'].v = 320; plain.p['position.y'].v = 260;
    project.layers = [typed, plain];
    PM.replaceProject(project);
    PM.Projects?.put?.(PM.proj);
    const api = PM.Kernel.api('changing-base-proof');
    // Match the unmodified fixture's typedCount (floor plus its epsilon).
    const counts = Array.from({ length: 90 }, (_, i) => Math.floor(String(typed.d.text).length * api.anim.evP(PM.proj.layers[0], PM.proj.layers[0].fx[0].p.progress, i / 30, `${fx.id}.progress`) / 100 + 1e-8));
    if (new Set(counts).size !== 90) throw new Error('Typewriter cadence requires 90 different typed prefixes.');
  });
}

/** The shape-tint project with the tinted ellipse's size keyed instead of
 *  its rotation (60×40 → 200×120 over 3 s): its base pixels change every
 *  frame, so no frame can reuse an earlier frame's base. */
async function shapeSizeProject(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Shape size cadence', w: 320, h: 180, fps: 30, dur: 3, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    const tinted = PM.mkLayer('shape', { name: 'Tinted', from: 0, dur: 3, d: { shape: 'ellipse', w: 120, h: 80, color: '#33CC66', stroke: 0 } }, project);
    tinted.id = 'tinted';
    tinted.d.w = PM.P(60, { kf: [PM.KF(0, 60), PM.KF(3, 200)] });
    tinted.d.h = PM.P(40, { kf: [PM.KF(0, 40), PM.KF(3, 120)] });
    tinted.p['position.x'].v = 120; tinted.p['position.y'].v = 90;
    const plain = PM.mkLayer('shape', { name: 'Plain', from: 0, dur: 3, d: { shape: 'rect', w: 80, h: 60, color: '#3366CC', stroke: 0 } }, project);
    plain.id = 'plain';
    plain.p['position.x'].v = 260; plain.p['position.y'].v = 90;
    project.layers = [tinted, plain];
    PM.replaceProject(project);
    PM.Projects?.put?.(PM.proj);
    const api = PM.Kernel.api('changing-base-proof');
    const sizes = Array.from({ length: 90 }, (_, i) => { const d = api.anim.resolveContent(PM.proj.layers[0], i / 30); return JSON.stringify([d.w, d.h]); });
    if (new Set(sizes).size !== 90) throw new Error('Shape cadence requires 90 different base sizes.');
  });
}

/** A 10 s 1280×720 loop whose overridden ellipse nearly fills the frame
 *  and changes size on every frame: 300 different rasters of about a
 *  viewer's size, more than the engine's compatibility store keeps
 *  (256 MiB), so the frames after the loop's wrap were evicted by the time
 *  playback wraps to them. */
async function shapeLongLoopProject(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Shape long loop', w: 1280, h: 720, fps: 30, dur: 10, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    const tinted = PM.mkLayer('shape', { name: 'Tinted', from: 0, dur: 10, d: { shape: 'ellipse', w: 1100, h: 600, color: '#33CC66', stroke: 0 } }, project);
    tinted.id = 'tinted';
    tinted.d.w = PM.P(1100, { kf: [PM.KF(0, 1100), PM.KF(10, 1260)] });
    tinted.d.h = PM.P(600, { kf: [PM.KF(0, 600), PM.KF(10, 710)] });
    tinted.p['position.x'].v = 640; tinted.p['position.y'].v = 360;
    project.layers = [tinted];
    PM.replaceProject(project);
    PM.Projects?.put?.(PM.proj);
    PM.loop = true;
    const api = PM.Kernel.api('changing-base-proof');
    const sizes = Array.from({ length: 300 }, (_, i) => { const d = api.anim.resolveContent(PM.proj.layers[0], i / 30); return JSON.stringify([d.w, d.h]); });
    if (new Set(sizes).size !== 300) throw new Error('The long loop requires 300 different base sizes.');
  });
}

/** The shape-tint project in a large document: `count` more shape layers
 *  (switched off, each with keyframes), so the document is megabytes of
 *  JSON, as large projects are. Returns the document's JSON size. */
async function largeDocumentProject(page: Page, count: number): Promise<number> {
  return page.evaluate((count) => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Large document', w: 640, h: 360, fps: 30, dur: 10, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    const tinted = PM.mkLayer('shape', { name: 'Tinted', from: 0, dur: 10, d: { shape: 'ellipse', w: 240, h: 160, color: '#33CC66', stroke: 0 } }, project);
    tinted.id = 'tinted';
    tinted.p['position.x'].v = 200; tinted.p['position.y'].v = 180;
    const others = Array.from({ length: count }, (_, i) => {
      const layer = PM.mkLayer('shape', { name: `Off ${i}`, from: 0, dur: 10, d: { shape: 'rect', w: 40 + (i % 50), h: 30, color: '#3366CC', stroke: 0 } }, project);
      layer.id = `off-${i}`;
      layer.on = false;
      layer.p['rotation'] = PM.P(0, { kf: [PM.KF(0, 0), PM.KF(5, i % 360), PM.KF(10, 0)] });
      layer.p['position.x'] = PM.P(0, { kf: [PM.KF(0, i % 640), PM.KF(10, (i * 7) % 640)] });
      return layer;
    });
    project.layers = [tinted, ...others];
    PM.replaceProject(project);
    PM.Projects?.put?.(PM.proj);
    return JSON.stringify(PM.proj).length;
  }, count);
}

/** A vector path whose vertex and tangent channels change on every frame. */
async function pathProject(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Path cadence', w: 320, h: 180, fps: 30, dur: 3, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    const channel = (v: any) => PM.P(v);
    const values = { x: 0, y: 0, rotation: 0, scaleX: 100, scaleY: 100, closed: true, fill: '#33CC66', fillEnabled: true, fillOpacity: 100, stroke: '#ffffff', strokeWidth: 2, strokeOpacity: 100, trimStart: 0, trimEnd: 100, trimOffset: 0, copies: 1, repeatX: 30, repeatY: 0, repeatRotation: 0 };
    const path = { id: 'outline', name: 'Animated outline', parent: null, p: Object.fromEntries(Object.entries(values).map(([k, v]) => [k, channel(v)])), vertices: [[-60, -40], [60, -40], [60, 40], [-60, 40]].map(([x, y], i) => ({ id: `v${i}`, p: Object.fromEntries(Object.entries({ x, y, inX: 0, inY: 0, outX: 0, outY: 0 }).map(([k, v]) => [k, channel(v)])) })) };
    path.vertices[1]!.p.x = PM.P(60, { kf: [PM.KF(0, 60), PM.KF(3, 100)] });
    path.vertices[1]!.p.outY = PM.P(0, { kf: [PM.KF(0, 0), PM.KF(3, 70)] });
    const tinted = PM.mkLayer('shape', { name: 'Tinted', from: 0, dur: 3, d: { paths: [path] } }, project);
    tinted.id = 'tinted';
    tinted.p['position.x'].v = 120; tinted.p['position.y'].v = 90;
    project.layers = [tinted];
    PM.replaceProject(project);
    PM.Projects?.put?.(PM.proj);
    const api = PM.Kernel.api('changing-base-proof');
    const layer = PM.proj.layers[0], vertexX = layer.d.paths[0].vertices[1].p.x;
    const vertices = Array.from({ length: 90 }, (_, i) => api.anim.evP(layer, vertexX, i / 30, 'g.outline.v.v1.x'));
    if (new Set(vertices).size !== 90) throw new Error('Path cadence requires 90 different vertex positions.');
  });
}

/** The fixtures' folder of each extension id the tests install. */
const FIXTURE: Record<string, string> = {
  'shape-tint': 'fixtures/compat-corpus/shape-tint',
  'compat-typewriter': 'fixtures/compat-typewriter'
};

/** Today's app: a fresh profile with `extension` installed and "Use New Engine" off. */
async function flagOffApp(extension: string | null) {
  const userData = await mkdtemp(path.join(os.tmpdir(), 'powermove-today-'));
  if (extension) await cp(path.join(__dirname, FIXTURE[extension]!), path.join(userData, 'extensions', extension), { recursive: true });
  try { return await launchApp({ userData, env: {} }); }
  catch (error) { await rm(userData, { recursive: true, force: true }); throw error; }
}

/** Today's exporter's PNG frames: the session closes, and today's app
 *  (a fresh profile with the same extension, "Use New Engine" off) sets the
 *  project up again and exports it the same way. Returns the frames' RGBA. */
async function todayPngExport(session: { userData: string; close(): Promise<void> }, extension: string, setup: (page: Page) => Promise<void>, folder: string, frames: number[]): Promise<Buffer[]> {
  await session.close();
  const off = await flagOffApp(extension);
  try {
    await off.openEditor();
    await off.page.waitForFunction((id) => (window as any).PM?.Kernel?.loader?.activeIds?.().includes(id), extension).catch(async (error) => {
      console.log('flag off, not active:', JSON.stringify(await off.page.evaluate((id) => ({ ids: (window as any).PM?.Kernel?.loader?.activeIds?.(), record: (window as any).PM?.Kernel?.api('corpus')?.extensions?.list?.().find((r: any) => r.id === id) }), extension).catch((e) => String(e))));
      throw error;
    });
    await setup(off.page);
    await stubSavePanel(off.app, folder, true);
    const result = await off.page.evaluate(async () => (window as any).PM.Export.run({ format: 'png', scale: 1, fps: 30, range: 'all', quality: 'high', mblur: false, audio: false, alpha: false }));
    expect(result).toEqual({ cancelled: false });
    expect(off.diagnostics.pageErrors).toEqual([]);
  } finally {
    await off.close();
    await rm(off.userData, { recursive: true, force: true });
  }
  const files = (await readdir(folder)).filter((f) => f.endsWith('.png')).sort();
  expect(files.length).toBe(90);
  return frames.map((f) => pngRgba(path.join(folder, files[f]!)));
}

/** The next save panel answers `file`. */
async function stubSavePanel(app: ElectronApplication, file: string, folder = false): Promise<void> {
  // Today's exporter picks an image sequence's folder (an existing one) with an open panel.
  if (folder) await mkdir(file, { recursive: true });
  await app.evaluate(({ dialog }, target) => {
    dialog.showSaveDialog = (async () => ({ canceled: false, filePath: target })) as typeof dialog.showSaveDialog;
    dialog.showOpenDialog = (async () => ({ canceled: false, filePaths: [target] })) as typeof dialog.showOpenDialog;
  }, file);
}

/** A PNG's pixels as 8-bit RGBA (through our FFmpeg). */
function pngRgba(file: string): Buffer {
  return execFileSync(ffmpegBinary, ['-v', 'error', '-i', file, '-f', 'rawvideo', '-pix_fmt', 'rgba', '-'], { maxBuffer: 64 * 1024 * 1024 });
}

test.describe('Use New Engine: existing extensions', () => {
  test.use({ desktopLaunchOptions: { env: ON } });

  test('a raster override (the typewriter fixture) is drawn by the engine as today, and disposing it restores the layer', async ({ session }, testInfo) => {
    await cp(path.join(__dirname, 'fixtures/compat-typewriter'), path.join(session.userData, 'extensions/compat-typewriter'), { recursive: true });
    await session.relaunch();
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => (window as any).PM?.Kernel?.loader?.activeIds?.().includes('compat-typewriter') && Boolean((window as any).PM?.EngineRender));
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
      /** Section 25's render measure over rows [from, to): the largest
       *  channel difference, and how many channel values differ by more
       *  than 1 (and by more than 8) of how many. */
      const diff = (a: Uint8ClampedArray, b: Uint8ClampedArray, from: number, to: number) => {
        let max = 0, over = 0, over1 = 0, values = 0;
        for (let y = from; y < to; y++) for (let x = 0; x < W; x++) {
          for (let c = 0; c < 3; c++) {
            const i = (y * W + x) * 4 + c;
            const d = Math.abs(a[i]! - b[i]!);
            values++;
            if (d > max) max = d;
            if (d > 1) over1++;
            if (d > 8) over++;
          }
        }
        return { max, over, over1, values };
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
      await PM.Kernel.loader.deactivate('compat-typewriter');
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
      expect(result.diffs[name].typed.max, `${name}: typed band`).toBeLessThanOrEqual(1);
    }
    // Viewer frames asked the producer for the frames they drew.
    expect(result.viewerAsked).toBeGreaterThan(0);
    // Disposed: the typed layer is the plain text again (full, no cursor), the plain layer as before.
    expect(result.restoredInk.plain).toBe(result.fullInk.plain);
    expect(result.restoredInk.typed).toBeGreaterThan(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
    // The plain layer the engine draws itself (no override) is today's too:
    // an existing project must not change (amendment 22; section 25: <= 1 CV
    // for 99.9% of values, <= 4 for any).
    for (const name of ['empty', 'partial', 'full']) {
      const plain = result.diffs[name].plain;
      expect(plain.over1, `${name}: unaffected text vs today`).toBeLessThanOrEqual(plain.values * 0.001);
      expect(plain.max, `${name}: unaffected text vs today`).toBeLessThanOrEqual(4);
      expect(result.ink[name].engine.plain, `${name}: unaffected text ink`).toBe(result.ink[name].today.plain);
    }
  });

  test('the typewriter project exports through the engine, the window drawing the override frame by frame', async ({ session }, testInfo) => {
    test.setTimeout(240_000);
    await cp(path.join(__dirname, 'fixtures/compat-typewriter'), path.join(session.userData, 'extensions/compat-typewriter'), { recursive: true });
    await session.relaunch();
    await session.openEditor();
    const { app, page } = session;
    await page.waitForFunction(() => (window as any).PM?.Kernel?.loader?.activeIds?.().includes('compat-typewriter') && Boolean((window as any).PM?.EngineRender));
    await typewriterProject(page);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-typewriter-export-'));
    try {
      const folder = path.join(root, 'Frames');
      await stubSavePanel(app, folder);
      const run = await page.evaluate(async () => {
        const PM = (window as any).PM;
        const before = PM.EngineRender.compatStats.exportAsks;
        PM.Export.lastRouted = null;
        const result = await PM.Export.run({ format: 'png', scale: 1, fps: 30, range: 'all', quality: 'high', mblur: false, audio: false, alpha: false });
        return { result, exportAsks: PM.EngineRender.compatStats.exportAsks - before, routed: PM.Export.lastRouted };
      });
      console.log(JSON.stringify(run));
      expect(run.result).toEqual({ cancelled: false });
      // The job engine drew it (amendment 21: never the current exporter),
      // asking this window for the typed layer at each frame.
      expect(run.routed, 'the engine refused the export (its reason)').toBeNull();
      expect(run.exportAsks, 'export asks (0: the export went to the current exporter)').toBeGreaterThanOrEqual(90);
      const files = (await readdir(folder)).filter((f) => f.endsWith('.png')).sort();
      expect(files.length).toBe(90);
      expect(session.diagnostics.pageErrors).toEqual([]);
      const frames = [0, 24, 75];
      const today = await todayPngExport(session, 'compat-typewriter', typewriterProject, path.join(root, 'Today'), frames);
      const diffs = frames.map((frame, i) => {
        const exported = pngRgba(path.join(folder, files[frame]!));
        let max = 0, over1 = 0, values = 0;
        for (let p = 0; p < 480 * 270 * 4; p += 4) for (let c = 0; c < 3; c++) {
          const d = Math.abs(exported[p + c]! - today[i]![p + c]!);
          values++;
          if (d > max) max = d;
          if (d > 1) over1++;
        }
        return { frame, max, over1, values };
      });
      console.log(JSON.stringify(diffs));
      for (const d of diffs) {
        expect(d.over1, `frame ${d.frame}: vs today's export`).toBeLessThanOrEqual(d.values * 0.001);
        expect(d.max, `frame ${d.frame}: vs today's export`).toBeLessThanOrEqual(4);
      }
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test('a render queue job with a raster override draws the project as queued, not as edited since', async ({ session }) => {
    test.setTimeout(240_000);
    await cp(path.join(__dirname, 'fixtures/compat-corpus/shape-tint'), path.join(session.userData, 'extensions/shape-tint'), { recursive: true });
    await session.relaunch();
    await session.openEditor();
    const { app, page } = session;
    await page.waitForFunction(() => (window as any).PM?.Kernel?.loader?.activeIds?.().includes('shape-tint') && Boolean((window as any).PM?.EngineRender));
    await shapeTintProject(page);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineRender.producing), { timeout: 10_000 }).toEqual(['rasterOverride']);
    const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-compat-queue-'));
    try {
      const folder = path.join(root, 'Queued');
      await stubSavePanel(app, folder);
      const frames = [0, 45, 89];
      // The export sheet's engine export at the queued state (held to today's
      // exporter by the export gate below), then an edit the job must not see.
      const sheetFolder = path.join(root, 'Sheet');
      await stubSavePanel(app, sheetFolder);
      expect(await page.evaluate(async () => (window as any).PM.Export.run({ format: 'png', scale: 1, fps: 30, range: 'all', quality: 'high', mblur: false, audio: false, alpha: false }))).toEqual({ cancelled: false });
      const sheetFiles = (await readdir(sheetFolder)).filter((f) => f.endsWith('.png')).sort();
      expect(sheetFiles.length).toBe(90);
      await stubSavePanel(app, folder);
      const today = await page.evaluate(async () => {
        const PM = (window as any).PM;
        const reference = null;
        const added = await PM.Export.addToQueue({ format: 'png', scale: 1, fps: 30, range: 'all', quality: 'high', mblur: false, audio: false, alpha: false });
        const tinted = PM.proj.layers.find((l: any) => l.id === 'tinted');
        // Its raster changes too (a narrower ellipse), not only where it sits.
        tinted.p['position.x'].v = 200;
        tinted.d.w = 60;
        PM.touch();
        return { reference, added };
      });
      expect(today.added).toMatchObject({ status: 'added' });
      const asksBefore = await page.evaluate(() => (window as any).PM.EngineRender.compatStats.exportAsks);
      await page.evaluate(() => (window as any).PM.Export.queueDialog());
      const panel = page.getByRole('dialog', { name: 'Render Queue', exact: true });
      await panel.getByRole('button', { name: 'Start Queue', exact: true }).click();
      await expect(panel.locator('.render-queue-job').nth(0)).toContainText('Done', { timeout: 180_000 });
      const asks = await page.evaluate(() => (window as any).PM.EngineRender.compatStats.exportAsks) - asksBefore;
      expect(asks).toBeGreaterThanOrEqual(90);
      const files = (await readdir(folder)).filter((f) => f.endsWith('.png')).sort();
      expect(files.length).toBe(90);
      const diffs = frames.map((frame) => {
        const exported = pngRgba(path.join(folder, files[frame]!));
        const sheet = pngRgba(path.join(sheetFolder, sheetFiles[frame]!));
        let max = 0, over1 = 0, values = 0;
        for (let p = 0; p < 320 * 180 * 4; p += 4) for (let c = 0; c < 3; c++) {
          const d = Math.abs(exported[p + c]! - sheet[p + c]!);
          values++;
          if (d > max) max = d;
          if (d > 1) over1++;
        }
        return { frame, max, over1, values };
      });
      console.log(JSON.stringify({ asks, diffs }));
      for (const d of diffs) expect(d.max, `frame ${d.frame}: the queued state`).toBe(0);
      expect(session.diagnostics.pageErrors).toEqual([]);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  /**
   * Plays 3 s with `extension` installed over the project `setup` builds,
   * on the engine and then in today's app (flag off, a fresh profile), and
   * holds the engine to today's cadence. The callers run it under the
   * CPU-load harness too (docs/engine/extensions-compat.md).
   */
  /**
   * `layer`: the overridden layer whose answers are counted per presented
   * time. `from`: where playback starts (seconds). `warmAt`: when set, the
   * playhead first rests there (the paused warm-up prepares its frames),
   * then seeks to `from` and plays in the same task: Play right after a
   * seek, before any warm-up of `from`. `wrapFrames`: playback loops
   * (today's default) and goes on through the wrap for that many frames;
   * the frames after the wrap are measured on their own (`wrap`).
   */
  async function cadenceGate(session: { userData: string; relaunch(): Promise<void>; openEditor(): Promise<void>; close(): Promise<void>; page: Page; diagnostics: { pageErrors: unknown[] } }, extension: string, setup: (page: Page) => Promise<void>, label: string, options: { layer?: string; from?: number; warmAt?: number | null; wrapFrames?: number } = {}) {
    const run = { layer: options.layer ?? 'tinted', from: options.from ?? 0, warmAt: options.warmAt ?? null, wrapFrames: options.wrapFrames ?? 0 };
    await cp(path.join(__dirname, FIXTURE[extension]!), path.join(session.userData, 'extensions', extension), { recursive: true });
    await session.relaunch();
    await session.openEditor();
    /**
     * Plays the 3 s composition from 0 through its last frame and records
     * when frames were presented. The window is the composition's span on
     * both sides, not a wall-clock 3 s: today's clock starts with its first
     * drawn frame (~200 ms after Play) and its busy main thread fires a 3 s
     * timer ~200 ms late, while the engine presents from Play on, so a
     * wall-clock window compared different spans of the composition
     * (measured 2026-10-21: today's last frame at 3166 to 3206 ms, the
     * engine's at 3003 ms). Playback stops once the last frame is presented,
     * or after 6 s; frames after a wrap to the start are not counted.
     */
    const play = (page: Page, traced = false) => page.evaluate(async ({ traced, run }) => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('cadence-probe');
      const at: number[] = [], times: number[] = [];
      // Diagnostics (PM_COMPAT_TRACE=1): the lookahead's state at each
      // presented frame, joined with the native and viewer traces below.
      const lookaheadTrace: number[][] = [];
      let wrapped = false;
      // The frames after the wrap, when the run goes on through it.
      const wrapAt: number[] = [], wrapTimes: number[] = [];
      const off: any = api.events.on('frame:rendered', (event: any) => {
        if (times.length && event.time < times.at(-1)! - 1e-9) wrapped = true;
        if (!wrapped) { at.push(performance.now()); times.push(event.time); }
        else if (wrapTimes.length < run.wrapFrames) { wrapAt.push(performance.now()); wrapTimes.push(event.time); }
        if (traced && !wrapped) {
          const la = PM.EngineRender?.lookaheadStats ?? {}, cs = PM.EngineRender?.compatStats ?? {}, bs = PM.RasterCompat?.stats ?? {};
          lookaheadTrace.push([Math.round(performance.now() * 10) / 10, Math.round(event.time * 30 * 100) / 100, la.inFlight ?? -1, la.pending ?? -1, la.answered ?? -1, la.prefetched ?? -1, la.obsolete ?? -1, (cs.rasters ?? 0) + (cs.reuses ?? 0), cs.needs ?? 0, bs.asyncLoads ?? -1, la.asked ?? -1]);
        }
      });
      const needs = PM.EngineRender?.compatStats?.needs ?? 0;
      const rasters = (PM.EngineRender?.compatStats?.rasters ?? 0) + (PM.EngineRender?.compatStats?.reuses ?? 0);
      PM.setTime(run.warmAt ?? run.from);
      await new Promise((r) => setTimeout(r, run.warmAt === null ? 300 : 800));
      at.length = 0; times.length = 0; wrapped = false; wrapAt.length = 0; wrapTimes.length = 0;
      if (run.wrapFrames) PM.loop = true;
      const longTasks: { startTime: number; duration: number }[] = [];
      const observer = new PerformanceObserver(list => { for (const entry of list.getEntries()) longTasks.push({ startTime: entry.startTime, duration: entry.duration }); });
      observer.observe({ type: 'longtask' });
      const last = PM.proj.dur - 1 / PM.proj.fps - 1e-6;
      const started = performance.now();
      const basesBefore = PM.RasterCompat?.stats ? JSON.parse(JSON.stringify(PM.RasterCompat.stats)) : null;
      const revision = PM.EngineDoc?.barrier?.revision ?? null;
      if (run.warmAt !== null) PM.setTime(run.from);
      PM.play();
      const limit = run.wrapFrames ? (PM.proj.dur - run.from) * 1000 + run.wrapFrames * 1000 / PM.proj.fps + 4000 : 6000;
      while (!(run.wrapFrames ? wrapTimes.length >= run.wrapFrames : wrapped || (times.length && times.at(-1)! >= last)) && performance.now() - started < limit) await new Promise((r) => setTimeout(r, 4));
      PM.pause();
      if (typeof off === 'function') off(); else off?.dispose?.();
      observer.disconnect();
      const gaps = at.slice(1).map((v, i) => v - at[i]!);
      const sorted = [...gaps].sort((a, b) => a - b);
      const firstFrameMs = Math.round(at.length ? at[0]! - started : 3000);
      // The run's first advancing frame: the first presented time past the
      // one presented first (a re-presentation of the start frame is not
      // the run moving).
      const advance = times.findIndex((t) => Math.abs(t - times[0]!) > 1e-9);
      const firstAdvanceMs = Math.round(advance > 0 ? at[advance]! - started : 3000);
      // Times presented more than once (each extra presentation is a
      // `frame:rendered` an extension hears again).
      const repeatCount = times.filter((t, i) => times.indexOf(t) !== i).length;
      // The wrap: the gap from the last frame before it to the first after
      // it, the longest gap from there over the frames after it, and the
      // answers for those frames.
      const wrap = run.wrapFrames ? (() => {
        const all = [...at, ...wrapAt];
        const gaps = wrapAt.map((v, i) => v - all[at.length + i - 1]!);
        const answered = [...new Set(wrapTimes)].map((t) => PM.EngineRender?.answeredAt?.(run.layer, Math.round(t * 705_600_000)) ?? null)
          .filter((a: any) => a && (revision === null || a.revision >= revision)).length;
        return { frames: wrapTimes.length, uniqueTimes: new Set(wrapTimes).size, firstTimes: wrapTimes.slice(0, 4).map((t) => Math.round(t * 30 * 100) / 100), lastBefore: Math.round((times.at(-1) ?? 0) * 30 * 100) / 100, gap: Math.round(gaps[0] ?? 3000), maxGap: Math.round(Math.max(0, ...gaps)), answered, repeats: wrapTimes.filter((t, i) => wrapTimes.indexOf(t) !== i).length, skipped: wrapTimes.flatMap((t, i) => i > 0 && Math.round((t - wrapTimes[i - 1]!) * 30) > 1 ? [[i, Math.round(wrapTimes[i - 1]! * 30), Math.round(t * 30)]] : []).slice(0, 6) };
      })() : null;
      // Answers counted by the frame they are for: the window's latest
      // answer for the overridden layer at each presented time, at this
      // revision, wherever it was made (asked by a frame, prepared by the
      // lookahead, or by the paused warm-up before Play). Raw ask counts
      // also count frames the lookahead prepared that playback never shows.
      const shown = [...new Set(times)].map((t) => PM.EngineRender?.answeredAt?.(run.layer, Math.round(t * 705_600_000)) ?? null)
        .filter((a: any) => a && (revision === null || a.revision >= revision));
      const answeredShown = shown.length, rasterShown = shown.filter((a: any) => a.kind !== 'native').length;
      const trace = traced ? { started, hostOffsetUs: PM.EngineConnection?.status?.()?.hostOffsetUs ?? null, lookahead: lookaheadTrace, viewer: await PM.Kernel.services.get('viewer')?.enginePresentation?.takeTrace?.().catch(() => null) ?? null } : undefined;
      return { answeredShown, rasterShown, trace, longTasks: longTasks.map(task => ({ at: Math.round(task.startTime - started), ms: Math.round(task.duration) })), basesBefore, firstFrameMs, firstAdvanceMs, repeatCount, wrap, frames: at.length, uniqueTimes: new Set(times).size, lastTime: times.at(-1), head: times.slice(0, 4).map((t, i) => [Math.round(t * 30 * 100) / 100, Math.round(at[i]! - started)]), repeats: times.flatMap((t, i) => i > 0 && times.indexOf(t) !== i ? [[i, Math.round(t * 30 * 100) / 100]] : []).slice(0, 6), skipped: times.flatMap((t, i) => i > 0 && Math.round((t - times[i - 1]!) * 30) > 1 ? [[i, Math.round(times[i - 1]! * 30), Math.round(t * 30)]] : []).slice(0, 6), lastMs: Math.round((at.at(-1) ?? started) - started), maxGap: Math.round(Math.max(0, ...gaps)), p95: Math.round(sorted[Math.floor(sorted.length * 0.95)] ?? 0), asked: (PM.EngineRender?.compatStats?.needs ?? 0) - needs, rasters: (PM.EngineRender?.compatStats?.rasters ?? 0) + (PM.EngineRender?.compatStats?.reuses ?? 0) - rasters, nativeStats: PM.EngineStats?.latest ?? null, presenter: PM.Kernel.services.get('viewer')?.enginePresentation?.stats?.() ?? null, snapshots: PM.EngineRender?.stats ?? null, lookahead: PM.EngineRender ? { ...PM.EngineRender.lookaheadStats } : null, bases: PM.RasterCompat?.stats ? { ...PM.RasterCompat.stats } : null };
    }, { traced, run });
    const { page } = session;
    await page.waitForFunction((id) => (window as any).PM?.Kernel?.loader?.activeIds?.().includes(id) && Boolean((window as any).PM?.EngineRender), extension);
    await setup(page);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.presenting === true), { timeout: 20_000 }).toBe(true);
    const profileDirectory = process.env.PM_COMPAT_CPU_PROFILE_DIR;
    const profiler = profileDirectory ? await page.context().newCDPSession(page) : null;
    if (profiler) { await profiler.send('Profiler.enable'); await profiler.send('Profiler.start'); }
    // Per-frame diagnostics: the viewer presenter's trace (arrivals, draws,
    // drops), the native presenter's (POWERMOVE_BRIDGE_TRACE=1: schedule,
    // render, GPU and send times), and the lookahead at each presented frame.
    const traced = process.env.PM_COMPAT_TRACE === '1';
    const nativeReport = () => session.app.evaluate(async () => {
      const report: any = await (globalThis as any).__powermoveNativeEngine?.engine?.report?.('interactive').catch(() => null);
      return { presentTrace: report?.presentTrace ?? null, presenter: report?.presenter ?? null, loadavg: (process as any).getBuiltinModule?.('os')?.loadavg?.() ?? null };
    });
    if (traced) {
      await page.evaluate(() => (window as any).PM.EngineConnection.setTrace(true));
      await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.presenting === true), { timeout: 20_000 }).toBe(true);
      await page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.takeTrace?.());
      await nativeReport();
    }
    const engine: Awaited<ReturnType<typeof play>> & { nativeAfter?: Awaited<ReturnType<typeof nativeReport>> } = await play(page, traced);
    if (traced) (engine as any).trace.native = await nativeReport();
    if (run.wrapFrames) engine.nativeAfter = await nativeReport();
    if (profiler && profileDirectory) {
      const { profile } = await profiler.send('Profiler.stop');
      await mkdir(profileDirectory, { recursive: true });
      await writeFile(path.join(profileDirectory, `${label.replaceAll(/[^a-z0-9]/gi, '-')}-${Date.now()}.cpuprofile`), JSON.stringify(profile));
      await profiler.detach();
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
    await session.close();
    const off = await flagOffApp(extension);
    let today: Awaited<ReturnType<typeof play>>;
    try {
      await off.openEditor();
      await off.page.waitForFunction((id) => (window as any).PM?.Kernel?.loader?.activeIds?.().includes(id), extension).catch(async (error) => {
        console.log('flag off, not active:', JSON.stringify(await off.page.evaluate((id) => ({ ids: (window as any).PM?.Kernel?.loader?.activeIds?.(), record: (window as any).PM?.Kernel?.api('corpus')?.extensions?.list?.().find((r: any) => r.id === id) }), extension).catch((e) => String(e))));
        throw error;
      });
      await setup(off.page);
      await off.page.waitForTimeout(800);
      today = await play(off.page);
    } finally {
      await off.close();
      await rm(off.userData, { recursive: true, force: true });
    }
    // The machine's load during this run (the campaigns record it per run).
    console.log(JSON.stringify({ cadence: label, load: os.loadavg(), engine, today }));
    // The window answered for the override's layer at every presented time.
    expect(engine.answeredShown, 'presented times the window answered for').toBe(engine.uniqueTimes);
    // No stall (an unanswered need waits a second before it is asked again),
    // and no fewer frames than today's compositor presents.
    // Today's longest gap plus one frame at 30 fps, at most.
    expect(engine.maxGap).toBeLessThanOrEqual(today.maxGap + Math.ceil(1000 / 30));
    expect(engine.firstFrameMs).toBeLessThanOrEqual(today.firstFrameMs + Math.ceil(1000 / 30));
    expect(engine.firstAdvanceMs, 'the run\'s first advancing frame').toBeLessThanOrEqual(today.firstAdvanceMs + Math.ceil(1000 / 30));
    // Every presentation reaches extensions as `frame:rendered`: the engine
    // presents no time more often than today (none at the start of a run).
    expect(engine.repeatCount, `times presented again: ${JSON.stringify(engine.repeats)}`).toBeLessThanOrEqual(today.repeatCount);
    expect(engine.lastMs).toBeLessThanOrEqual(today.lastMs + Math.ceil(1000 / 30));
    // (The two sides can start one frame apart: one frame of alignment.)
    expect(engine.frames).toBeGreaterThanOrEqual(today.frames - 1);
    expect(engine.uniqueTimes).toBeGreaterThanOrEqual(today.uniqueTimes - 1);
    return { engine, today };
  }

  test('asynchronous playback readback preserves synchronous raster pixels and geometry', async ({ session }) => {
    test.setTimeout(120_000);
    await session.openEditor();
    await shapeTintProject(session.page);
    await expect.poll(() => session.page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    const results = await session.page.evaluate(async () => {
      const PM = (window as any).PM, api = PM.Kernel.api('readback-proof');
      const results: any[] = [];
      for (const colorSpace of ['srgb', 'display-p3']) {
        const cv = document.createElement('canvas'); cv.width = 256; cv.height = 64;
        const ctx = cv.getContext('2d', { colorSpace } as CanvasRenderingContext2DSettings) as CanvasRenderingContext2D;
        for (let x = 0; x < 256; x++) { ctx.fillStyle = `rgba(${x * 37 % 256},${x * 11 % 256},${x * 19 % 256},${x / 255})`; ctx.fillRect(x, 0, 1, 64); }
        ctx.fillStyle = '#E8202A80'; ctx.beginPath(); ctx.arc(128, 32, 24.5, 0, Math.PI * 2); ctx.fill();
        const surface = { cv, w: 256, h: 64, anchorX: 20, anchorY: 30, key: `readback:${colorSpace}` };
        const registration = api.services.register('raster', () => surface);
        try {
          const need = { composition: PM.proj.compId ?? PM.proj.id, layer: PM.proj.layers[0].id, time: 352_800_000, source: 'rasterOverride', scale: 1 };
          const revision = PM.EngineDoc.barrier.revision;
          await PM.RasterCompat.prepare(need, revision);
          const prepared = PM.RasterCompat.answer(need, revision, { prepared: true });
          const sync = PM.RasterCompat.answer(need, revision, { fresh: true });
          if (prepared?.kind !== 'raster' || sync?.kind !== 'raster') throw new Error('Both readback paths must return their raster.');
          let max = 0;
          for (let i = 0; i < sync.data.length; i++) max = Math.max(max, Math.abs(sync.data[i] - prepared.data[i]));
          results.push({ colorSpace, max, size: prepared.size, anchor: prepared.anchor, format: prepared.format, geometryEqual: JSON.stringify([prepared.size, prepared.anchor, prepared.uv]) === JSON.stringify([sync.size, sync.anchor, sync.uv]) });
        } finally { registration.dispose(); }
      }
      return results;
    });
    console.log(JSON.stringify({ asynchronousReadback: results }));
    for (const result of results) { expect(result.max).toBe(0); expect(result.geometryEqual).toBe(true); expect(result.format).toBe('rgba8Premultiplied'); }
  });

  test('playback with a raster override keeps today\'s cadence: every frame asks the window, none stalls', async ({ session }) => {
    test.setTimeout(240_000);
    await cadenceGate(session, 'shape-tint', shapeTintProject, 'shape-tint rotation');
  });

  test('playback with the typewriter override keyed 0 to 100% keeps today\'s cadence: a new typed base every frame', async ({ session }) => {
    test.setTimeout(240_000);
    const { engine } = await cadenceGate(session, 'compat-typewriter', typewriterCadenceProject, 'typewriter 0-100%', { layer: 'typed' });
    // The override drew the typed text itself for every presented time (its
    // pixels for that time, posted or reused), never the engine's text.
    expect(engine.rasterShown).toBeGreaterThanOrEqual(engine.uniqueTimes - 1);
    expect(engine.bases.asyncLoads).toBeGreaterThanOrEqual(engine.uniqueTimes - 1);
    expect(engine.bases.asyncLoads - engine.basesBefore.asyncLoads).toBeGreaterThanOrEqual(engine.uniqueTimes - 20);
    expect(engine.bases.syncLoads - engine.basesBefore.syncLoads, `synchronous base loads from ${JSON.stringify(engine.bases.syncLoadSites)}`).toBe(0);
  });

  test('playback with a raster override over a shape whose size animates keeps today\'s cadence', async ({ session }) => {
    test.setTimeout(240_000);
    const { engine } = await cadenceGate(session, 'shape-tint', shapeSizeProject, 'shape-tint animated size');
    expect(engine.rasterShown).toBeGreaterThanOrEqual(engine.uniqueTimes - 1);
    expect(engine.bases.asyncLoads).toBeGreaterThanOrEqual(engine.uniqueTimes - 1);
    expect(engine.bases.asyncLoads - engine.basesBefore.asyncLoads).toBeGreaterThanOrEqual(engine.uniqueTimes - 20);
    expect(engine.bases.syncLoads - engine.basesBefore.syncLoads, `synchronous base loads from ${JSON.stringify(engine.bases.syncLoadSites)}`).toBe(0);
  });

  test('playback with a raster override over a path whose vertices animate keeps today\'s cadence', async ({ session }) => {
    test.setTimeout(240_000);
    const { engine } = await cadenceGate(session, 'shape-tint', pathProject, 'shape-tint animated path');
    expect(engine.rasterShown).toBeGreaterThanOrEqual(engine.uniqueTimes - 1);
    expect(engine.bases.asyncLoads).toBeGreaterThanOrEqual(engine.uniqueTimes - 1);
    expect(engine.bases.asyncLoads - engine.basesBefore.asyncLoads).toBeGreaterThanOrEqual(engine.uniqueTimes - 20);
    expect(engine.bases.syncLoads - engine.basesBefore.syncLoads, `synchronous base loads from ${JSON.stringify(engine.bases.syncLoadSites)}`).toBe(0);
  });

  test('looping playback with a raster override longer than the engine keeps wraps at today\'s cadence', async ({ session }) => {
    test.setTimeout(360_000);
    // The whole 10 s loop, then 45 frames past the wrap (today's default
    // looping playback). By the wrap the store evicted the first frames'
    // rasters: the frames after it must be prepared before it.
    const { engine, today } = await cadenceGate(session, 'shape-tint', shapeLongLoopProject, 'shape-tint long loop wrap', { wrapFrames: 45 });
    console.log(JSON.stringify({ wrap: { engine: engine.wrap, today: today.wrap, evicted: engine.nativeAfter?.presenter?.compatEvicted ?? null, compatBytes: engine.nativeAfter?.presenter?.compatBytes ?? null } }));
    expect(engine.nativeAfter?.presenter?.compatEvicted, 'the store evicted rasters before the wrap').toBeGreaterThan(0);
    expect(engine.wrap!.frames).toBe(45);
    expect(engine.wrap!.firstTimes[0]).toBe(0);
    // The window answered for every frame after the wrap, and the wrap
    // stalls no more than today's (one frame at 30 fps of slack).
    expect(engine.wrap!.answered).toBe(engine.wrap!.uniqueTimes);
    expect(engine.wrap!.gap).toBeLessThanOrEqual(today.wrap!.gap + Math.ceil(1000 / 30));
    expect(engine.wrap!.maxGap).toBeLessThanOrEqual(today.wrap!.maxGap + Math.ceil(1000 / 30));
    expect(engine.wrap!.repeats).toBeLessThanOrEqual(today.wrap!.repeats);
    expect(engine.wrap!.uniqueTimes).toBeGreaterThanOrEqual(today.wrap!.uniqueTimes - 1);
    expect(engine.bases.syncLoads - engine.basesBefore.syncLoads, `synchronous base loads from ${JSON.stringify(engine.bases.syncLoadSites)}`).toBe(0);
  });

  test('a paused redraw announces the frame once, and redraws an override whose own state changed, as today', async ({ session }) => {
    test.setTimeout(240_000);
    // A wrapper whose tint follows its own state (a panel setting), then
    // `transport.invalidate()`: today's frame loop calls every wrapper again
    // on that redraw. Also a redraw with nothing changed (a selection
    // change redraws today): one `frame:rendered`, as today.
    const probe = (page: Page) => page.evaluate(async () => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('redraw-probe');
      const state = { color: '#E8202A' };
      const original = api.services.get('raster');
      const registration = api.services.register('raster', (layer: any, scale = 1, time = api.transport.time(), uploaded?: unknown, crop?: unknown) => {
        const base = original(layer, scale, time, uploaded, crop);
        if (layer.id !== 'tinted' || !base?.cv) return base;
        const cv = document.createElement('canvas'); cv.width = base.cv.width; cv.height = base.cv.height;
        const ctx = cv.getContext('2d')!;
        ctx.drawImage(base.cv, 0, 0); ctx.globalCompositeOperation = 'source-in'; ctx.fillStyle = state.color; ctx.fillRect(0, 0, cv.width, cv.height);
        return { ...base, cv, key: `redraw-probe:${state.color}:${layer.id}:${scale}:${time}:${api.anim.version()}` };
      });
      const count = async (act: () => void) => {
        let n = 0;
        const off: any = api.events.on('frame:rendered', () => { n++; });
        act();
        await new Promise((r) => setTimeout(r, 700));
        if (typeof off === 'function') off(); else off?.dispose?.();
        return n;
      };
      // The frame's pixel at the ellipse's centre (a capture of the time on screen).
      const pixel = async () => {
        const url: string = await api.render.snapshotAsync(PM.time, 320);
        const image = new Image();
        await new Promise((resolve, reject) => { image.onload = resolve; image.onerror = reject; image.src = url; });
        const cv = document.createElement('canvas'); cv.width = image.width; cv.height = image.height;
        const ctx = cv.getContext('2d')!;
        ctx.drawImage(image, 0, 0);
        const d = ctx.getImageData(Math.round(100 * image.width / 320), Math.round(90 * image.height / 180), 1, 1).data;
        return [d[0], d[1], d[2]];
      };
      try {
        api.transport.invalidate();
        PM.setTime(1);
        await new Promise((r) => setTimeout(r, 1200));
        const unchanged = await count(() => PM.invalidate());
        const unchangedWhy = PM.EnginePresentedEvents?.lastRedraw ?? null;
        const before = await pixel();
        state.color = '#2050F0';
        const changed = await count(() => api.transport.invalidate());
        const after = await pixel();
        return { unchanged, changed, before, after, redraws: PM.EnginePresentedEvents?.redraws ?? null, lastRedraw: PM.EnginePresentedEvents?.lastRedraw ?? null, unchangedWhy };
      } finally { registration.dispose(); }
    });
    await session.openEditor();
    await shapeTintProject(session.page);
    await expect.poll(() => session.page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    await expect.poll(() => session.page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.presenting === true), { timeout: 20_000 }).toBe(true);
    const engine = await probe(session.page);
    const replaced = await session.app.evaluate(async () => {
      const report: any = await (globalThis as any).__powermoveNativeEngine?.engine?.report?.('interactive').catch(() => null);
      return report?.presenter?.compatReplaced ?? null;
    });
    expect(session.diagnostics.pageErrors).toEqual([]);
    await session.close();
    const off = await flagOffApp(null);
    let today: Awaited<ReturnType<typeof probe>>;
    try {
      await off.openEditor();
      await shapeTintProject(off.page);
      await off.page.waitForTimeout(800);
      today = await probe(off.page);
    } finally {
      await off.close();
      await rm(off.userData, { recursive: true, force: true });
    }
    console.log(JSON.stringify({ pausedRedraw: { engine, today, replaced } }));
    expect(engine.unchanged).toBe(today.unchanged);
    expect(engine.changed).toBe(today.changed);
    expect(engine.after).toEqual(today.after);
    expect(engine.after).not.toEqual(engine.before);
    expect(replaced).toBeGreaterThan(0);
  });

  test('editing a large document with a raster override that reads its base copies the document only to seed the raster worker', async ({ session }) => {
    test.setTimeout(360_000);
    // Drags the overridden ellipse in a document of megabytes of JSON, one
    // move per presented frame: each move is a revision whose base raster
    // the override reads (a layer raster drawn by the engine's raster
    // worker), on the engine and in today's app.
    const drag = (page: Page) => page.evaluate(async () => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('drag-probe');
      let presented: (() => void) | null = null;
      const off: any = api.events.on('frame:rendered', () => { presented?.(); presented = null; });
      const latencies: number[] = [], missed: number[] = [], diagnostics: unknown[] = [];
      try {
        for (let i = 0; i < 60; i++) {
          const began = performance.now();
          const shown = new Promise<boolean>((resolve) => { presented = () => resolve(true); setTimeout(() => resolve(false), 2000); });
          const before = PM.EngineDoc?.barrier?.revision ?? null;
          PM.proj.layers.find((l: any) => l.id === 'tinted').p['position.x'].v = 200 + i * 4;
          // As the app's own edits do (anim.ts): the edit, then a redraw.
          PM.touch();
          PM.invalidate();
          if (await shown) latencies.push(performance.now() - began);
          else {
            missed.push(i);
            const status = PM.EngineConnection?.status?.() ?? null;
            diagnostics.push({ move: i, before, revision: PM.EngineDoc?.barrier?.revision ?? null, applied: status?.revision ?? null, route: status?.route ?? null, failures: status?.failures ?? null, compat: PM.EngineRender ? { ...PM.EngineRender.compatStats } : null, lookahead: PM.EngineRender ? { ...PM.EngineRender.lookaheadStats, recent: undefined, recentAnswers: undefined } : null });
          }
        }
      } finally { if (typeof off === 'function') off(); else off?.dispose?.(); }
      const sorted = [...latencies].sort((a, b) => a - b);
      const q = (p: number) => Math.round(sorted[Math.min(sorted.length - 1, Math.ceil(p * sorted.length) - 1)] ?? 0);
      return { moves: 60, shown: latencies.length, missed, diagnostics, p50: q(0.5), p95: q(0.95), max: q(1) };
    });
    await cp(path.join(__dirname, FIXTURE['shape-tint']!), path.join(session.userData, 'extensions', 'shape-tint'), { recursive: true });
    await session.relaunch();
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => (window as any).PM?.Kernel?.loader?.activeIds?.().includes('shape-tint') && Boolean((window as any).PM?.EngineRender));
    const bytes = await largeDocumentProject(page, 2500);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 90_000 }).toBe('engine');
    await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.presenting === true), { timeout: 30_000 }).toBe(true);
    const presenter = () => session.app.evaluate(async () => {
      const report: any = await (globalThis as any).__powermoveNativeEngine?.engine?.report?.('interactive').catch(() => null);
      const p = report?.presenter ?? {};
      return { documentCopies: p.documentCopies ?? null, seeds: p.layerRasterSeeds ?? null, reseeds: p.layerRasterReseeds ?? null, offThread: p.layerRastersOffThread ?? null, slowTurns: p.slowTurns ?? null, turnUsMax: p.turnUsMax ?? null, lastStall: p.lastStall ?? null };
    });
    // The first raster seeds the worker's mirror (the one copy, on the worker).
    await page.waitForTimeout(1500);
    const before = await presenter();
    const engine = await drag(page);
    await page.waitForTimeout(500);
    const after = await presenter();
    expect(session.diagnostics.pageErrors).toEqual([]);
    await session.close();
    const off = await flagOffApp('shape-tint');
    let today: Awaited<ReturnType<typeof drag>>;
    try {
      await off.openEditor();
      await off.page.waitForFunction(() => (window as any).PM?.Kernel?.loader?.activeIds?.().includes('shape-tint'));
      await largeDocumentProject(off.page, 2500);
      await off.page.waitForTimeout(1500);
      today = await drag(off.page);
    } finally {
      await off.close();
      await rm(off.userData, { recursive: true, force: true });
    }
    const slow = (a: number[] | null, b: number[] | null) => a && b ? a.map((n, i) => n - b[i]!) : null;
    console.log(JSON.stringify({ editDrag: { documentBytes: bytes, load: os.loadavg(), engine, today, before, after, slowTurnsDuringDrag: slow(after.slowTurns, before.slowTurns) } }));
    expect(bytes).toBeGreaterThan(4_000_000);
    // Every move drew new base rasters off the render thread, from the
    // worker's mirror: no copy of the document per revision, no reseed.
    expect(after.offThread - before.offThread).toBeGreaterThanOrEqual(30);
    expect(after.documentCopies - before.documentCopies).toBe(0);
    expect(after.reseeds - before.reseeds).toBe(0);
    expect(engine.shown).toBe(60);
    // No render-thread turn of 100 ms or more while dragging.
    expect(slow(after.slowTurns, before.slowTurns)?.slice(1)).toEqual([0, 0, 0]);
  });

  test('Play right after a seek with the typewriter override keeps today\'s cadence: the first frames wait in the native preroll, none is skipped', async ({ session }) => {
    test.setTimeout(240_000);
    // Rest at 0 (its frames warm), then seek to 1 s and Play in one task:
    // nothing of 1 s onward was prepared before Play.
    const { engine } = await cadenceGate(session, 'compat-typewriter', typewriterCadenceProject, 'typewriter seek then play', { layer: 'typed', from: 1, warmAt: 0 });
    expect(engine.head[0]?.[0]).toBe(30);
    // No frame of the run's start was skipped while its answers were on their way.
    expect(engine.skipped.filter(([index]: number[]) => index! <= 6)).toEqual([]);
    expect(engine.rasterShown).toBeGreaterThanOrEqual(engine.uniqueTimes - 1);
    expect(engine.bases.asyncLoads - engine.basesBefore.asyncLoads).toBeGreaterThanOrEqual(engine.uniqueTimes - 20);
    expect(engine.bases.syncLoads - engine.basesBefore.syncLoads, `synchronous base loads from ${JSON.stringify(engine.bases.syncLoadSites)}`).toBe(0);
  });

  test('an export of a project with a raster override draws the override, frame by frame', async ({ session }, testInfo) => {
    test.setTimeout(240_000);
    // Shapes: the job engine draws them without fonts, so the export runs
    // on the engine and asks this window for the override's layers.
    await cp(path.join(__dirname, 'fixtures/compat-corpus/shape-tint'), path.join(session.userData, 'extensions/shape-tint'), { recursive: true });
    await session.relaunch();
    await session.openEditor();
    const { app, page } = session;
    await page.waitForFunction(() => (window as any).PM?.Kernel?.loader?.activeIds?.().includes('shape-tint') && Boolean((window as any).PM?.EngineRender));
    await shapeTintProject(page);
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
      // The job engine drew it, asking this window for each shape at each frame.
      expect(run.exportAsks).toBeGreaterThanOrEqual(90);
      const files = (await readdir(folder)).filter((f) => f.endsWith('.png')).sort();
      expect(files.length).toBe(90);
      expect(session.diagnostics.pageErrors).toEqual([]);
      // Today's exporter: the same project and extension with "Use New
      // Engine" off, exported to PNG the same way.
      const frames = [0, 45, 89];
      const todayFolder = path.join(root, 'Today');
      const today = await todayPngExport(session, 'shape-tint', shapeTintProject, todayFolder, frames);
      const todayFiles = (await readdir(todayFolder)).filter((f) => f.endsWith('.png')).sort();
      // Diagnosis: today's export of the same project without any extension
      // (the plain rectangle is drawn the same either way).
      {
        const plainFolder = path.join(root, 'TodayPlain');
        const off = await flagOffApp(null);
        try {
          await off.openEditor();
          await shapeTintProject(off.page);
          await stubSavePanel(off.app, plainFolder, true);
          expect(await off.page.evaluate(async () => (window as any).PM.Export.run({ format: 'png', scale: 1, fps: 30, range: 'all', quality: 'high', mblur: false, audio: false, alpha: false }))).toEqual({ cancelled: false });
        } finally {
          await off.close();
          await rm(off.userData, { recursive: true, force: true });
        }
        const plainFiles = (await readdir(plainFolder)).filter((f) => f.endsWith('.png')).sort();
        const plain = pngRgba(path.join(plainFolder, plainFiles[45]!));
        const at = (px: Buffer, x: number, y: number) => [...px.subarray((y * 320 + x) * 4, (y * 320 + x) * 4 + 4)];
        console.log(JSON.stringify({ todayWithoutExtension_195_110: at(plain, 195, 110), todayWithExtension_195_110: at(today[1]!, 195, 110) }));
      }
      const diffs = frames.map((frame, i) => {
        const exported = pngRgba(path.join(folder, files[frame]!));
        let max = 0, over1 = 0, values = 0, red = 0, todayRed = 0;
        const worst: Array<{ x: number; y: number; exported: number[]; today: number[] }> = [];
        for (let p = 0; p < 320 * 180 * 4; p += 4) {
          let pixel = 0;
          for (let c = 0; c < 3; c++) {
            const d = Math.abs(exported[p + c]! - today[i]![p + c]!);
            values++;
            if (d > max) max = d;
            if (d > 1) over1++;
            pixel = Math.max(pixel, d);
          }
          if (pixel > 4 && worst.length < 6) worst.push({ x: (p / 4) % 320, y: Math.floor(p / 4 / 320), exported: [...exported.subarray(p, p + 4)], today: today[i]!.slice(p, p + 4) });
          if (exported[p]! > 200 && exported[p + 1]! < 80) red++;
          if (today[i]![p]! > 200 && today[i]![p + 1]! < 80) todayRed++;
        }
        return { frame, max, over1, values, red, todayRed, worst };
      });
      console.log(JSON.stringify(diffs));
      await testInfo.attach('export-frame-45', { path: path.join(folder, files[45]!), contentType: 'image/png' });
      await testInfo.attach('today-export-frame-45', { path: path.join(todayFolder, todayFiles[45]!), contentType: 'image/png' });
      for (const d of diffs) {
        // The override drew (the tint is red), and the frame is today's
        // (section 25: <= 1 CV for 99.9% of values, <= 4 for any).
        expect(d.red, `frame ${d.frame}: tinted`).toBeGreaterThan(1000);
        expect(d.over1, `frame ${d.frame}: vs today`).toBeLessThanOrEqual(d.values * 0.001);
        expect(d.max, `frame ${d.frame}: vs today`).toBeLessThanOrEqual(4);
      }
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});
