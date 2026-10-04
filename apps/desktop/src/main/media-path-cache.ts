import { createHash, randomUUID } from 'node:crypto';
import { mkdir, open, realpath, rename, rm, stat, type FileHandle } from 'node:fs/promises';
import path from 'node:path';

import type { IpcMain, IpcMainInvokeEvent } from 'electron';

import {
  MEDIA_PATH_IPC,
  MEDIA_STAGE_CHUNK_BYTES,
  type MediaPathLookupRequest,
  type MediaPathResult,
  type MediaPathStageRequest
} from '../shared/media-tools';

/*
 * Files the agent's media tools read. The renderer owns a project's media
 * (IndexedDB blobs plus the path each asset was imported from); ffmpeg and
 * the transcription engine need a path. A lookup answers with the original
 * source when it is still on disk and still the same bytes (the import
 * fingerprint is checked, so a file replaced in place is never used), the
 * host's own copy on `powermove serve`, or a file this cache staged earlier.
 * Otherwise the renderer streams the stored bytes here once, in chunks.
 *
 * Staged files live under one directory main owns. Each is tagged with the
 * windows and projects that use it; a window closing its project, the
 * window going away, or the app quitting removes them.
 */

const SAMPLE_BYTES = 64 * 1024;
const MEDIA_KEY = /^media:[A-Za-z0-9:._-]{1,200}$/;
const ASSET_ID = /^[A-Za-z0-9_.:-]{1,200}$/;
const PROJECT = /^[A-Za-z0-9_-]{1,120}$/;
const FINGERPRINT = /^v2:(\d+):([a-f0-9]{64})$/;
const MAX_STAGE_BYTES = 64 * 1024 * 1024 * 1024;
const EXTENSIONS: Record<string, string> = {
  'video/webm': 'webm', 'audio/webm': 'webm', 'video/mp4': 'mp4', 'audio/mp4': 'm4a', 'video/quicktime': 'mov',
  'audio/mpeg': 'mp3', 'audio/wav': 'wav', 'audio/x-wav': 'wav', 'audio/wave': 'wav', 'audio/ogg': 'ogg',
  'audio/flac': 'flac', 'audio/aac': 'aac', 'video/x-matroska': 'mkv', 'audio/opus': 'opus'
};

/** The renderer's import fingerprint (legacy/core/media.ts), computed from disk. */
export async function mediaFingerprint(file: string): Promise<string> {
  const handle = await open(file, 'r');
  try {
    const size = (await handle.stat()).size;
    const sample = Math.min(SAMPLE_BYTES, size);
    const offsets = [...new Set([0, Math.max(0, Math.floor((size - sample) / 2)), Math.max(0, size - sample)])];
    const hash = createHash('sha256').update(`powermove-media-v2\n${size}\n`);
    for (const offset of offsets) {
      const buffer = Buffer.alloc(sample);
      const { bytesRead } = await handle.read(buffer, 0, sample, offset);
      hash.update(buffer.subarray(0, bytesRead));
    }
    return `v2:${size}:${hash.digest('hex')}`;
  } finally { await handle.close(); }
}

function extensionFor(name: string, type?: string): string {
  const fromType = type ? EXTENSIONS[type.toLowerCase().split(';')[0]!.trim()] : undefined;
  if (fromType) return fromType;
  const fromName = path.extname(name).slice(1).toLowerCase();
  return /^[a-z0-9]{1,8}$/.test(fromName) ? fromName : 'bin';
}

interface CacheEntry { file: string; size: number; tags: Set<string> }
interface Staging { owner: number; tag: string; key: string; handle: FileHandle; file: string; size: number; received: number; queue: Promise<void> }

export interface MediaPathCacheOptions {
  /** False when reading the file would download it from a cloud provider. */
  isLocal?(file: string): Promise<boolean>;
  /** `powermove serve`: the host's content-addressed copies, by store key. */
  mediaStoreDir?: string;
}

export class MediaPathCache {
  private readonly entries = new Map<string, CacheEntry>();
  private readonly staging = new Map<string, Staging>();
  private ready: Promise<void> | null = null;

  constructor(private readonly root: string, private readonly options: MediaPathCacheOptions = {}) {}

  /** Clear whatever an earlier session left behind (a crash skips the quit cleanup). */
  private prepare(): Promise<void> {
    this.ready ??= rm(this.root, { recursive: true, force: true }).then(() => mkdir(this.root, { recursive: true, mode: 0o700 })).then(() => undefined);
    return this.ready;
  }

  async lookup(owner: number, request: MediaPathLookupRequest): Promise<MediaPathResult | null> {
    const source = await this.verifiedSource(request);
    if (source) return { path: source, origin: 'source' };
    const hosted = await this.hostCopy(request.storageKey);
    if (hosted) return { path: hosted, origin: 'host' };
    if (request.size !== undefined) {
      const entry = this.entries.get(cacheKey(request.assetId, request.storageKey, request.size));
      if (entry && await exists(entry.file)) {
        entry.tags.add(tagOf(owner, request.projectId));
        return { path: entry.file, origin: 'cache' };
      }
    }
    return null;
  }

