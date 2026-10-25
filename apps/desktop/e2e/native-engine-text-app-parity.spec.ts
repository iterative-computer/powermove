/**
 * G-T5: installed-app text pixels, geometry, and layout. The external
 * manifest links each numbered case to a passing G-T1/G-T2 outline report
 * and the same-head native RGBA buffer emitted with PM_TEXT_DUMP. Raster
 * cases attest the mapped SDR display buffer; frame cases use the G-T2
 * frame report's buffer. Their report hashes keep these boundaries separate.
 *
 * PM_TEXT_APP_PARITY=<external manifest.json>
 * {
 *   "rasters": [{
 *     "dir": "<capture>", "ids": ["<private id>"],
 *     "reference": {
 *       "report": "<G-T1 PM_TEXT_REPORT JSON>", "pixels": "<G-T1 PM_TEXT_DUMP dir>",
 *       "indices": [<numeric G-T1 case index aligned with ids>]
 *     }
 *   }],
 *   "frames": [{
 *     "dir": "<capture>", "project": "<private project>", "times": [<seconds>],
 *     "reference": {
 *       "report": "<G-T2 PM_TEXT_REPORT JSON>", "pixels": "<G-T2 PM_TEXT_DUMP dir>",
 *       "indices": [<numeric G-T2 frame index aligned with times>]
 *     }
 *   }]
 * }
 * All paths resolve relative to the manifest. Reports and pixels stay outside
 * Git; captured legacy output remains a witness, not the ideal reference.
 * Generate a fresh external manifest from passing native reports with
 * `node e2e/helpers/text-reference-manifest.mjs --raster-report ...
 * --raster-pixels ... --frame-report ... --frame-pixels ... --out ... --limit 8`.
 */
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import path from 'node:path';

import { expect, launchApp, test } from './helpers/app';

type ReferenceLink = { report: string; pixels: string; indices: number[] };
type Raster = { name: string; dir: string; ids: string[]; replayPrefix?: string; reference: ReferenceLink };
type Frames = { name: string; dir: string; project: string; times: number[]; reference: ReferenceLink };
type Manifest = { rasters?: Raster[]; frames?: Frames[] };
type Mode = 'off' | 'on';
type Verdict = {
  case: number; pass: boolean; width: number; height: number;
  engineSha256: string; displaySha256?: string; referenceModel: string;
};

const source = process.env.PM_TEXT_APP_PARITY;
const manifest: Manifest = source ? JSON.parse(readFileSync(source, 'utf8')) : {};
const root = source ? path.dirname(source) : '';
const readJson = async (file: string) => JSON.parse(await readFile(file, 'utf8'));
const external = (file: string) => path.resolve(root, file);

type Box = { x0: number; y0: number; x1: number; y1: number };

/**
 * TEXT-INK-1: a color-glyph line's selection is what is drawn. Today's Canvas
 * selection (Skia's whole-pixel round-out plus 1 px outset of CoreText's
 * metric rectangle) stays recorded as the witness; the engine's selection
 * must instead meet exactly the pixels its own raster made nonzero.
 */
const DRAWN_INK: Record<string, Box & { w: number; h: number }> = {
  'sdf-scripts-0024': { x0: 4.72265625, y0: -24, x1: 713, y1: 201, w: 708.27734375, h: 225 },
};

/** The device pixels with nonzero alpha in an RGBA8 buffer. */
function alphaBox(rgba: Uint8Array, width: number): Box {
  const box = { x0: Infinity, y0: Infinity, x1: -Infinity, y1: -Infinity };
  for (let i = 3; i < rgba.length; i += 4) {
    if (!rgba[i]) continue;
    const x = ((i - 3) / 4) % width, y = Math.floor((i - 3) / 4 / width);
    box.x0 = Math.min(box.x0, x); box.y0 = Math.min(box.y0, y);
    box.x1 = Math.max(box.x1, x + 1); box.y1 = Math.max(box.y1, y + 1);
  }
  return box;
}

