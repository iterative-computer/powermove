/*
 * Decision V1: with "Use New Engine" on, the viewer must look like today's.
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
 * shape, a video, and a level 1 SVG image, at 25%, 50%, 61%, 100% and 237%
 * on a 2x display, with the preview resolution at Auto, Full and Half. Each
 * case must hold the corpus bound (section 25's render class: every
 * channel within 1 code value for 99.9% of pixels, none beyond 4) outside
 * video. The untagged SD fixture's video uses an independent H.273 / SMPTE
 * 170M reference instead (docs/engine/quality-ledger.md, Video).
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
 * Above 100% the project is crop-safe, so today's canvas is the cropped
 * `previewRenderViewport`: 237% unpanned and panned by a fractional amount,
 * at every preview resolution. The engine renders today's crop from its
 * unrounded origin (decision V1b, `RenderRequest::origin`). Two more
 * projects are crops far from the composition's origin: 1920 × 1080 at
 * 800% and 3840 × 2160 at 250%, each flush with its right edge (today's
 * crop starts past 16,384 output pixels from the origin); the engine draws
 * them at today's full resolution.
 *
 * Every case asserts the corpus bound outside the video layer. Video cases
 * compare the whole red interior with the f64 reference and the entire video
 * box, including scaled edges, with today's original-video coverage after
 * replacing only its nonstandard source colour. Uncropped content checks the
 * native pixel against the f64 reference and today's pixel.
 * The video's editing copy is a separate known difference: with the preview resolution
 * at Auto or a quality below 1, today's viewer draws a video's editing copy
 * (`previewVideoElement`, the intra-1280 derivative) where the engine draws
 * the original (no proxies yet, section 14). Today's viewer is captured
 * again with the editing copy removed to supply its original-video coverage.
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

/** A view: zoom, pan in CSS pixels (or flush with the composition's right
 *  edge), preview resolution, and the far-crop project it shows (`comp`). */
type View = { zoom: number; pan: [number, number] | 'right'; resolution: string; comp?: [number, number] };
const name = (v: View) => `${v.comp ? `${v.comp[0]} × ${v.comp[1]} ` : ''}${Math.round(v.zoom * 100)}%${v.pan === 'right' ? ' right edge' : v.pan[0] || v.pan[1] ? ' panned' : ''} ${NAMES[v.resolution]}`;
/** The mixed project: 25 to 100% (whole canvas), and 237% (cropped canvas) unpanned and panned. */
const MIXED: View[] = [
  ...RESOLUTIONS.flatMap((resolution) => ZOOMS.map((zoom): View => ({ zoom, pan: [0, 0], resolution }))),
  ...RESOLUTIONS.flatMap((resolution) => [[0, 0], [-137.3, 61.7]].map((pan): View => ({ zoom: 2.37, pan: pan as [number, number], resolution }))),
];
/** Crops far from the composition's origin: 1920 × 1080 at 800% and
 *  3840 × 2160 at 250%, flush with the right edge (today's crop starts
 *  about 28,700 and 17,100 output pixels from the origin). */
const BEYOND: View[] = [
  { zoom: 8, pan: 'right', resolution: '1', comp: [1920, 1080] },
  { zoom: 2.5, pan: 'right', resolution: '1', comp: [3840, 2160] },
];

/** Cases where today's viewer also selects a video editing copy. */
const VIDEO_COPY = 'video editing copy at Auto or quality below 1 (Phase 4a proxies; issues.md)';
const KNOWN: Record<string, { why: string }> = {
  '25% Auto': { why: VIDEO_COPY },
  '25% Half': { why: VIDEO_COPY },
  '50% Half': { why: VIDEO_COPY },
  '61% Half': { why: VIDEO_COPY },
  '100% Half': { why: VIDEO_COPY },
};

