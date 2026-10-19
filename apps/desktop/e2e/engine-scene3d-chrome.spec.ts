import { mkdir, writeFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import os from 'node:os';
import { build } from 'esbuild';
import type { Page } from 'playwright';
import { expect, test, type LaunchedApp } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

/*
 * The window stays hidden. capturePage reads Chromium's composed pixels, not a
 * screen grab or fixture. Only this test temporarily selects the old overlay
 * branch to get a same-scene reference; the engine branch is asserted before
 * every native capture. The stage underneath the overlay is painted black in
 * both captures so its independent presentation does not enter the comparison.
 *
 * pm-electron-lock bunx playwright test e2e/engine-scene3d-chrome.spec.ts --workers=1
 */
test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });

type Image = { width: number; height: number; pixels: Buffer; png: Buffer; bounds: { x: number; y: number; width: number; height: number } };
type Color = 'grid' | 'selected' | 'active';
// Chromium capturePage's PNG RGB for the two THREE.Color shader uniforms in
// this hidden-window profile (the uniforms themselves are linear).
const OUTLINE_RGB = { selected: [206, 52, 31], active: [237, 113, 49] } as const;
const outlinePixel = (b: Buffer, i: number, color: Exclude<Color, 'grid'>): boolean => {
  const want = OUTLINE_RGB[color];
  return Math.abs(b[i]! - want[0]) <= 1 && Math.abs(b[i + 1]! - want[1]) <= 1 && Math.abs(b[i + 2]! - want[2]) <= 1;
};

async function settledNative(page: Page): Promise<void> {
  try { await expect.poll(() => page.evaluate(() => {
    const PM = (window as any).PM;
    const presentation = PM.Kernel.services.get('viewer')?.enginePresentation;
    const frame = presentation?.latest;
    const selected = PM.sel?.layers || [];
    const reported = frame?.depthId?.selection?.selected || [];
    return !!PM.EngineDoc?.enabled && !!presentation?.presenting &&
      frame?.depthId?.encoding === 'chromePlanesV1' &&
      frame.viewSeq === presentation.state?.viewSeq &&
      frame.revision >= (PM.EngineDoc.barrier?.revision ?? 0) &&
      selected.every((id: string) => reported.includes(id)) &&
      reported.every((id: string) => selected.includes(id));
  }), { timeout: 30_000 }).toBe(true); } catch (error) {
    console.log('Unsettled native chrome', await page.evaluate(() => {
      const PM = (window as any).PM, p = PM.Kernel.services.get('viewer')?.enginePresentation;
      return { enabled: PM.EngineDoc?.enabled, route: PM.EngineConnection?.status(), presenting: p?.presenting,
        frame: p?.latest, state: p?.state, revision: PM.EngineDoc?.barrier?.revision, selected: PM.sel?.layers,
        bridge: PM.Kernel.services.get('frameBridge')?.stats?.(), stats: PM.EngineStats?.snapshot?.() };
    }));
    throw error;
  }
  await expect(page.locator('[data-scene-overlay]')).toHaveAttribute('data-scene-overlay-source', 'engine');
}

async function source(page: Page, reference: boolean): Promise<void> {
  await page.evaluate(reference => {
    const PM = (window as any).PM;
    if (reference) {
      (window as any).__chromeOriginalEngineDoc = PM.EngineDoc;
      PM.EngineDoc = { ...PM.EngineDoc, enabled: false };
    } else {
      PM.EngineDoc = (window as any).__chromeOriginalEngineDoc;
      delete (window as any).__chromeOriginalEngineDoc;
    }
    PM.bus.emit('selection');
  }, reference);
  await expect(page.locator('[data-scene-overlay]')).toHaveAttribute('data-scene-overlay-source', reference ? 'reference' : 'engine');
  // Flush the WebGL drawing buffer and Chromium compositor before capture.
  await page.locator('[data-scene-overlay]').evaluate((canvas: HTMLCanvasElement) => { canvas.toDataURL(); });
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
}

