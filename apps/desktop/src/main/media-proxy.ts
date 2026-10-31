import { execFile } from 'node:child_process';
import { existsSync } from 'node:fs';
import { copyFile, link, mkdtemp, open, realpath, rm, stat, symlink, writeFile } from 'node:fs/promises';
import type { FileHandle } from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';
import { randomUUID } from 'node:crypto';

import type { IpcMain, IpcMainInvokeEvent } from 'electron';

import {
  IPC,
  REQUEST_ID,
  type MediaProxyReadRequest,
  type MediaProxyRequest,
  type MediaProxyResult
} from '../shared/ipc';
import { IpcValidationError } from '../shared/guards';
import { orderedSequence, validSequenceFps } from '../shared/image-sequence';
import { MAX_SEQUENCE_FRAMES } from '../shared/animated-image';
import { CONVERTED_VIDEO_EXTENSIONS, NATIVE_VIDEO_EXTENSIONS, mediaExtension, needsImageConversion } from '../shared/media-formats';
import { videoColorTreatment, type VideoColor } from '../shared/video-color';
import { parseProbe, type ProbedVideoStream } from './agent-tools/media-ffmpeg';
import { spawnWithRetry } from './spawn-retry';
import { isRendererUnavailable } from './renderer-recovery';

const execFileAsync = promisify(execFile);
const STILL_TIMEOUT_MS = 2 * 60 * 1000;
const TOKEN = /^[a-f0-9]{32}$/;
const VIDEO_EXTENSIONS = new Set([...NATIVE_VIDEO_EXTENSIONS, ...CONVERTED_VIDEO_EXTENSIONS].map(extension => `.${extension}`));
/** A decoded animation frame stays well under this even at 4K. */
const MAX_ANIMATION_FRAME_BYTES = 64 * 1024 * 1024;
const MAX_STILL_IMAGE_BYTES = 512 * 1024 * 1024;
const TRANSCODE_TIMEOUT_MS = 60 * 60 * 1000;
const PROBE_TIMEOUT_MS = 30 * 1000;
export const MAX_PROXY_CHUNK_BYTES = 4 * 1024 * 1024;

type ProxyEntry = {
  directory: string; file: string; size: number;
  reader?: Promise<FileHandle>;
  reads?: Set<Promise<Uint8Array>>;
  finish?: Promise<{ token: string; size: number }>;
  upload?: { total: number; received: number; queue: Promise<void>; finishing: boolean; handle?: FileHandle; close?: Promise<void> };
  animation?: { fps: number; repeats: number[]; written: number[]; queue: Promise<void>; finishing: boolean };
};
type Convert = (source: string, output: string) => Promise<void>;
/** A playback converter may write an MP4 beside `output` instead of WebM. */
type PlaybackOutput = { file: string; type: 'video/mp4' };
type ConvertPlayback = (source: string, output: string, signal?: AbortSignal) => Promise<void | PlaybackOutput>;

/** The first video stream as FFmpeg reads it. Probing never blocks a
 * conversion: without an answer, encoding keeps its alpha-preserving defaults. */
export async function probeVideoStream(binary: string, source: string, decoder: string[] = []): Promise<ProbedVideoStream | undefined> {
  let banner = '';
  try {
    // Without an output FFmpeg prints the input and exits with an error.
    banner = String((await execFileAsync(binary, ['-hide_banner', '-nostdin', ...decoder, '-i', source],
      { timeout: PROBE_TIMEOUT_MS, maxBuffer: 4 * 1024 * 1024 })).stderr);
  } catch (error) { banner = String((error as { stderr?: unknown })?.stderr ?? ''); }
  return parseProbe(banner).video[0];
}

/* HLG and PQ are linearized at the BT.2408 reference white (203 cd/m²), so
   diffuse content keeps its SDR level. A Möbius shoulder above 90% of white
   rolls highlights into range up to a 1000 cd/m² peak instead of clipping.
   zimg performs every matrix step: swscale would silently keep BT.2020
   coefficients when converting between two YUV formats. */
const tonemapFilter = (alpha: boolean) => `zscale=t=linear:npl=203:p=bt709,format=${alpha ? 'gbrapf32le' : 'gbrpf32le'},`
  + 'tonemap=tonemap=mobius:param=0.9:peak=4.926:desat=0,zscale=t=bt709:m=bt709:r=tv';
const REC709_TAGS = ['-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', '-color_range', 'tv'];

/** Filters and output tags that settle tagged HDR or wide-gamut color. */
export function proxyColorArgs(color: VideoColor | undefined, pixelFormat: string): string[] {
  const treatment = videoColorTreatment(color);
  if (!treatment) return ['-pix_fmt', pixelFormat];
  // A log curve is not a transfer to undo: keep R'G'B' and change only the matrix.
  const conversion = treatment === 'tonemap' ? tonemapFilter(pixelFormat.startsWith('yuva')) : 'zscale=m=bt709:r=tv';
  // Format the last step explicitly so zimg, not swscale, writes the new matrix.
  return ['-vf', `${conversion},format=${pixelFormat}`, ...REC709_TAGS];
}