// Test-only f64 reference, independent of production colour code. The public
// h264-aac.mp4 fixture's 0.5 s interior decodes Y'CbCr=(81,90,240). H.273
// gives limited-range BT.601 Y'CbCr; VIDEO-SD-1 reads this untagged 160×90
// source with SMPTE 170M primaries and an sRGB display curve. The fixture's
// decoded codes can be checked independently with FFmpeg.
type Vec3 = [number, number, number];
type Mat3 = [Vec3, Vec3, Vec3];
function referenceVideoColor(): Vec3 {
  const xyToXyz = ([x, y]: [number, number]): Vec3 => [x / y, 1, (1 - x - y) / y];
  const mv = (m: Mat3, v: Vec3): Vec3 => [0, 1, 2].map((row) =>
    m[row as 0 | 1 | 2][0] * v[0] + m[row as 0 | 1 | 2][1] * v[1] + m[row as 0 | 1 | 2][2] * v[2]) as Vec3;
  const inverse = (m: Mat3): Mat3 => {
    const [[a, b, c], [d, e, f], [g, h, i]] = m;
    const det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    return [
      [(e * i - f * h) / det, (c * h - b * i) / det, (b * f - c * e) / det],
      [(f * g - d * i) / det, (a * i - c * g) / det, (c * d - a * f) / det],
      [(d * h - e * g) / det, (b * g - a * h) / det, (a * e - b * d) / det],
    ];
  };
  const rgbToXyz = (red: [number, number], green: [number, number], blue: [number, number]): Mat3 => {
    const r = xyToXyz(red), g = xyToXyz(green), b = xyToXyz(blue);
    const primaries: Mat3 = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
    const scales = mv(inverse(primaries), xyToXyz([0.3127, 0.3290]));
    return [[r[0] * scales[0], g[0] * scales[1], b[0] * scales[2]],
      [r[1] * scales[0], g[1] * scales[1], b[1] * scales[2]],
      [r[2] * scales[0], g[2] * scales[1], b[2] * scales[2]]];
  };
  const y = (81 - 16) / 219, cb = (90 - 128) / 224, cr = (240 - 128) / 224;
  const kr = 0.299, kb = 0.114;
  const rp = y + 2 * (1 - kr) * cr, bp = y + 2 * (1 - kb) * cb;
  const gp = (y - kr * rp - kb * bp) / (1 - kr - kb);
  const eotf = (v: number) => v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  const source = rgbToXyz([0.630, 0.340], [0.310, 0.595], [0.155, 0.070]);
  const destination = rgbToXyz([0.640, 0.330], [0.300, 0.600], [0.150, 0.060]);
  const linear = mv(inverse(destination), mv(source, [eotf(rp), eotf(gp), eotf(bp)]));
  const oetf = (v: number) => v <= 0.0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - 0.055;
  return linear.map((value) => Math.max(0, Math.min(1, oetf(value))) * 255) as Vec3;
}
const VIDEO_REFERENCE = referenceVideoColor();
const referenceError = (rgb: readonly number[]) => Math.max(...VIDEO_REFERENCE.map((v, i) => Math.abs(rgb[i]! - v)));

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
    (window as any).__viewerCanvasVideo = videoAsset.id;
    PM.proj.layers = [title, small, line, star, art, box, clip];
    PM.ProjectIndex?.invalidate?.();
    PM.selectLayers?.([]);
    PM.touch();
    PM.Projects?.put?.(PM.proj);
    PM.pause?.();
    PM.setTime(0.5, { force: true });
  }, { svg: ART, video });
}

/** `w` × `h`: text, a rotated box and a hairline at its right edge. */
async function wideProject(page: Page, [w, h]: [number, number]): Promise<void> {
  await page.evaluate(({ w, h }) => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Viewer canvas wide', w, h, fps: 30, dur: 2, bg: '#1b2433' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#1b2433', position: 0 }] };
    PM.replaceProject(project);
    const at = (layer: any, x: number, y: number) => { layer.p['position.x'].v = x; layer.p['position.y'].v = y; return layer; };
    const y = h / 2 - 20;
    const title = at(PM.mkLayer('text', { name: 'Edge', d: { text: 'Edge', font: 'Arial', size: 14, color: '#ffffff' } }, PM.proj), w - 40, y);
    const box = at(PM.mkLayer('shape', { name: 'Box', d: { shape: 'rect', w: 30, h: 18, radius: 4, color: '#ff5500' } }, PM.proj), w - 25, y + 25);
    box.p.rotation.v = 9;
    const line = at(PM.mkLayer('shape', { name: 'Line', d: { shape: 'rect', w: 60, h: 1, color: '#ffffff' } }, PM.proj), w - 35, y + 15);
    PM.proj.layers = [title, line, box];
    PM.ProjectIndex?.invalidate?.();
    PM.selectLayers?.([]);
    PM.touch();
    PM.Projects?.put?.(PM.proj);
    PM.pause?.();
    PM.setTime(0.5, { force: true });
  }, { w, h });
}

