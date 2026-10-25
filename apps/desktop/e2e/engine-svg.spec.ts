/*
 * SVG images with "Use New Engine" on: the engine draws them natively at
 * output resolution (docs/engine/vector.md). Same or better than today:
 * each frame is compared, with today's compositor (`PM.GL.renderToPixels`)
 * and the engine (`PM.EngineRender.snapshotSync`), against Chromium's own
 * drawing of the same SVGs at 16x supersampling, box-filtered down:
 *   (a) interior pixels (more than 2 px from any reference edge): the engine
 *       within 1 CV of today;
 *   (b) edge pixels: per channel |engine − ref| ≤ |today − ref| + 1 CV for
 *       99.9%, + 4 CV for all, and the engine's mean error ≤ today's;
 *   (c) geometry of each layer alone: alpha area and alpha centroid of the
 *       engine within 1e-3 (relative, and px) of the reference;
 *   (d) no window SVG raster route exists.
 * Edits and engine restarts are compared engine to engine, exactly. The
 * job engine's export and both CLI entry points are compared with the
 * engine's frames.
 *
 * Run serially, one Electron at a time:
 *   pm-electron-lock bunx playwright test e2e/engine-svg.spec.ts --workers=1
 */
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { inflateSync } from 'node:zlib';
import os from 'node:os';
import path from 'node:path';
import type { Page } from 'playwright';

import { expect, test, launchApp, type LaunchedApp } from './helpers/app';
import { requestJobEngine } from './helpers/job-engine';

const ON = { POWERMOVE_NEW_ENGINE: '1' };
const TIMES = [0, 0.75, 1.5];

/** An 8-bit RGB or RGBA, non-interlaced PNG's pixels (as RGBA). */
function decodePng(file: Buffer): { width: number; height: number; data: Uint8Array } {
  let at = 8, width = 0, height = 0, channels = 4;
  const idat: Buffer[] = [];
  while (at < file.length) {
    const length = file.readUInt32BE(at), type = file.toString('latin1', at + 4, at + 8), body = file.subarray(at + 8, at + 8 + length);
    if (type === 'IHDR') {
      width = body.readUInt32BE(0); height = body.readUInt32BE(4);
      if (body[8] !== 8 || body[12] !== 0) throw new Error('only 8-bit, non-interlaced PNGs');
      channels = body[9] === 6 ? 4 : body[9] === 2 ? 3 : 0;
      if (!channels) throw new Error(`PNG color type ${body[9]}`);
    }
    if (type === 'IDAT') idat.push(body);
    at += 12 + length;
  }
  const raw = inflateSync(Buffer.concat(idat)), stride = width * channels;
  const rows = new Uint8Array(stride * height), data = new Uint8Array(width * height * 4);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)]!, src = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x++) {
      const left = x >= channels ? rows[y * stride + x - channels]! : 0, up = y ? rows[(y - 1) * stride + x]! : 0, ul = x >= channels && y ? rows[(y - 1) * stride + x - channels]! : 0;
      const p = left + up - ul, pa = Math.abs(p - left), pb = Math.abs(p - up), pc = Math.abs(p - ul);
      const predictor = [0, left, up, (left + up) >> 1, pa <= pb && pa <= pc ? left : pb <= pc ? up : ul][filter]!;
      rows[y * stride + x] = (src[x]! + predictor) & 255;
    }
  }
  for (let i = 0, j = 0; i < width * height; i++, j += channels) {
    data[i * 4] = rows[j]!; data[i * 4 + 1] = rows[j + 1]!; data[i * 4 + 2] = rows[j + 2]!; data[i * 4 + 3] = channels === 4 ? rows[j + 3]! : 255;
  }
  return { width, height, data };
}

/** The corpus bound between two RGBA frames: 99.9% of pixels within 1 CV in
 *  every channel, none beyond 4. */
function expectCorpusBound(label: string, a: ArrayLike<number>, b: ArrayLike<number>, pixels: number): void {
  expect(b.length, label).toBe(a.length);
  let withinOne = 0, max = 0;
  for (let i = 0; i < a.length; i += 4) {
    let delta = 0;
    for (let c = 0; c < 4; c++) delta = Math.max(delta, Math.abs(a[i + c]! - b[i + c]!));
    if (delta <= 1) withinOne++;
    max = Math.max(max, delta);
  }
  console.log(`${label}: ${JSON.stringify({ fraction: withinOne / pixels, max })}`);
  expect(withinOne / pixels, label).toBeGreaterThanOrEqual(0.999);
  expect(max, label).toBeLessThanOrEqual(4);
}

