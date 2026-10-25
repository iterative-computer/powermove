/*
 * SVG images with "Use New Engine" on: the engine draws them natively at
 * output resolution (docs/engine/vector.md). Same or better than today:
 * each frame is compared, with today's compositor (`PM.GL.renderToPixels`)
 * and the engine (`PM.EngineRender.snapshotSync`), against an independent
 * f64 render of the authored SVG and its live placement matrices:
 *   (a) interior pixels (more than 2 px from any reference edge): the engine
 *       within 1 CV of the authored reference;
 *   (b) edge pixels: max channel |engine − ref| ≤ |today − ref| + 1 CV for
 *       99.9%, + 4 CV for all; max, p99.9 and mean error ≤ today's + 0.5 CV;
 *   (c) geometry of each layer alone: alpha area and alpha centroid of the
 *       RGBA32F engine within 1e-3 (relative, and px) of the reference;
 *       the window's RGBA8 bytes equal that float frame's rounded endpoint;
 *   (d) no window SVG raster route exists.
 * Edits and engine restarts are compared engine to engine, exactly. The
 * job engine's export and both CLI entry points are compared with the
 * engine's frames.
 *
 * Run serially, one Electron at a time:
 *   PM_SVG_REFERENCE_BIN=<prebuilt svg_filters test binary>
 *     pm-electron-lock bunx playwright test -c playwright.consumers.config.ts
 *       e2e/engine-svg.spec.ts --workers=1
 */
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
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
    const project = PM.mkProject({ name: 'SVG images', w: 640, h: 360, fps: 30, dur: 2, bg: '#203040', colorDepth: 32 });
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

