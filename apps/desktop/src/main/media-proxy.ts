import { execFile } from 'node:child_process';
import { mkdtemp, open, realpath, rm, stat, symlink, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';
import { randomUUID } from 'node:crypto';

import type { IpcMain, IpcMainInvokeEvent } from 'electron';

import {
  IPC,
  type MediaProxyReadRequest,
  type MediaProxyRequest,
  type MediaProxyResult
} from '../shared/ipc';
import { IpcValidationError } from '../shared/guards';
import { orderedSequence, validSequenceFps } from '../shared/image-sequence';

const execFileAsync = promisify(execFile);
const TOKEN = /^[a-f0-9]{32}$/;
const VIDEO_EXTENSIONS = new Set(['.mov', '.mp4', '.m4v']);
const TRANSCODE_TIMEOUT_MS = 60 * 60 * 1000;
export const MAX_PROXY_CHUNK_BYTES = 4 * 1024 * 1024;

type ProxyEntry = { directory: string; file: string; size: number; upload?: { total: number; received: number; queue: Promise<void>; finishing: boolean } };
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

type ConvertSequence = (pattern: string, fps: number, count: number, output: string) => Promise<void>;

export function imageSequenceConverter(binary: string): ConvertSequence {
  return async (pattern, fps, count, output) => {
    await execFileAsync(binary, [
      '-hide_banner', '-loglevel', 'error', '-nostdin',
      '-framerate', String(fps), '-start_number', '0', '-i', pattern,
      '-frames:v', String(count), '-an', '-c:v', 'libvpx-vp9',
      '-pix_fmt', 'yuva420p', '-lossless', '1', '-b:v', '0',
      '-g', '15', '-deadline', 'good', '-cpu-used', '4', '-row-mt', '1', '-threads', '4',
      '-f', 'webm', '-y', output,
    ], { timeout: TRANSCODE_TIMEOUT_MS, maxBuffer: 1024 * 1024 });
  };
}

export class MediaProxyService {
  readonly #entries = new Map<string, ProxyEntry>();

  constructor(
    private readonly tempRoot: string,
    private readonly convert: Convert,
    private readonly convertSequence?: ConvertSequence,
    private readonly convertPreview?: Convert
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
      throw new Error('Only local MOV, MP4, and M4V video files can be optimized');
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

  async createSequence(sourcePaths: string[], fps: number): Promise<{ token: string; size: number }> {
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
      await this.convertSequence(path.join(directory, `frame-%08d${extension}`), fps, frames.length, output);
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
    await rm(entry.directory, { recursive: true, force: true });
  }

  async dispose(): Promise<void> {
    const tokens = [...this.#entries.keys()];
    await Promise.all(tokens.map(token => this.release(token)));
  }
}

function proxyRequest(value: unknown): MediaProxyRequest {
  if (!value || typeof value !== 'object') {
    throw new IpcValidationError(IPC.mediaProxyCreate, 'expected an object');
  }
  const sourcePath = (value as { sourcePath?: unknown }).sourcePath;
  const name = (value as { name?: unknown }).name;
  if (typeof sourcePath !== 'string' || sourcePath.length > 16_384 || !path.isAbsolute(sourcePath)) {
    throw new IpcValidationError(IPC.mediaProxyCreate, 'invalid source path');
  }
  if (typeof name !== 'string' || name.length < 1 || name.length > 1_000) {
    throw new IpcValidationError(IPC.mediaProxyCreate, 'invalid file name');
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
    const { sourcePaths, fps } = value as { sourcePaths: string[]; fps: number };
    try {
      const proxy = await service.createSequence(sourcePaths, fps);
      return { ok: true, ...proxy, type: 'video/webm' };
    } catch (error) {
      return { ok: false, error: error instanceof Error ? error.message : 'Could not import image sequence' };
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