type Capture = { width: number; height: number; data: Uint8Array; geometry: unknown; content?: unknown };

const ARGS = process.env.PM_VIEWER_CANVAS_PROFILE === 'display' ? [] : ['--force-color-profile=srgb'];

/** The frame's content without the scaling: the engine's snapshot and
 *  today's `renderToPixels` at the canvas size the viewer sent. */
const content = (page: Page) => page.evaluate((reference: number[]) => {
  const PM = (window as any).PM, V = PM.Kernel.services.get('viewer');
  const canvas = V.enginePresentation?.states?.at?.(-1)?.canvas;
  if (!canvas) return null;
  const project = PM.proj;
  // A crop's content is not a whole-composition snapshot: only the canvas is reported.
  if (canvas.crop.x !== 0 || canvas.crop.y !== 0 || canvas.crop.width !== project.w || canvas.crop.height !== project.h) return { canvas, cropped: true };
  const { width, height } = canvas;
  const engine = PM.EngineRender.snapshotSync({ time: PM.time, width, height, pixels: 'opaque', motionBlurSamples: 0, background: true });
  const sample = (data: Uint8Array, x: number, y: number) => Array.from(data.subarray((y * width + x) * 4, (y * width + x) * 4 + 3));
  const videoCenter = project.layers.some((layer: any) => layer.type === 'video')
    ? [Math.floor(530 * width / project.w), Math.floor(290 * height / project.h)] as const : null;
  const videoReference = (() => {
    if (!videoCenter) return;
    const [x0, x1] = [Math.ceil(456 * width / project.w), Math.floor(604 * width / project.w)];
    const [y0, y1] = [Math.ceil(251 * height / project.h), Math.floor(329 * height / project.h)];
    let pixels = 0, withinOne = 0, max = 0;
    for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) {
      const i = (y * width + x) * 4;
      const delta = Math.max(...reference.map((value, channel) => Math.abs(engine.data[i + channel]! - value)));
      pixels++;
      if (delta <= 1) withinOne++;
      max = Math.max(max, delta);
    }
    return { pixels, withinOne, max, passes: pixels > 0 && withinOne / pixels >= 0.999 && max <= 4 };
  })();
  // Today's renderToPixels twice: the first render of a size can differ
  // from the next (recorded, not hidden).
  const compare = () => {
    const legacy: Uint8Array = PM.GL.renderToPixels(PM.time, width, height, { opaque: true, topDownOpaque: true, mblur: false });
    let within = 0, max = 0;
    for (let i = 0; i < legacy.length; i += 4) {
      let m = 0;
      for (let c = 0; c < 3; c++) m = Math.max(m, Math.abs(legacy[i + c]! - engine.data[i + c]!));
      if (m <= 1) within++;
      max = Math.max(max, m);
    }
    return { fraction: within / (legacy.length / 4), max,
      ...(videoCenter ? { videoPixel: { engine: sample(engine.data, ...videoCenter), legacy: sample(legacy, ...videoCenter) } } : {}) };
  };
  const first = compare(), again = compare();
  return { canvas, pixels: width * height, fraction: first.fraction, max: first.max, videoPixel: first.videoPixel, videoReference, again };
}, VIDEO_REFERENCE.map(Math.round));

