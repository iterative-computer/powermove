/** Source-monitor video colour against an independent legacy app profile and
 * a double-precision reference built from the shared video colour table
 * (`pm_media::probe::color::video_reading`) that composition and export use.
 * Where today's CSS video already reads a stream that way it stays exact.
 * Elsewhere today differs (VideoToolbox's 1.961-gamma 709 curve, Chromium's
 * Rec.601 default below 720 lines, SMPTE 170M primaries for untagged SD), and
 * the monitor must be at least as close to the reference as today.
 * Run with the machine's DISPLAY profile (the normal Electron launch args).
 * Synthetic inputs and intermediate captures never enter the repository.
 */
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';
import { inflateSync } from 'node:zlib';
import type { CDPSession } from 'playwright';

import { expect, launchApp, test, type LaunchedApp } from './helpers/app';
import { fixturePath } from './helpers/media';
import { readPngMatrixTrcProfile, referenceDisplayCodes, type MatrixTrcProfile, type VideoTags } from './helpers/source-colour-reference';
import { capturedPixelDelta, decodeCapturedPng, sourceFfmpeg } from './helpers/source-media';

const run = promisify(execFile);
const WIDTH = 320, HEIGHT = 180;
const MEDIA_EVENT_LIMIT = 1_000, MEDIA_BYTES_LIMIT = 2 * 1024 * 1024;
const THEMES = ['light', 'dark'] as const;
/** Half a code of 8-bit output rounding, plus half a code for the 8-bit
 * planes Chromium samples before converting. */
const REFERENCE_PRECISION = 1;
/** A toast's --shadow-float reaches about 100 device px below it. */
const SHADOW_MARGIN = 128;
const SIZES = ['full', 'intrinsic'] as const;
type Fixture = { name: string; file: string; times: number[] };
const SD601: VideoTags = { matrix: 'bt601', fullRange: false, primaries: 'smpte170m' };
const HD709: VideoTags = { matrix: 'bt709', fullRange: false, primaries: 'bt709' };
const FULL709: VideoTags = { matrix: 'bt709', fullRange: true, primaries: 'bt709' };
const TAG709 = { primaries: 'bt709', trc: 'bt709', space: 'bt709' };
const RECIPES: Record<string, Recipe> = {
  'bars-gradient-709-limited.mp4': { codec: 'h264', matrix: 'bt709', tags: TAG709 },
  'bars-gradient-untagged-sd.mp4': { codec: 'h264', matrix: 'bt601' },
  'bars-gradient-709-full.mp4': { codec: 'h264', matrix: 'bt709', full: true, tags: TAG709 },
  'bars-gradient-709-full-hd.mp4': { codec: 'h264', width: 1280, height: 720, matrix: 'bt709', full: true, tags: TAG709 },
  'bars-gradient-untagged-hd.mp4': { codec: 'h264', width: 1280, height: 720, matrix: 'bt709' },
  'bars-gradient-untagged-portrait.mp4': { codec: 'h264', width: 480, height: 720, matrix: 'bt709' },
  'bars-gradient-matrix-only-709.mp4': { codec: 'h264', matrix: 'bt709', tags: { space: 'bt709' } },
  'bars-gradient-709-main10-hd.mp4': { codec: 'hevc10', width: 1280, height: 720, matrix: 'bt709', tags: TAG709 },
  'bars-gradient-170m-sd.mp4': { codec: 'h264', matrix: 'bt601', tags: { primaries: 'smpte170m', trc: 'smpte170m', space: 'smpte170m' } },
  'bars-gradient-470bg-pal.mp4': { codec: 'h264', width: 720, height: 576, matrix: 'bt601', tags: { primaries: 'bt470bg', trc: 'bt470bg', space: 'bt470bg' } },
  'bars-gradient-untagged-pal.mp4': { codec: 'h264', width: 720, height: 576, matrix: 'bt601' },
  'bars-gradient-2020-sdr-hd.mp4': { codec: 'hevc10', width: 1280, height: 720, matrix: 'bt2020', tags: { primaries: 'bt2020', trc: 'bt2020-10', space: 'bt2020nc' } },
  'bars-gradient-pq-hd.mp4': { codec: 'hevc10', width: 1280, height: 720, matrix: 'bt2020', tags: { primaries: 'bt2020', trc: 'smpte2084', space: 'bt2020nc' } },
  'bars-gradient-untagged-hd.webm': { codec: 'vp9', width: 1280, height: 720, matrix: 'bt709' },
  // A 1080p image sequence rendition written before the importer tagged them.
  'bars-gradient-sequence-1080.webm': { codec: 'vp9', width: 1920, height: 1080, matrix: 'bt601' }
};
/** The shared reading (`pm_media::probe::color::frame_reading`) of each
 * fixture. `exact` where today's CSS video reads the stream the same way;
 * the rest are reference-gated. A null reading only has to play: HDR tone
 * mapping has no reference here. */
