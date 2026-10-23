/** Legacy-app reference frames for the video and 3D color-depth gate.
 * Build apps/desktop outside the lock, then run:
 *   pm-electron-lock bun engine/corpus/src/color-depth-media.ts /external/output
 * The fixture is read from the repository at runtime. Only the supplied
 * external directory receives captures; launchLegacyApp removes its profile.
 */
import path from 'node:path';
import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { launchLegacyApp } from './app';
import { repoRoot } from './catalog';
import { encodePng } from './png';

const argument = process.argv[2];
if (!argument) throw new Error('Supply an output directory outside the repository');
const out = path.resolve(argument);
if (out === repoRoot || out.startsWith(repoRoot + path.sep)) throw new Error('Output must be outside the repository');
const fixture = path.join(repoRoot, 'apps/desktop/e2e/fixtures/h264-aac.mp4');
if (!existsSync(fixture)) throw new Error(`Missing repository video fixture: ${fixture}`);
mkdirSync(out, { recursive: true });
const app = await launchLegacyApp();
const samples: Array<Record<string, unknown>> = [];
try {
  for (const kind of ['video', 'scene3d'] as const) for (const depth of kind === 'scene3d' ? [16, 32, 8] : [8, 16, 32]) {
    const created = await app.evaluate(({ kind, depth }) => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: `Depth media ${kind}`, w: 320, h: 180, fps: 30, dur: 2,
        backgroundFill: { type: 'solid', angle: 0, stops: [{ id: 'bg', color: '#283346', position: 0 }] } });
      project.colorDepth = depth;
      PM.replaceProject(project); PM.pause();
      PM.setTime(0, { raw: true, force: true });
      if (kind === 'scene3d') {
        const api = PM.Kernel.api('color-depth-media').scene3d;
        for (const edit of [
          { operation: 'add_camera', id: 'camera', camera: { p: { x: 0, y: 0, z: 5 } } },
          { operation: 'add_light', light: { id: 'light', type: 'point', p: { x: -2, y: 3, z: 4, intensity: 25 } } },
          { operation: 'add_object', object: { id: 'box', source: { primitive: 'box' }, p: { x: 0, y: 0, z: 0, ry: 27 }, material: { p: { color: '#789DCF' } } } },
        ]) {
          const result = api.edit(edit);
          if (!result.ok) throw new Error(`3D fixture edit: ${result.message}`);
        }
      }
      return { id: PM.proj.id };
    }, { kind, depth });
    let media: { assetId: string; path: string }[] = [];
    if (kind === 'video') {
      await app.evaluate(() => {
        const input = document.createElement('input'); input.type = 'file'; input.id = 'color-depth-media-input';
        input.hidden = true; document.body.append(input);
      });
      try {
        await app.setInputFiles('#color-depth-media-input', [fixture]);
        const assetId = await app.evaluate(async () => {
          const PM = (window as any).PM;
          const input = document.querySelector('#color-depth-media-input') as HTMLInputElement;
          const file = input.files?.[0]; if (!file) throw new Error('Video fixture input is empty');
          const asset = await PM.assets.add(file, { newAsset: true, silent: true });
          const layer = PM.mkLayer('video', { name: 'Fixture clip', d: { asset: asset.id, fit: 'contain', w: 320, h: 180 },
            p: { 'position.x': 160, 'position.y': 90 } }, PM.proj);
          PM.proj.layers.unshift(layer); PM.touch(); PM.invalidate();
          return asset.id as string;
        });
        media = [{ assetId, path: fixture }];
      } finally {
        await app.evaluate(() => document.querySelector('#color-depth-media-input')?.remove());
      }
      await app.evaluate(async () => { await (window as any).PM.projectAssetsReady; });
    } else {
      await app.evaluate(() => {
        const PM = (window as any).PM;
        const overlay = PM.mkLayer('solid', { name: '2D overlay', d: { color: '#E1A46AA3', w: 130, h: 54 },
          p: { 'position.x': 36, 'position.y': 106, rotation: -8 } }, PM.proj);
        PM.proj.layers.unshift(overlay); PM.touch(); PM.invalidate();
      });
    }
    const times = kind === 'video' ? [0.5, 1.5] : [0.5];
    for (const time of times) {
      const capture = await app.evaluate(async ({ time, depth, kind }) => {
        const PM = (window as any).PM;
        PM.setTime(time, { raw: true, force: true });
        await PM.prepareFrame(time);
        const gl = PM.GL.gl;
        let previous: Float32Array | null = null;
        for (let attempt = 0; attempt < 12; attempt++) {
          for (let n = 0; n < 32 && gl.getError(); n++);
          let raw: Float32Array | undefined;
          const original = gl.readPixels.bind(gl);
          gl.readPixels = (...args: any[]) => {
            const result = original(...args);
            if (args[5] === gl.FLOAT) raw = new Float32Array(args[6]);
            return result;
          };
          let bytes: Uint8Array;
          try { bytes = PM.GL.renderToPixels(time, 320, 180, { mblur: false, exporting: true }); }
          finally { gl.readPixels = original; }
          const error = gl.getError();
          if (error || PM.GL.colorDepth !== depth) throw new Error(`Depth ${depth}, actual ${PM.GL.colorDepth}, GL error ${error}`);
          const pixels = new Float32Array(320 * 180 * 4);
          for (let y = 0; y < 180; y++) for (let x = 0; x < 320 * 4; x++) {
            const source = (179 - y) * 320 * 4 + x;
            pixels[y * 320 * 4 + x] = raw ? raw[source]! : bytes![source]! / 255;
          }
          // A first three.js render may be awaiting shader compilation. A
          // flat background and 2D overlay is never a valid 3D witness.
          const colors = new Set<number>();
          let visibleVideoPixels = 0;
          for (let i = 0; i < pixels.length; i += 4) {
            const rgb = [Math.round(pixels[i]! * 255), Math.round(pixels[i + 1]! * 255), Math.round(pixels[i + 2]! * 255)];
            if (kind === 'scene3d') colors.add((rgb[0]! << 16) | (rgb[1]! << 8) | rgb[2]!);
            // The fixture is red at 0.5 s and green at 1.5 s. Require the
            // expected image over most of the canvas, not a blank readback.
            const expectedVideoColor = time < 1
              ? rgb[0]! > 200 && rgb[1]! < 32 && rgb[2]! < 32
              : rgb[1]! > 200 && rgb[0]! < 32 && rgb[2]! < 32;
            if (kind === 'video' && expectedVideoColor)
              visibleVideoPixels++;
          }
          const complete = kind === 'scene3d' ? colors.size > 8 : visibleVideoPixels > 320 * 180 / 2;
          if (complete && previous && previous.every((value, i) => value === pixels[i]))
            return { doc: JSON.parse(JSON.stringify(PM.proj)), pixels: Array.from(pixels),
              dither: gl.isEnabled(gl.DITHER), supported: PM.GL.supportedColorDepths, attempts: attempt + 1,
              colors: colors.size, visibleVideoPixels };
          previous = complete ? pixels : null;
          await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
        }
        throw new Error(`The ${kind} depth-${depth} reference did not produce two complete identical frames`);
      }, { time, depth, kind });
      const id = `${kind}-${depth}-${Math.round(time * 10)}`;
      const rgba = new Float32Array(capture.pixels);
      writeFileSync(path.join(out, `${id}.json`), JSON.stringify(capture.doc));
      writeFileSync(path.join(out, `${id}.rgba32f`), new Uint8Array(rgba.buffer));
      writeFileSync(path.join(out, `${id}.png`), encodePng({ width: 320, height: 180, channels: 4,
        data: Uint8Array.from(rgba, v => Math.round(Math.min(1, Math.max(0, v)) * 255)) }));
      samples.push({ id, kind, depth, width: 320, height: 180, time, media,
        dither: capture.dither, supported: capture.supported, attempts: capture.attempts, colors: capture.colors,
        visibleVideoPixels: capture.visibleVideoPixels, projectId: created.id });
      console.log(`Captured ${id}`);
    }
  }
  writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ samples }, null, 2));
  if (app.pageErrors.length) throw new Error(`Legacy app page errors: ${app.pageErrors.join('; ')}`);
} finally { await app.close(); }
