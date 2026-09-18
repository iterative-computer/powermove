import { execFile } from 'node:child_process';
import { mkdtemp, open, realpath, rm, stat, symlink, writeFile } from 'node:fs/promises';
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

const execFileAsync = promisify(execFile);
const STILL_TIMEOUT_MS = 2 * 60 * 1000;
const TOKEN = /^[a-f0-9]{32}$/;
const VIDEO_EXTENSIONS = new Set([...NATIVE_VIDEO_EXTENSIONS, ...CONVERTED_VIDEO_EXTENSIONS].map(extension => `.${extension}`));
/** A decoded animation frame stays well under this even at 4K. */
const MAX_ANIMATION_FRAME_BYTES = 64 * 1024 * 1024;
const MAX_STILL_IMAGE_BYTES = 512 * 1024 * 1024;
const TRANSCODE_TIMEOUT_MS = 60 * 60 * 1000;
export const MAX_PROXY_CHUNK_BYTES = 4 * 1024 * 1024;

type ProxyEntry = {
  directory: string; file: string; size: number;
  upload?: { total: number; received: number; queue: Promise<void>; finishing: boolean };
  animation?: { fps: number; repeats: number[]; written: number[]; queue: Promise<void>; finishing: boolean };
};
type Convert = (source: string, output: string) => Promise<void>;

export function playbackConverter(binary: string): Convert {
  return async (source, output) => {
    // Chromium decodes VP9 alpha; H.264 proxies flatten ProRes 4444 cutouts.
    // Keep full dimensions, timing, and optional audio, with bounded workers.
    await execFileAsync(binary, [
      '-hide_banner', '-loglevel', 'error', '-nostdin', '-i', source,
      '-map', '0:v:0', '-map', '0:a:0?',
      '-c:v', 'libvpx-vp9', '-pix_fmt', 'yuva420p', '-b:v', '0', '-crf', '18',
      // Bound random seeks to half a second of decode at 30 fps. Long GOPs
      // make timeline scrubbing and frame-by-frame exports decode seconds repeatedly.
      '-g', '15', '-deadline', 'good', '-cpu-used', '4', '-row-mt', '1', '-threads', '4',
      '-c:a', 'libopus', '-b:a', '192k', '-f', 'webm', '-y', output
    ], { timeout: TRANSCODE_TIMEOUT_MS, maxBuffer: 1024 * 1024 });
  };
}

