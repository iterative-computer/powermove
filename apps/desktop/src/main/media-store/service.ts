/*
 * The media store as main serves it: windows read and write project media
 * (store.ts), imports are classified (classify.ts), linked originals are
 * verified and relinked by content hash (relink.ts), and engines get the
 * file behind an asset only through a capability handle main mints
 * (`resolveAsset`, contract 14).
 */
import { mkdir, open, rm, symlink, type FileHandle } from 'node:fs/promises';
import { randomBytes } from 'node:crypto';
import path from 'node:path';

import { mayNeedVideoProxy, mediaExtension, needsImageConversion } from '../../shared/media-formats';

import type { ContentHash, EngineProcess } from '../../shared/engine-protocol';
import { HASH_HEX, validStorageKey, type ClassifyResult, type ConversionKind, type MediaBinding, type MediaConversion, type MediaLink, type MediaReadOpen } from '../../shared/media-store';
import { ImportClassifier, type ClassifyOptions } from './classify';
import { hashFile, type HashExecutor } from './hasher';
import { findByContent, type RelinkOptions } from './relink';
import { MediaStore, MediaStoreError, STREAM_CHUNK_BYTES } from './store';

type ReadSession = { owner: number; handle: FileHandle; size: number };

/** Main's still converter (`MediaProxyService.createStillImage`): a PNG the
 *  page reads through the playback proxy reader. */
export type ConvertStill = (sourcePath: string) => Promise<{ token: string; size: number }>;
/** Main's playback proxy converter (`MediaProxyService.create`): a VP9 WebM
 *  the page reads through the playback proxy reader. */
export type ConvertVideo = (sourcePath: string) => Promise<{ token: string; size: number }>;

export type MediaStoreServiceOptions = {
  root: string;
  /** Converts TIFF and HEIF stills for the page's viewer (section 21: the
   *  original is the asset's bytes; the PNG is generated media). */
  convertStill?: ConvertStill;
  /** Converts kept video originals the page's viewer cannot decode. */
  convertVideo?: ConvertVideo;
  userData: string;
  executor: HashExecutor;
  /** "Use New Engine" as read at launch. */
  enabled: boolean;
  classify?: Omit<ClassifyOptions, 'userData'>;
  relink?: Partial<Omit<RelinkOptions, 'hash'>>;
  now?: () => number;
};

export const MAX_LINK_PATH = 4096;

export function validLink(value: unknown): value is MediaLink {
  const link = value as MediaLink;
  return !!link && typeof link === 'object' && typeof link.path === 'string' && path.isAbsolute(link.path)
    && link.path.length <= MAX_LINK_PATH && !link.path.includes('\0') && HASH_HEX.test(link.hash)
    && Number.isSafeInteger(link.size) && link.size >= 0
    && (link.mtimeMs === undefined || Number.isFinite(link.mtimeMs))
    && (link.dev === undefined || Number.isSafeInteger(link.dev)) && (link.ino === undefined || Number.isSafeInteger(link.ino));
}

export const hashBytes = (hex: string): ContentHash => Uint8Array.from(Buffer.from(hex, 'hex'));

export class MediaStoreService {
  readonly store: MediaStore;
  readonly classifier: ImportClassifier;
  private readonly reads = new Map<string, ReadSession>();
  /** Per window: the open project's media (asset id → where its bytes are). */
  private readonly bindings = new Map<number, { projectId: string; assets: Map<string, MediaBinding> }>();

  constructor(private readonly options: MediaStoreServiceOptions) {
    this.store = new MediaStore({ root: options.root, executor: options.executor, ...(options.now ? { now: options.now } : {}) });
    this.classifier = new ImportClassifier({ userData: options.userData, ...options.classify });
  }

  get enabled(): boolean {
    return this.options.enabled;
  }

  classify(file: string): Promise<ClassifyResult> {
    return this.classifier.classify(file);
  }

  /** Link to files: the original's identity, after checking it may be linked. */
  async link(file: string): Promise<MediaLink> {
    const classified = await this.classify(file);
    if (classified.storage !== 'linked') throw new MediaStoreError(`This file is always copied into the project (${classified.reason}).`);
    const identity = await this.store.identify(file);
    return { path: file, hash: identity.hash, size: identity.size, mtimeMs: identity.mtimeMs, dev: identity.dev, ino: identity.ino };
  }