/** A gradient, a clip, a stroke, a blur and an even-odd path. */
const ART = `<svg xmlns="http://www.w3.org/2000/svg" width="160" height="90" viewBox="0 0 160 90">
<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#4C8DFF"/><stop offset="1" stop-color="#E83A5E" stop-opacity=".6"/></linearGradient>
<clipPath id="c"><circle cx="120" cy="45" r="34"/></clipPath><filter id="b"><feGaussianBlur stdDeviation="2.5"/></filter></defs>
<rect x="4" y="4" width="152" height="82" rx="14" fill="url(#g)"/>
<g clip-path="url(#c)"><rect x="80" y="0" width="80" height="90" fill="#FFD23F"/><path d="M80 90 L160 0" stroke="#14161C" stroke-width="6"/></g>
<path fill-rule="evenodd" d="M20 20 h50 v50 h-50 z M32 32 h26 v26 h-26 z" fill="#FFFFFF" filter="url(#b)"/>
<path d="M10 80 Q 60 40 110 80" fill="none" stroke="#7CFFB2" stroke-width="3.5" stroke-dasharray="7 4" stroke-linecap="round"/>
</svg>`;
const RECOLORED = ART.replace('#FFD23F', '#00C2A8');

/** The project: three SVG image layers (contain at 100%; cover, rotated 23°
 *  at 230%; stretch with animated scale and 70% opacity) over a solid
 *  background, 640 × 360. */
async function svgProject(page: Page, svg = ART): Promise<void> {
  await page.evaluate(async (svg) => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'SVG images', w: 640, h: 360, fps: 30, dur: 2, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    PM.replaceProject(project);
    const asset = await PM.assets.add(new File([svg], 'art.svg', { type: 'image/svg+xml' }), { newAsset: true, silent: true });
    const layer = (name: string, d: any, x: number, y: number) => {
      const l = PM.mkLayer('image', { name, from: 0, dur: 2, d: { asset: asset.id, ...d } }, PM.proj);
      l.p['position.x'].v = x; l.p['position.y'].v = y;
      return l;
    };
    const a = layer('Contain', { fit: 'contain', w: 200, h: 140 }, 130, 100);
    const b = layer('Cover', { fit: 'cover', w: 120, h: 120 }, 420, 120);
    b.p.rotation.v = 23; b.p['scale.x'].v = 230; b.p['scale.y'].v = 230;
    const c = layer('Stretch', { fit: 'stretch', w: 160, h: 60 }, 300, 290);
    c.p['scale.x'].kf = [PM.KF(0, 60), PM.KF(2, 310)];
    c.p.opacity.v = 70;
    PM.proj.layers = [b, c, a];
    PM.ProjectIndex?.invalidate?.();
    PM.touch();
    PM.Projects?.put?.(PM.proj);
  }, svg);
}

/** Shows the SVG project in the editor with the engine drawing the viewer. */
async function showEngine(session: LaunchedApp, svg = ART): Promise<void> {
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.EngineRender));
  await svgProject(page, svg);
  await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.presenting === true), { timeout: 20_000 }).toBe(true);
}