/** The viewer in a state, settled, and the stage's pixels as the window shows them. */
async function capture(session: LaunchedApp, view: View, engine: boolean): Promise<Capture> {
  const { page } = session;
  await page.locator('select[aria-label="Preview resolution"]').selectOption(view.resolution);
  const geometry = await page.evaluate(({ zoom, pan, engine }) => {
    const PM = (window as any).PM, V = PM.Kernel.services.get('viewer');
    PM.selectLayers?.([]);
    V.setZoom(zoom);
    // Flush right: the frame's right edge on the stage's (`compositionFramePosition` centers, then pans).
    V.pan = pan === 'right' ? [Math.round(-(PM.proj.w * V.shown - V._sw) / 2), 0] : pan;
    V.layout();
    PM.invalidate?.('render');
    const viewport = PM.GL.previewViewport;
    return { shown: V.shown, pan: [...V.pan], stage: [V._sw, V._sh], dpr: window.devicePixelRatio, quality: PM.quality, resolution: PM.previewResolution,
      ...(engine ? {} : { crop: viewport ? [viewport.x, viewport.y, viewport.width, viewport.height, viewport.renderWidth, viewport.renderHeight] : null }) };
  }, { zoom: view.zoom, pan: view.pan, engine });
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
    const size = image.getSize();
    // Base64: a number array of 5 MB per capture is most of the run's time.
    return { width: size.width, height: size.height, data: image.toBitmap().toString('base64') };
  }, rect);
  // The canvas the frame on screen was drawn at (the engine) or today's crop (above).
  const shown = engine ? await page.evaluate(() => {
    const V = (window as any).PM.Kernel.services.get('viewer');
    const frame = V.engineFrame();
    const canvas = frame ? V.enginePresentation.stateFor(frame.viewSeq)?.canvas : null;
    return canvas ? [canvas.crop.x, canvas.crop.y, canvas.crop.width, canvas.crop.height, canvas.width, canvas.height] : null;
  }) : undefined;
  return { width: shot.width, height: shot.height, data: new Uint8Array(Buffer.from(shot.data, 'base64')), geometry: { ...geometry, ...(engine ? { crop: shown && (shown[0] || shown[1] || geometry.shown > 1) ? shown : null } : {}) }, ...(engine ? { content: await content(page) } : {}) };
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
  if (!(e as any).video) return [0, 0, 0, 0];
  const css = (e.content as any).canvas.css, crop = (e.content as any).canvas.crop;
  const z = (css.width / crop.width) * 2, x0 = (css.x - crop.x * css.width / crop.width) * 2, y0 = (css.y - crop.y * css.height / crop.height) * 2;
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

/** The whole visible red field, six composition pixels inside the clip's
 *  edges so crop and browser scaling cannot include the antialiased border. */
function videoReference(c: Capture): { stats: Stats; center: number[] } | undefined {
  const [left, top, right, bottom] = videoBox(c);
  if (right <= left || bottom <= top) return;
  const inset = 6 * (right - left) / 168;
  const [x0, y0] = [Math.max(0, Math.ceil(left + inset)), Math.max(0, Math.ceil(top + inset))];
  const [x1, y1] = [Math.min(c.width, Math.floor(right - inset)), Math.min(c.height, Math.floor(bottom - inset))];
  if (x0 >= x1 || y0 >= y1) return;
  let withinOne = 0, exact = 0, max = 0;
  const expected = VIDEO_REFERENCE.map(Math.round);
  for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) {
    const i = (y * c.width + x) * 4;
    const delta = Math.max(Math.abs(c.data[i + 2]! - expected[0]!), Math.abs(c.data[i + 1]! - expected[1]!), Math.abs(c.data[i]! - expected[2]!));
    if (delta <= 1) withinOne++;
    if (delta === 0) exact++;
    max = Math.max(max, delta);
  }
  const x = Math.floor((x0 + x1) / 2), y = Math.floor((y0 + y1) / 2), i = (y * c.width + x) * 4;
  return { stats: stats(withinOne, exact, max, (x1 - x0) * (y1 - y0)), center: [c.data[i + 2]!, c.data[i + 1]!, c.data[i]!] };
}

/** Decision V1 geometry reference, across the entire video box including its
 *  border. Today's original-video capture has the same authored rectangle and
 *  Chromium canvas scaling; its red channel gives the coverage at each stage
 *  pixel. Replace only its nonstandard Rec.709 reading with the independent
 *  SMPTE 170M -> Rec.709 colour reference. The clip lies over the uniform
 *  authored background and is the project's last layer. */