const REFERENCE: Record<string, { tags: VideoTags | null; exact: boolean; sequence?: boolean; alpha?: boolean }> = {
  'h264-aac.mp4': { tags: SD601, exact: true },
  'bars-gradient-709-limited.mp4': { tags: HD709, exact: true },
  'bars-gradient-untagged-sd.mp4': { tags: SD601, exact: true },
  'bars-gradient-709-full.mp4': { tags: FULL709, exact: true },
  'bars-gradient-709-full-hd.mp4': { tags: FULL709, exact: false },
  'bars-gradient-untagged-hd.mp4': { tags: HD709, exact: false },
  'bars-gradient-untagged-portrait.mp4': { tags: HD709, exact: false },
  // Partial tags: the tagged BT.709 matrix wins; SD size gives 170M primaries.
  'bars-gradient-matrix-only-709.mp4': { tags: { ...HD709, primaries: 'smpte170m' }, exact: false },
  // 10-bit 4:2:0 HEVC: VideoToolbox P010 through the IOSurface route.
  'bars-gradient-709-main10-hd.mp4': { tags: HD709, exact: false },
  'bars-gradient-170m-sd.mp4': { tags: SD601, exact: true },
  // BT.470 gamma 2.8 reads as the display encoding, like other SDR curves.
  'bars-gradient-470bg-pal.mp4': { tags: { ...SD601, primaries: 'bt470bg' }, exact: false },
  // Untagged 576-line video reads the 625-line (EBU, BT.470BG) primaries.
  'bars-gradient-untagged-pal.mp4': { tags: { ...SD601, primaries: 'bt470bg' }, exact: false },
  // Converted in the engine as composition reads it.
  'bars-gradient-2020-sdr-hd.mp4': { tags: { matrix: 'bt2020', fullRange: false, primaries: 'bt2020' }, exact: false },
  'bars-gradient-pq-hd.mp4': { tags: null, exact: false },
  'bars-gradient-untagged-hd.webm': { tags: HD709, exact: false },
  'bars-gradient-sequence-1080.webm': { tags: { ...HD709, matrix: 'bt601' }, exact: false, sequence: true },
  // Untagged SD ProRes: today's VideoToolbox monitor reads Rec.709 primaries.
  'prores422.mov': { tags: SD601, exact: false },
  // Untagged SD VP9 with alpha: today's software decode reads Rec.709 primaries.
  // Only opaque samples are measured.
  'bars-gradient-alpha-vp9.webm': { tags: SD601, exact: false, alpha: true }
};
type Yuv420 = { width: number; height: number; bits: 8 | 10; data: Buffer; alpha: boolean };
type Closeness = { pixels: number; max: number; mean: number; over1: number; worst: unknown };
type Capture = { file: string; geometry: unknown; covered: number[][]; monitorTime?: number | null; pngColor: ReturnType<typeof pngColorMetadata>; effectiveVideoFrameColorSpace?: unknown; legacyVideoFrameGeometry?: unknown; legacyCodedPlaneEdges?: unknown };

function syntheticPpm(width = WIDTH, height = HEIGHT): Buffer {
  // Every neutral code value across the top, then eight saturated and mixed
  // bars. The top ramp stays visible above the monitor transport at small sizes.
  const bars = [[224, 64, 32], [16, 180, 41], [24, 64, 232], [245, 203, 22],
    [21, 213, 222], [205, 34, 188], [255, 255, 255], [0, 0, 0]];
  const pixels = Buffer.alloc(width * height * 3);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    const rgb = y < Math.round(height * 64 / HEIGHT) ? [Math.round(x * 255 / (width - 1))] : bars[Math.floor(x / (width / bars.length))]!;
    const offset = (y * width + x) * 3;
    for (let channel = 0; channel < 3; channel++) pixels[offset + channel] = rgb[channel] ?? rgb[0]!;
  }
  return Buffer.concat([Buffer.from(`P6\n${width} ${height}\n255\n`), pixels]);
}

/** A synthetic clip: how its bars are encoded and the colour tags it carries.
 * `bt2020` converts the sRGB bars into BT.2020 (in gamut, so a Rec.709
 * working space keeps them); other recipes encode the code values as-is. */
type Recipe = { codec: 'h264' | 'hevc10' | 'vp9'; width?: number; height?: number; matrix: 'bt601' | 'bt709' | 'bt2020';
  full?: boolean; tags?: { primaries?: string; trc?: string; space?: string } };

async function makeSyntheticFixture(scratch: string, name: string, recipe: Recipe): Promise<string> {
  const { width = WIDTH, height = HEIGHT } = recipe;
  const file = path.join(scratch, name);
  const ppm = path.join(scratch, `bars-gradient-${width}x${height}.ppm`);
  await writeFile(ppm, syntheticPpm(width, height));
  const range = recipe.full ? 'pc' : 'tv';
  const ten = recipe.codec === 'hevc10';
  const encode = recipe.matrix === 'bt2020'
    ? `scale=${width}:${height}:out_color_matrix=bt709:out_range=tv,format=yuv444p,setparams=range=tv:color_primaries=bt709:color_trc=bt709:colorspace=bt709,colorspace=all=bt2020:format=yuv420p10,format=p010le`
    : `scale=${width}:${height}:out_color_matrix=${recipe.matrix}:out_range=${range},format=${ten ? 'p010le' : 'yuv420p'}`;
  const { primaries = 'unknown', trc = 'unknown', space = 'unknown' } = recipe.tags ?? {};
  const tagged = recipe.tags ? ['-color_range', range,
    ...(recipe.tags.primaries ? ['-color_primaries', primaries] : []), ...(recipe.tags.trc ? ['-color_trc', trc] : []),
    ...(recipe.tags.space ? ['-colorspace', space] : [])] : [];
  await run(sourceFfmpeg(), ['-hide_banner', '-loglevel', 'error', '-nostdin', '-y',
    '-loop', '1', '-framerate', '2', '-i', ppm, '-frames:v', '3',
    '-vf', `${encode},setparams=range=${recipe.full ? 'full' : 'limited'}:color_primaries=${primaries}:color_trc=${trc}:colorspace=${space}`,
    ...(recipe.codec === 'vp9' ? ['-c:v', 'libvpx-vp9', '-lossless', '1']
      : ten ? ['-c:v', 'hevc_videotoolbox', '-profile:v', 'main10', '-tag:v', 'hvc1', '-allow_sw', '1', '-b:v', '5M']
        : ['-c:v', 'h264_videotoolbox', '-allow_sw', '1', '-b:v', '5M']),
    '-g', '1', ...tagged, file], { timeout: 60_000 });
  return file;
}