async function capture(session: LaunchedApp): Promise<Image> {
  const rect = await session.page.locator('#stage').evaluate(stage => {
    const r = stage.getBoundingClientRect();
    const width = Math.min(600, Math.floor(r.width) - 4);
    const height = Math.min(360, Math.floor(r.height) - 4);
    return { x: Math.round(r.x + (r.width - width) / 2), y: Math.round(r.y + (r.height - height) / 2), width, height };
  });
  expect(rect.width).toBeGreaterThan(200);
  expect(rect.height).toBeGreaterThan(150);
  const bitmap = await session.app.evaluate(async ({ BrowserWindow }, bounds) => {
    const window = BrowserWindow.getAllWindows().find(w => !w.isDestroyed() && w.webContents.getURL().startsWith('app:'))!;
    const image = await window.webContents.capturePage(bounds);
    return { png: image.toPNG().toString('base64'), size: image.getSize() };
  }, rect);
  // Score the very PNG saved on failure. Electron's toBitmap() has a
  // platform-specific premultiplied BGRA layout and different color values.
  const png = Buffer.from(bitmap.png, 'base64');
  const pixels = execFileSync(ffmpegBinary, ['-v', 'error', '-f', 'png_pipe', '-i', 'pipe:0', '-frames:v', '1', '-f', 'rawvideo', '-pix_fmt', 'rgba', 'pipe:1'],
    { input: png, maxBuffer: 16 << 20 });
  expect(pixels.length).toBe(bitmap.size.width * bitmap.size.height * 4);
  return { width: bitmap.size.width, height: bitmap.size.height, pixels, png, bounds: rect };
}

let referenceBundle: Promise<string> | null = null;
async function msaaEdges(page: Page, capture: Image): Promise<{ mask: Uint8Array; samples: number }> {
  referenceBundle ??= build({
    entryPoints: [path.join(__dirname, 'helpers/chrome-reference.ts')], bundle: true,
    write: false, format: 'iife', platform: 'browser', target: 'chrome120', define: { 'import.meta.env': '{}' },
  }).then(result => result.outputFiles![0]!.text);
  const installed = await page.evaluate(() => typeof (window as any).__chromeReferenceEdges === 'function');
  if (!installed) await page.evaluate(await referenceBundle);
  const result = await page.evaluate(({ bounds, width, height }) =>
    (window as any).__chromeReferenceEdges({ ...bounds, outputWidth: width, outputHeight: height }),
  { bounds: capture.bounds, width: capture.width, height: capture.height });
  const mask = Buffer.from(result.mask, 'base64');
  expect(mask.length).toBe(capture.width * capture.height);
  expect(result.samples).toBeGreaterThanOrEqual(2);
  return { mask, samples: result.samples };
}

function counts(image: Image): Record<Color, number> {
  const out = { grid: 0, selected: 0, active: 0 };
  const b = image.pixels;
  for (let i = 0; i < b.length; i += 4) {
    const red = b[i]!, green = b[i + 1]!, blue = b[i + 2]!;
    if (red > 16 || green > 16 || blue > 16) out.grid++;
    if (outlinePixel(b, i, 'selected')) out.selected++;
    if (outlinePixel(b, i, 'active')) out.active++;
  }
  return out;
}

const gaussian = (() => {
  const weights = Array.from({ length: 11 }, (_, i) => Math.exp(-((i - 5) ** 2) / (2 * 1.5 ** 2)));
  const sum = weights.reduce((a, b) => a + b, 0);
  return weights.map(value => value / sum);
})();

function blur(src: Float64Array, width: number, height: number): Float64Array {
  const tmp = new Float64Array(src.length), out = new Float64Array(src.length);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    let sum = 0;
    for (let k = 0; k < 11; k++) sum += gaussian[k]! * src[y * width + Math.max(0, Math.min(width - 1, x + k - 5))]!;
    tmp[y * width + x] = sum;
  }
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    let sum = 0;
    for (let k = 0; k < 11; k++) sum += gaussian[k]! * tmp[Math.max(0, Math.min(height - 1, y + k - 5)) * width + x]!;
    out[y * width + x] = sum;
  }
  return out;
}