  private async verifiedSource(request: MediaPathLookupRequest): Promise<string | null> {
    const { sourcePath, fingerprint } = request;
    if (!sourcePath || !fingerprint || !path.isAbsolute(sourcePath)) return null;
    const expected = FINGERPRINT.exec(fingerprint);
    if (!expected) return null;
    try {
      const resolved = await realpath(sourcePath);
      const info = await stat(resolved);
      if (!info.isFile() || info.size !== Number(expected[1])) return null;
      if (this.options.isLocal && !await this.options.isLocal(resolved)) return null;
      return await mediaFingerprint(resolved) === fingerprint ? resolved : null;
    } catch { return null; }
  }

  private async hostCopy(storageKey?: string): Promise<string | null> {
    if (!this.options.mediaStoreDir || !storageKey || !MEDIA_KEY.test(storageKey)) return null;
    const file = path.join(this.options.mediaStoreDir, storageKey.replace(/[^A-Za-z0-9._-]/g, '_'));
    return await exists(file) ? file : null;
  }

  async begin(owner: number, request: MediaPathStageRequest): Promise<{ token: string } | MediaPathResult> {
    const known = await this.lookup(owner, { assetId: request.assetId, storageKey: request.storageKey, size: request.size, ...(request.projectId ? { projectId: request.projectId } : {}) });
    if (known) return known;
    await this.prepare();
    const key = cacheKey(request.assetId, request.storageKey, request.size);
    const file = path.join(this.root, `${key}.${extensionFor(request.name, request.type)}`);
    const partial = `${file}.${randomUUID()}.part`;
    const handle = await open(partial, 'wx', 0o600);
    const token = randomUUID().replaceAll('-', '');
    this.staging.set(token, { owner, tag: tagOf(owner, request.projectId), key, handle, file: partial, size: request.size, received: 0, queue: Promise.resolve() });
    return { token };
  }

  async chunk(owner: number, token: string, offset: number, data: Uint8Array): Promise<void> {
    const job = this.staging.get(token);
    if (!job || job.owner !== owner) throw new Error('Unknown media staging');
    if (!(data instanceof Uint8Array) || data.byteLength < 1 || data.byteLength > MEDIA_STAGE_CHUNK_BYTES
      || offset !== job.received || offset + data.byteLength > job.size) throw new Error('Invalid media staging chunk');
    job.received += data.byteLength;
    job.queue = job.queue.then(async () => { await job.handle.write(data, 0, data.byteLength, offset); });
    await job.queue;
  }

  async finish(owner: number, token: string): Promise<MediaPathResult> {
    const job = this.staging.get(token);
    if (!job || job.owner !== owner) throw new Error('Unknown media staging');
    this.staging.delete(token);
    try {
      await job.queue;
      await job.handle.close();
      if (job.received !== job.size) throw new Error('Incomplete media staging');
      const target = job.file.replace(/\.[a-f0-9-]+\.part$/, '');
      await rename(job.file, target);
      const entry = this.entries.get(job.key);
      if (entry && entry.file !== target) await rm(entry.file, { force: true });
      const tags = entry?.tags ?? new Set<string>();
      tags.add(job.tag);
      this.entries.set(job.key, { file: target, size: job.size, tags });
      return { path: target, origin: 'cache' };
    } catch (error) {
      await rm(job.file, { force: true });
      throw error;
    }
  }

  async abort(owner: number, token: string): Promise<void> {
    const job = this.staging.get(token);
    if (!job || job.owner !== owner) return;
    this.staging.delete(token);
    await job.queue.catch(() => undefined);
    await job.handle.close().catch(() => undefined);
    await rm(job.file, { force: true });
  }

  /** A window closed a project (`projectId`) or went away (none): drop files only it used. */
  async release(owner: number, projectId?: string): Promise<void> {
    const prefix = `${owner}:`;
    const tag = projectId === undefined ? null : tagOf(owner, projectId);
    for (const [key, entry] of [...this.entries]) {
      for (const candidate of [...entry.tags]) if (tag ? candidate === tag : candidate.startsWith(prefix)) entry.tags.delete(candidate);
      if (entry.tags.size) continue;
      this.entries.delete(key);
      await rm(entry.file, { force: true });
    }
    if (tag === null) for (const [token, job] of [...this.staging]) if (job.owner === owner) await this.abort(owner, token);
  }

  async dispose(): Promise<void> {
    for (const [token, job] of [...this.staging]) await this.abort(job.owner, token);
    this.entries.clear();
    await rm(this.root, { recursive: true, force: true });
  }
}