/** Typography anchoring (`font-anchor.ts` / pm-eval `font_anchor_offset`). */
function fontAnchorOffset(reference: Box, bounds: Box, ax: number, ay: number) {
  const offset = (start: number, end: number, nextStart: number, nextEnd: number, anchor: number) =>
    end <= start ? 0 : anchor - (nextStart + (anchor - start) / (end - start) * (nextEnd - nextStart));
  return { x: offset(reference.x0, reference.x1, bounds.x0, bounds.x1, ax),
    y: offset(reference.y0, reference.y1, bounds.y0, bounds.y1, ay) };
}

function compare(a: Uint8Array, b: Uint8Array) {
  expect(a.length).toBe(b.length);
  const values = new Array<number>(a.length / 4);
  let sum = 0, overOne = 0;
  for (let i = 0; i < values.length; i++) {
    let max = 0;
    for (let c = 0; c < 4; c++) max = Math.max(max, Math.abs(a[i * 4 + c]! - b[i * 4 + c]!));
    values[i] = max;
    sum += max;
    overOne += Number(max > 1);
  }
  values.sort((a, b) => a - b);
  return { max: values.at(-1) ?? 0, p999: values[Math.max(0, Math.ceil(values.length * .999) - 1)] ?? 0,
    mean: values.length ? sum / values.length : 0, overOne };
}

async function attested(link: ReferenceLink, position: number, stem: 'case' | 'frame',
  size: [number, number]) {
  expect(link, 'G-T5 requires external G-T1/G-T2 reference evidence').toBeTruthy();
  expect(link.indices.length).toBeGreaterThan(position);
  const index = link.indices[position]!;
  expect(Number.isSafeInteger(index) && index >= 0).toBe(true);
  const report: Verdict[] = await readJson(external(link.report));
  const row = report.find(row => row.case === index);
  expect(row, `reference verdict for numeric case ${index}`).toBeTruthy();
  expect(row!.pass, `independent reference gate for numeric case ${index}`).toBe(true);
  expect(row!.referenceModel, `numeric case ${index} must use outlines`).toMatch(
    /^outline(?:-with-original-bound-gap)?$/,
  );
  expect([row!.width, row!.height]).toEqual(size);
  const display = stem === 'case';
  if (display) expect(row!.displaySha256, `numeric case ${index}: mapped display proof`).toBeTruthy();
  const pixels = await readFile(path.join(external(link.pixels),
    `${stem}-${index}-${display ? 'display' : 'engine'}.rgba`));
  expect(pixels.length).toBe(size[0] * size[1] * 4);
  expect(createHash('sha256').update(pixels).digest('hex')).toBe(
    display ? row!.displaySha256 : row!.engineSha256);
  return { index, pixels };
}

if (!source) {
  test('G-T5 external text parity replay', () => {
    test.skip(true, 'PM_TEXT_APP_PARITY names no external corpus manifest');
  });
}