function syntheticRgba(): Buffer {
  const rgb = syntheticPpm().subarray(Buffer.byteLength(`P6\n${WIDTH} ${HEIGHT}\n255\n`));
  const frame = Buffer.alloc(WIDTH * HEIGHT * 4);
  for (let y = 0; y < HEIGHT; y++) for (let x = 0; x < WIDTH; x++) {
    const at = y * WIDTH + x, dst = at * 4, src = at * 3;
    frame[dst] = rgb[src]!; frame[dst + 1] = rgb[src + 1]!; frame[dst + 2] = rgb[src + 2]!;
    // Fully transparent, partial, and opaque cells are all visible against
    // the source preview's real backdrop and transport controls.
    frame[dst + 3] = y < HEIGHT / 3 ? Math.round(x * 255 / (WIDTH - 1)) :
      y < HEIGHT * 2 / 3 ? [0, 64, 128, 192][Math.floor(x / (WIDTH / 4))]! : 255;
  }
  return frame;
}

async function makeAlphaVp9Fixture(scratch: string): Promise<string> {
  const raw = path.join(scratch, 'bars-gradient-alpha.rgba');
  const file = path.join(scratch, 'bars-gradient-alpha-vp9.webm');
  const frame = syntheticRgba();
  await writeFile(raw, Buffer.concat([frame, frame, frame]));
  await run(sourceFfmpeg(), ['-hide_banner', '-loglevel', 'error', '-nostdin', '-y',
    '-f', 'rawvideo', '-pix_fmt', 'rgba', '-s', `${WIDTH}x${HEIGHT}`, '-framerate', '2', '-i', raw,
    '-frames:v', '3', '-vf', 'format=yuva420p', '-c:v', 'libvpx-vp9', '-lossless', '1',
    '-auto-alt-ref', '0', '-pix_fmt', 'yuva420p', '-g', '1', file], { timeout: 60_000 });
  return file;
}

async function assertVp9Alpha(file: string): Promise<void> {
  const { stdout } = await run(sourceFfmpeg(), ['-hide_banner', '-loglevel', 'error', '-nostdin',
    '-c:v', 'libvpx-vp9', '-i', file, '-frames:v', '1', '-pix_fmt', 'rgba', '-f', 'rawvideo', 'pipe:1'],
  { timeout: 30_000, encoding: 'buffer', maxBuffer: WIDTH * HEIGHT * 4 + 4096 });
  const pixels = Buffer.from(stdout);
  if (pixels.length !== WIDTH * HEIGHT * 4) throw new Error('VP9 alpha fixture did not decode to RGBA');
  let hasTransparent = false, hasPartial = false, hasOpaque = false;
  for (let at = 3; at < pixels.length; at += 4) {
    const alpha = pixels[at]!;
    hasTransparent ||= alpha === 0;
    hasPartial ||= alpha > 0 && alpha < 255;
    hasOpaque ||= alpha === 255;
  }
  if (!hasTransparent || !hasPartial || !hasOpaque) throw new Error('VP9 fixture lost transparent, partial, or opaque alpha');
}

async function probe(file: string) {
  const ffprobe = sourceFfmpeg().replace(/ffmpeg$/, 'ffprobe');
  const { stdout } = await run(ffprobe, ['-v', 'error', '-select_streams', 'v:0',
    '-show_entries', 'stream=codec_name,width,height,pix_fmt,color_range,color_space,color_transfer,color_primaries:stream_tags',
    '-of', 'json', file], { timeout: 30_000 });
  return JSON.parse(stdout).streams?.[0] as Record<string, unknown> | undefined;
}

function pngColorMetadata(png: Buffer) {
  const chunks: string[] = [];
  let iccSha256: string | null = null;
  for (let at = 8; at + 12 <= png.length;) {
    const size = png.readUInt32BE(at), type = png.toString('ascii', at + 4, at + 8);
    const dataStart = at + 8, dataEnd = dataStart + size;
    if (dataEnd + 4 > png.length) throw new Error('Truncated source-monitor PNG');
    if (['iCCP', 'sRGB', 'gAMA', 'cHRM', 'cICP'].includes(type)) chunks.push(type);
    if (type === 'iCCP') {
      const separator = png.indexOf(0, dataStart);
      if (separator < dataStart || separator + 2 > dataEnd) throw new Error('Invalid PNG iCCP chunk');
      iccSha256 = createHash('sha256').update(inflateSync(png.subarray(separator + 2, dataEnd))).digest('hex');
    }
    at = dataEnd + 4;
    if (type === 'IEND') break;
  }
  return { width: png.readUInt32BE(16), height: png.readUInt32BE(20), iccSha256, colorChunks: chunks };
}

async function decodeYuv420(file: string, time: number, width: number, height: number, bits: 8 | 10, alpha = false): Promise<Yuv420> {
  // Planar 4:2:0 at the stream's depth and range: no range or matrix conversion.
  const { stdout } = await run(sourceFfmpeg(), ['-hide_banner', '-loglevel', 'error', '-nostdin', '-ss', String(time),
    ...(alpha ? ['-c:v', 'libvpx-vp9'] : []), '-i', file, '-frames:v', '1',
    ...(alpha ? ['-pix_fmt', 'yuva420p'] : bits === 10 ? ['-pix_fmt', 'yuv420p10le'] : []), '-f', 'rawvideo', 'pipe:1'],
  { timeout: 30_000, encoding: 'buffer', maxBuffer: width * height * 4 });
  const data = Buffer.from(stdout);
  if (data.length !== (width * height * (alpha ? 2 : 1) + 2 * Math.ceil(width / 2) * Math.ceil(height / 2)) * (bits / 8 > 1 ? 2 : 1))
    throw new Error(`${file} did not decode to ${bits}-bit 4:2:0`);
  return { width, height, bits, data, alpha };
}

/** Per-channel code distance to the reference where the result does not
 * depend on the presenter's filter: chroma is constant within four chroma
 * samples (Chromium's GPU chroma sampling reaches past the 2×2 block) and,
 * when scaled, luma within two. Chrome over the video and its shadow are
 * excluded. */
