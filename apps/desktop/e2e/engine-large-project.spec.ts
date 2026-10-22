/*
 * A large real-world-shaped project with "Use New Engine" on (C1, the
 * bigproject lane; docs/engine/host-processes.md, "Large documents").
 *
 * A real user's production project (258 layers baked to a keyframe per
 * frame: a 180 MB document, 1.2 million keyframes, 50 embedded PNGs) did not
 * open with the engine on: a raster wrapper's cache key over non-ASCII text
 * broke pm-protocol's 512-byte bound, the engine closed the window's
 * connection and nothing reconnected; the interactive engine held the
 * document many times over (13 GB); the renderer ran out of V8 heap. This is
 * the same shape with synthetic data, small enough for a laptop run:
 *
 * - flag off: import generated PNGs, bake keyframes on every layer, add text
 *   layers a `raster` wrapper (registered as an apiVersion 1 extension would)
 *   redraws with keys carrying their text, save it as a PMV4 file;
 * - flag off, fresh profile: open the file, measure the JS heap, take frames;
 * - flag on, fresh profile: open it again; the open completes, the engine
 *   stays connected (one connection, a session, the wrapper's rasters
 *   posted), the heap is today's or lower, the engine process stays within a
 *   bound of the document, the frames match today's at the corpus bound,
 *   playback runs without a disconnect, and a connection the engine closes
 *   (the same refused key, sent raw) comes back by itself.
 *
 * Run serially, one Electron at a time, without the suite's 10 minute cap
 * (three launches and two opens of the project take a few minutes):
 *   pm-electron-lock bunx playwright test e2e/engine-large-project.spec.ts --workers=1 --global-timeout=0
 */
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { deflateSync } from 'node:zlib';
import { execFileSync } from 'node:child_process';
import type { Page } from 'playwright';

import { expect, launchApp, test, type LaunchedApp } from './helpers/app';

/** The synthetic project's size: about 18 MB of document JSON, over the
 *  12 MiB inline resync limit, so every engine path that stages it runs. */
const IMAGES = 16;
const TEXTS = 16;
const SHAPES = 32;
const DURATION = 16;
const FPS = 30;
const SIZE = { w: 960, h: 540 };
/** Frames compared with today (seconds), inside the wrapped text's span. */
const TIMES = [0, 2.5, 6, 15.9];

/** A small RGBA PNG (stored, no filter) for the import path. */
function png(width: number, height: number, seed: number): Buffer {
  const raw = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (width * 4 + 1)] = 0;
    for (let x = 0; x < width; x++) {
      const at = y * (width * 4 + 1) + 1 + x * 4;
      raw[at] = (x * 3 + seed * 37) & 255;
      raw[at + 1] = (y * 5 + seed * 11) & 255;
      raw[at + 2] = ((x ^ y) + seed * 53) & 255;
      raw[at + 3] = x < 4 || y < 4 ? 0 : 255;
    }
  }
  const crcTable = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc = (buf: Buffer) => { let c = 0xffffffff; for (const b of buf) c = crcTable[(c ^ b) & 255]! ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
    const out = Buffer.alloc(8 + data.length + 4);
    out.writeUInt32BE(data.length, 0); body.copy(out, 4); out.writeUInt32BE(crc(body), 8 + data.length);
    return out;
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0); header.writeUInt32BE(height, 4); header[8] = 8; header[9] = 6;
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', header), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}

/**
 * The wrapper: text layers named `wrapped …` get their own surface (the base
 * raster with a band over it) under a key that carries the text and the
 * time, as text-revealing extensions build theirs: with this text, about
 * 1,300 bytes of UTF-8.
 */