/** (a)–(c) for one frame, measured in the window (see the header). */
const measure = (page: Page, time: number) => page.evaluate(async (t) => {
  const PM = (window as any).PM, W = 640, H = 360;
  type Draw = { image: CanvasImageSource; sw: number; sh: number; fit: string; bw: number; bh: number; m: number[]; alpha: number };
  // Bottom layer first: today's compositor draws the list from its end.
  const drawsOf = async (layers: any[]): Promise<Draw[]> => {
    const draws: Draw[] = [];
    for (const L of [...layers].reverse()) {
      const asset = PM.assets.get(L.d.asset);
      await asset.el.decode?.();
      draws.push({ image: asset.el, sw: asset.w, sh: asset.h, fit: L.d.fit, bw: L.d.w, bh: L.d.h, m: [...PM.worldMatrix(L, t)], alpha: Math.min(1, Math.max(0, PM.ev(L, 'opacity', t) / 100)) });
    }
    return draws;
  };
  // Chromium's drawing at S× of the region (x0, y0, w, h), box-filtered to
  // one sample per pixel: premultiplied RGBA (0–255, unrounded), and each
  // pixel's spread (the largest channel range among its S × S samples).
  // Each layer is drawn as today's quad (box w × h about its centre, the
  // image fitted as today's UV window fits it) into its own canvas, then
  // composited with its opacity, as today's layer texture is.
  const reference = (draws: Draw[], S: number, x0: number, y0: number, w: number, h: number, background: string | null) => {
    const T = Math.floor(2048 / S), size = T * S;
    const make = () => {
      const canvas = document.createElement('canvas');
      canvas.width = size; canvas.height = size;
      return [canvas, canvas.getContext('2d', { willReadFrequently: true })!] as const;
    };
    const [, ctx] = make(), [layerCanvas, layer] = make();
    const color = new Float64Array(w * h * 4), spread = new Uint8Array(w * h);
    for (let ty = 0; ty < h; ty += T) for (let tx = 0; tx < w; tx += T) {
      const tw = Math.min(T, w - tx), th = Math.min(T, h - ty);
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.clearRect(0, 0, size, size);
      if (background) { ctx.fillStyle = background; ctx.fillRect(0, 0, size, size); }
      for (const d of draws) {
        layer.clearRect(0, 0, size, size);
        layer.save();
        const m = d.m;
        layer.setTransform(S * m[0]!, S * m[1]!, S * m[2]!, S * m[3]!, S * (m[4]! - x0 - tx), S * (m[5]! - y0 - ty));
        const { bw, bh } = d, ar = d.sw / d.sh, br = bw / bh;
        if (d.fit === 'cover') {
          layer.beginPath(); layer.rect(-bw / 2, -bh / 2, bw, bh); layer.clip();
          const [iw, ih] = ar > br ? [bh * ar, bh] : [bw, bw / ar];
          layer.drawImage(d.image, -iw / 2, -ih / 2, iw, ih);
        } else if (d.fit === 'contain') {
          const [iw, ih] = ar > br ? [bw, bw / ar] : [bh * ar, bh];
          layer.drawImage(d.image, -iw / 2, -ih / 2, iw, ih);
        } else {
          layer.drawImage(d.image, -bw / 2, -bh / 2, bw, bh);
        }
        layer.restore();
        ctx.globalAlpha = d.alpha;
        ctx.drawImage(layerCanvas, 0, 0);
        ctx.globalAlpha = 1;
      }
      const data = ctx.getImageData(0, 0, tw * S, th * S).data, rowBytes = tw * S * 4, n = S * S;
      for (let y = 0; y < th; y++) for (let x = 0; x < tw; x++) {
        let r = 0, g = 0, b = 0, a = 0, rl = 255, rh = 0, gl = 255, gh = 0, bl = 255, bh = 0;
        for (let sy = 0; sy < S; sy++) {
          let i = (y * S + sy) * rowBytes + x * S * 4;
          for (let sx = 0; sx < S; sx++, i += 4) {
            const R = data[i]!, G = data[i + 1]!, B = data[i + 2]!, A = data[i + 3]!;
            r += R * A; g += G * A; b += B * A; a += A;
            if (R < rl) rl = R; if (R > rh) rh = R;
            if (G < gl) gl = G; if (G > gh) gh = G;
            if (B < bl) bl = B; if (B > bh) bh = B;
          }
        }
        const p = (ty + y) * w + tx + x;
        color[p * 4] = r / 255 / n; color[p * 4 + 1] = g / 255 / n; color[p * 4 + 2] = b / 255 / n; color[p * 4 + 3] = a / n;
        spread[p] = Math.max(rh - rl, gh - gl, bh - bl);
      }
    }
    return { color, spread };
  };
  const snapshot = (pixels: string, background: boolean): Uint8ClampedArray =>
    PM.EngineRender.snapshotSync({ time: t, width: W, height: H, pixels, motionBlurSamples: 0, background }).data;

  // The frame: engine, today, and the reference over the same background.
  const engine = snapshot('opaque', true);
  const today: Uint8Array = PM.GL.renderToPixels(t, W, H, { opaque: true, topDownOpaque: true, mblur: false });
  const ref = reference(await drawsOf(PM.proj.layers), 16, 0, 0, W, H, '#203040');
  // A reference edge: a pixel whose samples spread more than 4 CV (a
  // gradient moves about 1 CV a pixel). Interior: no edge within 2 px.
  const near = new Uint8Array(W * H);
  for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
    if (ref.spread[y * W + x]! <= 4) continue;
    for (let dy = -2; dy <= 2; dy++) for (let dx = -2; dx <= 2; dx++) {
      const yy = y + dy, xx = x + dx;
      if (yy >= 0 && yy < H && xx >= 0 && xx < W) near[yy * W + xx] = 1;
    }
  }
  const interior = { count: 0, max: 0 };
  const edges = { pixels: 0, samples: 0, over1: 0, maxExcess: -Infinity, engineMean: 0, todayMean: 0 };
  let nonBackground = 0;
  for (let p = 0; p < W * H; p++) {
    const i = p * 4;
    if (today[i] !== 0x20 || today[i + 1] !== 0x30 || today[i + 2] !== 0x40) nonBackground++;
    if (!near[p]) {
      // Opaque reference pixels only (the frame is opaque throughout).
      if (ref.color[i + 3]! < 254.5) continue;
      interior.count++;
      for (let c = 0; c < 3; c++) interior.max = Math.max(interior.max, Math.abs(engine[i + c]! - today[i + c]!));
      continue;
    }
    edges.pixels++;
    for (let c = 0; c < 3; c++) {
      const e = Math.abs(engine[i + c]! - ref.color[i + c]!), d = Math.abs(today[i + c]! - ref.color[i + c]!);
      edges.samples++;
      if (e > d + 1) edges.over1++;
      edges.maxExcess = Math.max(edges.maxExcess, e - d);
      edges.engineMean += e; edges.todayMean += d;
    }
  }
  edges.engineMean /= edges.samples; edges.todayMean /= edges.samples;

  // Geometry: each layer alone, without the background.
  const moments = (alpha: (x: number, y: number) => number, x0: number, y0: number, w: number, h: number) => {
    let s = 0, sx = 0, sy = 0;
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
      const a = alpha(x, y);
      s += a; sx += a * (x0 + x + 0.5); sy += a * (y0 + y + 0.5);
    }
    return { area: s / 255, x: sx / s, y: sy / s };
  };
  const geometry = [];
  const layers = PM.proj.layers;
  for (const L of layers) {
    PM.proj.layers = [L]; PM.ProjectIndex?.invalidate?.(); PM.touch();
    let alone: Uint8ClampedArray;
    try { alone = snapshot('premultiplied', false); }
    finally { PM.proj.layers = layers; PM.ProjectIndex?.invalidate?.(); PM.touch(); }
    const [draw] = await drawsOf([L]);
    // The layer's box on the frame, 2 px wider.
    const m = draw!.m, xs: number[] = [], ys: number[] = [];
    for (const [X, Y] of [[-1, -1], [1, -1], [1, 1], [-1, 1]]) {
      const bx = X! * draw!.bw / 2, by = Y! * draw!.bh / 2;
      xs.push(m[0]! * bx + m[2]! * by + m[4]!); ys.push(m[1]! * bx + m[3]! * by + m[5]!);
    }
    const x0 = Math.max(0, Math.floor(Math.min(...xs)) - 2), y0 = Math.max(0, Math.floor(Math.min(...ys)) - 2);
    const x1 = Math.min(W, Math.ceil(Math.max(...xs)) + 2), y1 = Math.min(H, Math.ceil(Math.max(...ys)) + 2);
    const at = (S: number) => {
      const { color } = reference([draw!], S, x0, y0, x1 - x0, y1 - y0, null);
      return moments((x, y) => color[(y * (x1 - x0) + x) * 4 + 3]!, x0, y0, x1 - x0, y1 - y0);
    };
    geometry.push({ layer: L.name, engine: moments((x, y) => alone[(y * W + x) * 4 + 3]!, 0, 0, W, H), ref16: at(16), ref8: at(8) });
  }
  return { nonBackground, interior, edges, geometry };
}, time);