function tagOf(owner: number, projectId?: string): string {
  return `${owner}:${projectId ?? ''}`;
}

function cacheKey(assetId: string, storageKey: string | undefined, size: number): string {
  return createHash('sha256').update(`${assetId}\n${storageKey ?? ''}\n${size}`).digest('hex').slice(0, 40);
}

async function exists(file: string): Promise<boolean> {
  try { return (await stat(file)).isFile(); } catch { return false; }
}

/* ── IPC ─────────────────────────────────────────────────── */

function text(value: unknown, pattern: RegExp): string | undefined {
  return typeof value === 'string' && pattern.test(value) ? value : undefined;
}

export function parseLookupRequest(value: unknown): MediaPathLookupRequest {
  if (!value || typeof value !== 'object') throw new Error('media-path: expected an object');
  const input = value as Record<string, unknown>;
  const assetId = text(input.assetId, ASSET_ID);
  if (!assetId) throw new Error('media-path: invalid asset id');
  const sourcePath = typeof input.sourcePath === 'string' && input.sourcePath.length <= 16_384 && !input.sourcePath.includes('\0') && path.isAbsolute(input.sourcePath) ? input.sourcePath : undefined;
  const size = typeof input.size === 'number' && Number.isSafeInteger(input.size) && input.size > 0 ? input.size : undefined;
  return {
    assetId,
    ...(text(input.projectId, PROJECT) ? { projectId: input.projectId as string } : {}),
    ...(sourcePath ? { sourcePath } : {}),
    ...(text(input.fingerprint, FINGERPRINT) ? { fingerprint: input.fingerprint as string } : {}),
    ...(text(input.storageKey, MEDIA_KEY) ? { storageKey: input.storageKey as string } : {}),
    ...(size ? { size } : {})
  };
}

export function parseStageRequest(value: unknown): MediaPathStageRequest {
  const base = parseLookupRequest(value);
  const input = value as Record<string, unknown>;
  const size = input.size;
  if (typeof size !== 'number' || !Number.isSafeInteger(size) || size < 1 || size > MAX_STAGE_BYTES) throw new Error('media-path: invalid size');
  const name = typeof input.name === 'string' && input.name.length >= 1 && input.name.length <= 1000 ? input.name : 'media';
  const type = typeof input.type === 'string' && input.type.length <= 200 ? input.type : undefined;
  return {
    assetId: base.assetId, name, size,
    ...(base.projectId ? { projectId: base.projectId } : {}),
    ...(base.storageKey ? { storageKey: base.storageKey } : {}),
    ...(type ? { type } : {})
  };
}

interface Sender { id: number; once(event: 'destroyed', listener: () => void): unknown; isDestroyed(): boolean }

export function registerMediaPathIpc(
  ipcMain: Pick<IpcMain, 'handle'>,
  cache: MediaPathCache,
  { isTrustedSender }: { isTrustedSender(event: IpcMainInvokeEvent): boolean }
): void {
  const watched = new WeakSet<object>();
  const owner = (event: IpcMainInvokeEvent): number => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const sender = event.sender as unknown as Sender;
    if (!watched.has(sender)) {
      watched.add(sender);
      sender.once('destroyed', () => { void cache.release(sender.id).catch(() => undefined); });
    }
    return sender.id;
  };
  ipcMain.handle(MEDIA_PATH_IPC.lookup, (event, value: unknown) => cache.lookup(owner(event), parseLookupRequest(value)));
  ipcMain.handle(MEDIA_PATH_IPC.stageBegin, (event, value: unknown) => cache.begin(owner(event), parseStageRequest(value)));
  ipcMain.handle(MEDIA_PATH_IPC.stageChunk, (event, value: unknown) => {
    const id = owner(event);
    if (!value || typeof value !== 'object') throw new Error('Invalid media staging chunk');
    const { token, offset, data } = value as { token?: unknown; offset?: unknown; data?: unknown };
    if (typeof token !== 'string' || !Number.isSafeInteger(offset) || !(data instanceof Uint8Array)) throw new Error('Invalid media staging chunk');
    return cache.chunk(id, token, offset as number, data);
  });
  ipcMain.handle(MEDIA_PATH_IPC.stageFinish, (event, token: unknown) => {
    const id = owner(event);
    if (typeof token !== 'string') throw new Error('Unknown media staging');
    return cache.finish(id, token);
  });
  ipcMain.handle(MEDIA_PATH_IPC.stageAbort, (event, token: unknown) => {
    const id = owner(event);
    return typeof token === 'string' ? cache.abort(id, token) : undefined;
  });
  ipcMain.handle(MEDIA_PATH_IPC.release, (event, projectId: unknown) => {
    const id = owner(event);
    return cache.release(id, typeof projectId === 'string' && PROJECT.test(projectId) ? projectId : undefined);
  });
}