function videoBoxReference(todayOriginal: Capture, engine: Capture, errors?: {
  today: Uint32Array; native: Uint32Array; regression: { pixels: number; withinOne: number; max: number };
}): Stats {
  expect([todayOriginal.width, todayOriginal.height]).toEqual([engine.width, engine.height]);
  const [left, top, right, bottom] = videoBox(engine);
  const background = [27, 36, 51] as const; // #1b2433
  let pixels = 0, withinOne = 0, exact = 0, max = 0;
  for (let y = Math.max(0, Math.ceil(top)); y < Math.min(engine.height, Math.ceil(bottom)); y++) {
    for (let x = Math.max(0, Math.ceil(left)); x < Math.min(engine.width, Math.ceil(right)); x++) {
      const i = (y * engine.width + x) * 4;
      const coverage = Math.max(0, Math.min(1, (todayOriginal.data[i + 2]! - background[0]) / (254 - background[0])));
      const expected = VIDEO_REFERENCE.map((value, channel) => Math.round(background[channel]! + coverage * (value - background[channel]!)));
      let oldDelta = 0, delta = 0;
      for (let channel = 0; channel < 3; channel++) {
        const oldError = Math.abs(todayOriginal.data[i + 2 - channel]! - expected[channel]!);
        const nativeError = Math.abs(engine.data[i + 2 - channel]! - expected[channel]!);
        oldDelta = Math.max(oldDelta, oldError);
        delta = Math.max(delta, nativeError);
      }
      if (errors) {
        errors.today[oldDelta] = (errors.today[oldDelta] ?? 0) + 1;
        errors.native[delta] = (errors.native[delta] ?? 0) + 1;
        const excess = delta - oldDelta;
        errors.regression.pixels++;
        if (excess <= 1) errors.regression.withinOne++;
        errors.regression.max = Math.max(errors.regression.max, excess);
      }
      pixels++;
      if (delta <= 1) withinOne++;
      if (delta === 0) exact++;
      max = Math.max(max, delta);
    }
  }
  return stats(withinOne, exact, max, pixels);
}

const rgba = (c: Capture) => { const out = new Uint8Array(c.data.length); for (let i = 0; i < c.data.length; i += 4) { out[i] = c.data[i + 2]!; out[i + 1] = c.data[i + 1]!; out[i + 2] = c.data[i]!; out[i + 3] = 255; } return out; };