/** An alpha plane is a second VP9 stream, and Chromium decodes alpha VP9 in
 * software. Opaque footage must not pay for one. Unknown sources keep alpha. */
function proxyPixelFormat(stream: ProbedVideoStream | undefined): string {
  return !stream || stream.alpha ? 'yuva420p' : 'yuv420p';
}

/** Realtime libvpx matches the quality of `good` at CRF 18 within a few
 * hundredths of a dB here, while encoding 4K camera footage about ten times faster. */
const VP9_SPEED = ['-deadline', 'realtime', '-cpu-used', '8', '-row-mt', '1', '-threads', '4'];

export function playbackProxyArgs(source: string, output: string, stream: ProbedVideoStream | undefined): string[] {
  // Chromium decodes VP9 alpha; H.264 proxies flatten ProRes 4444 cutouts.
  // Keep full dimensions, timing, and optional audio, with bounded workers.
  return [
    '-hide_banner', '-loglevel', 'error', '-nostdin', '-i', source,
    '-map', '0:v:0', '-map', '0:a:0?',
    ...proxyColorArgs(stream?.color, proxyPixelFormat(stream)),
    '-c:v', 'libvpx-vp9', '-b:v', '0', '-crf', '18',
    // Bound random seeks to half a second of decode at 30 fps. Long GOPs
    // make timeline scrubbing and frame-by-frame exports decode seconds repeatedly.
    '-g', '15', ...VP9_SPEED,
    '-c:a', 'libopus', '-b:a', '192k', '-f', 'webm', '-y', output
  ];
}

/** The packaged helper sits beside FFmpeg; development uses the node-gyp build. */
export function hdrProxyHelper(appPath: string, resourcesPath?: string): string | undefined {
  if (process.platform !== 'darwin') return undefined;
  const helper = resourcesPath
    ? path.join(resourcesPath, 'encoder', 'powermove-hdr-proxy')
    : path.join(appPath, 'node_modules', '@powermove', 'macos-haptics', 'build', 'Release', 'powermove-hdr-proxy');
  return existsSync(helper) ? helper : undefined;
}

/* FFmpeg's float tone map runs below real time for 4K on the CPU, so opaque
   HDR footage goes to powermove-hdr-proxy (packages/macos-haptics) when it is
   available: the media engine decodes and encodes HEVC and Metal applies the
   same curve, several times faster. Alpha needs VP9, and any failure of the
   helper falls back to FFmpeg. */
export function playbackConverter(binary: string, hdrProxy?: string): ConvertPlayback {
  return async (source, output, signal) => {
    // FFmpeg ignores SIGTERM mid-encode; a cancelled conversion must stop now.
    const cancel = signal ? { signal, killSignal: 'SIGKILL' as const } : {};
    const stream = await probeVideoStream(binary, source);
    if (hdrProxy && stream && !stream.alpha && videoColorTreatment(stream.color) === 'tonemap') {
      const file = path.join(path.dirname(output), 'playback.mp4');
      try {
        await execFileAsync(hdrProxy, [source, file], { timeout: TRANSCODE_TIMEOUT_MS, maxBuffer: 1024 * 1024, ...cancel });
        return { file, type: 'video/mp4' };
      } catch (error) {
        await rm(file, { force: true });
        signal?.throwIfAborted();
        console.warn('GPU HDR conversion failed; converting with FFmpeg.', error);
      }
    }
    await execFileAsync(binary, playbackProxyArgs(source, output, stream),
      { timeout: TRANSCODE_TIMEOUT_MS, maxBuffer: 1024 * 1024, ...cancel });
  };
}

const PREVIEW_SCALE = "scale=w='min(1280,iw)':h='min(1280,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2";

export function previewArgs(source: string, output: string, stream: ProbedVideoStream | undefined): string[] {
  return [
    '-hide_banner', '-loglevel', 'error', '-nostdin',
    // libvpx preserves WebM alpha; the native VP9 decoder discards it.
    ...(path.extname(source) === '.webm' ? ['-c:v', 'libvpx-vp9'] : []),
    '-i', source, '-map', '0:v:0', '-an',
    // The preview stands in for the element it is built from, which Chromium
    // color-manages itself. Its tags carry through unchanged.
    '-vf', `${PREVIEW_SCALE},format=${proxyPixelFormat(stream)}`,
    '-c:v', 'libvpx-vp9', '-b:v', '0', '-crf', '18',
    '-g', '1', '-deadline', 'realtime', '-cpu-used', '6', '-row-mt', '1', '-threads', '4',
    '-f', 'webm', '-y', output,
  ];
}

/** All-intra preview frames avoid decoding a long 4K GOP for every arrow press. */
export function previewConverter(binary: string): Convert {
  return async (source, output) => {
    const decoder = path.extname(source) === '.webm' ? ['-c:v', 'libvpx-vp9'] : [];
    await execFileAsync(binary, previewArgs(source, output, await probeVideoStream(binary, source, decoder)),
      { timeout: TRANSCODE_TIMEOUT_MS, maxBuffer: 1024 * 1024 });
  };
}