async function referenceCloseness(capture: Capture, covered: number[][], yuv: Yuv420, tags: VideoTags, profile: MatrixTrcProfile, intrinsic: boolean): Promise<Closeness> {
  const { size: [width, height], pixels } = await decodeCapturedPng(capture.file);
  const { dpr, surface } = capture.geometry as { dpr: number; surface: Record<'x' | 'y' | 'width' | 'height', number> };
  const mask = new Uint8Array(width! * height!);
  for (const [x, y, w, h] of covered) {
    const left = Math.max(0, Math.floor(x! - SHADOW_MARGIN)), right = Math.min(width!, Math.ceil(x! + w! + SHADOW_MARGIN));
    for (let my = Math.max(0, Math.floor(y! - SHADOW_MARGIN)); my < Math.min(height!, Math.ceil(y! + h! + SHADOW_MARGIN)); my++)
      if (right > left) mask.fill(1, left + my * width!, right + my * width!);
  }
  const cw = Math.ceil(yuv.width / 2), ch = Math.ceil(yuv.height / 2), u0 = yuv.width * yuv.height, v0 = u0 + cw * ch;
  const at = yuv.bits === 10
    ? (plane: number, stride: number, x: number, y: number) => yuv.data.readUInt16LE(2 * (plane + y * stride + x))
    : (plane: number, stride: number, x: number, y: number) => yuv.data[plane + y * stride + x]!;
  const flat = (plane: number, stride: number, w: number, h: number, x: number, y: number, r: number) => {
    if (x - r < 0 || y - r < 0 || x + r >= w || y + r >= h) return false;
    const value = at(plane, stride, x, y);
    for (let dy = -r; dy <= r; dy++) for (let dx = -r; dx <= r; dx++) if (at(plane, stride, x + dx, y + dy) !== value) return false;
    return true;
  };
  const scale = Math.min(surface.width * dpr / yuv.width, surface.height * dpr / yuv.height);
  const left = surface.x * dpr + (surface.width * dpr - yuv.width * scale) / 2, top = surface.y * dpr + (surface.height * dpr - yuv.height * scale) / 2;
  const cache = new Map<number, readonly [number, number, number]>();
  let count = 0, max = 0, sum = 0, over1 = 0, worst: unknown = null;
  for (let py = 0; py < height!; py++) for (let px = 0; px < width!; px++) {
    if (mask[py * width! + px]) continue;
    const sx = Math.round((px + .5 - left) / scale - .5), sy = Math.round((py + .5 - top) / scale - .5);
    if (sx < 2 || sy < 2 || sx >= yuv.width - 2 || sy >= yuv.height - 2) continue;
    if (!flat(u0, cw, cw, ch, sx >> 1, sy >> 1, 4) || !flat(v0, cw, cw, ch, sx >> 1, sy >> 1, 4)) continue;
    if (!intrinsic && !flat(0, yuv.width, yuv.width, yuv.height, sx, sy, 2)) continue;
    if (yuv.alpha && (at(v0 + cw * ch, yuv.width, sx, sy) !== 255 || !flat(v0 + cw * ch, yuv.width, yuv.width, yuv.height, sx, sy, 2))) continue;
    const sample: [number, number, number] = [at(0, yuv.width, sx, sy), at(u0, cw, sx >> 1, sy >> 1), at(v0, cw, sx >> 1, sy >> 1)];
    const key = (sample[0] * 1024 + sample[1]) * 1024 + sample[2];
    let reference = cache.get(key);
    if (!reference) cache.set(key, reference = referenceDisplayCodes(profile, sample, tags, yuv.bits));
    const offset = (py * width! + px) * 4;
    let pixelMax = 0;
    for (let channel = 0; channel < 3; channel++) {
      const error = Math.abs(pixels[offset + channel]! - reference[channel]!);
      pixelMax = Math.max(pixelMax, error); sum += error;
    }
    if (pixelMax > 1) over1++;
    if (pixelMax > max) {
      max = pixelMax;
      worst = { at: [px, py], source: [sx, sy], yuv: sample, captured: [...pixels.subarray(offset, offset + 3)], reference };
    }
    count++;
  }
  return { pixels: count, max, mean: count ? sum / (count * 3) : 0, over1, worst };
}

async function importPath(session: LaunchedApp, fixture: Fixture): Promise<string> {
  const { page } = session;
  const chooser = page.waitForEvent('filechooser');
  await page.evaluate(() => (window as any).PM.pickFiles());
  await (await chooser).setFiles(fixture.file);
  await page.waitForFunction(name => {
    const PM = (window as any).PM;
    return Object.values<any>(PM.proj.assets).some(asset => asset.name === name)
      && [...PM.assets.map.values()].some((asset: any) => asset.name === name);
  }, fixture.name);
  return page.evaluate(name => Object.values<any>((window as any).PM.proj.assets).find(asset => asset.name === name).id, fixture.name);
}