/** The parity scorer's 11×11 Gaussian SSIM, over actual ink windows. */
function inkSsim(reference: Image, native: Image): number {
  const { width, height } = reference, n = width * height;
  const inkA = new Float64Array(n), inkB = new Float64Array(n);
  for (let i = 0; i < n; i++) {
    inkA[i] = Math.max(reference.pixels[i * 4]!, reference.pixels[i * 4 + 1]!, reference.pixels[i * 4 + 2]!) / 255;
    inkB[i] = Math.max(native.pixels[i * 4]!, native.pixels[i * 4 + 1]!, native.pixels[i * 4 + 2]!) / 255;
  }
  const localInkA = blur(inkA, width, height), localInkB = blur(inkB, width, height);
  const valid = new Uint8Array(n);
  let validCount = 0;
  for (let i = 0; i < n; i++) if (localInkA[i]! > 1 / 512 || localInkB[i]! > 1 / 512) { valid[i] = 1; validCount++; }
  expect(validCount, 'SSIM must include overlay ink').toBeGreaterThan(100);
  const c1 = (0.01 * 255) ** 2, c2 = (0.03 * 255) ** 2;
  let score = 0;
  // capturePage's PNG is opaque RGBA. Excluding its constant alpha is stricter than
  // the repository's four-channel scorer; no empty channel inflates the score.
  for (let channel = 0; channel < 3; channel++) {
    const a = new Float64Array(n), b = new Float64Array(n), aa = new Float64Array(n), bb = new Float64Array(n), ab = new Float64Array(n);
    for (let i = 0; i < n; i++) {
      const x = reference.pixels[i * 4 + channel]!, y = native.pixels[i * 4 + channel]!;
      a[i] = x; b[i] = y; aa[i] = x * x; bb[i] = y * y; ab[i] = x * y;
    }
    const ma = blur(a, width, height), mb = blur(b, width, height);
    const va = blur(aa, width, height), vb = blur(bb, width, height), cab = blur(ab, width, height);
    for (let i = 0; i < n; i++) {
      if (!valid[i]) continue;
      const x = ma[i]!, y = mb[i]!;
      const varianceA = va[i]! - x * x, varianceB = vb[i]! - y * y, covariance = cab[i]! - x * y;
      score += ((2 * x * y + c1) * (2 * covariance + c2)) / ((x * x + y * y + c1) * (varianceA + varianceB + c2));
    }
  }
  return score / (validCount * 3);
}

function compare(reference: Image, native: Image, edges: Uint8Array): { within2: number; maxDelta: number; maxOutside: number; excluded: number; ink: number; ssim: number } {
  expect([native.width, native.height]).toEqual([reference.width, reference.height]);
  expect(edges.length).toBe(native.width * native.height);
  let within2 = 0, maxDelta = 0, maxOutside = 0, excluded = 0, ink = 0;
  for (let i = 0; i < native.pixels.length; i += 4) {
    const a = reference.pixels, b = native.pixels;
    const delta = Math.max(Math.abs(a[i]! - b[i]!), Math.abs(a[i + 1]! - b[i + 1]!), Math.abs(a[i + 2]! - b[i + 2]!));
    if (delta <= 2) within2++;
    maxDelta = Math.max(maxDelta, delta);
    // A mesh's MSAA silhouette can meet the selection outline. Preserve
    // exact selected/active colors; a broad red test also catches the grid
    // axis and would incorrectly disable its legitimate MSAA exclusion.
    const outlineTint = (p: Buffer) => outlinePixel(p, i, 'selected') || outlinePixel(p, i, 'active');
    if (edges[i / 4] && !outlineTint(a) && !outlineTint(b)) excluded++;
    else maxOutside = Math.max(maxOutside, delta);
    if (Math.max(a[i]!, a[i + 1]!, a[i + 2]!, b[i]!, b[i + 1]!, b[i + 2]!) > 16) ink++;
  }
  return { within2: within2 / (native.width * native.height), maxDelta, maxOutside, excluded, ink, ssim: inkSsim(reference, native) };
}

function colorMismatch(reference: Image, native: Image, color: Exclude<Color, 'grid'>): { count: number; mismatch: number } {
  let count = 0, mismatch = 0;
  for (let i = 0; i < reference.pixels.length; i += 4) {
    const a = outlinePixel(reference.pixels, i, color), b = outlinePixel(native.pixels, i, color);
    if (a) count++;
    if (a !== b) mismatch++;
  }
  return { count, mismatch };
}