type ConvertSequence = (pattern: string, fps: number, count: number, output: string, onProgress?: (completed: number) => void) => Promise<void>;

/** FFmpeg can split its line-oriented progress records across stdout chunks. */
export function sequenceProgressReader(count: number, onProgress?: (completed: number) => void) {
  let pending = '', previous = -1;
  return (chunk: string | Buffer) => {
    pending += chunk.toString();
    const lines = pending.split('\n');
    pending = lines.pop()!;
    for (const line of lines) {
      const match = /^frame=\s*(\d+)\s*$/.exec(line);
      if (!match) continue;
      const completed = Math.min(count, Number(match[1]));
      if (completed <= previous) continue;
      previous = completed;
      onProgress?.(completed);
    }
  };
}

export function imageSequenceConverter(binary: string): ConvertSequence {
  return async (pattern, fps, count, output, onProgress) => {
    const child = await spawnWithRetry(binary, [
      '-hide_banner', '-loglevel', 'error', '-nostdin',
      '-progress', 'pipe:1', '-nostats', '-stats_period', '1',
      '-framerate', String(fps), '-start_number', '0', '-i', pattern,
      '-frames:v', String(count), '-an', '-c:v', 'libvpx-vp9',
      '-pix_fmt', 'yuva420p', '-lossless', '1', '-b:v', '0',
      '-g', '15', '-deadline', 'good', '-cpu-used', '4', '-row-mt', '1', '-threads', '4',
      '-f', 'webm', '-y', output,
    ], { stdio: ['ignore', 'pipe', 'pipe'] }, 'image sequence encoder');
    child.stdout?.on('data', sequenceProgressReader(count, onProgress));
    await new Promise<void>((resolve, reject) => {
      let stderr = '';
      let timedOut = false;
      child.stderr?.on('data', chunk => { stderr = `${stderr}${String(chunk)}`.slice(-1024 * 1024); });
      const timer = setTimeout(() => { timedOut = true; child.kill('SIGKILL'); }, TRANSCODE_TIMEOUT_MS);
      timer.unref();
      child.once('error', error => { clearTimeout(timer); reject(error); });
      child.once('close', code => {
        clearTimeout(timer);
        if (code === 0) resolve();
        else reject(new Error(timedOut ? 'Image sequence conversion timed out.' : stderr.trim() || `Image sequence encoder stopped with code ${code ?? 'unknown'}.`));
      });
    });
  };
}

type ConvertStill = (source: string, extension: string, output: string) => Promise<void>;

/** Chromium decodes neither TIFF nor HEIF, so those stills become PNG on import. */
export function stillImageConverter(binary: string): ConvertStill {
  return async (source, extension, output) => {
    if (extension === 'heic' || extension === 'heif') {
      // FFmpeg 6 cannot open the HEIF variants Apple devices write; sips, which
      // ships with macOS, reads them through the same decoder Preview uses.
      await execFileAsync('/usr/bin/sips', ['-s', 'format', 'png', source, '--out', output],
        { timeout: STILL_TIMEOUT_MS, maxBuffer: 1024 * 1024 });
      return;
    }
    await execFileAsync(binary, [
      '-hide_banner', '-loglevel', 'error', '-nostdin',
      '-i', source, '-frames:v', '1', '-update', '1', '-c:v', 'png', '-f', 'image2', '-y', output,
    ], { timeout: STILL_TIMEOUT_MS, maxBuffer: 1024 * 1024 });
  };
}

export class MediaProxyService {
  readonly #entries = new Map<string, ProxyEntry>();
  readonly #releases = new Map<string, Promise<void>>();
  readonly #releaseListeners = new Set<(token: string) => void>();
  /** One conversion per unchanged source. Reopening a project while its
   * clips are still converting joins that work instead of starting more. */
  readonly #conversions = new Map<string, { waiters: number; controller: AbortController; done: Promise<{ directory: string; file: string; size: number; type: 'video/webm' | 'video/mp4' }> }>();
  #disposed = false;

  constructor(
    private readonly tempRoot: string,
    private readonly convert: ConvertPlayback,
    private readonly convertSequence?: ConvertSequence,
    private readonly convertPreview?: Convert,
    private readonly convertStill?: ConvertStill
  ) {}

  #add(entry: ProxyEntry): string {
    if (this.#disposed) throw new Error('The playback proxy service has closed');
    const token = randomUUID().replaceAll('-', '');
    this.#entries.set(token, entry);
    return token;
  }

  #active(token: string, entry: ProxyEntry): void {
    if (this.#entries.get(token) !== entry) throw new Error('The playback proxy has expired');
  }

