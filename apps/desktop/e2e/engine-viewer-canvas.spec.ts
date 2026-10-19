/*
 * Decision V1: with "Use New Engine" on, the viewer looks like today's.
 * Today's viewer draws a view into a WebGL canvas it sizes itself
 * (`previewRenderSize`: `min(1, zoom × min(2, DPR)) × quality` of the
 * composition below fit; the explicit preview resolution; the cropped
 * `previewRenderViewport` when zoomed into a crop-safe project), and the
 * browser scales that canvas into its CSS box. The engine renders at the
 * same canvas and scales it the same way (pm-node `compose::canvas_placement`).
 *
 * The two viewers are compared by what the window shows: `capturePage` of
 * the stage (the engine's presentation canvas is composited by Chromium
 * like today's WebGL canvas), for a project with text, shapes, a blurred
 * shape, a video, and a level 1 SVG image, at 25%, 50%, 61% and 100% on a
 * 2x display, with the preview resolution at Auto, Full and Half. Each
 * case must hold the corpus bound (section 25's render class: every
 * channel within 1 code value for 99.9% of pixels, none beyond 4).
 *
 * Both windows run with `--force-color-profile=srgb` (set
 * PM_VIEWER_CANVAS_PROFILE=display to keep the display's profile): on a
 * wide-gamut display the engine presents a display-p3 canvas, and the
 * 8-bit P3 round trip alone moves single channels of smooth gradients by
 * 2 code values in `capturePage`, at every zoom, including 50% where both
 * canvases map 1:1 (docs/engine/bridge.md, "Today's viewer canvas").
 * Each case also compares the frame's content with the scaling taken
 * away: the engine's snapshot against today's `renderToPixels`, both at
 * the case's canvas size.
 *
 * The video layer is judged apart: with the preview resolution at Auto or
 * a quality below 1, today's viewer draws a video's editing copy
 * (`previewVideoElement`, the intra-1280 derivative) where the engine and
 * today's snapshots draw the original (the engine's media preference has no
 * proxies yet, section 14); scaled to half size, their edge pixels differ.
 * Every case holds the bound outside the video layer; the whole stage must
 * hold it where today draws the original (Full).
 *
 * Evidence (today, engine, differences in red) goes to
 * $PM_VIEWER_CANVAS_EVIDENCE when set.
 *
 * Run serially, one Electron at a time:
 *   pm-electron-lock bunx playwright test e2e/engine-viewer-canvas.spec.ts --workers=1
 */
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { deflateSync } from 'node:zlib';
import type { Page } from 'playwright';

import { expect, test, launchApp, repoRoot, type LaunchedApp } from './helpers/app';

const ZOOMS = [0.25, 0.5, 0.61, 1] as const;
const RESOLUTIONS = ['auto', '1', '0.5'] as const;
const NAMES: Record<string, string> = { auto: 'Auto', '1': 'Full', '0.5': 'Half' };

const ART = `<svg xmlns="http://www.w3.org/2000/svg" width="160" height="90" viewBox="0 0 160 90">
<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#4C8DFF"/><stop offset="1" stop-color="#E83A5E"/></linearGradient></defs>
<rect x="4" y="4" width="152" height="82" rx="14" fill="url(#g)"/>
<path d="M10 80 Q 60 20 150 70" fill="none" stroke="#FFD23F" stroke-width="3"/>
</svg>`;