async function registerWrapper(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const api = PM.Kernel.api('large-project-wrapper');
    const base = api.services.get('raster');
    api.services.register('raster', (layer: any, scale: number, time: number, uploaded: unknown, crop: unknown) => {
      const surface = base(layer, scale, time, uploaded, crop);
      if (layer?.type !== 'text' || !String(layer.name).startsWith('wrapped') || !surface?.cv) return surface;
      const source = surface.cv;
      const canvas = document.createElement('canvas');
      canvas.width = source.width; canvas.height = source.height;
      const ctx = canvas.getContext('2d')!;
      ctx.drawImage(source, 0, 0);
      ctx.fillStyle = 'rgba(255, 100, 30, 0.8)';
      ctx.fillRect(0, Math.floor(canvas.height * 0.45), Math.max(1, Math.round(canvas.width * Math.min(1, (time % 4) / 4))), Math.max(1, Math.floor(canvas.height * 0.1)));
      const text = String(layer.d?.text ?? '');
      return { ...surface, cv: canvas, key: `wrap:${layer.id}:${text.repeat(6)}:${time.toFixed(4)}:${scale}` };
    });
  });
}

const started = Date.now();
const step = (what: string) => console.log(`[large project +${((Date.now() - started) / 1000).toFixed(0)} s] ${what}`);

async function heapAfterGc(page: Page): Promise<number> {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('HeapProfiler.collectGarbage');
  await cdp.send('HeapProfiler.collectGarbage');
  const used = await page.evaluate(() => (performance as any).memory?.usedJSHeapSize as number);
  await cdp.detach();
  return used;
}

async function openFile(session: LaunchedApp, file: string): Promise<number> {
  await session.app.evaluate(({ dialog }, filePath) => { dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] }); }, file);
  const started = Date.now();
  const opened = await session.page.evaluate(async () => {
    const w = window as any;
    await w.PM.openProject();
    return { layers: w.PM.proj.layers.length, name: w.PM.proj.name };
  });
  expect(opened.layers).toBe(IMAGES + TEXTS + SHAPES);
  return (Date.now() - started) / 1000;
}

async function frames(page: Page): Promise<Buffer[]> {
  // The project's media is read after the open resolves; until then image
  // layers draw nothing, in either engine.
  await page.evaluate(async () => { const PM = (window as any).PM; await PM.projectAssetsReady; await PM.HostMedia?.flushBindings?.(); });
  await page.waitForFunction(() => { const PM = (window as any).PM; return !(PM.assetsRestoring > 0) && !(PM.assets?.loading?.size > 0); }, null, { timeout: 300_000, polling: 250 });
  const out: Buffer[] = [];
  for (const time of TIMES) {
    const rgba = await page.evaluate(async ({ time, w, h }) => {
      const PM = (window as any).PM, api = PM.Kernel.api('large-project-wrapper');
      PM.pause(); PM.setTime(time, { force: true });
      await PM.prepareFrame?.(time);
      await PM.HostMedia?.flushBindings?.();
      const canvas = await api.render.renderFrameToAsync(time, w, h, { mblur: false });
      const bytes = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data;
      let raw = ''; for (let at = 0; at < bytes.length; at += 32768) raw += String.fromCharCode(...bytes.subarray(at, at + 32768));
      return btoa(raw);
    }, { time, ...SIZE });
    out.push(Buffer.from(rgba, 'base64'));
  }
  return out;
}

function compare(a: Buffer, b: Buffer) {
  let max = 0, over2 = 0, values = 0;
  expect(a.length).toBe(b.length);
  for (let i = 0; i < a.length; i++) if (i % 4 !== 3) { const d = Math.abs(a[i]! - b[i]!); if (d > max) max = d; if (d > 2) over2++; values++; }
  return { max, within2: 1 - over2 / values };
}

/** Closes a launch, and kills it if quitting a large project takes long:
 *  one Electron at a time, so a stuck quit must not hold the machine lock. */
async function closeSoon(session: LaunchedApp): Promise<void> {
  const child = session.app.process();
  const closed = session.close().then(() => true, () => true);
  const done = await Promise.race([closed, new Promise<boolean>((resolve) => setTimeout(() => resolve(false), 30_000))]);
  if (!done) child.kill('SIGKILL');
  await closed;
}

