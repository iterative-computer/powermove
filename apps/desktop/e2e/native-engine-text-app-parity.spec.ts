/**
 * App-level FONTS-R parity replay: text rasters and composed frames drawn
 * through the installed public render API, with "Use New Engine" off and on,
 * against the private corpus captures of today's renderer.
 *
 * The captures are renders of private projects and licensed fonts, so they
 * never enter the repository. Every case identifier, capture directory and
 * project path comes from a manifest in the private corpus directory:
 *
 *   PM_TEXT_APP_PARITY=<corpus>/<manifest>.json
 *
 *   {
 *     "rasters": [{ "name": "…", "dir": "<capture dir>", "ids": ["…"],
 *                   "replayPrefix": "…" }],
 *     "frames":  [{ "name": "…", "dir": "<capture dir>", "project": "<id>",
 *                   "times": [16, 18], "boundCv": 2 }]
 *   }
 *
 * Paths are relative to the manifest's directory. `replayPrefix` replays
 * every capture case with that prefix in capture order (the distance-field
 * atlas placement depends on the glyphs drawn before), comparing only `ids`.
 * Without the variable the spec skips; it covers the app's raster crop and
 * stride path end to end, which the Rust gates cannot.
 *
 * Run under the Electron lock, one worker:
 *   pm-electron-lock bun run --cwd apps/desktop test:e2e -- \
 *     e2e/native-engine-text-app-parity.spec.ts --workers=1
 */
import { readFileSync } from 'node:fs';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { expect, launchApp, test } from './helpers/app';

type Raster = { name: string; dir: string; ids: string[]; replayPrefix?: string };
type Frames = { name: string; dir: string; project: string; times: number[]; boundCv: number };
type Manifest = { rasters?: Raster[]; frames?: Frames[] };

const MANIFEST = process.env.PM_TEXT_APP_PARITY;
const OUT = process.env.PM_TEXT_APP_PARITY_OUT ?? path.join(os.tmpdir(), 'powermove-text-app-parity');
// The raster gate's strict bound (text_raster: 1 CV); frames carry their own.
const RASTER_BOUND_CV = 1;

const manifest: Manifest = MANIFEST ? JSON.parse(readFileSync(MANIFEST, 'utf8')) : {};
const root = MANIFEST ? path.dirname(MANIFEST) : '';
const readJson = async (file: string) => JSON.parse(await readFile(file, 'utf8'));

/** Per-pixel maximum channel difference against `bound` (straight RGBA8). */
function compare(a: Uint8Array, b: Uint8Array, bound: number) {
  if (a.length !== b.length) throw new Error(`Length mismatch ${a.length} / ${b.length}`);
  let maxCv = 0, pixelsOverBound = 0, pixelsDifferent = 0;
  for (let i = 0; i < a.length; i += 4) {
    let d = 0;
    for (let c = 0; c < 4; c++) d = Math.max(d, Math.abs(a[i + c]! - b[i + c]!));
    maxCv = Math.max(maxCv, d);
    if (d > bound) pixelsOverBound++;
    if (d) pixelsDifferent++;
  }
  return { maxCv, pixelsOverBound, pixelsDifferent, boundCv: bound };
}

const MODES = ['off', 'on'] as const;

// Without the private manifest the replay is listed as skipped, not absent.
if (!MANIFEST) {
  test('text app parity replay', () => {
    test.skip(true, 'PM_TEXT_APP_PARITY names no private corpus manifest');
  });
}

for (const mode of MODES) for (const group of manifest.rasters ?? []) {
  test(`app ${mode}: ${group.name} text rasters`, async ({}, testInfo) => {
    test.setTimeout(300_000);
    await mkdir(OUT, { recursive: true });
    const capture = path.join(root, group.dir);
    const corpus = await readJson(path.join(capture, 'cases.json'));
    const shots = await readJson(path.join(capture, 'manifest.json'));
    const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: mode === 'on' ? '1' : '0' } });
    const rows: any[] = [];
    try {
      await session.openEditor();
      await session.page.evaluate(() => {
        const PM = (window as any).PM;
        PM.pause();
        PM.replaceProject(PM.mkProject({ w: 1920, h: 1080, dur: 10, name: 'Text parity' }));
      });
      if (mode === 'on') {
        await expect.poll(() => session.page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 60_000 }).toBe('engine');
      }
      const cases = corpus.cases.filter((c: any) => group.replayPrefix ? c.id.startsWith(group.replayPrefix) : group.ids.includes(c.id));
      expect(cases.filter((c: any) => group.ids.includes(c.id)).length, 'every listed case is in the capture').toBe(group.ids.length);
      for (const item of cases) {
        const result = await session.page.evaluate(item => {
          const PM = (window as any).PM;
          const layer = PM.mkLayer(item.kind === 'text' ? 'text' : 'captions',
            { id: 'probe', from: 0, dur: 10, d: item.kind === 'text' ? item.d : item.content }, PM.proj);
          // The public raster API, then the same canvas-to-texture upload
          // the compositor uses (premultiplied, no flip).
          const r = PM.Kernel.api('e2e-text-parity').render.raster(layer, item.scale, item.kind === 'text' ? 0 : item.local);
          if (!r?.cv) throw new Error(`No raster: ${item.id}`);
          const cv = r.cv;
          const g = document.createElement('canvas').getContext('webgl2', { premultipliedAlpha: true, antialias: false })!;
          const tex = g.createTexture(), fb = g.createFramebuffer();
          g.bindTexture(g.TEXTURE_2D, tex);
          g.pixelStorei(g.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
          g.pixelStorei(g.UNPACK_FLIP_Y_WEBGL, false);
          g.texImage2D(g.TEXTURE_2D, 0, g.RGBA, g.RGBA, g.UNSIGNED_BYTE, cv);
          g.bindFramebuffer(g.FRAMEBUFFER, fb);
          g.framebufferTexture2D(g.FRAMEBUFFER, g.COLOR_ATTACHMENT0, g.TEXTURE_2D, tex, 0);
          const px = new Uint8Array(cv.width * cv.height * 4);
          g.readPixels(0, 0, cv.width, cv.height, g.RGBA, g.UNSIGNED_BYTE, px);
          const error = g.getError();
          g.deleteFramebuffer(fb); g.deleteTexture(tex); g.getExtension('WEBGL_lose_context')?.loseContext();
          if (error) throw new Error(`GL ${error}`);
          let s = '';
          for (let i = 0; i < px.length; i += 32768) s += String.fromCharCode(...px.subarray(i, i + 32768));
          return { width: cv.width, height: cv.height, rgba: btoa(s),
            enabled: (window as any).__powermoveTestBridge.nativeEngine.enabledSync() };
        }, item);
        if (!group.ids.includes(item.id)) continue;
        const shot = shots.cases.find((c: any) => c.id === item.id);
        const pixels = Buffer.from(result.rgba, 'base64');
        await writeFile(path.join(OUT, `${mode}-${item.id}.rgba`), pixels);
        const metric = compare(pixels, await readFile(path.join(capture, `${item.id}.rgba`)), RASTER_BOUND_CV);
        rows.push({ id: item.id, mode, width: result.width, height: result.height,
          expected: [shot.width, shot.height], enabled: result.enabled, ...metric });
      }
    } finally {
      await session.close();
    }
    await testInfo.attach('parity', { body: Buffer.from(JSON.stringify({ rows, teardown: session.teardown }, null, 2)), contentType: 'application/json' });
    for (const row of rows) {
      expect(row.enabled).toBe(mode === 'on');
      expect([row.width, row.height]).toEqual(row.expected);
      expect(row.pixelsOverBound, JSON.stringify(row)).toBe(0);
    }
  });
}