  /**
   * A linked original, checked: the recorded file when it is still the same
   * content, else the file with that content found nearby (`relinked`).
   */
  async locate(link: MediaLink, searchRoots: readonly string[] = []): Promise<{ path: string; relinked?: MediaLink } | { status: 'missing' | 'changed' }> {
    const checked = await this.store.verifyLink(link);
    if (checked.status === 'ok') return { path: checked.path };
    const found = await this.relink(link, searchRoots);
    return found ? { path: found.path, relinked: found } : { status: checked.status };
  }

  /** Content-hash relink (section 21). */
  async relink(link: MediaLink, searchRoots: readonly string[] = []): Promise<MediaLink | null> {
    const roots = new Set(searchRoots);
    // Media that moved together: folders where this window's other links are.
    for (const binding of this.bindings.values()) {
      for (const media of binding.assets.values()) if (media.link?.path) roots.add(path.dirname(media.link.path));
    }
    const found = await findByContent(link, [...roots], {
      hash: (file) => hashFile(this.options.executor, file, { settleMs: 0 }),
      // The app's own data (and the store's objects) never stand in for an original.
      exclude: [this.options.userData, this.options.root],
      ...this.options.relink
    });
    return found?.link ?? null;
  }

  /** Opens bytes for reading: a store key, a content hash, or a link. */
  async open(owner: number, request: { key?: string; hash?: string; link?: MediaLink; searchRoots?: string[] }): Promise<MediaReadOpen> {
    let file: string | null = null;
    let hash = '';
    let type = '';
    let relinked: MediaLink | undefined;
    if (request.link !== undefined) {
      if (!validLink(request.link)) throw new MediaStoreError('Invalid media link.');
      const located = await this.locate(request.link, request.searchRoots ?? []);
      if ('status' in located) return { ok: false, status: located.status === 'changed' ? 'changed' : 'offline' };
      file = located.path;
      hash = request.link.hash;
      relinked = located.relinked;
    } else if (request.key !== undefined) {
      if (!validStorageKey(request.key)) throw new MediaStoreError('Invalid media storage key.');
      const record = await this.store.record(request.key);
      if (!record) return { ok: false, status: 'missing' };
      file = this.store.objectPath(record.hash);
      hash = record.hash;
      type = record.type;
    } else if (request.hash !== undefined) {
      if (!HASH_HEX.test(request.hash)) throw new MediaStoreError('Invalid content hash.');
      if (!await this.store.hasObject(request.hash)) return { ok: false, status: 'missing' };
      file = this.store.objectPath(request.hash);
      hash = request.hash;
    } else throw new MediaStoreError('Name a key, a hash, or a link.');
    let handle: FileHandle;
    try { handle = await open(file, 'r'); } catch { return { ok: false, status: request.link ? 'offline' : 'missing' }; }
    const size = (await handle.stat()).size;
    const token = randomBytes(16).toString('hex');
    this.reads.set(token, { owner, handle, size });
    return { ok: true, token, size, type, hash, ...(request.link ? { path: file } : {}), ...(relinked ? { relinked } : {}) };
  }

  async read(owner: number, token: string, offset: number, length: number): Promise<Uint8Array> {
    const session = this.reads.get(token);
    if (!session || session.owner !== owner) throw new MediaStoreError('Unknown media read.', 'notFound');
    if (!Number.isSafeInteger(offset) || offset < 0 || offset > session.size || !Number.isSafeInteger(length) || length < 1 || length > STREAM_CHUNK_BYTES) {
      throw new MediaStoreError('Invalid media read.');
    }
    const want = Math.min(length, session.size - offset);
    const data = new Uint8Array(want);
    let filled = 0;
    while (filled < want) {
      const { bytesRead } = await session.handle.read(data, filled, want - filled, offset + filled);
      if (!bytesRead) throw new MediaStoreError('The media file ended early.', 'changed');
      filled += bytesRead;
    }
    return data;
  }

  async close(owner: number, token: string): Promise<void> {
    const session = this.reads.get(token);
    if (!session || session.owner !== owner) return;
    this.reads.delete(token);
    await session.handle.close().catch(() => undefined);
  }

  /**
   * The PNG the page's viewer shows for a TIFF or HEIF original that the
   * store keeps (or that is linked): converted from the stored object or
   * the verified original. `name` picks the decoder by its extension; the
   * converter reads the file through a link carrying that name, since store
   * objects are named by hash alone.
   */
  convertStill(request: { hash?: string; link?: MediaLink; name: string }): Promise<MediaConversion> {
    return this.convertOriginal('still', request);
  }