  #reader(entry: ProxyEntry): Promise<FileHandle> {
    if (!entry.reader) {
      const pending = open(entry.file, 'r').catch(error => {
        // A failed open must not permanently poison an otherwise valid token.
        if (entry.reader === pending) delete entry.reader;
        throw error;
      });
      entry.reader = pending;
    }
    return entry.reader;
  }

  #closeUpload(upload: NonNullable<ProxyEntry['upload']>): Promise<void> {
    return upload.close ??= upload.handle?.close() ?? Promise.resolve();
  }

  onRelease(listener: (token: string) => void): void {
    this.#releaseListeners.add(listener);
  }

  has(token: string): boolean { return this.#entries.has(token); }

  async beginPreview(total: number): Promise<string> {
    if (!this.convertPreview || !Number.isSafeInteger(total) || total < 1) throw new Error('Invalid preview source size');
    const directory = await mkdtemp(path.join(this.tempRoot, 'powermove-preview-'));
    try {
      return this.#add({ directory, file: path.join(directory, 'source'), size: 0,
        upload: { total, received: 0, queue: Promise.resolve(), finishing: false } });
    } catch (error) { await rm(directory, { recursive: true, force: true }); throw error; }
  }

  async writePreview(token: string, offset: number, data: Uint8Array): Promise<void> {
    const entry = this.#entries.get(token), upload = entry?.upload;
    if (!upload || upload.finishing || offset !== upload.received || !(data instanceof Uint8Array)
      || data.length < 1 || data.length > MAX_PROXY_CHUNK_BYTES || offset + data.length > upload.total) throw new Error('Invalid preview source chunk');
    upload.received += data.length;
    upload.queue = upload.queue.then(async () => {
      const handle = upload.handle ??= await open(entry!.file, 'a', 0o600);
      await handle.writeFile(data);
    });
    try { await upload.queue; }
    catch (error) { await this.release(token); throw error; }
  }

  async finishPreview(token: string): Promise<{ token: string; size: number }> {
    const entry = this.#entries.get(token), upload = entry?.upload;
    if (!entry || !upload || upload.finishing || upload.received !== upload.total) throw new Error('Incomplete preview source');
    upload.finishing = true;
    const finish = (async () => {
      await upload.queue;
      await this.#closeUpload(upload);
      this.#active(token, entry);
      // Detect the WebM container signature without trusting a user filename.
      const handle = await open(entry.file, 'r'), prefix = Buffer.alloc(4);
      try { await handle.read(prefix, 0, 4, 0); } finally { await handle.close(); }
      if (prefix.equals(Buffer.from([0x1a, 0x45, 0xdf, 0xa3]))) {
        const named = entry.file + '.webm';
        await symlink(entry.file, named); entry.file = named;
      }
      const output = path.join(entry.directory, 'preview.webm');
      this.#active(token, entry);
      await this.convertPreview!(entry.file, output);
      this.#active(token, entry);
      const info = await stat(output);
      if (!info.isFile() || info.size < 1) throw new Error('The preview was empty');
      this.#active(token, entry);
      entry.file = output; entry.size = info.size; delete entry.upload;
      return { token, size: info.size };
    })();
    entry.finish = finish;
    try { return await finish; }
    catch (error) { await this.release(token); throw error; }
    finally { delete entry.finish; }
  }

  async create(sourcePath: string, signal?: AbortSignal): Promise<{ token: string; size: number; type: 'video/webm' | 'video/mp4' }> {
    if (!path.isAbsolute(sourcePath) || !VIDEO_EXTENSIONS.has(path.extname(sourcePath).toLowerCase())) {
      throw new Error('This video container is not one Powermove can convert');
    }
    const resolved = await realpath(sourcePath);
    const source = await stat(resolved);
    if (!source.isFile() || source.size <= 0) {
      throw new Error('The selected video file is not readable');
    }

    signal?.throwIfAborted();
    const key = `${resolved}\0${source.size}\0${source.mtimeMs}`;
    let conversion = this.#conversions.get(key);
    if (!conversion) {
      const controller = new AbortController();
      const done = (async () => {
        const directory = await mkdtemp(path.join(this.tempRoot, 'powermove-media-proxy-'));
        const output = path.join(directory, 'playback.webm');
        try {
          const written = await this.convert(resolved, output, controller.signal);
          const file = written && path.dirname(written.file) === directory ? written.file : output;
          const converted = await stat(file);
          if (!converted.isFile() || converted.size <= 0) throw new Error('The playback proxy was empty');
          return { directory, file, size: converted.size, type: written && file === written.file ? written.type : 'video/webm' as const };
        } catch (error) {
          await rm(directory, { recursive: true, force: true });
          throw error;
        }
      })();
      conversion = { waiters: 0, controller, done };
      this.#conversions.set(key, conversion);
    }
    const current = conversion;
    current.waiters++;
    let directory: string | undefined;
    try {
      // A caller whose window has gone stops waiting; the last one cancels the work.
      const shared = await (signal ? Promise.race([current.done, new Promise<never>((_, reject) => {
        if (signal.aborted) reject(signal.reason);
        signal.addEventListener('abort', () => reject(signal.reason), { once: true });
      })]) : current.done);
      // Each token owns its directory, so releasing one never removes another's file.
      directory = await mkdtemp(path.join(this.tempRoot, 'powermove-media-proxy-'));
      const file = path.join(directory, path.basename(shared.file));
      await link(shared.file, file).catch(() => copyFile(shared.file, file));
      const token = this.#add({ directory, file, size: shared.size });
      return { token, size: shared.size, type: shared.type };
    } catch (error) {
      if (directory) await rm(directory, { recursive: true, force: true });
      throw error;
    } finally {
      if (--current.waiters === 0) {
        if (this.#conversions.get(key) === current) this.#conversions.delete(key);
        current.controller.abort();
        await current.done.then(shared => rm(shared.directory, { recursive: true, force: true }), () => undefined);
      }
    }
  }

  async createSequence(sourcePaths: string[], fps: number, onProgress?: (completed: number) => void): Promise<{ token: string; size: number }> {
    if (!validSequenceFps(fps)) throw new Error('Frame rate must be between 1 and 240 fps');
    if (!Array.isArray(sourcePaths) || sourcePaths.length < 2
      || sourcePaths.some(source => typeof source !== 'string' || source.length > 16_384 || !path.isAbsolute(source))) {
      throw new Error('Choose local numbered image files');
    }
    if (!this.convertSequence) throw new Error('Image sequence conversion is unavailable');
    const frames = orderedSequence(sourcePaths.map(source => ({ name: path.basename(source), source })));
    const directory = await mkdtemp(path.join(this.tempRoot, 'powermove-image-sequence-'));
    const output = path.join(directory, 'sequence.webm');
    try {
      const extension = path.extname(frames[0]!.name).toLowerCase();
      // Only generated names enter FFmpeg's pattern; user filenames never become
      // demuxer directives. Symlinks avoid copying an entire sequence into RAM.
      for (let index = 0; index < frames.length; index++) {
        const source = await realpath(frames[index]!.source);
        const info = await stat(source);
        if (!info.isFile() || info.size <= 0) throw new Error(`Could not read ${frames[index]!.name}`);
        await symlink(source, path.join(directory, `frame-${String(index).padStart(8, '0')}${extension}`));
      }
      await this.convertSequence(path.join(directory, `frame-%08d${extension}`), fps, frames.length, output, onProgress);
      const converted = await stat(output);
      if (!converted.isFile() || converted.size <= 0) throw new Error('The image sequence was empty');
      const token = this.#add({ directory, file: output, size: converted.size });
      return { token, size: converted.size };
    } catch (error) {
      await rm(directory, { recursive: true, force: true });
      throw error;
    }
  }

  /* Animated GIF, APNG and animated WebP have no FFmpeg demuxer worth trusting,
     but Chromium decodes all of them. The renderer sends the frames it decoded,
     each with the number of sequence slots it has to fill, and they encode
     through exactly the same converter numbered image sequences already use. */
  async beginAnimation(fps: number, repeats: number[]): Promise<string> {
    if (!this.convertSequence) throw new Error('Animated image conversion is unavailable');
    if (!validSequenceFps(fps)) throw new Error('Frame rate must be between 1 and 240 fps');
    if (!Array.isArray(repeats) || repeats.length < 1
      || repeats.some(count => !Number.isSafeInteger(count) || count < 1 || count > MAX_SEQUENCE_FRAMES)) {
      throw new Error('Invalid animation frame timing');
    }
    const frames = repeats.reduce((total, count) => total + count, 0);
    if (frames > MAX_SEQUENCE_FRAMES) throw new Error('This animation has too many frames to import');
    const directory = await mkdtemp(path.join(this.tempRoot, 'powermove-animation-'));
    try {
      return this.#add({ directory, file: '', size: 0,
        animation: { fps, repeats, written: repeats.map(() => 0), queue: Promise.resolve(), finishing: false } });
    } catch (error) { await rm(directory, { recursive: true, force: true }); throw error; }
  }

  async writeAnimationFrame(token: string, index: number, offset: number, data: Uint8Array): Promise<void> {
    const entry = this.#entries.get(token), animation = entry?.animation;
    if (!animation || animation.finishing || !Number.isSafeInteger(index)
      || index < 0 || index >= animation.repeats.length
      || !(data instanceof Uint8Array) || data.length < 1 || data.length > MAX_PROXY_CHUNK_BYTES
      || offset !== animation.written[index]
      || offset + data.length > MAX_ANIMATION_FRAME_BYTES) throw new Error('Invalid animation frame chunk');
    animation.written[index] = offset + data.length;
    // Only generated names reach the filesystem; the decoded frame carries none.
    const file = path.join(entry!.directory, `source-${String(index).padStart(8, '0')}.png`);
    animation.queue = animation.queue.then(() => writeFile(file, data, { flag: offset ? 'a' : 'w', mode: 0o600 }));
    await animation.queue;
  }

  async finishAnimation(token: string, onProgress?: (completed: number) => void): Promise<{ token: string; size: number }> {
    const entry = this.#entries.get(token), animation = entry?.animation;
    if (!entry || !animation || animation.finishing) throw new Error('Unknown animation import');
    if (animation.written.some(bytes => bytes < 1)) throw new Error('The animation is missing frames');
    animation.finishing = true;
    const finish = (async () => {
      await animation.queue;
      this.#active(token, entry);
      // Repeating a frame holds it on screen for its own delay at a fixed rate.
      let slot = 0;
      for (let index = 0; index < animation.repeats.length; index++) {
        const source = path.join(entry.directory, `source-${String(index).padStart(8, '0')}.png`);
        for (let repeat = 0; repeat < animation.repeats[index]!; repeat++) {
          await symlink(source, path.join(entry.directory, `frame-${String(slot++).padStart(8, '0')}.png`));
        }
      }
      const output = path.join(entry.directory, 'animation.webm');
      this.#active(token, entry);
      await this.convertSequence!(path.join(entry.directory, 'frame-%08d.png'), animation.fps, slot, output, onProgress);
      this.#active(token, entry);
      const converted = await stat(output);
      if (!converted.isFile() || converted.size < 1) throw new Error('The converted animation was empty');
      this.#active(token, entry);
      entry.file = output; entry.size = converted.size; delete entry.animation;
      return { token, size: converted.size };
    })();
    entry.finish = finish;
    try { return await finish; }
    catch (error) { await this.release(token); throw error; }
    finally { delete entry.finish; }
  }

  /** TIFF and HEIF stills become a PNG the renderer can decode like any other. */
  async createStillImage(sourcePath: string): Promise<{ token: string; size: number }> {
    if (!this.convertStill) throw new Error('Image conversion is unavailable');
    const extension = mediaExtension(sourcePath);
    if (!path.isAbsolute(sourcePath) || !needsImageConversion(extension)) {
      throw new Error('This image format does not need converting');
    }
    const resolved = await realpath(sourcePath);
    const source = await stat(resolved);
    if (!source.isFile() || source.size <= 0) throw new Error('The selected image file is not readable');
    if (source.size > MAX_STILL_IMAGE_BYTES) throw new Error('This image file is too large to convert');
    const directory = await mkdtemp(path.join(this.tempRoot, 'powermove-image-'));
    const output = path.join(directory, 'image.png');
    try {
      // The requested name picks the decoder, never wherever a symlink points.
      await this.convertStill(resolved, extension, output);
      const converted = await stat(output);
      if (!converted.isFile() || converted.size <= 0) throw new Error('The converted image was empty');
      const token = this.#add({ directory, file: output, size: converted.size });
      return { token, size: converted.size };
    } catch (error) {
      await rm(directory, { recursive: true, force: true });
      throw error;
    }
  }

  async read(token: string, offset: number, length: number): Promise<Uint8Array> {
    if (!TOKEN.test(token)) throw new Error('Unknown playback proxy');
    const entry = this.#entries.get(token);
    if (!entry) throw new Error('The playback proxy has expired');
    if (!Number.isSafeInteger(offset) || offset < 0 || offset > entry.size
      || !Number.isSafeInteger(length) || length < 1 || length > MAX_PROXY_CHUNK_BYTES) {
      throw new Error('Invalid playback proxy range');
    }
    const bytesToRead = Math.min(length, entry.size - offset);
    if (bytesToRead <= 0) return new Uint8Array();
    const reading = (async () => {
      const handle = await this.#reader(entry);
      const buffer = new Uint8Array(bytesToRead);
      const { bytesRead } = await handle.read(buffer, 0, bytesToRead, offset);
      return bytesRead === bytesToRead ? buffer : buffer.slice(0, bytesRead);
    })();
    (entry.reads ??= new Set()).add(reading);
    try {
      return await reading;
    } finally { entry.reads.delete(reading); }
  }

  async release(token: string): Promise<void> {
    if (!TOKEN.test(token)) return;
    const pending = this.#releases.get(token);
    if (pending) return pending;
    const entry = this.#entries.get(token);
    if (!entry) return;
    this.#entries.delete(token);
    const releasing = (async () => {
      // Previously accepted requests finish before descriptors close or files
      // disappear. Deleting the token above blocks every new request immediately.
      await Promise.allSettled([
        entry.upload?.queue, entry.animation?.queue, entry.finish,
        ...(entry.reads ?? []),
      ]);
      try {
        await Promise.all([
          entry.reader?.then(handle => handle.close(), () => undefined),
          entry.upload && this.#closeUpload(entry.upload),
        ]);
      } finally { await rm(entry.directory, { recursive: true, force: true }); }
    })();
    this.#releases.set(token, releasing);
    let notificationFailed = false, notificationError: unknown;
    try {
      for (const listener of this.#releaseListeners) {
        try { listener(token); }
        catch (error) { notificationFailed = true; notificationError ??= error; }
      }
      await releasing;
      if (notificationFailed) throw notificationError;
    }
    finally { this.#releases.delete(token); }
  }

  async dispose(): Promise<void> {
    this.#disposed = true;
    // Conversions are child processes; quitting must not leave them running.
    for (const conversion of this.#conversions.values()) conversion.controller.abort();
    const tokens = [...this.#entries.keys()];
    await Promise.all([...this.#releases.values(), ...tokens.map(token => this.release(token))]);
    this.#releaseListeners.clear();
  }
}

function proxyRequest(value: unknown, channel: string = IPC.mediaProxyCreate): MediaProxyRequest {
  if (!value || typeof value !== 'object') {
    throw new IpcValidationError(channel, 'expected an object');
  }
  const sourcePath = (value as { sourcePath?: unknown }).sourcePath;
  const name = (value as { name?: unknown }).name;
  if (typeof sourcePath !== 'string' || sourcePath.length > 16_384 || !path.isAbsolute(sourcePath)) {
    throw new IpcValidationError(channel, 'invalid source path');
  }
  if (typeof name !== 'string' || name.length < 1 || name.length > 1_000) {
    throw new IpcValidationError(channel, 'invalid file name');
  }
  return { sourcePath, name };
}

export function registerMediaProxyIpc(
  ipcMain: IpcMain,
  service: MediaProxyService,
  { isTrustedSender }: { isTrustedSender(event: IpcMainInvokeEvent): boolean }
): void {
  type Owner = { sender: IpcMainInvokeEvent['sender']; tokens: Set<string>; destroyed: () => void };
  const owners = new WeakMap<IpcMainInvokeEvent['sender'], Owner>();
  const tokens = new Map<string, Owner>();
  const forget = (token: string) => {
    const owner = tokens.get(token);
    if (!owner) return;
    tokens.delete(token); owner.tokens.delete(token);
    if (owner.tokens.size === 0) {
      owner.sender.removeListener('destroyed', owner.destroyed);
      owner.sender.removeListener('render-process-gone', owner.destroyed);
      owners.delete(owner.sender);
    }
  };
  service.onRelease(forget);
  const own = async (event: IpcMainInvokeEvent, token: string): Promise<string> => {
    if (!service.has(token)) throw new Error('The playback proxy has expired');
    if (isRendererUnavailable(event.sender)) {
      await service.release(token);
      throw new Error('The media renderer has closed or crashed');
    }
    let owner = owners.get(event.sender);
    if (!owner) {
      owner = { sender: event.sender, tokens: new Set(), destroyed: () => {
        for (const token of [...owner!.tokens]) void service.release(token).catch(() => undefined);
      } };
      owners.set(event.sender, owner);
      event.sender.once('destroyed', owner.destroyed);
      event.sender.once('render-process-gone', owner.destroyed);
    }
    owner.tokens.add(token); tokens.set(token, owner);
    return token;
  };
  // Every in-flight request from one window shares a single pair of listeners.
  // Reopening a project while its clips convert must not stack one per request.
  const requestWatchers = new WeakMap<IpcMainInvokeEvent['sender'], { pending: Set<() => void>; lost: () => void }>();
  const watchRequest = (sender: IpcMainInvokeEvent['sender'], gone: () => void): (() => void) => {
    let watcher = requestWatchers.get(sender);
    if (!watcher) {
      const pending = new Set<() => void>();
      const created = { pending, lost: () => { for (const notify of [...pending]) notify(); } };
      requestWatchers.set(sender, created);
      sender.on('render-process-gone', created.lost);
      sender.on('destroyed', created.lost);
      watcher = created;
    }
    const current = watcher;
    current.pending.add(gone);
    return () => {
      current.pending.delete(gone);
      if (current.pending.size || requestWatchers.get(sender) !== current) return;
      requestWatchers.delete(sender);
      sender.removeListener('render-process-gone', current.lost);
      sender.removeListener('destroyed', current.lost);
    };
  };
  const createOwned = async <T extends { token: string }>(event: IpcMainInvokeEvent,
    create: (isCurrent: () => boolean, signal: AbortSignal) => Promise<T>): Promise<T> => {
    // A recovered WebContents is the same object, but its old request cannot
    // deliver tokens or progress to the replacement renderer.
    let rendererGone = false;
    const cancel = new AbortController();
    const gone = () => { rendererGone = true; cancel.abort(new Error('The media renderer has closed or crashed')); };
    const isCurrent = () => !rendererGone && !isRendererUnavailable(event.sender);
    const unwatch = watchRequest(event.sender, gone);
    try {
      const result = await create(isCurrent, cancel.signal);
      if (!isCurrent()) {
        await service.release(result.token);
        throw new Error('The media renderer has closed or crashed');
      }
      await own(event, result.token);
      return result;
    } finally { unwatch(); }
  };
  ipcMain.handle(IPC.mediaPreviewBegin, async (event, size: number) => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    return (await createOwned(event, async () => ({ token: await service.beginPreview(size) }))).token;
  });
  ipcMain.handle(IPC.mediaPreviewChunk, async (event, value: unknown) => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!value || typeof value !== 'object') throw new Error('Invalid preview source chunk');
    const { token, offset, data } = value as { token: string; offset: number; data: Uint8Array };
    return service.writePreview(token, offset, data);
  });
  ipcMain.handle(IPC.mediaPreviewFinish, async (event, token: string) => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    return service.finishPreview(token);
  });
  ipcMain.handle(IPC.mediaProxyCreate, async (event, value: unknown): Promise<MediaProxyResult> => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const request = proxyRequest(value);
    try {
      const proxy = await createOwned(event, (_isCurrent, signal) => service.create(request.sourcePath, signal));
      return {
        ok: true,
        token: proxy.token,
        type: proxy.type,
        size: proxy.size
      };
    } catch (error) {
      return {
        ok: false,
        error: error instanceof Error && error.message
          ? `Could not optimize this video for playback · ${error.message}`
          : 'Could not optimize this video for playback'
      };
    }
  });

  ipcMain.handle(IPC.mediaSequenceCreate, async (event, value: unknown): Promise<MediaProxyResult> => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!value || typeof value !== 'object') throw new IpcValidationError(IPC.mediaSequenceCreate, 'expected an object');
    const { sourcePaths, fps, requestId } = value as { sourcePaths: string[]; fps: number; requestId?: string };
    if (requestId !== undefined && (typeof requestId !== 'string' || !REQUEST_ID.test(requestId))) {
      throw new IpcValidationError(IPC.mediaSequenceCreate, 'invalid request id');
    }
    try {
      const proxy = await createOwned(event, isCurrent => service.createSequence(sourcePaths, fps, completed => {
        if (requestId && isCurrent()) event.sender.send(IPC.mediaSequenceProgress, { requestId, completed });
      }));
      return { ok: true, ...proxy, type: 'video/webm' };
    } catch (error) {
      return { ok: false, error: error instanceof Error ? error.message : 'Could not import image sequence' };
    }
  });

  ipcMain.handle(IPC.mediaAnimationBegin, async (event, value: unknown) => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!value || typeof value !== 'object') throw new IpcValidationError(IPC.mediaAnimationBegin, 'expected an object');
    const { fps, repeats } = value as { fps: number; repeats: number[] };
    return (await createOwned(event, async () => ({ token: await service.beginAnimation(fps, repeats) }))).token;
  });

  ipcMain.handle(IPC.mediaAnimationFrame, async (event, value: unknown) => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!value || typeof value !== 'object') throw new IpcValidationError(IPC.mediaAnimationFrame, 'expected an object');
    const { token, index, offset, data } = value as { token: string; index: number; offset: number; data: Uint8Array };
    if (typeof token !== 'string' || !TOKEN.test(token)) {
      throw new IpcValidationError(IPC.mediaAnimationFrame, 'invalid animation token');
    }
    return service.writeAnimationFrame(token, index, offset, data);
  });

  ipcMain.handle(IPC.mediaAnimationFinish, async (event, value: unknown): Promise<MediaProxyResult> => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!value || typeof value !== 'object') throw new IpcValidationError(IPC.mediaAnimationFinish, 'expected an object');
    const { token, requestId } = value as { token: string; requestId?: string };
    if (typeof token !== 'string' || !TOKEN.test(token)) {
      throw new IpcValidationError(IPC.mediaAnimationFinish, 'invalid animation token');
    }
    if (requestId !== undefined && (typeof requestId !== 'string' || !REQUEST_ID.test(requestId))) {
      throw new IpcValidationError(IPC.mediaAnimationFinish, 'invalid request id');
    }
    try {
      const proxy = await service.finishAnimation(token, completed => {
        if (requestId && service.has(token) && !isRendererUnavailable(event.sender)) event.sender.send(IPC.mediaAnimationProgress, { requestId, completed });
      });
      return { ok: true, ...proxy, type: 'video/webm' };
    } catch (error) {
      return { ok: false, error: error instanceof Error && error.message
        ? `Could not import this animation · ${error.message}`
        : 'Could not import this animation' };
    }
  });

  ipcMain.handle(IPC.mediaImageCreate, async (event, value: unknown): Promise<MediaProxyResult> => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const request = proxyRequest(value, IPC.mediaImageCreate);
    try {
      const proxy = await createOwned(event, () => service.createStillImage(request.sourcePath));
      return { ok: true, ...proxy, type: 'image/png' };
    } catch (error) {
      return { ok: false, error: error instanceof Error && error.message
        ? `Could not convert this image · ${error.message}`
        : 'Could not convert this image' };
    }
  });

  ipcMain.handle(IPC.mediaProxyRead, async (event, value: unknown): Promise<Uint8Array> => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!value || typeof value !== 'object') {
      throw new IpcValidationError(IPC.mediaProxyRead, 'expected an object');
    }
    const { token, offset, length } = value as Partial<MediaProxyReadRequest>;
    if (typeof token !== 'string' || !TOKEN.test(token)
      || !Number.isSafeInteger(offset) || !Number.isSafeInteger(length)) {
      throw new IpcValidationError(IPC.mediaProxyRead, 'invalid proxy range');
    }
    return service.read(token, offset!, length!);
  });

  ipcMain.handle(IPC.mediaProxyRelease, async (event, value: unknown): Promise<void> => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (typeof value !== 'string' || !TOKEN.test(value)) {
      throw new IpcValidationError(IPC.mediaProxyRelease, 'invalid proxy token');
    }
    await service.release(value);
  });
}