/** A page question with a deadline (a busy renderer must fail the test, not hang it). */
async function ask<T>(page: Page, what: string, fn: () => T, ms = 30_000): Promise<T> {
  return Promise.race([
    page.evaluate(fn),
    new Promise<never>((_, reject) => setTimeout(() => reject(new Error(`${what}: no answer in ${ms} ms`)), ms)),
  ]) as Promise<T>;
}

function rssMiB(pid: number): number {
  return Math.round(Number(execFileSync('ps', ['-o', 'rss=', '-p', String(pid)], { encoding: 'utf8' }).trim()) / 1024);
}

test('a large project with embedded images and raster-wrapped text opens with the engine on, stays connected, and matches today', async () => {
  test.setTimeout(1_500_000);
  const dir = await mkdtemp(path.join(os.tmpdir(), 'powermove-large-project-'));
  try {
    const file = path.join(dir, 'Large.pmv');
    const images: string[] = [];
    // Distinct names without frame numbers: numbered files would be offered
    // as one image sequence.
    const names = ['harbor', 'meadow', 'canyon', 'glacier', 'orchard', 'lagoon', 'summit', 'prairie', 'delta', 'tundra', 'grove', 'fjord', 'mesa', 'bayou', 'atoll', 'steppe', 'marsh', 'dune', 'ridge', 'cove', 'basin', 'cliff', 'bluff', 'reef'];
    for (let i = 0; i < IMAGES; i++) {
      const image = path.join(dir, `${names[i]}.png`);
      await writeFile(image, png(160 + (i % 5) * 32, 120 + (i % 3) * 24, i));
      images.push(image);
    }
    // ── build and save it with today's engine ───────────────────────────
    const author = await launchApp({ userData: path.join(dir, 'author-profile'), env: { POWERMOVE_NEW_ENGINE: '0' } });
    let documentBytes = 0;
    try {
      await author.openEditor();
      await author.page.evaluate(({ w, h, dur, fps }) => {
        const PM = (window as any).PM;
        const project = PM.mkProject({ name: 'Large', w, h, fps, dur, bg: '#182028' });
        PM.replaceProject(project);
      }, { w: SIZE.w, h: SIZE.h, dur: DURATION, fps: FPS });
      const chooser = author.page.waitForEvent('filechooser');
      await author.page.evaluate(() => (window as any).PM.pickFiles());
      await (await chooser).setFiles(images);
      await author.page.waitForFunction((n) => (window as any).PM.proj.layers.filter((l: any) => l.type === 'image').length === n, IMAGES, { timeout: 120_000 });
      step(`imported ${IMAGES} images`);
      documentBytes = await author.page.evaluate(({ texts, shapes, dur, fps }) => {
        const PM = (window as any).PM, proj = PM.proj;
        for (let i = 0; i < texts; i++) {
          const text = PM.mkLayer('text', { name: `${i % 2 ? 'wrapped' : 'plain'} text ${i}`, from: 0, dur, d: { text: `Harbor · ridge — “${i}” ✨ meadow · cove`, font: 'Helvetica', size: 28, color: '#FFFFFF' } }, proj);
          proj.layers.push(text);
        }
        for (let i = 0; i < shapes; i++) {
          const shape = PM.mkLayer('shape', { name: `marker ${i}`, from: 0, dur, d: { shape: i % 2 ? 'ellipse' : 'rect', w: 40 + (i % 7) * 6, h: 24 + (i % 5) * 4, color: '#3FA9F5', stroke: 0 } }, proj);
          proj.layers.push(shape);
        }
        // Baked motion: a keyframe per frame on seven properties of every
        // layer, as the private project's export from another tool has.
        const frames = Math.round(dur * fps);
        proj.layers.forEach((layer: any, index: number) => {
          const channels: Record<string, (f: number) => number> = {
            'position.x': (f) => 80 + ((index * 37 + f * 2) % 800),
            'position.y': (f) => 60 + ((index * 23 + f) % 420),
            'scale.x': (f) => 90 + 10 * Math.sin((f + index) / 9),
            'scale.y': (f) => 90 + 10 * Math.cos((f + index) / 11),
            rotation: (f) => ((f + index * 5) % 60) - 30,
            opacity: (f) => 60 + 40 * Math.abs(Math.sin((f + index) / 17)),
            skew: (f) => ((f + index) % 20) / 4,
          };
          for (const [name, value] of Object.entries(channels)) {
            const prop = layer.p[name] ?? (layer.p[name] = PM.P(value(0)));
            prop.kf = Array.from({ length: frames }, (_, f) => PM.KF(f / fps, value(f)));
          }
        });
        PM.touch?.();
        PM.bus.emit('layers');
        return JSON.stringify(proj).length;
      }, { texts: TEXTS, shapes: SHAPES, dur: DURATION, fps: FPS });
      await author.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, file);
      expect(await author.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
    } finally {
      await closeSoon(author);
    }
    // Over the inline limit by a wide margin; small enough for a laptop.
    expect(documentBytes).toBeGreaterThan(14 * 1024 * 1024);
    step(`saved ${(documentBytes / 2 ** 20).toFixed(1)} MiB of document`);

    // ── today's open: the reference heap and frames ─────────────────────
    const today = await launchApp({ userData: path.join(dir, 'today-profile'), env: { POWERMOVE_NEW_ENGINE: '0' } });
    let todayHeap = 0, todaySeconds = 0;
    let reference: Buffer[] = [];
    try {
      await today.openEditor();
      await registerWrapper(today.page);
      todaySeconds = await openFile(today, file);
      step(`today opened in ${todaySeconds.toFixed(1)} s`);
      todayHeap = await heapAfterGc(today.page);
      reference = await frames(today.page);
      step('today frames taken');
    } finally {
      await closeSoon(today);
    }

    // ── the same open with the engine on ───────────────────────────────
    const engine = await launchApp({ userData: path.join(dir, 'engine-profile'), env: { POWERMOVE_NEW_ENGINE: '1' } });
    try {
      await engine.openEditor();
      await engine.page.waitForFunction(() => (window as any).PM?.EngineConnection?.status?.().connected === true, null, { timeout: 60_000 });
      await registerWrapper(engine.page);
      const engineSeconds = await openFile(engine, file);
      step(`engine opened in ${engineSeconds.toFixed(1)} s`);
      // A viewer frame with the wrapped text in view: its needs are answered.
      await engine.page.evaluate(() => { const PM = (window as any).PM; PM.setTime(2.5, { force: true }); });
      await expect.poll(() => engine.page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 120_000 }).toBe('engine');
      await expect.poll(() => engine.page.evaluate(() => (window as any).PM.EngineRender.compatStats.rasters), { timeout: 60_000 }).toBeGreaterThan(0);
      const status = await engine.page.evaluate(() => (window as any).PM.EngineConnection.status());
      expect(status.connected).toBe(true);
      expect(status.connections, 'the engine never closed the window\'s connection').toBe(1);
      expect(status.sessions).toBeGreaterThanOrEqual(1);
      expect(status.failures.filter((f: string) => /connection|key length/i.test(f))).toEqual([]);
      const engineHeap = await heapAfterGc(engine.page);
      console.log(`large project (${(documentBytes / 2 ** 20).toFixed(1)} MiB): open today ${todaySeconds.toFixed(1)} s, engine ${engineSeconds.toFixed(1)} s; heap after GC today ${(todayHeap / 2 ** 20).toFixed(0)} MiB, engine ${(engineHeap / 2 ** 20).toFixed(0)} MiB`);
      // Today's heap, within measurement noise (the engine's shadow of the
      // document replaces the store's clone of the last autosave). Before
      // the fix the engine-on heap grew by several copies of the document.
      // The opening gate is tied to today's path; extra whole-document
      // diff/hash/resync work must not be hidden behind a fixed allowance.
      // The interactive engine holds the document about once: a pm-model
      // tree is about 11 times its JSON. Before the shared tree it held
      // five or more copies.
      const pid = await engine.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.status().interactive.pid as number);
      const rss = rssMiB(pid);
      console.log(`interactive engine RSS after the open: ${rss} MiB`);
      expect(rss).toBeLessThan(1024 + Math.round((documentBytes * 24) / 2 ** 20));

      // The same frames as today, at the corpus bound.
      const drawn = await frames(engine.page);
      drawn.forEach((frame, i) => {
        const result = compare(reference[i]!, frame);
        console.log(`frame at ${TIMES[i]} s: max ${result.max} CV, ${(result.within2 * 100).toFixed(3)} % within 2 CV`);
        expect(result.within2, `frame at ${TIMES[i]} s`).toBeGreaterThanOrEqual(0.999);
        expect(result.max, `frame at ${TIMES[i]} s`).toBeLessThanOrEqual(4);
      });

      // Playback: frames keep coming and nothing disconnects.
      const before = await ask(engine.page, 'frames before play', () => (window as any).PM.EngineConnection.status().frames as number);
      await ask(engine.page, 'play', () => { const PM = (window as any).PM; PM.setTime(0, { force: true }); PM.play(); return true; });
      let state: string | undefined;
      for (let waited = 0; waited < 20_000 && state !== 'playing'; waited += 250) {
        await engine.page.waitForTimeout(250);
        state = await ask(engine.page, 'clock state', () => (window as any).PM.EngineConnection.status().clock?.state as string | undefined);
      }
      step(`engine clock ${state}`);
      expect(state).toBe('playing');
      const t0 = await ask(engine.page, 'time at start', () => (window as any).PM.time as number);
      await engine.page.waitForTimeout(3000);
      const playing = await ask(engine.page, 'time after 3 s', () => { const PM = (window as any).PM; const s = PM.EngineConnection.status(); const time = PM.time as number; PM.pause(); return { time, s }; });
      step(`played from ${t0.toFixed(2)} s to ${playing.time.toFixed(2)} s; ${playing.s.frames - before} frames`);
      expect(playing.time - t0).toBeGreaterThan(1.5);
      expect(playing.s.connected).toBe(true);
      expect(playing.s.connections).toBe(1);
      expect(playing.s.frames).toBeGreaterThan(before);

      // A connection the engine closes (the refused key, sent raw) comes
      // back by itself, with a session that draws again.
      await ask(engine.page, 'refused message', () => {
        const PM = (window as any).PM, session = PM.EngineConnection.session();
        const layer = PM.proj.layers.find((l: any) => String(l.name).startsWith('wrapped'));
        return session.post({ type: 'compat', compat: 'reuse', id: { composition: String(PM.proj.compId ?? PM.proj.id), layer: layer.id, time: 0, source: 'rasterOverride', key: '·'.repeat(400) }, scale: 1, revision: 1 });
      });
      const dropped = Date.now();
      await expect.poll(() => ask(engine.page, 'connections', () => (window as any).PM.EngineConnection.status().connections), { timeout: 30_000 }).toBe(2);
      await expect.poll(() => ask(engine.page, 'route', () => { const s = (window as any).PM.EngineConnection.status(); return s.connected && s.sessions > 0 && s.route; }), { timeout: 120_000 }).toBe('engine');
      step(`reconnected and drawing ${((Date.now() - dropped) / 1000).toFixed(1)} s after the engine closed the connection`);
      expect(await ask(engine.page, 'checkpoint resume', () => (window as any).PM.EngineConnection.session()?.resumes ?? 0)).toBeGreaterThan(0);
      const after = await ask(engine.page, 'snapshot after reconnect', () => (window as any).PM.Kernel.api('large-project-wrapper').render.snapshot(2.5, 320) as string, 60_000);
      expect(after.startsWith('data:image/jpeg')).toBe(true);
      // Keep strict performance failures, after observing the independent
      // pixel and reconnect gates so a timing miss cannot hide their evidence.
      expect(engineHeap).toBeLessThanOrEqual(todayHeap * 1.05 + 32 * 2 ** 20);
      expect(engineSeconds).toBeLessThanOrEqual(todaySeconds);
      expect(engine.diagnostics.pageErrors).toEqual([]);
    } finally {
      await closeSoon(engine);
    }
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