/** Text, shapes, a blurred star, a video and a level 1 SVG image, 640 × 360. */
async function mixedProject(page: Page, video: number[]): Promise<void> {
  await page.evaluate(async ({ svg, video }) => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Viewer canvas', w: 640, h: 360, fps: 30, dur: 2, bg: '#1b2433' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#1b2433', position: 0 }] };
    PM.replaceProject(project);
    const at = (layer: any, x: number, y: number) => { layer.p['position.x'].v = x; layer.p['position.y'].v = y; return layer; };
    const title = at(PM.mkLayer('text', { name: 'Title', d: { text: 'Powermove viewer 61%', font: 'Arial', size: 34, color: '#ffffff' } }, PM.proj), 320, 48);
    const small = at(PM.mkLayer('text', { name: 'Caption', d: { text: 'fine print, 11 px', font: 'Arial', size: 11, color: '#d0e0ff' } }, PM.proj), 110, 340);
    const box = at(PM.mkLayer('shape', { name: 'Box', d: { shape: 'rect', w: 150, h: 90, radius: 12, color: '#ff5500' } }, PM.proj), 110, 150);
    box.p.rotation.v = 7;
    const line = at(PM.mkLayer('shape', { name: 'Line', d: { shape: 'rect', w: 300, h: 1, color: '#ffffff' } }, PM.proj), 320, 95);
    const star = at(PM.mkLayer('shape', { name: 'Star', d: { shape: 'star', w: 90, h: 90, color: '#55ccff' } }, PM.proj), 560, 140);
    const blur = PM.mkEffect('blur');
    if (!blur) throw new Error('Built-in blur unavailable');
    star.fx.push(blur);
    const svgAsset = await PM.assets.add(new File([svg], 'art.svg', { type: 'image/svg+xml' }), { newAsset: true, silent: true });
    const art = at(PM.mkLayer('image', { name: 'Art', d: { asset: svgAsset.id, fit: 'contain', w: 200, h: 112 } }, PM.proj), 330, 190);
    art.p.rotation.v = -4;
    const videoAsset = await PM.assets.add(new File([new Uint8Array(video)], 'clip.mp4', { type: 'video/mp4' }), { newAsset: true, silent: true });
    const clip = at(PM.mkLayer('video', { name: 'Clip', d: { asset: videoAsset.id, w: 160, h: 90, embeddedAudio: false } }, PM.proj), 530, 290);
    PM.proj.layers = [title, small, line, star, art, box, clip];
    PM.ProjectIndex?.invalidate?.();
    PM.selectLayers?.([]);
    PM.touch();
    PM.Projects?.put?.(PM.proj);
    PM.pause?.();
    PM.setTime(0.5, { force: true });
  }, { svg: ART, video });
}

type Capture = { width: number; height: number; data: Uint8Array; geometry: unknown; content?: unknown };

const ARGS = process.env.PM_VIEWER_CANVAS_PROFILE === 'display' ? [] : ['--force-color-profile=srgb'];

/** The frame's content without the scaling: the engine's snapshot and
 *  today's `renderToPixels` at the canvas size the viewer sent. */
const content = (page: Page) => page.evaluate(() => {
  const PM = (window as any).PM, V = PM.Kernel.services.get('viewer');
  const canvas = V.enginePresentation?.states?.at?.(-1)?.canvas;
  if (!canvas) return null;
  const { width, height } = canvas;
  const legacy: Uint8Array = PM.GL.renderToPixels(PM.time, width, height, { opaque: true, topDownOpaque: true, mblur: false });
  const engine = PM.EngineRender.snapshotSync({ time: PM.time, width, height, pixels: 'opaque', motionBlurSamples: 0, background: true });
  let within = 0, max = 0;
  for (let i = 0; i < legacy.length; i += 4) {
    let m = 0;
    for (let c = 0; c < 3; c++) m = Math.max(m, Math.abs(legacy[i + c]! - engine.data[i + c]!));
    if (m <= 1) within++;
    max = Math.max(max, m);
  }
  const pixels = legacy.length / 4;
  return { canvas, pixels, fraction: within / pixels, max };
});