for (const [groupIndex, group] of (manifest.rasters ?? []).entries()) {
  test(`G-T5 raster group ${groupIndex}: reference pixels and exact layout`, async ({}, testInfo) => {
    test.setTimeout(600_000);
    const capture = external(group.dir);
    const corpus = await readJson(path.join(capture, 'cases.json'));
    const shots = await readJson(path.join(capture, 'manifest.json'));
    // Preserve the capture's measurement-cache warmup order through the last
    // selected case, even when only a few buffers are attested.
    const last = corpus.cases.reduce((last: number, c: any, i: number) =>
      group.ids.includes(c.id) ? i : last, -1);
    const cases = corpus.cases.slice(0, last + 1).filter((c: any) =>
      !group.replayPrefix || c.id.startsWith(group.replayPrefix));
    expect(cases.filter((c: any) => group.ids.includes(c.id)).length).toBe(group.ids.length);
    expect(group.reference.indices.length).toBe(group.ids.length);

    const run = async (mode: Mode) => {
      // Native reference font-cache seeds use the captured DPR 2 environment.
      const session = await launchApp({ args: ['--force-device-scale-factor=2', '--force-color-profile=srgb'],
        env: { POWERMOVE_NEW_ENGINE: mode === 'on' ? '1' : '0' } });
      try {
        await session.openEditor();
        expect(await session.page.evaluate(() => window.devicePixelRatio)).toBe(2);
        await session.page.evaluate(() => {
          const PM = (window as any).PM;
          PM.pause();
          PM.replaceProject(PM.mkProject({ w: 1920, h: 1080, dur: 10, name: 'Text parity' }));
        });
        await session.page.evaluate(async () => { await document.fonts.ready; });
        if (mode === 'on') {
          await expect.poll(
            () => session.page.evaluate(() => (window as any).PM.EngineConnection.status().route),
            { timeout: 60_000 },
          ).toBe('engine');
        }
        const rows: any[] = [];
        for (const item of cases) {
          const probe = { ...item, witness: DRAWN_INK[item.id] ?? null };
          const actual = await session.page.evaluate(item => {
            const PM = (window as any).PM;
            const api = PM.Kernel.api('e2e-text-reference-parity');
            const layer = PM.mkLayer(item.kind === 'text' ? 'text' : 'captions',
              { id: 'probe', from: 0, dur: 10, d: item.kind === 'text' ? item.d : item.content }, PM.proj);
            const time = item.kind === 'text' ? 0 : item.local;
            const raster = api.render.raster(layer, item.scale, time);
            if (!raster?.cv) throw new Error('No text raster');
            // The compositor's canvas-to-texture upload and crop path.
            const cv = raster.cv;
            const gl = document.createElement('canvas').getContext('webgl2',
              { premultipliedAlpha: true, antialias: false })!;
            const tex = gl.createTexture(), fb = gl.createFramebuffer();
            gl.bindTexture(gl.TEXTURE_2D, tex);
            gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
            gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
            gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, cv);
            gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
            gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
            const px = new Uint8Array(cv.width * cv.height * 4);
            gl.readPixels(0, 0, cv.width, cv.height, gl.RGBA, gl.UNSIGNED_BYTE, px);
            const error = gl.getError();
            gl.deleteFramebuffer(fb); gl.deleteTexture(tex);
            gl.getExtension('WEBGL_lose_context')?.loseContext();
            if (error) throw new Error(`GL ${error}`);
            let encoded = '';
            for (let i = 0; i < px.length; i += 32768) encoded += String.fromCharCode(...px.subarray(i, i + 32768));
            // A layer saved today with typography anchoring records today's
            // selection; the anchor follows the current bounds from there.
            let anchored = null;
            if (item.witness) {
              const saved = PM.mkLayer('text', { id: 'probe-anchored', from: 0, dur: 10,
                d: { ...item.d, fontAnchorBounds: item.witness } }, PM.proj);
              const placed = api.render.raster(saved, item.scale, time);
              anchored = { anchorX: placed.anchorX, anchorY: placed.anchorY,
                ax: PM.ev(saved, 'anchor.x', time), ay: PM.ev(saved, 'anchor.y', time) };
            }
            return {
              anchored,
              compat: layer.compat ?? null,
              layout: item.kind === 'text' ? api.render.textLayout(layer, time) : null,
              captionBounds: item.kind === 'captions' ? PM.captionBounds(layer, time) : null,
              geometry: {
                w: raster.w, h: raster.h, anchorX: raster.anchorX, anchorY: raster.anchorY,
                canvas: [cv.width, cv.height], selection: raster.selection,
                fontOffset: raster.fontOffset,
              },
              rgba: btoa(encoded),
              enabled: (window as any).__powermoveTestBridge.nativeEngine.enabledSync(),
            };
          }, probe);
          if (!group.ids.includes(item.id)) continue;
          const shot = shots.cases.find((c: any) => c.id === item.id);
          expect(shot).toBeTruthy();
          expect(actual.enabled).toBe(mode === 'on');
          expect(actual.geometry.canvas).toEqual([shot.width, shot.height]);
          rows.push(actual);
        }
        // Replay may include cache-warming cases. Reference indices are
        // explicitly aligned to ids, so return the selected cases in that
        // order even when the capture stores them differently.
        const selected = cases.filter((item: any) => group.ids.includes(item.id));
        return group.ids.map(id => {
          const position = selected.findIndex((item: any) => item.id === id);
          expect(position, 'listed raster exists exactly once').toBeGreaterThanOrEqual(0);
          return rows[position]!;
        });
      } finally {
        await session.close();
      }
    };
    const today = await run('off');
    const engine = await run('on');
    const metrics = [];
    for (let i = 0; i < engine.length; i++) {
      const { rgba: engine64, enabled: engineEnabled, ...engineSemantic } = engine[i]!;
      const { rgba: today64, enabled: todayEnabled, ...todaySemantic } = today[i]!;
      expect([engineEnabled, todayEnabled]).toEqual([true, false]);
      const size = engine[i]!.geometry.canvas as [number, number];
      const id = group.ids[i]!, witness = DRAWN_INK[id];
      if (witness) {
        // Today keeps its recorded Canvas selection and anchors to it unmoved.
        const { geometry: g, anchored: a } = todaySemantic;
        expect(g.selection, `${id}: today's Canvas witness`).toEqual(witness);
        expect([a.anchorX, a.anchorY]).toEqual([g.anchorX, g.anchorY]);
        // The engine's unpadded selection, rounded out to device pixels, is
        // exactly the box of nonzero alpha its raster drew.
        const e = engineSemantic.geometry, sel = e.selection;
        const size0 = Number(cases.find((c: any) => c.id === id)?.d?.size) || 16;
        const pad = Math.min(12, Math.max(3, Math.max(1, size0) * 0.055));
        const [sx, sy] = [size[0] / e.w, size[1] / e.h];
        expect(alphaBox(Buffer.from(engine64, 'base64'), size[0]), `${id}: selection is the drawn ink`).toEqual({
          x0: Math.floor((sel.x0 + pad + e.anchorX) * sx), y0: Math.floor((sel.y0 + pad + e.anchorY) * sy),
          x1: Math.ceil((sel.x1 - pad + e.anchorX) * sx), y1: Math.ceil((sel.y1 - pad + e.anchorY) * sy),
        });
        // Typography anchoring maps the recorded Canvas box onto the drawn one.
        const b = engineSemantic.anchored, offset = fontAnchorOffset(witness, sel, b.ax, b.ay);
        expect([b.anchorX, b.anchorY], `${id}: font anchor follows the drawn ink`).toEqual(
          [e.anchorX - offset.x, e.anchorY - offset.y]);
        expect([b.ax, b.ay]).toEqual([a.ax, a.ay]);
        for (const side of [engineSemantic, todaySemantic]) {
          side.geometry = { ...side.geometry, selection: undefined };
          side.anchored = undefined;
        }
      }
      expect(engineSemantic, `numeric case ${group.reference.indices[i]} (compat ${engine[i]!.compat}): exact semantics`).toEqual(todaySemantic);
      const { index, pixels } = await attested(group.reference, i, 'case', size);
      const result = compare(Buffer.from(engine64, 'base64'), pixels);
      metrics.push({ case: index, ...result,
        witness: compare(Buffer.from(today64, 'base64'), pixels) });
      expect(result.overOne, `numeric case ${index}: app raster versus attested native buffer`).toBe(0);
    }
    await testInfo.attach('numeric-pixel-metrics', {
      body: Buffer.from(JSON.stringify(metrics)), contentType: 'application/json',
    });
  });
}