async function showAt(session: LaunchedApp, id: string, time: number, theme: string, size: typeof SIZES[number], intrinsic: [number, number], enabled: boolean) {
  const { page } = session;
  await page.evaluate(({ id, theme, size, intrinsic }) => {
    const PM = (window as any).PM;
    PM.theme.apply(theme);
    const root = document.querySelector<HTMLElement>('#source-preview')!;
    if (size === 'intrinsic') {
      root.style.width = `${intrinsic[0] / devicePixelRatio}px`;
      root.style.height = `${intrinsic[1] / devicePixelRatio}px`;
      root.style.right = root.style.bottom = 'auto';
      root.style.left = root.style.top = '0px';
    } else {
      root.style.removeProperty('width'); root.style.removeProperty('height');
      root.style.removeProperty('right'); root.style.removeProperty('bottom');
      root.style.removeProperty('left'); root.style.removeProperty('top');
    }
    PM.Kernel.services.get('viewer').preview.show(id);
    if (size === 'intrinsic') {
      // Playwright's element clip rounds CSS bounds before device scaling.
      // The stage begins at a fractional viewport coordinate, so a 45 CSS px
      // monitor otherwise produces a 92 px PNG at DPR 2. Translate only the
      // test monitor's origin onto the whole CSS/device-pixel grid; its size,
      // internal layout, fit and video pixels remain untouched.
      const rect = root.getBoundingClientRect();
      root.style.left = `${Math.round(rect.left) - rect.left}px`;
      root.style.top = `${Math.round(rect.top) - rect.top}px`;
    }
  }, { id, theme, size, intrinsic });
  await expect(page.locator('#source-preview[data-open="true"]')).toBeVisible();
  await page.waitForFunction(enabled => {
    if (!enabled) return (document.querySelector<HTMLVideoElement>('#source-preview .sp-media > video')?.readyState ?? 0) >= 2;
    const monitor = (window as any).PM.EngineSourceMedia?.monitorValue;
    if (monitor?.error) throw new Error(`Native source monitor: ${monitor.error.message ?? String(monitor.error)}`);
    return Boolean(monitor?.surface instanceof HTMLVideoElement && monitor.surface.dataset.nativeSource === 'true'
      && monitor.surface.readyState >= 2 && !monitor.rendering && monitor.paintedSize);
  }, enabled, { timeout: 15_000 });
  await page.evaluate(async ({ time, enabled, size, intrinsic }) => {
    const root = document.querySelector<HTMLElement>('#source-preview')!;
    const surface = root.querySelector<HTMLElement>('.sp-media > video, .sp-media > canvas')!;
    const source: any = enabled ? (window as any).PM.EngineSourceMedia.monitorValue : surface;
    const event = enabled ? 'loadeddata' : 'seeked';
    if (Math.abs(source.currentTime - time) > .0005 || (!enabled && source.readyState < 2)) {
      await new Promise<void>((resolve, reject) => {
        const cleanup = () => { clearTimeout(timer); source.removeEventListener(event, done); source.removeEventListener('error', failed); };
        const timer = setTimeout(() => { cleanup(); reject(new Error('Source-monitor seek timed out')); }, 15_000);
        const done = () => { cleanup(); resolve(); };
        const failed = () => { cleanup(); reject(new Error(`Native source monitor: ${source.error?.message ?? String(source.error)}`)); };
        source.addEventListener(event, done);
        if (enabled) source.addEventListener('error', failed);
        source.currentTime = time;
      });
    }
    const until = performance.now() + 15_000;
    while (true) {
      const rect = root.getBoundingClientRect();
      const dimensionsReady = size === 'full' ||
        Math.round(rect.width * devicePixelRatio) === intrinsic[0] && Math.round(rect.height * devicePixelRatio) === intrinsic[1];
      const nativeReady = !enabled || (() => {
        const monitor = (window as any).PM.EngineSourceMedia.monitorValue;
        if (monitor.error) throw new Error(`Native source monitor: ${monitor.error.message ?? String(monitor.error)}`);
        return monitor.surface instanceof HTMLVideoElement && monitor.surface.readyState >= 2 && !monitor.rendering
          && monitor.paintedSize?.width === Math.round(surface.clientWidth * devicePixelRatio)
          && monitor.paintedSize?.height === Math.round(surface.clientHeight * devicePixelRatio);
      })();
      if (dimensionsReady && nativeReady) break;
      if (performance.now() > until) throw new Error('Source-monitor size or paint timed out');
      await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    }
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
  }, { time, enabled, size, intrinsic });
}