/** The viewer in a state, settled, and the stage's pixels as the window shows them. */
async function capture(session: LaunchedApp, zoom: number, resolution: string, engine: boolean): Promise<Capture> {
  const { page } = session;
  await page.locator('select[aria-label="Preview resolution"]').selectOption(resolution);
  const geometry = await page.evaluate((z) => {
    const PM = (window as any).PM, V = PM.Kernel.services.get('viewer');
    PM.selectLayers?.([]);
    V.setZoom(z); V.pan = [0, 0]; V.layout();
    PM.invalidate?.('render');
    return { shown: V.shown, stage: [V._sw, V._sh], dpr: window.devicePixelRatio, quality: PM.quality, resolution: PM.previewResolution };
  }, zoom);
  // Settled: today's frame drawn (video decoded) or the engine's frame for
  // this view on screen, then nothing new for half a second.
  await page.waitForTimeout(800);
  if (engine) {
    await expect.poll(() => page.evaluate(() => {
      const V = (window as any).PM.Kernel.services.get('viewer');
      const frame = V.engineFrame();
      return !!frame && !frame.standIn && frame.viewSeq === V.enginePresentation?.states?.at?.(-1)?.viewSeq;
    }), { timeout: 20_000 }).toBe(true);
  }
  await page.evaluate(() => new Promise((r) => { let n = 0; const tick = () => (++n > 30 ? r(null) : requestAnimationFrame(tick)); tick(); }));
  await page.waitForTimeout(500);
  const rect = await page.evaluate(() => {
    const r = (window as any).PM.Kernel.services.get('viewer').stage.getBoundingClientRect();
    return { x: Math.ceil(r.left), y: Math.ceil(r.top), width: Math.floor(r.right) - Math.ceil(r.left), height: Math.floor(r.bottom) - Math.ceil(r.top) };
  });
  const shot = await session.app.evaluate(async ({ BrowserWindow }, at) => {
    const window = BrowserWindow.getAllWindows().find((w) => !w.isDestroyed() && w.webContents.getURL().startsWith('app:'))!;
    const image = await window.webContents.capturePage(at);
    const bitmap = image.toBitmap();
    const size = image.getSize();
    return { width: size.width, height: size.height, data: Array.from(bitmap) };
  }, rect);
  return { width: shot.width, height: shot.height, data: Uint8Array.from(shot.data), geometry, ...(engine ? { content: await content(page) } : {}) };
}

/** A minimal RGBA PNG (evidence). */
function png(width: number, height: number, rgba: Uint8Array): Buffer {
  const table = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc = (buf: Buffer) => { let c = 0xffffffff; for (const b of buf) c = table[(c ^ b) & 0xff]! ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
  const chunk = (type: string, body: Buffer) => {
    const head = Buffer.alloc(8); head.writeUInt32BE(body.length, 0); head.write(type, 4, 'latin1');
    const tail = Buffer.alloc(4); tail.writeUInt32BE(crc(Buffer.concat([head.subarray(4), body])), 0);
    return Buffer.concat([head, body, tail]);
  };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(width, 0); ihdr.writeUInt32BE(height, 4); ihdr[8] = 8; ihdr[9] = 6;
  const raw = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y++) Buffer.from(rgba.buffer, rgba.byteOffset + y * width * 4, width * 4).copy(raw, y * (width * 4 + 1) + 1);
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}

type Stats = { pixels: number; withinOne: number; exact: number; max: number; fraction: number; passes: boolean };

/** The video layer's box (plus 4 composition pixels) in capture pixels:
 *  the clip is 160 × 90 at (530, 290) of the 640 × 360 composition. */
function videoBox(e: Capture): [number, number, number, number] {
  const css = (e.content as any).canvas.css, crop = (e.content as any).canvas.crop;
  const z = (css.width / crop.width) * 2, x0 = css.x * 2, y0 = css.y * 2;
  return [x0 + 446 * z, y0 + 241 * z, x0 + 614 * z, y0 + 339 * z];
}

function stats(withinOne: number, exact: number, max: number, pixels: number): Stats {
  const fraction = withinOne / pixels;
  return { pixels, withinOne, exact, max, fraction, passes: fraction >= 0.999 && max <= 4 };
}