/** Engine frames kept in the window by name. */
const keep = (page: Page, name: string, time: number) => page.evaluate(({ name, time }) => {
  const w = window as any;
  (w.__svgFrames ??= {})[name] = w.PM.EngineRender.snapshotSync({ time, width: 640, height: 360, pixels: 'opaque', motionBlurSamples: 0, background: true }).data.slice();
}, { name, time });

/** Two kept frames: the bytes that differ. */
const differ = (page: Page, a: string, b: string) => page.evaluate(({ a, b }) => {
  const frames = (window as any).__svgFrames, x = frames[a], y = frames[b];
  let differ = 0, max = 0;
  for (let i = 0; i < x.length; i++) {
    const d = Math.abs(x[i] - y[i]);
    if (d) differ++;
    if (d > max) max = d;
  }
  return { differ, max };
}, { a, b });

/** Keeps the first engine frame that differs from frame `from` as `into`. */
const nextFrame = (page: Page, from: string, into: string, time: number) => page.evaluate(async ({ from, into, time }) => {
  const w = window as any, old = w.__svgFrames[from];
  const until = performance.now() + 20_000;
  while (performance.now() < until) {
    const now = w.PM.EngineRender.snapshotSync({ time, width: 640, height: 360, pixels: 'opaque', motionBlurSamples: 0, background: true }).data;
    if (now.some((v: number, i: number) => v !== old[i])) { w.__svgFrames[into] = now.slice(); return true; }
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  return false;
}, { from, into, time });

const SAME = { differ: 0, max: 0 };

test.describe('Use New Engine: SVG images', () => {
  test.use({ desktopLaunchOptions: { env: ON } });

  test('the engine draws SVG images natively, at least as close to Chromium’s own drawing as today', async ({ session }) => {
    test.setTimeout(240_000);
    const { page } = session;
    await session.openEditor();
    await showEngine(session);
    // (d) No window SVG raster route.
    expect(await page.evaluate(() => (window as any).PM.EngineSvgRasters)).toBeUndefined();
    for (const t of TIMES) {
      const m = await measure(page, t);
      console.log(`SVG frame at ${t}: ${JSON.stringify(m)}`);
      expect(m.nonBackground).toBeGreaterThan(20_000);
      // (a) Interiors.
      expect(m.interior.count).toBeGreaterThan(100_000);
      expect(m.interior.max, `interior at ${t}`).toBeLessThanOrEqual(1);
      // (b) Edges.
      expect(m.edges.pixels).toBeGreaterThan(1_000);
      expect(m.edges.over1 / m.edges.samples, `edges beyond today + 1 CV at ${t}`).toBeLessThanOrEqual(0.001);
      expect(m.edges.maxExcess, `edges beyond today at ${t}`).toBeLessThanOrEqual(4);
      expect(m.edges.engineMean, `mean edge error at ${t}`).toBeLessThanOrEqual(m.edges.todayMean);
      // (c) Geometry. The 16x reference is itself an approximation: its
      // error is about its change from 8x (supersampling converges), so
      // where that change exceeds 1e-3 it is the bound instead.
      for (const g of m.geometry) {
        const areaBound = Math.max(1e-3, Math.abs(g.ref16.area - g.ref8.area) / g.ref16.area);
        const centroidBound = Math.max(1e-3, Math.hypot(g.ref16.x - g.ref8.x, g.ref16.y - g.ref8.y));
        expect(Math.abs(g.engine.area - g.ref16.area) / g.ref16.area, `${g.layer} area at ${t}`).toBeLessThanOrEqual(areaBound);
        expect(Math.hypot(g.engine.x - g.ref16.x, g.engine.y - g.ref16.y), `${g.layer} centroid at ${t}`).toBeLessThanOrEqual(centroidBound);
      }
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('a replaced SVG is drawn from its new bytes at once, as a fresh render of them', async ({ session }) => {
    const { page } = session;
    await session.openEditor();
    await showEngine(session, RECOLORED);
    await keep(page, 'fresh', 0.5);
    await showEngine(session);
    await keep(page, 'before', 0.5);
    expect(await differ(page, 'before', 'fresh')).not.toEqual(SAME);
    await page.evaluate(async (svg) => {
      const PM = (window as any).PM;
      const asset = Object.values(PM.proj.assets).find((a: any) => a.format === 'svg') as any;
      await PM.assets.replace(asset.id, new File([svg], 'art.svg', { type: 'image/svg+xml' }));
    }, RECOLORED);
    // The first frame that changed is the new bytes' whole frame.
    expect(await nextFrame(page, 'before', 'replaced', 0.5)).toBe(true);
    expect(await differ(page, 'replaced', 'fresh')).toEqual(SAME);
    // Undo draws the old bytes again, and redo the new ones.
    await page.evaluate(() => (window as any).PM.hist.undo());
    expect(await nextFrame(page, 'replaced', 'undone', 0.5)).toBe(true);
    expect(await differ(page, 'undone', 'before')).toEqual(SAME);
    await page.evaluate(() => (window as any).PM.hist.redo());
    expect(await nextFrame(page, 'undone', 'redone', 0.5)).toBe(true);
    expect(await differ(page, 'redone', 'fresh')).toEqual(SAME);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('a restarted engine draws the same frames', async ({ session }) => {
    const { page } = session;
    await session.openEditor();
    await showEngine(session);
    for (const t of TIMES) await keep(page, `before ${t}`, t);
    const connections = await page.evaluate(() => (window as any).PM.EngineConnection.status().connections as number);
    const pid = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.status().interactive.pid as number);
    expect(pid).toBeGreaterThan(0);
    process.kill(pid, 'SIGKILL');
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().connections), { timeout: 45_000 }).toBeGreaterThan(connections);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    for (const t of TIMES) {
      await keep(page, `after ${t}`, t);
      expect({ t, ...await differ(page, `before ${t}`, `after ${t}`) }).toEqual({ t, ...SAME });
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});

test('powermove render draws SVG images as the desktop export', async () => {
  test.setTimeout(240_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-engine-svg-'));
  let session: LaunchedApp | undefined;
  const repo = path.resolve(__dirname, '../../..');
  const binary = process.env.POWERMOVE_PM_CLI ?? path.join(repo, 'engine/target/debug/pm-cli');
  const dist = path.join(repo, 'packages/cli/dist');
  const run = promisify(execFile);
  try {
    session = await launchApp({ env: ON });
    const { app, page } = session;
    await session.openEditor();
    await requestJobEngine(app);
    await expect.poll(() => app.evaluate(() => {
      const status = (globalThis as { __powermoveNativeEngine?: { engine: { status(): Record<string, { state: string }> } | null } }).__powermoveNativeEngine?.engine?.status();
      return status ? `${status['interactive']!.state}/${status['job']!.state}` : 'missing';
    }), { timeout: 30_000 }).toBe('ready/ready');
    await showEngine(session);
    const save = (filePath: string) => app.evaluate(({ dialog }, filePath) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath });
    }, filePath);
    const projectFile = path.join(root, 'svg.pmv');
    await save(projectFile);
    await page.evaluate(async () => { await (window as any).PM.saveProject(); });
    for (const time of [0, 1, 1.5]) {
      // The job engine's still draws the interactive engine's frame.
      const desktopFile = path.join(root, `desktop-${time}.png`);
      await save(desktopFile);
      const result = await page.evaluate(async (time) => {
        const PM = (window as any).PM;
        PM.setTime(time, { raw: true, force: true });
        const out = await PM.Export.run({ format: 'still', scale: 1, fps: 30, range: 'all', quality: 'high', mblur: false, audio: false });
        const frame = PM.EngineRender.snapshotSync({ time, width: 640, height: 360, pixels: 'opaque', motionBlurSamples: 0, background: true });
        return { out, frame: Array.from(frame.data as Uint8ClampedArray) };
      }, time);
      expect(result.out).toEqual({ cancelled: false });
      const desktop = decodePng(await readFile(desktopFile));
      expect([desktop.width, desktop.height]).toEqual([640, 360]);
      expectCorpusBound(`desktop export vs engine frame at ${time}`, desktop.data, result.frame, 640 * 360);
      for (const entry of ['powermove', 'pm-cli'] as const) {
        const file = path.join(root, `${entry}-${time}.png`);
        const options = ['--format', 'still', '--range', `${time}:${time + 1 / 30}`];
        const command = entry === 'powermove' ? process.execPath : binary;
        const args = entry === 'powermove'
          ? [path.join(repo, 'packages/cli/bin/powermove.mjs'), 'render', projectFile, file, ...options]
          : ['render', projectFile, '-o', file, ...options];
        const output = await run(command, args, {
          timeout: 45_000,
          env: { ...process.env, POWERMOVE_PM_CLI: binary, POWERMOVE_USER_DATA: session.userData,
            POWERMOVE_ANGLE_ENDPOINT: undefined, POWERMOVE_ANGLE_TOKEN: undefined,
            POWERMOVE_ANGLE_HELPER: path.join(dist, 'native-angle-child.cjs') },
        });
        expect(output.stderr).not.toContain('approximate SVG');
        expect(JSON.parse(output.stdout).approximateSvg).toBeUndefined();
        const cli = decodePng(await readFile(file));
        expect([cli.width, cli.height]).toEqual([640, 360]);
        expectCorpusBound(`desktop export vs ${entry} at ${time}`, desktop.data, cli.data, 640 * 360);
      }
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    try { await session?.close(); } finally { await rm(root, { recursive: true, force: true }); }
  }
});