async function capture(session: LaunchedApp, file: string, enabled: boolean): Promise<Capture> {
  const geometry = await session.page.evaluate(() => {
    const root = document.querySelector<HTMLElement>('#source-preview')!;
    const measure = (element: Element) => {
      const outer = root.getBoundingClientRect(), rect = element.getBoundingClientRect();
      return { x: rect.x - outer.x, y: rect.y - outer.y, width: rect.width, height: rect.height };
    };
    return { dpr: devicePixelRatio, root: measure(root), media: measure(root.querySelector('.sp-media')!),
      surface: measure(root.querySelector('.sp-media > *')!), foot: measure(root.querySelector('.sp-foot')!) };
  });
  const legacyVideoFrame = enabled ? undefined : await session.page.evaluate(async () => {
    const video = document.querySelector<HTMLVideoElement>('#source-preview .sp-media > video')!;
    const frame = new VideoFrame(video);
    try {
      const rect = frame.visibleRect;
      let codedPlaneEdges: unknown = null;
      if (rect && frame.codedWidth > rect.x + rect.width) {
        try {
          const format: string | null = frame.format;
          if (!format) throw new Error('The decoded VideoFrame has no copyable pixel format.');
          const options = { rect: { x: 0, y: 0, width: frame.codedWidth, height: frame.codedHeight } };
          const byteLength = frame.allocationSize(options);
          if (byteLength > 32 * 1024 * 1024) throw new Error(`Coded frame copy exceeds diagnostic limit: ${byteLength} bytes`);
          const data = new Uint8Array(byteLength);
          const layout = await frame.copyTo(data, options);
          const isBiplanar = format === 'NV12' || format === 'NV21' || format === 'P010';
          const is444 = format.startsWith('I444');
          const is420 = format.startsWith('I420') || isBiplanar;
          const alphaIndex = format.endsWith('A') ? layout.length - 1 : -1;
          const wordBytes = format.includes('P10') || format.includes('P12') || format === 'P010' ? 2 : 1;
          const planeEdges = layout.map((plane, index) => {
            const chroma = index > 0 && index !== alphaIndex;
            const horizontalShift = chroma && !is444 ? 1 : 0;
            const verticalShift = chroma && is420 ? 1 : 0;
            const sampleBytes = wordBytes * (chroma && isBiplanar ? 2 : 1);
            const width = Math.ceil(frame.codedWidth / (1 << horizontalShift));
            const firstPadding = Math.ceil((rect.x + rect.width) / (1 << horizontalShift));
            const lastVisible = firstPadding - 1;
            const lastVisibleRow = Math.ceil((rect.y + rect.height) / (1 << verticalShift)) - 1;
            const sample = (row: number, column: number) => {
              if (row < 0 || column < 0 || column >= width || column * sampleBytes + sampleBytes > plane.stride) return null;
              const at = plane.offset + row * plane.stride + column * sampleBytes;
              return at + sampleBytes <= data.byteLength ? Array.from(data.subarray(at, at + sampleBytes)) : null;
            };
            const edge = (row: number) => ({ row, visibleLast: sample(row, lastVisible),
              paddingFirstFour: Array.from({ length: Math.min(4, Math.max(0, width - firstPadding)) }, (_, offset) => sample(row, firstPadding + offset)) });
            return { index, horizontalShift, verticalShift, sampleBytes, row0: edge(0),
              lastVisibleRow: edge(lastVisibleRow), codedLastRow: edge(Math.ceil(frame.codedHeight / (1 << verticalShift)) - 1) };
          });
          codedPlaneEdges = { format, byteLength, layout, planes: planeEdges };
        } catch (error) {
          codedPlaneEdges = { format: frame.format, error: String(error) };
        }
      }
      return {
        colorSpace: frame.colorSpace.toJSON(),
        geometry: { codedWidth: frame.codedWidth, codedHeight: frame.codedHeight,
          displayWidth: frame.displayWidth, displayHeight: frame.displayHeight,
          visibleRect: rect ? { x: rect.x, y: rect.y, width: rect.width, height: rect.height } : null },
        codedPlaneEdges
      };
    } finally { frame.close(); }
  });
  // Device-pixel rects over the video: cells whose topmost element is not the
  // video surface (the transport) and toasts, which do not take pointer
  // events. Shadows are not hit-testable; the reference check widens these.
  const covered = await session.page.evaluate(() => {
    const root = document.querySelector<HTMLElement>('#source-preview')!, media = root.querySelector('.sp-media')!;
    const outer = root.getBoundingClientRect(), step = 4, dpr = devicePixelRatio, rects: number[][] = [];
    for (let y = 0; y < outer.height; y += step) for (let x = 0; x < outer.width; x += step) {
      const top = document.elementFromPoint(outer.x + x + step / 2, outer.y + y + step / 2);
      if (!top || !media.contains(top)) rects.push([x * dpr, y * dpr, step * dpr, step * dpr]);
    }
    for (const toast of document.querySelectorAll('#toasts .toast')) {
      const rect = toast.getBoundingClientRect();
      rects.push([(rect.x - outer.x) * dpr, (rect.y - outer.y) * dpr, rect.width * dpr, rect.height * dpr]);
    }
    return rects;
  });
  await session.page.locator('#source-preview').screenshot({ path: file, animations: 'disabled', scale: 'device' });
  const monitor = enabled ? await session.page.evaluate(() => {
    const source = (window as any).PM.EngineSourceMedia;
    return { time: source.monitorValue?.currentTime ?? null };
  }) : null;
  return { file, geometry, covered, monitorTime: monitor?.time ?? null, pngColor: pngColorMetadata(await readFile(file)),
    effectiveVideoFrameColorSpace: legacyVideoFrame?.colorSpace,
    legacyVideoFrameGeometry: legacyVideoFrame?.geometry,
    legacyCodedPlaneEdges: legacyVideoFrame?.codedPlaneEdges };
}

