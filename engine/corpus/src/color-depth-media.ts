/** Legacy-app reference frames for the video and 3D color-depth gate.
 * Build apps/desktop outside the lock, prepare the generated clip, then run:
 *   bun engine/corpus/src/color-depth-media.ts /external/output --prepare
 *   pm-electron-lock bun engine/corpus/src/color-depth-media.ts /external/output
 * The fixture is read from the repository at runtime. Only the supplied
 * external directory receives captures; launchLegacyApp removes its profile.
 */
import path from 'node:path';
import os from 'node:os';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, renameSync, rmSync, writeFileSync } from 'node:fs';
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
const detailFixture = path.join(out, 'detail.mp4');
if (process.argv[3] === '--prepare') {
  const ffmpeg = path.join(process.env.PM_FFMPEG_DIR ?? path.join(os.homedir(), 'Library/Caches/powermove-engine/ffmpeg/out'), 'bin/ffmpeg');
  if (!existsSync(ffmpeg)) throw new Error(`Missing pinned FFmpeg: ${ffmpeg}`);
  const temporary = path.join(out, 'detail.tmp.mp4');
  try {
    const result = spawnSync(ffmpeg, [
      '-hide_banner', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=s=160x90:r=30:d=2',
      '-vf', "format=rgb24,lutrgb=r='val*0.55+50':g='val*0.55+50':b='val*0.55+50',format=yuv420p",
      '-c:v', 'h264_videotoolbox', '-b:v', '1M', '-color_primaries', 'bt709', '-color_trc', 'bt709',
      '-colorspace', 'bt709', '-color_range', 'tv', '-an', temporary,
    ], { encoding: 'utf8' });
    if (result.error || result.status !== 0) throw new Error(`Pinned FFmpeg detail fixture failed: ${result.error ?? result.stderr}`);
    renameSync(temporary, detailFixture);
  } finally { rmSync(temporary, { force: true }); }
  console.log(`Prepared ${detailFixture}`);
  process.exit(0);
}
if (!existsSync(detailFixture)) throw new Error(`Generate detailed video outside the Electron lock first: bun ${import.meta.path} ${out} --prepare`);
const app = await launchLegacyApp();
const samples: Array<Record<string, unknown>> = [];
try {
  for (const kind of ['video', 'video-detail', 'scene3d'] as const) for (const depth of kind === 'scene3d' ? [16, 32, 8] : [8, 16, 32]) {
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
    if (kind === 'video' || kind === 'video-detail') {
      const source = kind === 'video-detail' ? detailFixture : fixture;
      await app.evaluate(() => {
        const input = document.createElement('input'); input.type = 'file'; input.id = 'color-depth-media-input';
        input.hidden = true; document.body.append(input);
      });
      try {
        await app.setInputFiles('#color-depth-media-input', [source]);
        const assetId = await app.evaluate(async ({ kind }) => {
          const PM = (window as any).PM;
          const input = document.querySelector('#color-depth-media-input') as HTMLInputElement;
          const file = input.files?.[0]; if (!file) throw new Error('Video fixture input is empty');
          const asset = await PM.assets.add(file, { newAsset: true, silent: true });
          const detail = kind === 'video-detail';
          const layer = PM.mkLayer('video', { name: detail ? 'Detailed clip' : 'Fixture clip',
            d: { asset: asset.id, fit: 'contain', w: detail ? 240 : 320, h: detail ? 135 : 180 },
            p: { 'position.x': 160, 'position.y': 90, ...(detail ? { rotation: 14, 'scale.x': 92, 'scale.y': 83, opacity: 67 } : {}) } }, PM.proj);
          PM.proj.layers.unshift(layer); PM.touch(); PM.invalidate();
          return asset.id as string;
        }, { kind });
        media = [{ assetId, path: source }];
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
    const times = kind === 'scene3d' ? [0.5] : [0.5, 1.5];
    for (const time of times) {
      const capture = await app.evaluate(async ({ time, depth, kind }) => {
        const PM = (window as any).PM;
        PM.setTime(time, { raw: true, force: true });
        await PM.prepareFrame(time);
        // prepareFrame publishes the canvas that the compositor passes to
        // texFor. Save its decoded pixels before the layer transform/blend so
        // a parity mismatch can be located on either side of that upload.
        let source: { width: number; height: number; layer: string; asset: string; pixels: number[] } | undefined;
        if (kind === 'video-detail') {
          const layer = PM.proj.layers.find((item: any) => item.type === 'video');
          if (!layer) throw new Error('Detailed video layer is missing');
          const canvas = PM.preparedVideoFrames?.get(layer.id + '@' + time) as HTMLCanvasElement | undefined;
          if (!canvas?.width || !canvas.height) throw new Error('Detailed video has no prepared source canvas');
          const data = canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data;
          source = { width: canvas.width, height: canvas.height, layer: layer.id, asset: layer.d.asset, pixels: Array.from(data) };
        }
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
          let detailNonbackgroundPixels = 0;
          let detailMidtonePixels = 0;
          for (let i = 0; i < pixels.length; i += 4) {
            const rgb = [Math.round(pixels[i]! * 255), Math.round(pixels[i + 1]! * 255), Math.round(pixels[i + 2]! * 255)];
            if (kind !== 'video') colors.add((rgb[0]! << 16) | (rgb[1]! << 8) | rgb[2]!);
            // The fixture is red at 0.5 s and green at 1.5 s. Require the
            // expected image over most of the canvas, not a blank readback.
            const expectedVideoColor = time < 1
              ? rgb[0]! > 200 && rgb[1]! < 32 && rgb[2]! < 32
              : rgb[1]! > 200 && rgb[0]! < 32 && rgb[2]! < 32;
            if (kind === 'video' && expectedVideoColor)
              visibleVideoPixels++;
            if (kind === 'video-detail') {
              if (Math.max(Math.abs(rgb[0]! - 40), Math.abs(rgb[1]! - 51), Math.abs(rgb[2]! - 70)) > 16)
                detailNonbackgroundPixels++;
              if (rgb.every(channel => channel > 32 && channel < 224)) detailMidtonePixels++;
            }
          }
          const complete = kind === 'scene3d' ? colors.size > 8
            : kind === 'video-detail' ? colors.size > 64 && detailNonbackgroundPixels > 7200 && detailMidtonePixels > 7200
              : visibleVideoPixels > 320 * 180 / 2;
          if (complete && previous && previous.every((value, i) => value === pixels[i]))
            return { doc: JSON.parse(JSON.stringify(PM.proj)), pixels: Array.from(pixels), source,
              dither: gl.isEnabled(gl.DITHER), supported: PM.GL.supportedColorDepths, attempts: attempt + 1,
              colors: colors.size, visibleVideoPixels, detailNonbackgroundPixels, detailMidtonePixels };
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
      let sourceMetadata: Record<string, unknown> | undefined;
      if (capture.source) {
        const { pixels, ...metadata } = capture.source;
        sourceMetadata = metadata;
        writeFileSync(path.join(out, `${id}.source.rgba`), new Uint8Array(pixels));
        writeFileSync(path.join(out, `${id}.source.json`), JSON.stringify(metadata));
      }
      samples.push({ id, kind, depth, width: 320, height: 180, time, media,
        dither: capture.dither, supported: capture.supported, attempts: capture.attempts, colors: capture.colors,
        visibleVideoPixels: capture.visibleVideoPixels, detailNonbackgroundPixels: capture.detailNonbackgroundPixels,
        detailMidtonePixels: capture.detailMidtonePixels, source: sourceMetadata, projectId: created.id });
      console.log(`Captured ${id}`);
    }
  }
  writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ samples }, null, 2));
  if (app.pageErrors.length) throw new Error(`Legacy app page errors: ${app.pageErrors.join('; ')}`);
} finally { await app.close(); }