function compare(a: Capture, b: Capture): { stats: Stats; outside: Stats; diff: Uint8Array } {
  expect([b.width, b.height]).toEqual([a.width, a.height]);
  let withinOne = 0, exact = 0, max = 0;
  let oPixels = 0, oWithin = 0, oExact = 0, oMax = 0;
  const [vx0, vy0, vx1, vy1] = videoBox(b);
  const pixels = a.width * a.height, diff = new Uint8Array(pixels * 4);
  for (let i = 0; i < pixels; i++) {
    let m = 0;
    for (let c = 0; c < 3; c++) m = Math.max(m, Math.abs(a.data[i * 4 + c]! - b.data[i * 4 + c]!));
    if (m <= 1) withinOne++;
    if (m === 0) exact++;
    max = Math.max(max, m);
    const x = i % a.width, y = Math.floor(i / a.width);
    if (!(x >= vx0 && x < vx1 && y >= vy0 && y < vy1)) {
      oPixels++; if (m <= 1) oWithin++; if (m === 0) oExact++; oMax = Math.max(oMax, m);
    }
    // BGRA → RGBA, today's pixel dimmed, differences in red.
    const o = i * 4;
    if (m > 1) { diff[o] = 255; diff[o + 1] = 0; diff[o + 2] = 0; } else { diff[o] = a.data[o + 2]! >> 1; diff[o + 1] = a.data[o + 1]! >> 1; diff[o + 2] = a.data[o]! >> 1; }
    diff[o + 3] = 255;
  }
  return { stats: stats(withinOne, exact, max, pixels), outside: stats(oWithin, oExact, oMax, oPixels), diff };
}

const rgba = (c: Capture) => { const out = new Uint8Array(c.data.length); for (let i = 0; i < c.data.length; i += 4) { out[i] = c.data[i + 2]!; out[i + 1] = c.data[i + 1]!; out[i + 2] = c.data[i]!; out[i + 3] = 255; } return out; };

test('the engine viewer shows today’s canvas, scaled as today’s, at 25 to 100% and every preview resolution', async () => {
  test.setTimeout(600_000);
  const video = Array.from(await readFile(path.join(repoRoot, 'e2e/fixtures/h264-aac.mp4')));
  const run = async (engine: boolean) => {
    const session = await launchApp({ env: engine ? { POWERMOVE_NEW_ENGINE: '1' } : {}, args: ARGS });
    try {
      await session.openEditor();
      await mixedProject(session.page, video);
      if (engine) {
        await expect.poll(() => session.page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
        await expect.poll(() => session.page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.presenting === true), { timeout: 20_000 }).toBe(true);
      }
      const out: Record<string, Capture> = {};
      for (const resolution of RESOLUTIONS) for (const zoom of ZOOMS) out[`${Math.round(zoom * 100)}% ${NAMES[resolution]}`] = await capture(session, zoom, resolution, engine);
      expect(session.diagnostics.pageErrors).toEqual([]);
      return out;
    } finally {
      await session.close();
    }
  };
  const today = await run(false);
  const engine = await run(true);
  const evidence = process.env.PM_VIEWER_CANVAS_EVIDENCE;
  if (evidence) await mkdir(evidence, { recursive: true });
  const results: Record<string, Stats & { content?: unknown; outsideVideo: Stats }> = {};
  for (const name of Object.keys(today)) {
    const t = today[name]!, e = engine[name]!;
    expect(e.geometry, name).toEqual(t.geometry);
    const { stats, outside, diff } = compare(t, e);
    results[name] = { ...stats, outsideVideo: outside, content: e.content };
    console.log(`viewer canvas ${name}: ${t.width}×${t.height} ${JSON.stringify(t.geometry)} ${JSON.stringify(stats)} outside the video ${JSON.stringify(outside)} content ${JSON.stringify(e.content)}`);
    if (evidence) {
      const slug = name.replace(/[^a-z0-9]+/gi, '-').toLowerCase();
      await writeFile(path.join(evidence, `${slug}-today.png`), png(t.width, t.height, rgba(t)));
      await writeFile(path.join(evidence, `${slug}-engine.png`), png(e.width, e.height, rgba(e)));
      await writeFile(path.join(evidence, `${slug}-diff.png`), png(t.width, t.height, diff));
    }
  }
  if (evidence) await writeFile(path.join(evidence, 'results.json'), JSON.stringify(results, null, 1));
  for (const [name, r] of Object.entries(results)) {
    expect.soft({ name, outsideVideo: r.outsideVideo.passes, r }).toMatchObject({ name, outsideVideo: true });
    if (name.endsWith('Full')) expect.soft({ name, passes: r.passes, r }).toMatchObject({ name, passes: true });
  }
});