for (const [groupIndex, group] of (manifest.frames ?? []).entries()) {
  test(`G-T5 frame group ${groupIndex}: reference pixels and preview/export`, async ({}, testInfo) => {
    test.setTimeout(600_000);
    const dir = external(group.dir);
    const doc = await readJson(path.join(dir, group.project, 'loaded.json'));
    const project = (await readJson(path.join(dir, 'manifest.json'))).projects.find(
      (p: any) => p.dir === group.project);
    expect(project).toBeTruthy();
    expect(group.reference.indices.length).toBe(group.times.length);

    const run = async (mode: Mode) => {
      // Native reference font-cache seeds use the captured DPR 2 environment.
      const session = await launchApp({ args: ['--force-device-scale-factor=2', '--force-color-profile=srgb'],
        env: { POWERMOVE_NEW_ENGINE: mode === 'on' ? '1' : '0' } });
      try {
        await session.openEditor();
        expect(await session.page.evaluate(() => window.devicePixelRatio)).toBe(2);
        await session.page.evaluate(doc => {
          const PM = (window as any).PM;
          PM.replaceProject(doc); PM.pause();
        }, doc);
        await session.page.evaluate(async () => { await document.fonts.ready; });
        if (mode === 'on') {
          await expect.poll(
            () => session.page.evaluate(() => (window as any).PM.EngineConnection.status().route),
            { timeout: 60_000 },
          ).toBe('engine');
        }
        const rows: any[] = [];
        for (const time of group.times) {
          const sample = project.samples.find((s: any) => s.time === time);
          expect(sample).toBeTruthy();
          const actual = await session.page.evaluate(async ({ time, width, height, mode }) => {
            const PM = (window as any).PM;
            const api = PM.Kernel.api('e2e-text-reference-parity');
            if (mode === 'on') {
              // Match the reference density so one composition pixel on
              // screen maps to one export pixel. Keep the viewer active.
              const viewer = PM.Kernel.services.get('viewer');
              const dpr = window.devicePixelRatio || 1;
              for (const [key, value] of Object.entries({
                width: `${width / dpr}px`, height: `${height / dpr}px`,
                'min-width': `${width / dpr}px`, 'min-height': `${height / dpr}px`,
                'max-width': 'none', 'max-height': 'none',
              })) viewer.stage.style.setProperty(key, value as string, 'important');
              PM.previewResolution = 'auto'; PM.quality = 1; PM.perf.auto = false;
              viewer.fit = false; viewer.zoom = width / (PM.proj.w * dpr); viewer.pan = [0, 0];
              viewer.layout();
            }
            PM.pause(); PM.setTime(time, { raw: true, force: true });
            await PM.prepareFrame(time);
            const opts = { mblur: true, mbSamples: 16, shutter: PM.proj.shutter || .5 };
            // The paused viewer is a separate transport from the public
            // export calls; all three must show the same frame.
            const exportCanvas = api.render.renderFrameTo(time, width, height, opts);
            const asyncCanvas = await api.render.renderFrameToAsync(time, width, height, opts);
            const encode = (cv: HTMLCanvasElement) => {
              if (!cv || cv.width !== width || cv.height !== height) throw new Error('No frame');
              const px = cv.getContext('2d')!.getImageData(0, 0, width, height).data;
              let text = '';
              for (let i = 0; i < px.length; i += 32768) text += String.fromCharCode(...px.subarray(i, i + 32768));
              return btoa(text);
            };
            let viewerRgba: string | null = null;
            if (mode === 'on') {
              const viewer = PM.Kernel.services.get('viewer');
              PM.invalidate?.('render');
              const deadline = performance.now() + 20_000;
              while (true) {
                const frame = viewer.engineFrame();
                const surface = viewer.enginePresentation?.canvas as HTMLCanvasElement | undefined;
                const state = frame ? viewer.enginePresentation.stateFor(frame.viewSeq) : null;
                if (frame && !frame.standIn && frame.pts === Math.round(time * 705_600_000)
                  && frame.revision === PM.EngineDoc.barrier.revision
                  && frame.viewSeq === viewer.enginePresentation.states.at(-1)?.viewSeq
                  && state?.canvas.width === width && state?.canvas.height === height && surface) {
                  const copy = document.createElement('canvas');
                  copy.width = width; copy.height = height;
                  // Presentation covers the whole stage, including letterbox
                  // space. Read the composition rectangle from this frame's
                  // committed state at exactly one device pixel per output pixel.
                  const rect = state.canvas.css, dpr = state.devicePixelRatio;
                  const crop = [rect.x * dpr, rect.y * dpr, rect.width * dpr, rect.height * dpr];
                  if (crop.some(value => !Number.isInteger(value))
                    || crop[2] !== width || crop[3] !== height) throw new Error(`Nonintegral viewer crop ${crop}`);
                  copy.getContext('2d')!.drawImage(surface, crop[0]!, crop[1]!, width, height, 0, 0, width, height);
                  viewerRgba = encode(copy);
                  break;
                }
                if (performance.now() > deadline) throw new Error(`Paused viewer size/state: ${JSON.stringify({
                  expected: [width, height], surface: surface && [surface.width, surface.height],
                  canvas: state?.canvas && [state.canvas.width, state.canvas.height],
                  shown: viewer.shown, dpr: window.devicePixelRatio,
                  revision: frame?.revision, wantedRevision: PM.EngineDoc.barrier.revision,
                  pts: frame?.pts, wantedPts: Math.round(time * 705_600_000),
                })}`);
                await new Promise(resolve => requestAnimationFrame(resolve));
              }
            }
            const text = PM.proj.layers.filter((layer: any) => layer.type === 'text' || layer.type === 'captions');
            return {
              layers: text.map((layer: any) => ({
                type: layer.type,
                layout: layer.type === 'text' ? api.render.textLayout(layer, time) : null,
                captionBounds: layer.type === 'captions' ? PM.captionBounds(layer, time) : null,
                world: PM.worldMatrix(layer, time),
                geometry: layer.type === 'text' ? PM.textRasterGeometry(layer, 1, time) : null,
              })),
              rgba: encode(exportCanvas), asyncRgba: encode(asyncCanvas), viewerRgba,
              route: PM.EngineConnection?.status(),
              enabled: (window as any).__powermoveTestBridge.nativeEngine.enabledSync(),
            };
          }, { time, width: sample.width, height: sample.height, mode });
          expect(actual.enabled).toBe(mode === 'on');
          if (mode === 'on') {
            expect(actual.route.route).toBe('engine');
            expect(actual.route.failures).toEqual([]);
          }
          expect(actual.layers.length).toBeGreaterThan(0);
          expect(actual.asyncRgba, 'sync/async export pixels must be identical').toBe(actual.rgba);
          if (mode === 'on') expect(actual.viewerRgba, 'paused viewer/export pixels must be identical').toBe(actual.rgba);
          rows.push({ ...actual, size: [sample.width, sample.height] });
        }
        return rows;
      } finally {
        await session.close();
      }
    };
    const today = await run('off');
    const engine = await run('on');
    const metrics = [];
    for (let i = 0; i < engine.length; i++) {
      expect(engine[i]!.layers).toEqual(today[i]!.layers);
      const { index, pixels } = await attested(group.reference, i, 'frame', engine[i]!.size);
      const result = compare(Buffer.from(engine[i]!.rgba, 'base64'), pixels);
      metrics.push({ case: index, ...result,
        witness: compare(Buffer.from(today[i]!.rgba, 'base64'), pixels) });
      expect(result.overOne, `numeric frame ${index}: app export versus attested native buffer`).toBe(0);
    }
    await testInfo.attach('numeric-pixel-metrics', {
      body: Buffer.from(JSON.stringify(metrics)), contentType: 'application/json',
    });
  });
}