test('the engine viewer shows today’s canvas, scaled as today’s, from 25 to 800% and every preview resolution', async () => {
  test.setTimeout(900_000);
  const video = Array.from(await readFile(path.join(repoRoot, 'e2e/fixtures/h264-aac.mp4')));
  const run = async (engine: boolean) => {
    const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: engine ? '1' : '0' }, args: ARGS });
    try {
      await session.openEditor();
      await mixedProject(session.page, video);
      if (engine) {
        await expect.poll(() => session.page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
        await expect.poll(() => session.page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.presenting === true), { timeout: 20_000 }).toBe(true);
      }
      const out: Record<string, Capture> = {};
      for (const view of MIXED) out[name(view)] = { ...(await capture(session, view, engine)), video: true } as Capture;
      if (!engine) {
        // The control: today's viewer drawing the original video (its
        // editing copy removed) where it drew the copy.
        await session.page.evaluate(() => {
          const PM = (window as any).PM, asset = PM.assets.get((window as any).__viewerCanvasVideo);
          if (!asset?.preview?.el) throw new Error('the video has no editing copy to remove');
          asset.preview = null;
          PM.invalidate?.('render');
        });
        for (const view of MIXED.filter((v) => KNOWN[name(v)]?.why === VIDEO_COPY)) out[`${name(view)} original video`] = { ...(await capture(session, view, engine)), video: true } as Capture;
      }
      for (const view of BEYOND) {
        await wideProject(session.page, view.comp!);
        if (engine) await expect.poll(() => session.page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.presenting === true), { timeout: 20_000 }).toBe(true);
        out[name(view)] = await capture(session, view, engine);
      }
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
  const results: Record<string, Stats & { content?: unknown; outsideVideo: Stats; geometry: unknown; known?: string; videoPixel?: number[]; videoReference?: Stats; videoBoxReference?: Stats }> = {};
  const aggregateErrors = { today: new Uint32Array(256), native: new Uint32Array(256),
    regression: { pixels: 0, withinOne: 0, max: -Infinity } };
  const record = async (label: string, t: Capture, e: Capture) => {
    const { stats, outside, diff } = compare(t, e);
    const reference = videoReference(e);
    const original = KNOWN[label] ? today[`${label} original video`] : t;
    results[label] = { ...stats, outsideVideo: outside, content: e.content, geometry: { today: t.geometry, engine: e.geometry },
      ...(reference ? { videoPixel: reference.center, videoReference: reference.stats,
        videoBoxReference: videoBoxReference(original!, e, MIXED.some((view) => name(view) === label) ? aggregateErrors : undefined) } : {}),
      ...(KNOWN[label] ? { known: KNOWN[label]!.why } : {}) };
    console.log(`viewer canvas ${label}: ${t.width}×${t.height} ${JSON.stringify(t.geometry)} ${JSON.stringify(stats)} outside the video ${JSON.stringify(outside)} content ${JSON.stringify(e.content)}`);
    if (evidence) {
      const slug = label.replace(/[^a-z0-9]+/gi, '-').toLowerCase();
      await writeFile(path.join(evidence, `${slug}-today.png`), png(t.width, t.height, rgba(t)));
      await writeFile(path.join(evidence, `${slug}-engine.png`), png(e.width, e.height, rgba(e)));
      await writeFile(path.join(evidence, `${slug}-diff.png`), png(t.width, t.height, diff));
    }
  };
  for (const view of [...MIXED, ...BEYOND]) {
    const label = name(view), t = today[label]!, e = engine[label]!;
    const { crop: todayCrop, ...todayView } = t.geometry as any, { crop: engineCrop, ...engineView } = e.geometry as any;
    expect(engineView, label).toEqual(todayView);
    // The engine drew the crop today drew (the pan buffer is kept alike).
    expect(engineCrop, label).toEqual(todayCrop);
    await record(label, t, e);
  }
  for (const label of Object.keys(today).filter((k) => k.endsWith(' original video'))) await record(label, today[label]!, engine[label.replace(' original video', '')]!);
  const summarize = (bins: Uint32Array) => {
    let samples = 0, sum = 0, max = 0;
    for (let error = 0; error < bins.length; error++) {
      const count = bins[error]!;
      samples += count; sum += error * count;
      if (count) max = error;
    }
    let seen = 0, p999 = 0;
    for (let error = 0; error < bins.length; error++) {
      seen += bins[error]!;
      if (seen >= Math.ceil(samples * 0.999)) { p999 = error; break; }
    }
    return { samples, max, p999, mean: sum / samples };
  };
  const aggregate = { today: summarize(aggregateErrors.today), native: summarize(aggregateErrors.native),
    regression: { ...aggregateErrors.regression, fraction: aggregateErrors.regression.withinOne / aggregateErrors.regression.pixels } };
  console.log(`viewer video corrected-reference aggregate ${JSON.stringify(aggregate)}`);
  expect(aggregate.native.samples).toBe(aggregate.today.samples);
  expect(aggregate.regression.pixels).toBe(aggregate.native.samples);
  expect.soft(aggregate.regression.fraction, 'full video box: native error exceeds today by at most 1 CV for 99.9% of pixels').toBeGreaterThanOrEqual(0.999);
  expect.soft(aggregate.regression.max, 'full video box: native error exceeds today by at most 4 CV').toBeLessThanOrEqual(4);
  for (const metric of ['max', 'p999', 'mean'] as const)
    expect.soft(aggregate.native[metric], `full video box: native ${metric} no worse than today`).toBeLessThanOrEqual(aggregate.today[metric]);
  if (evidence) await writeFile(path.join(evidence, 'results.json'), JSON.stringify(results, null, 1));
  for (const [label, r] of Object.entries(results)) {
    expect.soft(r.outsideVideo.passes, `${label}: nonvideo viewer parity`).toBe(true);
    expect.soft(!!r.videoPixel, `${label}: video reference is present exactly in the mixed project`)
      .toBe(!BEYOND.some((view) => name(view) === label));
    if (!r.videoPixel) {
      expect.soft(r.passes, `${label}: whole stage without video`).toBe(true);
      continue;
    }
    // The browser-scaled stage is checked across the clip's uniform red interior.
    expect.soft(r.videoReference?.passes, `${label}: displayed video interior vs f64 reference`).toBe(true);
    expect.soft(r.videoBoxReference?.passes, `${label}: video border and viewport scaling vs corrected original-video coverage`).toBe(true);
    expect.soft(referenceError(r.videoPixel), `${label}: displayed video center vs f64 reference`).toBeLessThanOrEqual(0.59);
    const sampled = (r.content as any)?.videoPixel;
    if (sampled) {
      expect.soft((r.content as any).videoReference?.passes, `${label}: native snapshot video interior vs f64 reference`).toBe(true);
      expect.soft(referenceError(sampled.engine), `${label}: native source colour vs f64 reference`).toBeLessThanOrEqual(0.59);
      expect.soft(referenceError(sampled.engine), `${label}: native source no worse than today`).toBeLessThanOrEqual(referenceError(sampled.legacy));
      expect.soft(sampled.engine, `${label}: displayed video and native snapshot agree`).toEqual(r.videoPixel);
    }
  }
});