/** All-intra preview frames avoid decoding a long 4K GOP for every arrow press. */
export function previewConverter(binary: string): Convert {
  return async (source, output) => {
    await execFileAsync(binary, [
      '-hide_banner', '-loglevel', 'error', '-nostdin',
      // libvpx preserves WebM alpha; the native VP9 decoder discards it.
      ...(path.extname(source) === '.webm' ? ['-c:v', 'libvpx-vp9'] : []),
      '-i', source, '-map', '0:v:0', '-an',
      '-vf', "scale=w='min(1280,iw)':h='min(1280,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2",
      '-c:v', 'libvpx-vp9', '-pix_fmt', 'yuva420p', '-b:v', '0', '-crf', '18',
      '-g', '1', '-deadline', 'realtime', '-cpu-used', '6', '-row-mt', '1', '-threads', '4',
      '-f', 'webm', '-y', output,
    ], { timeout: TRANSCODE_TIMEOUT_MS, maxBuffer: 1024 * 1024 });
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
    const conversion = execFileAsync(binary, [
      '-hide_banner', '-loglevel', 'error', '-nostdin',
      '-progress', 'pipe:1', '-nostats', '-stats_period', '1',
      '-framerate', String(fps), '-start_number', '0', '-i', pattern,
      '-frames:v', String(count), '-an', '-c:v', 'libvpx-vp9',
      '-pix_fmt', 'yuva420p', '-lossless', '1', '-b:v', '0',
      '-g', '15', '-deadline', 'good', '-cpu-used', '4', '-row-mt', '1', '-threads', '4',
      '-f', 'webm', '-y', output,
    ], { timeout: TRANSCODE_TIMEOUT_MS, maxBuffer: 1024 * 1024 });
    conversion.child.stdout?.on('data', sequenceProgressReader(count, onProgress));
    await conversion;
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

  constructor(
    private readonly tempRoot: string,
    private readonly convert: Convert,
    private readonly convertSequence?: ConvertSequence,
    private readonly convertPreview?: Convert,
    private readonly convertStill?: ConvertStill
  ) {}

  async beginPreview(total: number): Promise<string> {
    if (!this.convertPreview || !Number.isSafeInteger(total) || total < 1) throw new Error('Invalid preview source size');
    const directory = await mkdtemp(path.join(this.tempRoot, 'powermove-preview-'));
    const token = randomUUID().replaceAll('-', '');
    this.#entries.set(token, { directory, file: path.join(directory, 'source'), size: 0,
      upload: { total, received: 0, queue: Promise.resolve(), finishing: false } });
    return token;
  }

  async writePreview(token: string, offset: number, data: Uint8Array): Promise<void> {
    const entry = this.#entries.get(token), upload = entry?.upload;
    if (!upload || upload.finishing || offset !== upload.received || !(data instanceof Uint8Array)
      || data.length < 1 || data.length > MAX_PROXY_CHUNK_BYTES || offset + data.length > upload.total) throw new Error('Invalid preview source chunk');
    upload.received += data.length;
    upload.queue = upload.queue.then(() => writeFile(entry!.file, data, { flag: 'a', mode: 0o600 }));
    await upload.queue;
  }

  async finishPreview(token: string): Promise<{ token: string; size: number }> {
    const entry = this.#entries.get(token), upload = entry?.upload;
    if (!entry || !upload || upload.finishing || upload.received !== upload.total) throw new Error('Incomplete preview source');
    upload.finishing = true;
    try {
      await upload.queue;
      // Detect the WebM container signature without trusting a user filename.
      const handle = await open(entry.file, 'r'), prefix = Buffer.alloc(4);
      try { await handle.read(prefix, 0, 4, 0); } finally { await handle.close(); }
      if (prefix.equals(Buffer.from([0x1a, 0x45, 0xdf, 0xa3]))) {
        const named = entry.file + '.webm';
        await symlink(entry.file, named); entry.file = named;
      }
      const output = path.join(entry.directory, 'preview.webm');
      await this.convertPreview!(entry.file, output);
      const info = await stat(output);
      if (!info.isFile() || info.size < 1) throw new Error('The preview was empty');
      entry.file = output; entry.size = info.size; delete entry.upload;
      return { token, size: info.size };
    } catch (error) { await this.release(token); throw error; }
  }

  async create(sourcePath: string): Promise<{ token: string; size: number }> {
    if (!path.isAbsolute(sourcePath) || !VIDEO_EXTENSIONS.has(path.extname(sourcePath).toLowerCase())) {
      throw new Error('This video container is not one Powermove can convert');
    }
    const resolved = await realpath(sourcePath);
    const source = await stat(resolved);
    if (!source.isFile() || source.size <= 0) {
      throw new Error('The selected video file is not readable');
    }

    const directory = await mkdtemp(path.join(this.tempRoot, 'powermove-media-proxy-'));
    const output = path.join(directory, 'playback.webm');
    try {
      await this.convert(resolved, output);
      const converted = await stat(output);
      if (!converted.isFile() || converted.size <= 0) throw new Error('The playback proxy was empty');
      const token = randomUUID().replaceAll('-', '');
      this.#entries.set(token, { directory, file: output, size: converted.size });
      return { token, size: converted.size };
    } catch (error) {
      await rm(directory, { recursive: true, force: true });
      throw error;
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
      const token = randomUUID().replaceAll('-', '');
      this.#entries.set(token, { directory, file: output, size: converted.size });
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
    const token = randomUUID().replaceAll('-', '');
    this.#entries.set(token, { directory, file: '', size: 0,
      animation: { fps, repeats, written: repeats.map(() => 0), queue: Promise.resolve(), finishing: false } });
    return token;
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
    try {
      await animation.queue;
      // Repeating a frame holds it on screen for its own delay at a fixed rate.
      let slot = 0;
      for (let index = 0; index < animation.repeats.length; index++) {
        const source = path.join(entry.directory, `source-${String(index).padStart(8, '0')}.png`);
        for (let repeat = 0; repeat < animation.repeats[index]!; repeat++) {
          await symlink(source, path.join(entry.directory, `frame-${String(slot++).padStart(8, '0')}.png`));
        }
      }
      const output = path.join(entry.directory, 'animation.webm');
      await this.convertSequence!(path.join(entry.directory, 'frame-%08d.png'), animation.fps, slot, output, onProgress);
      const converted = await stat(output);
      if (!converted.isFile() || converted.size < 1) throw new Error('The converted animation was empty');
      entry.file = output; entry.size = converted.size; delete entry.animation;
      return { token, size: converted.size };
    } catch (error) { await this.release(token); throw error; }
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
      const token = randomUUID().replaceAll('-', '');
      this.#entries.set(token, { directory, file: output, size: converted.size });
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
    const buffer = Buffer.allocUnsafe(bytesToRead);
    const handle = await open(entry.file, 'r');
    try {
      const { bytesRead } = await handle.read(buffer, 0, bytesToRead, offset);
      return new Uint8Array(buffer.buffer, buffer.byteOffset, bytesRead);
    } finally {
      await handle.close();
    }
  }

  async release(token: string): Promise<void> {
    if (!TOKEN.test(token)) return;
    const entry = this.#entries.get(token);
    if (!entry) return;
    this.#entries.delete(token);
    await entry.upload?.queue.catch(() => undefined);
    await entry.animation?.queue.catch(() => undefined);
    await rm(entry.directory, { recursive: true, force: true });
  }

  async dispose(): Promise<void> {
    const tokens = [...this.#entries.keys()];
    await Promise.all(tokens.map(token => this.release(token)));
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
  ipcMain.handle(IPC.mediaPreviewBegin, async (event, size: number) => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    return service.beginPreview(size);
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
      const proxy = await service.create(request.sourcePath);
      return {
        ok: true,
        token: proxy.token,
        type: 'video/webm',
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
      const proxy = await service.createSequence(sourcePaths, fps, completed => {
        if (requestId && !event.sender.isDestroyed()) event.sender.send(IPC.mediaSequenceProgress, { requestId, completed });
      });
      return { ok: true, ...proxy, type: 'video/webm' };
    } catch (error) {
      return { ok: false, error: error instanceof Error ? error.message : 'Could not import image sequence' };
    }
  });

  ipcMain.handle(IPC.mediaAnimationBegin, async (event, value: unknown) => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!value || typeof value !== 'object') throw new IpcValidationError(IPC.mediaAnimationBegin, 'expected an object');
    const { fps, repeats } = value as { fps: number; repeats: number[] };
    return service.beginAnimation(fps, repeats);
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
        if (requestId && !event.sender.isDestroyed()) event.sender.send(IPC.mediaAnimationProgress, { requestId, completed });
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
      const proxy = await service.createStillImage(request.sourcePath);
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