/** Live authored placement and actual 8-bit endpoints. Reference math runs in Rust. */
const measure = async (page: Page, time: number) => {
  const captured = await page.evaluate((t) => {
    const PM = (window as any).PM, W = 640, H = 360;
    const original = PM.proj.layers;
    const layers = [...original].reverse().map((L: any) => ({
      name: L.name, fit: L.d.fit, bw: L.d.w, bh: L.d.h,
      m: [...PM.worldMatrix(L, t)], alpha: Math.min(1, Math.max(0, PM.ev(L, 'opacity', t) / 100)),
    }));
    const project = JSON.parse(JSON.stringify(PM.proj));
    const asset_id = original[0]?.d.asset;
    const engine = Array.from(PM.EngineRender.snapshotSync({ time: t, width: W, height: H, pixels: 'opaque', motionBlurSamples: 0, background: true }).data as Uint8ClampedArray);
    const today = Array.from(PM.GL.renderToPixels(t, W, H, { opaque: true, topDownOpaque: true, mblur: false }) as Uint8Array);
    const geometry = [];
    for (const L of [...original].reverse()) {
      PM.proj.layers = [L]; PM.ProjectIndex?.invalidate?.(); PM.touch();
      try {
        const alone = PM.EngineRender.snapshotSync({ time: t, width: W, height: H, pixels: 'premultiplied', motionBlurSamples: 0, background: false }).data as Uint8ClampedArray;
        let area = 0, sx = 0, sy = 0;
        for (let i = 0; i < W * H; i++) {
          const a = alone[i * 4 + 3]! / 255;
          area += a; sx += a * (i % W + 0.5); sy += a * (Math.floor(i / W) + 0.5);
        }
        geometry.push({ layer: L.name, engine: { area, x: sx / area, y: sy / area }, bytes: Array.from(alone) });
      } finally {
        PM.proj.layers = original; PM.ProjectIndex?.invalidate?.(); PM.touch();
      }
    }
    return { input: { width: W, height: H, layers, project, asset_id, time: t }, engine, today, geometry };
  }, time);
  const binary = process.env.PM_SVG_REFERENCE_BIN;
  expect(binary, 'prebuild svg_filters and set PM_SVG_REFERENCE_BIN before Electron').toBeTruthy();
  const temporary = await mkdtemp(path.join(os.tmpdir(), 'pm-svg-f64-'));
  let reference: Buffer, native: Buffer, endpoint: Buffer;
  try {
    const input = path.join(temporary, 'input.json'), output = path.join(temporary, 'reference.f64');
    const nativeOutput = path.join(temporary, 'native.f32'), endpointOutput = path.join(temporary, 'endpoint.rgba');
    const artPath = path.join(temporary, 'art.svg');
    await writeFile(artPath, ART);
    await writeFile(input, JSON.stringify({ ...captured.input, svg_sha256: createHash('sha256').update(ART).digest('hex') }));
    const ffmpegLib = path.join(process.env.PM_FFMPEG_DIR ?? path.join(os.homedir(), 'Library/Caches/powermove-engine/ffmpeg/out'), 'lib');
    await promisify(execFile)(binary!, ['--exact', 'art_reference_export', '--nocapture'], {
      env: { ...process.env, DYLD_LIBRARY_PATH: [ffmpegLib, process.env.DYLD_LIBRARY_PATH].filter(Boolean).join(':'),
        PM_SVG_REFERENCE_INPUT: input, PM_SVG_REFERENCE_OUTPUT: output,
        PM_SVG_NATIVE_OUTPUT: nativeOutput, PM_SVG_ENDPOINT_OUTPUT: endpointOutput,
        PM_SVG_ART_PATH: artPath },
      timeout: 180_000,
    });
    reference = await readFile(output);
    native = await readFile(nativeOutput);
    endpoint = await readFile(endpointOutput);
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
  const W = 640, H = 360, count = W * H, layerCount = captured.input.layers.length;
  expect(reference.byteLength).toBe((4 + layerCount) * count * 8);
  expect(native.byteLength).toBe((1 + layerCount) * count * 4 * 4);
  expect(endpoint.byteLength).toBe((1 + layerCount) * count * 4);
  const ref = new Float64Array(reference.buffer.slice(reference.byteOffset, reference.byteOffset + reference.byteLength));
  const actual32 = new Float32Array(native.buffer.slice(native.byteOffset, native.byteOffset + native.byteLength));
  const nativePixel = (frame: number, i: number, c: number) => actual32[(frame * count + i) * 4 + c]!;
  const color = (i: number, c: number) => ref[i * 4 + c]! * 255;
  const alpha = (layer: number, i: number) => ref[4 * count + layer * count + i]!;
  const edge = new Uint8Array(count), near = new Uint8Array(count);
  for (let i = 0; i < count; i++) {
    const x = i % W, y = Math.floor(i / W);
    if (captured.input.layers.some((_, j) => { const a = alpha(j, i); return a > 1e-6 && a < 1 - 1e-6; })) { edge[i] = 1; continue; }
    for (const n of [x > 0 ? i - 1 : i, x + 1 < W ? i + 1 : i, y > 0 ? i - W : i, y + 1 < H ? i + W : i]) {
      if ([0, 1, 2, 3].some((c) => Math.abs(color(i, c) - color(n, c)) > 2)) { edge[i] = 1; break; }
    }
  }
  for (let i = 0; i < count; i++) if (edge[i]) {
    const x = i % W, y = Math.floor(i / W);
    for (let dy = -2; dy <= 2; dy++) for (let dx = -2; dx <= 2; dx++) {
      const xx = x + dx, yy = y + dy;
      if (xx >= 0 && xx < W && yy >= 0 && yy < H) near[yy * W + xx] = 1;
    }
  }
  const errors: number[] = [], todayErrors: number[] = [], excess: number[] = [];
  const worstPixels: Array<{ x: number; y: number; engine: number[]; native32: number[];
    nativeLayers: number[][]; today: number[]; reference: number[]; layerAlpha: number[];
    engineError: number; todayError: number; excess: number }> = [];
  let interiorCount = 0, interiorMax = 0, nonBackground = 0;
  for (let i = 0; i < count; i++) {
    const k = i * 4;
    if (captured.today[k] !== 0x20 || captured.today[k + 1] !== 0x30 || captured.today[k + 2] !== 0x40) nonBackground++;
    if (!near[i]) {
      interiorCount++;
      for (let c = 0; c < 3; c++) interiorMax = Math.max(interiorMax, Math.abs(captured.engine[k + c]! - captured.today[k + c]!));
    } else {
      let e = 0, d = 0;
      for (let c = 0; c < 3; c++) {
        e = Math.max(e, Math.abs(captured.engine[k + c]! - color(i, c)));
        d = Math.max(d, Math.abs(captured.today[k + c]! - color(i, c)));
      }
      errors.push(e); todayErrors.push(d); excess.push(e - d);
      if (e - d >= 3.5) {
        worstPixels.push({ x: i % W, y: Math.floor(i / W),
          engine: captured.engine.slice(k, k + 3),
          native32: [0, 1, 2].map((c) => nativePixel(0, i, c) * 255),
          nativeLayers: captured.input.layers.map((_, j) => [0, 1, 2, 3].map((c) =>
            nativePixel(j + 1, i, c) * 255)),
          today: captured.today.slice(k, k + 3),
          reference: [0, 1, 2].map((c) => color(i, c)),
          layerAlpha: captured.input.layers.map((_, j) => alpha(j, i)),
          engineError: e, todayError: d, excess: e - d });
      }
    }
  }
  const stats = (values: number[]) => {
    values.sort((a, b) => a - b);
    return { count: values.length, max: values.at(-1) ?? 0, p999: values[Math.ceil(0.999 * values.length) - 1] ?? 0,
      mean: values.reduce((sum, value) => sum + value, 0) / values.length };
  };
  const geometryError = (actual: { area: number; x: number; y: number },
    wanted: { area: number; x: number; y: number }) => ({
    areaRelative: Math.abs(actual.area - wanted.area) / wanted.area,
    centroidPx: Math.hypot(actual.x - wanted.x, actual.y - wanted.y),
  });
  const geometryRows: string[] = [];
  for (let j = 0; j < captured.geometry.length; j++) {
    const g = captured.geometry[j]!;
    let area = 0, sx = 0, sy = 0, fArea = 0, fSx = 0, fSy = 0;
    for (let i = 0; i < count; i++) {
      const a = alpha(j, i); area += a;
      sx += a * (i % W + 0.5); sy += a * (Math.floor(i / W) + 0.5);
      const f = nativePixel(j + 1, i, 3);
      fArea += f; fSx += f * (i % W + 0.5); fSy += f * (Math.floor(i / W) + 0.5);
    }
    // Keep each layer's raw moments and derived errors in one serialized
    // value before measuring the next layer. A frozen object still allowed a
    // prior full-suite run to pair Contain's reference with Stretch's engine
    // moments; a primitive string cannot retain a cross-layer object alias.
    const engine = { area: Number(g.engine.area), x: Number(g.engine.x), y: Number(g.engine.y) };
    const referenceGeometry = { area, x: sx / area, y: sy / area };
    const nativeGeometry = { area: fArea, x: fSx / fArea, y: fSy / fArea };
    const byteError = geometryError(engine, referenceGeometry);
    const floatError = geometryError(nativeGeometry, referenceGeometry);
    const row = JSON.stringify({ layer: String(g.layer), engine, reference: referenceGeometry,
      areaRelative: byteError.areaRelative, centroidPx: byteError.centroidPx,
      float32: { ...nativeGeometry, ...floatError } });
    const immediate = JSON.parse(row) as { layer: string; engine: typeof engine;
      reference: typeof referenceGeometry; areaRelative: number; centroidPx: number };
    const raw = geometryError(immediate.engine, immediate.reference);
    expect(immediate.areaRelative, `serialized area consistency at ${time}, ${immediate.layer}`).toBe(raw.areaRelative);
    expect(immediate.centroidPx, `serialized centroid consistency at ${time}, ${immediate.layer}`).toBe(raw.centroidPx);
    geometryRows.push(row);
  }
  type GeometryRow = {
    layer: string; engine: { area: number; x: number; y: number };
    reference: { area: number; x: number; y: number }; areaRelative: number; centroidPx: number;
    float32: { area: number; x: number; y: number; areaRelative: number; centroidPx: number };
  };
  const geometry: GeometryRow[] = [];
  for (const serialized of geometryRows) {
    const g = JSON.parse(serialized) as GeometryRow;
    const raw = geometryError(g.engine, g.reference);
    const evidence = JSON.stringify({ time, layer: g.layer, engine: g.engine,
      reference: g.reference, reported: [g.areaRelative, g.centroidPx], recomputed: raw });
    expect(g.areaRelative, `reported area consistency: ${evidence}`).toBe(raw.areaRelative);
    expect(g.centroidPx, `reported centroid consistency: ${evidence}`).toBe(raw.centroidPx);
    geometry.push(g);
  }
  const byteEndpoint = [captured.engine, ...captured.geometry.map((g) => g.bytes)].map((windowBytes, frame) => {
    let mismatches = 0, maxDelta = 0, maxQuantizationCv = 0;
    let maxExactProductCv = 0, maxF32ProductRoundoffCv = 0;
    for (let i = 0; i < count; i++) for (let c = 0; c < 4; c++) {
      const index = (frame * count + i) * 4 + c;
      const value = endpoint[index]!;
      const delta = Math.abs(windowBytes[i * 4 + c]! - value);
      mismatches += Number(delta !== 0); maxDelta = Math.max(maxDelta, delta);
      if (frame !== 0 || c !== 3) {
        const f = Math.max(0, Math.min(1, nativePixel(frame, i, c)));
        const exactProduct = f * 255;
        const storageProduct = Math.fround(exactProduct); // Rust snapshot_byte multiplies as f32.
        maxQuantizationCv = Math.max(maxQuantizationCv, Math.abs(storageProduct - value));
        maxExactProductCv = Math.max(maxExactProductCv, Math.abs(exactProduct - value));
        maxF32ProductRoundoffCv = Math.max(maxF32ProductRoundoffCv, Math.abs(exactProduct - storageProduct));
      }
    }
    return { frame: frame === 0 ? 'full' : captured.geometry[frame - 1]!.layer,
      mismatches, maxDelta, maxQuantizationCv, storageBoundCv: 0.5,
      maxExactProductCv, maxF32ProductRoundoffCv };
  });
  return { placement: { layers: captured.input.layers, svgSha256: createHash('sha256').update(ART).digest('hex') },
    nonBackground, interior: { count: interiorCount, max: interiorMax },
    edges: { pixels: errors.length, engine: stats(errors), today: stats(todayErrors), excess: stats(excess),
      over1Fraction: excess.filter((value) => value > 1).length / excess.length,
      worstPixels: worstPixels.sort((a, b) => b.excess - a.excess).slice(0, 16) }, geometry, byteEndpoint };
};

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

  test('large plain SVGs and drop-shadow sizes keep the native device alive', async ({ session }) => {
    test.setTimeout(240_000);
    const { page } = session;
    await session.openEditor();
    await page.waitForFunction(() => Boolean((window as any).PM?.EngineRender));
    for (const [width, height, boxWidth, boxHeight, sigma] of [
      [1440, 810, 1440, 810, null],
      [1920, 1080, 1440, 810, null],
      [1920, 1080, 1920, 1080, null],
      [3840, 2160, 3840, 2160, null],
      [1920, 1080, 1920, 1080, 0.5],
      [1920, 1080, 1920, 1080, 20],
    ] as const) {
      const measured = await page.evaluate(async ({ width, height, boxWidth, boxHeight, sigma }) => {
        const PM = (window as any).PM;
        const project = PM.mkProject({ name: 'SVG device limits', w: width, h: height, fps: 30, dur: 1, bg: '#000000' });
        PM.replaceProject(project);
        const source = sigma === null
          ? '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180"><rect width="320" height="180" fill="#5A93F2"/></svg>'
          : `<svg xmlns="http://www.w3.org/2000/svg" width="960" height="540"><defs><filter id="b" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="${sigma}"/></filter></defs><circle cx="480" cy="270" r="150" fill="#5A93F2" filter="url(#b)"/></svg>`;
        const asset = await PM.assets.add(new File([source], 'limits.svg', { type: 'image/svg+xml' }), { newAsset: true, silent: true });
        const layer = PM.mkLayer('image', { name: 'Native SVG', from: 0, dur: 1, d: { asset: asset.id, fit: 'contain', w: boxWidth, h: boxHeight } }, PM.proj);
        layer.p['position.x'].v = width / 2; layer.p['position.y'].v = height / 2;
        PM.proj.layers = [layer];
        PM.ProjectIndex?.invalidate?.(); PM.touch(); PM.Projects?.put?.(PM.proj);
        const snapshot = PM.EngineRender.snapshotSync({ time: 0, width, height, pixels: 'opaque', motionBlurSamples: 0, background: true });
        const at = (Math.floor(height / 2) * width + Math.floor(width / 2)) * 4;
        const center = Array.from(snapshot.data.slice(at, at + 4)) as number[];
        // A second request catches a lost device after the first submission.
        const next = PM.EngineRender.snapshotSync({ time: 0, width: 64, height: 36, pixels: 'opaque', motionBlurSamples: 0, background: true });
        return { width: snapshot.width, height: snapshot.height, center, nextBytes: next.data.length, route: PM.EngineConnection.status().route };
      }, { width, height, boxWidth, boxHeight, sigma });
      expect([measured.width, measured.height]).toEqual([width, height]);
      for (const [channel, value] of [0x5a, 0x93, 0xf2].entries()) {
        expect(Math.abs(measured.center[channel]! - value), `${width}×${height} blur=${sigma} channel=${channel}`).toBeLessThanOrEqual(1);
      }
      expect(measured.center[3]).toBe(255);
      expect(measured.nextBytes).toBe(64 * 36 * 4);
      expect(measured.route).toBe('engine');
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('the engine draws SVG images natively, at least as close to f64 as today', async ({ session }, testInfo) => {
    test.setTimeout(240_000);
    const { page } = session;
    await session.openEditor();
    await showEngine(session);
    // (d) No window SVG raster route.
    expect(await page.evaluate(() => (window as any).PM.EngineSvgRasters)).toBeUndefined();
    const measurements = [];
    for (const t of TIMES) {
      const m = await measure(page, t);
      measurements.push({ time: t, ...m });
      const report = process.env.PM_SVG_QUALITY_REPORT ?? testInfo.outputPath('svg-quality.json');
      await mkdir(path.dirname(report), { recursive: true });
      await writeFile(report, JSON.stringify({ reference: 'independent f64 authored ART and analytic Gaussian; each SVG image resolves before layer compositing',
        fixture: 'unchanged ART, three live placements',
        standards: ['https://www.w3.org/TR/filter-effects-1/', 'https://www.w3.org/TR/SVG11/filters.html'],
        frames: measurements }, null, 2));
      console.log(`SVG frame at ${t}: ${JSON.stringify(m)}`);
      expect(m.nonBackground).toBeGreaterThan(20_000);
      // (a) Interiors.
      expect(m.interior.count).toBeGreaterThan(100_000);
      expect(m.interior.max, `interior at ${t}`).toBeLessThanOrEqual(1);
      // (b) Edges.
      expect(m.edges.pixels).toBeGreaterThan(1_000);
      expect(m.edges.over1Fraction, `edges beyond today + 1 CV at ${t}`).toBeLessThanOrEqual(0.001);
      expect(m.edges.excess.p999, `edge p99.9 excess at ${t}`).toBeLessThanOrEqual(1);
      expect(m.edges.excess.max, `edge max excess at ${t}`).toBeLessThanOrEqual(4);
      for (const statistic of ['max', 'p999', 'mean'] as const) {
        expect(m.edges.engine[statistic], `${statistic} edge error at ${t}`).toBeLessThanOrEqual(m.edges.today[statistic] + 0.5);
      }
      // (c) Exact authored geometry uses the native RGBA32F live project.
      // The reported RGBA8 geometry retains the visible snapshot evidence.
      for (const g of m.geometry) {
        expect(g.float32.areaRelative, `${g.layer} f32 area at ${t}`).toBeLessThanOrEqual(1e-3);
        expect(g.float32.centroidPx, `${g.layer} f32 centroid at ${t}`).toBeLessThanOrEqual(1e-3);
      }
      // The window's 8-bit API must be the exact round-to-even endpoint of
      // those same float frames (with the declared half-CV storage interval).
      for (const endpoint of m.byteEndpoint) {
        expect(endpoint.maxQuantizationCv, `${endpoint.frame} 8-bit storage at ${t}`).toBeLessThanOrEqual(0.5);
        expect(endpoint.mismatches, `${endpoint.frame} window/native bytes at ${t}`).toBe(0);
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