  /** The playback proxy the page's viewer plays for a kept video original
   *  it cannot decode (ProRes, MKV, AVI…), from the store or a link. */
  convertVideo(request: { hash?: string; link?: MediaLink; name: string }): Promise<MediaConversion> {
    return this.convertOriginal('video', request);
  }

  /**
   * Converts bytes the page uploaded (`beginStream`, then chunks) without
   * keeping them: with Use New Engine off, a kept original the store lacks
   * is only in IndexedDB (a project file opened on another Mac), and the
   * legacy viewer still needs its rendition. The upload is checked against
   * the page's BLAKE3 and removed afterwards; the store stays read-only.
   */
  async convertUpload(owner: number, token: string, request: { kind: ConversionKind; name: string; blake3: string }): Promise<MediaConversion> {
    const refused = this.refuse(request.kind, request.name);
    if (refused) { await this.store.abortStream(owner, token); return refused; }
    try {
      return await this.store.useStream(owner, token, request.blake3, (file) => this.convertFile(request.kind, file, mediaExtension(request.name)));
    } catch (error) {
      if (error instanceof MediaStoreError && error.code === 'notFound') throw error;
      return { ok: false, error: error instanceof Error && error.message ? error.message : 'The upload could not be read' };
    }
  }

  private refuse(kind: ConversionKind, name: string): Extract<MediaConversion, { ok: false }> | null {
    const extension = mediaExtension(name);
    if (kind === 'still') {
      if (!this.options.convertStill) return { ok: false, error: 'Image conversion is unavailable' };
      if (!needsImageConversion(extension)) return { ok: false, error: 'This image format does not need converting' };
    } else if (kind === 'video') {
      if (!this.options.convertVideo) return { ok: false, error: 'Video conversion is unavailable' };
      if (!mayNeedVideoProxy(extension)) return { ok: false, error: 'This video container is not one Powermove can convert' };
    } else return { ok: false, error: 'Unknown conversion' };
    return null;
  }

  private async convertOriginal(kind: ConversionKind, request: { hash?: string; link?: MediaLink; name: string }): Promise<MediaConversion> {
    const refused = this.refuse(kind, request.name);
    if (refused) return refused;
    let file: string | null = null;
    if (request.link !== undefined) {
      if (!validLink(request.link)) throw new MediaStoreError('Invalid media link.');
      const located = await this.locate(request.link);
      if ('path' in located) file = located.path;
      else if (await this.store.hasObject(request.link.hash, request.link.size)) file = this.store.objectPath(request.link.hash);
    } else if (request.hash !== undefined) {
      if (!HASH_HEX.test(request.hash)) throw new MediaStoreError('Invalid content hash.');
      if (await this.store.hasObject(request.hash)) file = this.store.objectPath(request.hash);
    } else throw new MediaStoreError('Name a hash or a link.');
    if (!file) return { ok: false, error: 'The original file is no longer available', missing: true };
    return this.convertFile(kind, file, mediaExtension(request.name));
  }

  /** Runs main's converter on `file` through a temporary name that carries
   *  the original's extension (which picks the decoder). */
  private async convertFile(kind: ConversionKind, file: string, extension: string): Promise<MediaConversion> {
    await this.store.init();
    const folder = path.join(this.store.root, 'tmp', `${kind}-${randomBytes(8).toString('hex')}`);
    await mkdir(folder, { recursive: true, mode: 0o700 });
    try {
      const named = path.join(folder, `original.${extension}`);
      await symlink(file, named);
      if (kind === 'still') return { ok: true, ...await this.options.convertStill!(named), type: 'image/png' };
      return { ok: true, ...await this.options.convertVideo!(named), type: 'video/webm' };
    } catch (error) {
      const failed = kind === 'still' ? 'Could not convert this image' : 'Could not optimize this video for playback';
      return { ok: false, error: error instanceof Error && error.message ? `${failed} · ${error.message}` : failed };
    } finally {
      await rm(folder, { recursive: true, force: true });
    }
  }

  /** Desktop Save: the store object for `hash` when it is `length` bytes. */
  async storedFile(hash: string, length: number): Promise<string | null> {
    if (!HASH_HEX.test(hash) || !Number.isSafeInteger(length)) return null;
    return await this.store.hasObject(hash, length) ? this.store.objectPath(hash) : null;
  }