async function parity(session: LaunchedApp, label: string, required: Partial<Record<Color, number>>): Promise<string | null> {
  const { page } = session;
  await settledNative(page);
  await source(page, true);
  let reference: Image, edgeMask: Uint8Array, samples: number;
  try {
    reference = await capture(session);
    ({ mask: edgeMask, samples } = await msaaEdges(page, reference));
  }
  finally { await source(page, false); }
  await settledNative(page);
  const native = await capture(session);
  const legacyCounts = counts(reference), nativeCounts = counts(native);
  const result = compare(reference, native, edgeMask);
  const outline = Object.fromEntries((['selected', 'active'] as const).map(color => [color, colorMismatch(reference, native, color)]));
  const detail = `${label}: <=2CV ${(100 * result.within2).toFixed(3)}%, max ${result.maxDelta}CV, max outside proven ${samples}x MSAA edges ${result.maxOutside}CV (${result.excluded} excluded), ink SSIM ${result.ssim.toFixed(6)}; reference=${JSON.stringify(legacyCounts)}, native=${JSON.stringify(nativeCounts)}, outline=${JSON.stringify(outline)}`;
  console.log(detail);
  const directory = process.env.PM_CHROME_PARITY_DIR || path.join(os.tmpdir(), 'powermove-scene3d-chrome-parity');
  await mkdir(directory, { recursive: true });
  const name = label.replace(/[^a-z0-9]+/gi, '-').toLowerCase();
  await Promise.all([
    writeFile(path.join(directory, `${name}-reference.png`), reference.png),
    writeFile(path.join(directory, `${name}-engine.png`), native.png),
    writeFile(path.join(directory, `${name}-metrics.txt`), `${detail}\n`),
    writeFile(path.join(directory, `${name}-msaa.raw`), edgeMask),
  ]);

  try {
    for (const [color, minimum] of Object.entries(required) as [Color, number][]) {
      expect(legacyCounts[color], `${label}: reference ${color} pixels`).toBeGreaterThan(minimum);
      expect(nativeCounts[color], `${label}: native ${color} pixels`).toBeGreaterThan(minimum);
    }
    expect(result.ink, detail).toBeGreaterThan(100);
    // The existing 3D bar is 99.5% of all pixels within 2 CV and 12 CV
    // outside reference MSAA edges. Ink SSIM 0.995 is stricter than
    // its 0.99 object metric; this chrome-only SSIM is an overlay quality
    // witness, and no outline-color pixel is omitted from the 12-CV bar.
    expect(result.within2, detail).toBeGreaterThanOrEqual(0.995);
    expect(result.excluded / (native.width * native.height), detail).toBeLessThan(0.02);
    expect(result.maxOutside, detail).toBeLessThanOrEqual(12);
    expect(result.ssim, detail).toBeGreaterThanOrEqual(0.995);
    for (const color of ['selected', 'active'] as const) {
      if (!required[color]) continue;
      const mask = outline[color]!;
      expect(mask.mismatch / mask.count, `${detail}; ${color} silhouette/width mismatch`).toBeLessThanOrEqual(0.01);
    }
  } catch (error) {
    return `${label}: ${(error as Error).message}`;
  }
  return null;
}