for (const mode of MODES) for (const group of manifest.frames ?? []) {
  test(`app ${mode}: ${group.name} frames`, async ({}, testInfo) => {
    test.setTimeout(600_000);
    await mkdir(OUT, { recursive: true });
    const dir = path.join(root, group.dir);
    const doc = await readJson(path.join(dir, group.project, 'loaded.json'));
    const project = (await readJson(path.join(dir, 'manifest.json'))).projects.find((p: any) => p.id === group.project);
    const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: mode === 'on' ? '1' : '0' } });
    const rows: any[] = [];
    try {
      await session.openEditor();
      await session.page.evaluate(({ doc, time }) => {
        const PM = (window as any).PM;
        PM.replaceProject(doc); PM.pause(); PM.time = time; PM.invalidate();
      }, { doc, time: group.times[0] ?? 0 });
      await session.page.evaluate(async () => { await document.fonts.ready; });
      if (mode === 'on') {
        await expect.poll(() => session.page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 60_000 }).toBe('engine');
      }
      for (const time of group.times) {
        const sample = project.samples.find((s: any) => s.time === time);
        const result = await session.page.evaluate(async ({ time, width, height }) => {
          const PM = (window as any).PM;
          PM.pause(); PM.setTime(time, { raw: true, force: true }); await PM.prepareFrame(time);
          // The public capture API routes to the engine when it is enabled.
          const cv = PM.Kernel.api('e2e-text-parity').render.renderFrameTo(time, width, height,
            { mblur: true, mbSamples: 16, shutter: PM.proj.shutter || .5 });
          const px = cv?.getContext('2d').getImageData(0, 0, width, height).data;
          if (!px || px.length !== width * height * 4) throw new Error('No frame');
          let s = '';
          for (let i = 0; i < px.length; i += 32768) s += String.fromCharCode(...px.subarray(i, i + 32768));
          return { rgba: btoa(s), route: PM.EngineConnection?.status() ?? null,
            enabled: (window as any).__powermoveTestBridge.nativeEngine.enabledSync() };
        }, { time, width: sample.width, height: sample.height });
        const pixels = Buffer.from(result.rgba, 'base64');
        await writeFile(path.join(OUT, `${mode}-${group.name}-${time}.rgba`), pixels);
        // Chromium decodes the PNG reference; no image dependency here.
        const reference = await session.page.evaluate(async b64 => {
          const img = await createImageBitmap(new Blob([Uint8Array.from(atob(b64), c => c.charCodeAt(0))], { type: 'image/png' }));
          const c = new OffscreenCanvas(img.width, img.height), ctx = c.getContext('2d')!;
          ctx.drawImage(img, 0, 0); img.close();
          const px = ctx.getImageData(0, 0, c.width, c.height).data;
          let s = '';
          for (let i = 0; i < px.length; i += 32768) s += String.fromCharCode(...px.subarray(i, i + 32768));
          return btoa(s);
        }, (await readFile(path.join(dir, sample.png.file))).toString('base64'));
        rows.push({ time, mode, route: result.route, enabled: result.enabled,
          ...compare(pixels, Buffer.from(reference, 'base64'), group.boundCv) });
      }
    } finally {
      await session.close();
    }
    await testInfo.attach('parity', { body: Buffer.from(JSON.stringify({ rows, teardown: session.teardown }, null, 2)), contentType: 'application/json' });
    for (const row of rows) {
      expect(row.enabled).toBe(mode === 'on');
      expect(row.pixelsOverBound, JSON.stringify(row)).toBe(0);
      if (mode === 'on') {
        expect(row.route.route).toBe('engine');
        expect(row.route.failures).toEqual([]);
      }
    }
  });
}