  /** Frozen bindings for a job snapshot; independent of live windows. */
  private readonly jobBindings = new Map<number, Map<string, MediaBinding>>();

  async bindJob(session: number, project: Record<string, any> | null): Promise<void> {
    if (!project) { this.jobBindings.delete(session); return; }
    this.jobBindings.set(session, await this.snapshotBindings(project));
  }

  /** Persist content references when a queue entry is added, before its
   * window closes or an old storage key is reused for different bytes. */
  async snapshotProject(project: Record<string, any>): Promise<Record<string, any>> {
    const bindings = await this.snapshotBindings(project);
    return { ...project, assets: Object.fromEntries(Object.entries<any>(project['assets'] || {}).map(([key, meta]) => {
      const hash = bindings.get(meta?.id)?.hash;
      return [key, hash ? { ...meta, contentHash: hash } : meta];
    })) };
  }

  private async snapshotBindings(project: Record<string, any>): Promise<Map<string, MediaBinding>> {
    const assets = new Map<string, MediaBinding>();
    for (const meta of Object.values<any>(project['assets'] || {})) {
      if (!meta || typeof meta.id !== 'string') continue;
      if (meta.storage === 'linked' && validLink(meta.link)) {
        assets.set(meta.id, { link: { ...meta.link } });
      } else {
        const key = validStorageKey(meta.storageKey) ? meta.storageKey : meta.id;
        const hash = typeof meta.contentHash === 'string' && HASH_HEX.test(meta.contentHash)
          ? meta.contentHash : (await this.store.record(key))?.hash;
        // Freeze the hash now: later key writes must not change this snapshot.
        if (hash) assets.set(meta.id, { hash });
      }
    }
    return assets;
  }

  bind(owner: number, projectId: string, assets: Record<string, MediaBinding>): void {
    const map = new Map<string, MediaBinding>();
    for (const [id, binding] of Object.entries(assets)) {
      if (!/^[A-Za-z0-9_.:-]{1,200}$/.test(id) || !binding || typeof binding !== 'object') continue;
      const clean: MediaBinding = {};
      if (binding.key !== undefined && validStorageKey(binding.key)) clean.key = binding.key;
      if (binding.hash !== undefined && HASH_HEX.test(binding.hash)) clean.hash = binding.hash;
      if (binding.link !== undefined && validLink(binding.link)) clean.link = binding.link;
      if (clean.key || clean.hash || clean.link) map.set(id, clean);
    }
    this.bindings.set(owner, { projectId, assets: map });
  }

  /** A window went away: its reads, uploads, and bindings go with it. */
  async release(owner: number): Promise<void> {
    this.bindings.delete(owner);
    for (const [token, session] of [...this.reads]) if (session.owner === owner) await this.close(owner, token);
    await this.store.abandon(owner);
  }

  /**
   * The engine's `mediaRead` resolution (contract 14): the file an asset's
   * bytes are in and its content hash, or null when the asset is offline,
   * unknown, or bound to different content in two windows.
   */
  async resolveAsset(asset: string, context?: { process: EngineProcess; session: number }): Promise<{ path: string; contentHash: ContentHash | null } | null> {
    const candidates = context?.process === 'job' && this.jobBindings.has(context.session)
      ? [this.jobBindings.get(context.session)!.get(asset)]
      : [...this.bindings.values()].map((b) => b.assets.get(asset));
    const bound = candidates.filter((b): b is MediaBinding => !!b);
    if (!bound.length) return null;
    const hashes = new Set(bound.map((b) => b.link?.hash ?? b.hash).filter(Boolean));
    if (hashes.size > 1) return null;
    for (const binding of bound) {
      if (binding.link) {
        const located = await this.locate(binding.link);
        if ('path' in located) return { path: located.path, contentHash: hashBytes(binding.link.hash) };
        continue;
      }
      if (binding.hash && await this.store.hasObject(binding.hash)) return { path: this.store.objectPath(binding.hash), contentHash: hashBytes(binding.hash) };
      if (binding.key) {
        const record = await this.store.record(binding.key);
        if (record) return { path: this.store.objectPath(record.hash), contentHash: hashBytes(record.hash) };
      }
    }
    return null;
  }
}