test('engine viewport chrome matches Chromium legacy pixels for occlusion, outlines, X-ray, glyph selection and imported pose', async ({ session }) => {
  test.setTimeout(180_000);
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.EngineConnection));
  await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('chrome-pixel-parity');
    const add = (edit: any) => { const result = api.scene3d.edit(edit); if (!result.ok) throw new Error(`Scene fixture edit failed: ${result.message}`); };
    add({ operation: 'add_camera', camera: { p: { x: 0, y: 1.5, z: 7 } } });
    add({ operation: 'add_light', light: { id: 'key', type: 'point', p: { x: -1, y: 1, z: .5, intensity: 30 } } });
    add({ operation: 'add_object', object: { id: 'rear', source: { primitive: 'box' }, p: { x: -.4, z: -.7 }, material: { p: { color: '#3388ff' } } } });
    add({ operation: 'add_object', object: { id: 'front', source: { primitive: 'box' }, p: { x: .4, z: .5 }, material: { p: { color: '#ff7733' } } } });
    PM.selectLayers([]); PM.invalidate();
    const style = document.createElement('style');
    style.id = 'chrome-pixel-parity-stage';
    style.textContent = '#stage { background: #000 !important } #stage > :not([data-scene-overlay]) { visibility: hidden !important }';
    document.head.append(style);
  });
  const stage = await page.locator('#stage').boundingBox();
  expect(stage).not.toBeNull();
  const center = { x: stage!.x + stage!.width / 2, y: stage!.y + stage!.height / 2 };
  await page.mouse.move(center.x, center.y);
  await page.mouse.down({ button: 'middle' });
  await page.mouse.move(center.x - 90, center.y - 45, { steps: 8 });
  await page.mouse.up({ button: 'middle' });
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.api('chrome-pixel-parity').scene3d.viewport.state().view.mode)).toBe('editor');

  const failures: string[] = [];
  const check = async (label: string, required: Partial<Record<Color, number>>) => {
    const failure = await parity(session, label, required);
    if (failure) failures.push(failure);
  };
  await check('grid clipped by overlapping boxes', { grid: 500 });

  await page.evaluate(() => {
    const PM = (window as any).PM, viewport = PM.Kernel.api('chrome-pixel-parity').scene3d.viewport;
    viewport.run('select.click', 'rear', false);
    viewport.run('select.click', 'front', true);
    const state = viewport.state();
    if (!state.selected.includes('rear') || !state.selected.includes('front') || state.active !== 'front') throw new Error(`Wrong outline fixture selection: ${JSON.stringify(state)}`);
  });
  await check('selected rear and active front outlines, including occlusion', { selected: 15, active: 15 });

  await page.mouse.move(center.x, center.y);
  await page.keyboard.press('Alt+z');
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.api('chrome-pixel-parity').scene3d.viewport.state().xray)).toBe(true);
  await check('X-ray grid and outlines', { grid: 500, selected: 15, active: 15 });
  await page.keyboard.press('Alt+z');

  await page.evaluate(() => (window as any).PM.selectLayers([]));
  await page.mouse.move(center.x, center.y);
  await page.keyboard.press('b');
  await expect(page.locator('.vp-op-header')).toHaveText('Box Select');
  await page.mouse.move(stage!.x + 8, stage!.y + 8);
  await page.mouse.down();
  await page.mouse.move(stage!.x + stage!.width - 8, stage!.y + stage!.height - 8, { steps: 6 });
  await page.mouse.up();
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.api('chrome-pixel-parity').scene3d.viewport.state().selected.includes('key'))).toBe(true);
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.latest?.depthId?.glyphs?.some((glyph: any) => glyph.id === 'key'))).toBe(true);
  await check('box-selected light glyph', { grid: 500, selected: 15 });

  await page.evaluate(async () => {
    const PM = (window as any).PM, api = PM.Kernel.api('chrome-pixel-parity');
    const positions = new Float32Array([-1, -1, 0, 1, -1, 0, 0, 1, 0]);
    const data = btoa(String.fromCharCode(...new Uint8Array(positions.buffer)));
    const gltf = { asset: { version: '2.0' }, scene: 0, scenes: [{ nodes: [0] }], nodes: [{ mesh: 0 }],
      meshes: [{ primitives: [{ attributes: { POSITION: 0 } }] }],
      buffers: [{ byteLength: positions.byteLength, uri: `data:application/octet-stream;base64,${data}` }],
      bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: positions.byteLength }],
      accessors: [{ bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [-1, -1, 0], max: [1, 1, 0] }] };
    const asset = await PM.assets.add(new File([JSON.stringify(gltf)], 'chrome-pose.gltf', { type: 'model/gltf+json' }), { silent: true });
    const result = api.scene3d.edit({ operation: 'add_object', object: { id: 'imported', source: { assetId: asset.id }, p: { x: 1.3, y: .4, ry: 35 }, useSourceMaterials: true } });
    if (!result.ok) throw new Error(`Imported fixture edit failed: ${result.message}`);
    PM.selectLayers(['imported']); PM.invalidate();
  });
  await check('posed imported glTF outline', { active: 15 });
  expect(session.diagnostics.pageErrors).toEqual([]);
  expect(failures, failures.join('\n\n')).toEqual([]);
});