test('source monitor video matches CSS video exactly, or the colour reference where it corrects today', async ({}, info) => {
  test.setTimeout(900_000);
  const evidence = process.env.PM_SOURCE_EVIDENCE ? path.resolve(process.env.PM_SOURCE_EVIDENCE) : null;
  const workspace = path.resolve(__dirname, '../../..');
  if (evidence) {
    const relative = path.relative(workspace, evidence);
    if (!relative.startsWith('..') && !path.isAbsolute(relative)) throw new Error('PM_SOURCE_EVIDENCE must be outside the public repository');
  }
  const scratch = await mkdtemp(path.join(os.tmpdir(), 'pm-source-compositor-input-'));
  let captures = evidence ? path.join(evidence, 'source-compositor') : '';
  const results: Record<string, Record<string, Capture>> = { legacy: {}, native: {} };
  const caseErrors: Array<{ mode: string; key: string; error: string }> = [];
  let failed = false;
  try {
    if (!evidence) captures = await mkdtemp(path.join(os.tmpdir(), 'pm-source-compositor-captures-'));
    await mkdir(captures, { recursive: true });
    const fixtures: Fixture[] = [
      { name: 'h264-aac.mp4', file: fixturePath('h264-aac.mp4'), times: [.5, 1.5] },
      { name: 'prores422.mov', file: fixturePath('prores422.mov'), times: [.5, 1.5] },
      ...await Promise.all(Object.entries(RECIPES).map(async ([name, recipe]) => ({ name, file: await makeSyntheticFixture(scratch, name, recipe), times: [.5] }))),
      { name: 'bars-gradient-alpha-vp9.webm', file: await makeAlphaVp9Fixture(scratch), times: [.5] }
    ];
    const probed = Object.fromEntries(await Promise.all(fixtures.map(async fixture => [fixture.name, await probe(fixture.file)] as const)));
    const metadata: Record<string, unknown> = { requestedColorProfile: 'DISPLAY (launchApp defaults; no test override)', fixtures: probed, sessions: {} };
    await writeFile(path.join(captures, 'fixture-tags.json'), JSON.stringify(probed, null, 2));
    await writeFile(info.outputPath('fixture-tags.json'), JSON.stringify(probed, null, 2));
    const codecName = { h264: 'h264', hevc10: 'hevc', vp9: 'vp9' } as const;
    for (const [name, recipe] of Object.entries(RECIPES)) {
      const p = probed[name]!;
      expect(p.codec_name, `${name} codec`).toBe(codecName[recipe.codec]);
      expect([p.width, p.height], `${name} size`).toEqual([recipe.width ?? WIDTH, recipe.height ?? HEIGHT]);
      if (recipe.codec === 'hevc10') expect(p.pix_fmt, `${name} depth`).toBe('yuv420p10le');
      for (const [field, want] of [['color_primaries', recipe.tags?.primaries], ['color_transfer', recipe.tags?.trc], ['color_space', recipe.tags?.space]] as const)
        if (want) expect(p[field], `${name} ${field}`).toBe(want);
        else expect([undefined, 'unknown'], `${name} unexpectedly tagged ${field}`).toContain(p[field]);
    }
    const alpha = probed['bars-gradient-alpha-vp9.webm']!;
    expect(alpha.codec_name).toBe('vp9');
    expect((alpha.tags as Record<string, unknown> | undefined)?.alpha_mode).toBe('1');
    await assertVp9Alpha(fixtures.find(fixture => fixture.name === 'bars-gradient-alpha-vp9.webm')!.file);
    const prores = probed['prores422.mov']!;
    expect(prores.codec_name).toBe('prores');
    expect(prores.pix_fmt).toBe('yuv422p10le');
    const expectedKeys = fixtures.flatMap(fixture => THEMES.flatMap(theme => SIZES.flatMap(size =>
      fixture.times.map(time => `${fixture.name}-${theme}-${size}-${String(time).replace('.', '_')}`))));
    for (const enabled of [false, true]) {
      const label = enabled ? 'native' : 'legacy';
      const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: enabled ? '1' : '0' } });
      const media = { events: [] as unknown[], totalEvents: 0, droppedEvents: 0, bytes: 0,
        maxEvents: MEDIA_EVENT_LIMIT, maxBytes: MEDIA_BYTES_LIMIT, error: null as string | null };
      let cdp: CDPSession | null = null;
      const onMediaProperties = (event: unknown) => {
        media.totalEvents++;
        const serialized = JSON.stringify(event) ?? '';
        const size = Buffer.byteLength(serialized, 'utf8');
        if (media.events.length >= MEDIA_EVENT_LIMIT || media.bytes + size > MEDIA_BYTES_LIMIT) {
          media.droppedEvents++;
          return;
        }
        media.events.push(event);
        media.bytes += size;
      };
      try {
        const appMeta = await session.app.evaluate(({ BrowserWindow, screen, app }) => {
          const window = BrowserWindow.getAllWindows()[0]!;
          const display = screen.getDisplayMatching(window.getBounds());
          return { versions: process.versions, displayColorSpace: (display as any).colorSpace ?? null,
            scaleFactor: display.scaleFactor, displayId: display.id,
            gpuFeatureStatus: app.getGPUFeatureStatus(),
            colorProfileSwitchPresent: app.commandLine.hasSwitch('force-color-profile'),
            colorProfileSwitchValue: app.commandLine.getSwitchValue('force-color-profile'),
            argv: process.argv.filter(arg => arg.includes('color-profile')) };
        });
        (metadata.sessions as Record<string, unknown>)[label] = { ...appMeta, mediaPlayerProperties: media };
        await writeFile(info.outputPath('app-metadata.json'), JSON.stringify(metadata, null, 2));
        if (evidence) await writeFile(path.join(captures, 'app-metadata.json'), JSON.stringify(metadata, null, 2));
        try {
          cdp = await session.page.context().newCDPSession(session.page);
          cdp.on('Media.playerPropertiesChanged', onMediaProperties);
          await cdp.send('Media.enable');
        } catch (error) { media.error = String(error); }
        await session.openEditor();
        for (const fixture of fixtures) {
          const id = await importPath(session, fixture);
          if (REFERENCE[fixture.name]?.sequence) await session.page.evaluate(id => {
            const PM = (window as any).PM;
            const mark = () => { PM.proj.assets[id].imageSequence = { fps: 2, frames: 3 }; };
            if (PM.doc?.write) PM.doc.write([['assets', id, 'imageSequence']], mark); else mark();
          }, id);
          const dimensions: [number, number] = [Number(probed[fixture.name]!.width), Number(probed[fixture.name]!.height)];
          for (const theme of THEMES) for (const size of SIZES) for (const time of fixture.times) {
            const key = `${fixture.name}-${theme}-${size}-${String(time).replace('.', '_')}`;
            try {
              await showAt(session, id, time, theme, size, dimensions, enabled);
              const folder = path.join(captures, label);
              await mkdir(folder, { recursive: true });
              const image = await capture(session, path.join(folder, `${key}.png`), enabled);
              await writeFile(image.file.replace(/\.png$/i, '.json'), JSON.stringify({
                file: path.basename(image.file), geometry: image.geometry, pngColor: image.pngColor,
                effectiveVideoFrameColorSpace: image.effectiveVideoFrameColorSpace ?? null,
                legacyVideoFrameGeometry: image.legacyVideoFrameGeometry ?? null,
                legacyCodedPlaneEdges: image.legacyCodedPlaneEdges ?? null
              }, null, 2));
              results[label]![key] = image;
              if (size === 'intrinsic') {
                if (image.pngColor.width !== dimensions[0] || image.pngColor.height !== dimensions[1]) {
                  failed = true;
                  if (!evidence) await copyFile(image.file, info.outputPath(`${key}-${label}-wrong-size.png`));
                }
                expect.soft(image.pngColor.width, `${label} ${key}: physical width`).toBe(dimensions[0]);
                expect.soft(image.pngColor.height, `${label} ${key}: physical height`).toBe(dimensions[1]);
              }
            } catch (error) {
              failed = true;
              caseErrors.push({ mode: label, key, error: String(error) });
            }
          }
          await session.page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').preview.clear());
        }
      } finally {
        cdp?.off('Media.playerPropertiesChanged', onMediaProperties);
        await cdp?.detach().catch(() => undefined);
        // Retain decoder and color-space evidence from both independent app
        // profiles even if an import or pixel capture failed midway through.
        await writeFile(path.join(captures, 'app-metadata.json'), JSON.stringify(metadata, null, 2));
        await writeFile(info.outputPath('app-metadata.json'), JSON.stringify(metadata, null, 2));
        await session.close();
      }
    }
    const comparisons = [];
    const decoded = new Map<string, Yuv420>();
    const measured = new Map<string, number>();
    for (const fixture of fixtures) for (const time of fixture.times) if (REFERENCE[fixture.name])
      decoded.set(`${fixture.name}-${time}`, await decodeYuv420(fixture.file, time, Number(probed[fixture.name]!.width), Number(probed[fixture.name]!.height),
        /p10le$/.test(String(probed[fixture.name]!.pix_fmt)) ? 10 : 8, REFERENCE[fixture.name]!.alpha));
    const sdrPng = results.legacy![expectedKeys.find(key => key.startsWith('bars-gradient-709-limited.mp4-'))!]?.pngColor;
    for (const key of expectedKeys) {
      const reference = results.legacy![key];
      const actual = results.native![key];
      if (!reference || !actual) {
        failed = true;
        comparisons.push({ key, error: 'Missing source capture', legacy: !!reference, native: !!actual });
        expect.soft(Boolean(reference && actual), `Missing source capture: ${key}`).toBe(true);
        continue;
      }
      let delta: Awaited<ReturnType<typeof capturedPixelDelta>>;
      try { delta = await capturedPixelDelta(reference.file, actual.file); }
      catch (error) {
        failed = true;
        if (!evidence) {
          await copyFile(reference.file, info.outputPath(`${key}-legacy.png`));
          await copyFile(actual.file, info.outputPath(`${key}-native.png`));
        }
        comparisons.push({ key, error: String(error) });
        expect.soft(false, `${key}: PNG comparison failed: ${String(error)}`).toBe(true);
        continue;
      }
      const geometryEqual = JSON.stringify(reference.geometry) === JSON.stringify(actual.geometry);
      const fixture = fixtures.find(item => key.startsWith(`${item.name}-`))!;
      const time = fixture.times.find(value => key.endsWith(`-${String(value).replace('.', '_')}`))!;
      const reading = REFERENCE[fixture.name];
      // Today's monitor puts HDR on screen (its capture loses the display
      // ICC); the native monitor tone-maps as composition does, so its
      // capture keeps the SDR display profile of every other capture.
      const profileOf = (png: typeof sdrPng) => JSON.stringify([png?.iccSha256, png?.colorChunks]);
      const pngColorEqual = reading?.tags === null ? profileOf(actual.pngColor) === profileOf(sdrPng)
        : JSON.stringify(reference.pngColor) === JSON.stringify(actual.pngColor);
      let colour: Record<string, Closeness> | null = null;
      if (reading?.tags) {
        const profile = readPngMatrixTrcProfile(await readFile(reference.file));
        const yuv = decoded.get(`${fixture.name}-${time}`)!, intrinsic = key.includes('-intrinsic-');
        const covered = [...reference.covered, ...actual.covered];
        colour = { legacy: await referenceCloseness(reference, covered, yuv, reading.tags, profile, intrinsic),
          native: await referenceCloseness(actual, covered, yuv, reading.tags, profile, intrinsic) };
      }
      const referenceGated = reading?.exact === false;
      const playsOnly = reading?.tags === null;
      // Chromium's compositor keeps ~2 codes of error on 10-bit frames whether
      // they arrive as a P010 IOSurface or a WebCodecs I420P10 buffer (both
      // measured); 10-bit sources must still be at least as close as today.
      const precise = decoded.get(`${fixture.name}-${time}`)?.bits === 10 || colour?.native!.max! <= REFERENCE_PRECISION;
      // Small intrinsic monitors can be entirely under the transport; such a
      // capture is unmeasured, and the fixture's full-size captures must be.
      const closer = !colour || colour.native!.pixels === 0 || (colour.native!.max <= colour.legacy!.max && colour.native!.mean <= colour.legacy!.mean
        && precise);
      if (colour?.native!.pixels) measured.set(fixture.name, (measured.get(fixture.name) ?? 0) + colour.native!.pixels);
      const pass = delta.sameSize && (playsOnly || (referenceGated ? closer : delta.max === 0)) && geometryEqual && pngColorEqual;
      comparisons.push({ key, ...delta, referenceGated, colour, geometryEqual, pngColorEqual,
        legacy: { ...reference, covered: reference.covered.length, file: path.basename(reference.file) },
        native: { ...actual, covered: actual.covered.length, file: path.basename(actual.file) } });
      if (!pass) {
        failed = true;
        if (!evidence) {
          await copyFile(reference.file, info.outputPath(`${key}-legacy.png`));
          await copyFile(actual.file, info.outputPath(`${key}-native.png`));
        }
      }
      expect.soft(delta.sameSize, key).toBe(true);
      if (playsOnly) { /* It played: no case error and a same-size capture. */ }
      else if (referenceGated) expect.soft(closer, `${key} reference closeness: ${JSON.stringify(colour)}`).toBe(true);
      else expect.soft(delta.max, key).toBe(0);
      expect.soft(geometryEqual, `${key} geometry`).toBe(true);
      expect.soft(pngColorEqual, `${key} PNG colour metadata`).toBe(true);
    }
    for (const name of Object.keys(REFERENCE).filter(name => REFERENCE[name]!.tags))
      expect.soft(measured.get(name) ?? 0, `${name}: reference-measured pixels`).toBeGreaterThanOrEqual(1000);
    for (const failure of caseErrors) expect.soft(false, `${failure.mode} ${failure.key}: ${failure.error}`).toBe(true);
    const report = { ...metadata, caseErrors, comparisons };
    if (evidence) await writeFile(path.join(captures, 'comparison.json'), JSON.stringify(report, null, 2));
    if (failed) await writeFile(info.outputPath('comparison.json'), JSON.stringify(report, null, 2));
  } finally {
    await rm(scratch, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
    if (!evidence && captures) await rm(captures, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
});
