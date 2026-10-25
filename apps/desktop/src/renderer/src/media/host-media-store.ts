/*
 * The renderer side of the host media store (plan section 21;
 * docs/engine/media-store.md).
 *
 * `PM.MediaStore` keeps its shape (put, get, getForSave, remove). With "Use
 * New Engine" on, this wrapper makes main's media store the source of
 * bytes: imports copy into it (a file with a path is copied by main, other
 * bytes stream there and are verified by hash), reads come from it, linked
 * assets read their original file, and IndexedDB is only read, never
 * written or deleted: anything found there is copied into the store and
 * verified by hash. With the flag off, IndexedDB stays the source and the
 * store is a read-only fallback for media imported while the flag was on,
 * so turning the new engine off and on again loses nothing.
 */
import { Blake3 } from '../../../shared/blake3';
import type { AssetStorage, ClassifyResult, ConversionKind, MediaBinding, MediaConversion, MediaLink, MediaReadOpen, MediaRecordSource, MediaStoreBridge, MediaStoreRecord } from '../../../shared/media-store';
import { compactEditLog } from '../core/edit-log';
import { applyEngineStamp } from '../../../shared/engine-stamp';
import { mimeOf } from '../../../shared/media-mime';
export { mimeOf } from '../../../shared/media-mime';

/** Bytes per IPC chunk (main caps reads and writes at 4 MiB). */
export const CHUNK_BYTES = 4 * 1024 * 1024;
/** Hashing runs in slices with a yield between, so it never holds the
 *  renderer's thread for more than a couple of milliseconds. */
const HASH_SLICE = 1024 * 1024;
/** Chunks read from main are folded into a Blob every 64 MiB, so a large
 *  file is never held in the page's own memory all at once. */
const FOLD_BYTES = 64 * 1024 * 1024;
/** Bytes main copies per `putProjectRange` call (main's limit). */
const PROJECT_RANGE_BYTES = 64 * 1024 * 1024;
/** A BLAKE3 digest in hex. */
const HASH = /^[a-f0-9]{64}$/;

export type LegacyMediaStore = {
  put(id: string, blob: Blob, meta?: any, revision?: string): Promise<boolean>;
  get(value: any): Promise<Blob | null>;
  getForSave?(value: any): Promise<{ blob: Blob; revision: string } | null>;
  remove(value: any): Promise<boolean>;
};

export type ImportDecision =
  | { storage: 'copied'; reason?: ClassifyResult['reason'] | 'model' | 'requested' | 'setting' | 'conversion' }
  | { storage: 'linked'; link: MediaLink };

type Registry = Record<string, any>;

const yieldTask = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

const storageKeyOf = (value: any): string | null => {
  if (value && typeof value === 'object') return value.storageKey || value.id || null;
  return value || null;
};

const contentHashOf = (value: any): string | null =>
  value && typeof value === 'object' && typeof value.contentHash === 'string' && HASH.test(value.contentHash) ? value.contentHash : null;

export const isLinked = (meta: any): boolean => meta?.storage === 'linked' && !!meta?.link && typeof meta.link.path === 'string';

/** BLAKE3 of a Blob, read in slices with yields. */
export async function hashBlob(blob: Blob, onChunk?: (data: Uint8Array, offset: number) => Promise<void>): Promise<string> {
  const hasher = new Blake3();
  for (let offset = 0; offset < blob.size; offset += CHUNK_BYTES) {
    const data = new Uint8Array(await blob.slice(offset, offset + CHUNK_BYTES).arrayBuffer());
    for (let at = 0; at < data.length; at += HASH_SLICE) {
      hasher.update(data.subarray(at, at + HASH_SLICE));
      if (at + HASH_SLICE < data.length) await yieldTask();
    }
    await onChunk?.(data, offset);
  }
  return hasher.hex();
}

/** Streams a Blob into the store under `key`; main commits it only when
 *  the bytes it wrote hash to what was read here. */
export async function streamToStore(host: MediaStoreBridge, key: string | null, blob: Blob,
  options: { type?: string; revision?: string | null; origin?: MediaRecordSource; onProgress?: (fraction: number) => void } = {}): Promise<MediaStoreRecord & { sha256: string }> {
  const token = await host.beginPut({ key, type: options.type ?? blob.type ?? '', size: blob.size, revision: options.revision ?? null, origin: options.origin ?? 'stream' });
  try {
    const hash = await hashBlob(blob, async (data, offset) => {
      await host.putChunk(token, offset, data);
      options.onProgress?.(blob.size ? (offset + data.length) / blob.size : 1);
    });
    return await host.finishPut(token, { blake3: hash });
  } catch (error) {
    await host.abortPut(token).catch(() => undefined);
    throw error;
  }
}

/** Reads an opened store file into a Blob. */
export async function readOpened(host: MediaStoreBridge, opened: Extract<MediaReadOpen, { ok: true }>, type: string): Promise<Blob> {
  try {
    let folded: Blob = new Blob([], { type });
    let parts: Uint8Array[] = [];
    let partBytes = 0;
    for (let offset = 0; offset < opened.size;) {
      const chunk = await host.read(opened.token, offset, Math.min(CHUNK_BYTES, opened.size - offset));
      if (!(chunk instanceof Uint8Array) || !chunk.length) throw new Error('The media file ended early.');
      parts.push(chunk);
      partBytes += chunk.length;
      offset += chunk.length;
      if (partBytes >= FOLD_BYTES) {
        folded = new Blob([folded, ...parts as BlobPart[]], { type });
        parts = [];
        partBytes = 0;
      }
    }
    return new Blob([folded, ...parts as BlobPart[]], { type });
  } finally {
    await host.close(opened.token).catch(() => undefined);
  }
}

export type HostMediaStoreOptions = {
  host: MediaStoreBridge;
  legacy: LegacyMediaStore;
  enabled: boolean;
  /** The native path of a picked or dropped File, when it has one. */
  sourcePath(file: File): string | null;
  /** A linked original was found elsewhere by content hash. */
  onRelinked?(meta: any, link: MediaLink): void;
  /** Folders worth searching when a linked original moved. */
  searchRoots?(): string[];
  warn?(message: string, error: unknown): void;
};

export type HostMediaStore = Omit<LegacyMediaStore, 'getForSave'> & {
  getForSave(value: any): Promise<{ blob: Blob; revision: string } | null>;
  readonly enabled: boolean;
  /** True when imports keep the original and store renditions separately. */
  readonly keepsOriginals: boolean;
  /** The store record a key last resolved to (filled by put and get). */
  recordFor(key: string | null | undefined): MediaStoreRecord | null;
  /** The store record `put` made of this Blob. Unlike `recordFor`, it is
   *  not taken over by a concurrent put of other bytes under the same key. */
  storedRecord(blob: Blob): MediaStoreRecord | null;
  /** Desktop Save: bytes main copies straight from the store. */
  saveSource(asset: any): Promise<{ hash: string; size: number; type: string; revision: string } | null>;
  /**
   * Browser renditions (playback proxies, PNGs of TIFF or HEIF stills) kept
   * as generated media, named by the BLAKE3 of the original they render. A
   * storage key would not do: two files can share its sampled fingerprint.
   */
  rendition: { get(originalHash: string | null | undefined): Promise<Blob | null>; put(originalHash: string | null | undefined, blob: Blob): Promise<boolean> };
  /**
   * Has main convert a kept original (a TIFF or HEIF still, a video the
   * viewer cannot decode) into the viewer's rendition: from the store
   * object or the linked file when main has one, else from `original`, the
   * bytes this page holds (with Use New Engine off, IndexedDB's copy of a
   * project opened on another Mac). Read the result with
   * `media.readPlaybackProxy`.
   */
  convert(kind: ConversionKind, original: Blob | null, ref: { hash?: string | null; link?: MediaLink | null; name: string }): Promise<MediaConversion>;
  /**
   * Opening a project file with the new engine: main copies an embedded
   * media range from the open file (`readToken`) straight into the store,
   * checked against the file's recorded SHA-256 and named by its BLAKE3.
   * False when the host cannot (flag off, older preload); the caller then
   * restores the range through the page as before.
   */
  putProjectRange(readToken: string, source: { id: string; type: string; offset: number; length: number; revision?: string; sha256?: string },
    meta: any, onBytes?: (bytes: number) => void): Promise<boolean>;
  /** Copies IndexedDB media under these assets into the store (verified). */
  migrate(assets: any[], onProgress?: (done: number, total: number) => void): Promise<{ copied: number; present: number; missing: number; failed: number }>;
  readonly host: MediaStoreBridge;
};

/** The store key of the rendition of the original with this BLAKE3. */
export const renditionKey = (originalHash: string) => `rendition:${originalHash}`;

export function createHostMediaStore(options: HostMediaStoreOptions): HostMediaStore {
  const { host, legacy, enabled } = options;
  const records = new Map<string, MediaStoreRecord>();
  const stored = new WeakMap<Blob, MediaStoreRecord>();
  const migrations = new Map<string, Promise<boolean>>();
  const warn = options.warn ?? ((message, error) => console.warn(`[media-store] ${message}`, error));

  /** Reads what a key names; with `hash`, only when the key names that content. */
  const readKey = async (key: string, fallbackType: string, hash?: string | null): Promise<Blob | null> => {
    const opened = await host.open({ key });
    if (!opened.ok) return null;
    if (hash && opened.hash !== hash) { await host.close(opened.token).catch(() => undefined); return null; }
    return readOpened(host, opened, opened.type || fallbackType);
  };
  const readHash = async (hash: string, type: string): Promise<Blob | null> => {
    const opened = await host.open({ hash });
    return opened.ok ? readOpened(host, opened, type) : null;
  };
  const readLink = async (meta: any): Promise<Blob | null> => {
    const type = meta.type || mimeOf(meta.name);
    const opened = await host.open({ link: meta.link });
    if (opened.ok) {
      if (opened.relinked) options.onRelinked?.(meta, opened.relinked);
      return readOpened(host, opened, type);
    }
    // The same content in the store (a project file that embedded it) is
    // the same media: content identity, not location, names it.
    return (await readHash(meta.link.hash, type)) ?? (meta.storageKey ? await readKey(meta.storageKey, type) : null);
  };
  const remember = (record: MediaStoreRecord) => { records.set(record.key, record); return record; };

  /** Copies one IndexedDB blob into the store and verifies it by hash.
   *  IndexedDB is never written or deleted here. */
  const migrateBlob = (key: string, blob: Blob, revision: string | null, hash: string | null = null, onProgress?: (fraction: number) => void): Promise<boolean> => {
    let job = migrations.get(key);
    if (!job) {
      job = (async () => {
        const existing = await host.record(key);
        if (existing && existing.size === blob.size && (!hash || existing.hash === hash)) { remember(existing); return true; }
        /* The key already names other content that shares the sampled
           fingerprint: keep this asset's bytes by content alone. */
        const keep = existing && hash && existing.hash !== hash ? null : key;
        const record = await streamToStore(host, keep, blob, { type: blob.type, revision, origin: 'indexeddb', onProgress });
        if (keep) remember(record);
        return !hash || record.hash === hash;
      })().catch((error) => { warn(`could not copy ${key} from IndexedDB`, error); return false; })
        .finally(() => migrations.delete(key));
      migrations.set(key, job);
    }
    return job;
  };

  /* Content identity names an asset's bytes (section 21). The storage key
     comes from a sampled fingerprint, so two different files can share one;
     an asset that knows its content hash reads exactly that content. */
  const storeGet = async (value: any): Promise<Blob | null> => {
    if (isLinked(value)) return readLink(value);
    const key = storageKeyOf(value);
    const type = (value && typeof value === 'object' && (value.type || mimeOf(value.name))) || '';
    const hash = contentHashOf(value);
    let blob = hash ? await readHash(hash, type) : null;
    if (!blob && key) blob = await readKey(key, type, hash);
    if (!blob && value && typeof value === 'object' && value.id && value.id !== key) blob = await readKey(value.id, type, hash);
    return blob;
  };

  /* The store record holding an asset's own content: its content hash when
     it has one (the key's record may name other content that shares the
     fingerprint), else whatever its key names. */
  const recordOf = async (value: any): Promise<MediaStoreRecord | null> => {
    const key = storageKeyOf(value);
    const hash = contentHashOf(value);
    const record = key ? await host.record(key) : null;
    if (!hash || record?.hash === hash) return record;
    const opened = await host.open({ hash });
    if (!opened.ok) return null;
    await host.close(opened.token).catch(() => undefined);
    return { key: key ?? `blake3:${hash}`, hash, size: opened.size, type: (value && typeof value === 'object' && value.type) || record?.type || '', revision: hash, at: 0 };
  };

  const store: HostMediaStore = {
    enabled,
    keepsOriginals: enabled,
    host,
    recordFor: (key) => (key ? records.get(key) ?? null : null),
    storedRecord: (blob) => (blob ? stored.get(blob) ?? null : null),

    async put(id, blob, meta = {}, savedRevision) {
      if (!enabled) return legacy.put(id, blob, meta, savedRevision);
      const key = meta?.storageKey || id;
      if (!key || !blob) return false;
      try {
        const path = typeof File !== 'undefined' && blob instanceof File ? options.sourcePath(blob) : null;
        if (path) {
          try {
            const record = await host.putFile({ path, key, type: blob.type || meta?.type || '', revision: savedRevision ?? null });
            if (record.size === blob.size) { stored.set(blob, remember(record)); return true; }
          } catch (error) { warn(`copying ${path} failed; sending its bytes instead`, error); }
        }
        stored.set(blob, remember(await streamToStore(host, key, blob, { type: blob.type || meta?.type || '', revision: savedRevision ?? null, origin: meta?.origin ?? 'stream' })));
        return true;
      } catch (error) {
        warn(`could not store ${key}`, error);
        return false;
      }
    },

    async get(value) {
      if (!enabled) {
        // Legacy engine: IndexedDB first; the store holds what the new engine imported.
        if (isLinked(value)) return readLink(value);
        const blob = await legacy.get(value);
        if (blob) return blob;
        return storeGet(value).catch(() => null);
      }
      const blob = await storeGet(value).catch((error) => { warn('could not read from the media store', error); return null; });
      if (blob) return blob;
      const old = await legacy.get(value);
      if (old) {
        const key = storageKeyOf(value);
        const saved = await legacy.getForSave?.(value).catch(() => null);
        if (key) void migrateBlob(key, old, saved?.revision ?? null, contentHashOf(value));
      }
      return old;
    },

    async getForSave(value) {
      if (!enabled) {
        if (isLinked(value)) return null;
        const saved = await legacy.getForSave?.(value);
        if (saved) return saved;
        /* Media the new engine imported lives only in the store. Its key may
           name other content that shares the sampled fingerprint, so the
           asset's own content hash picks the bytes, as with the flag on. */
        const record = await recordOf(value).catch(() => null);
        const blob = record ? await readHash(record.hash, record.type) : null;
        return blob && record ? { blob, revision: record.revision } : null;
      }
      // Linked originals are not stored in the desktop project file.
      if (isLinked(value)) return null;
      const key = storageKeyOf(value);
      const record = await recordOf(value);
      if (record) {
        const blob = await readHash(record.hash, record.type);
        return blob ? { blob, revision: record.revision } : null;
      }
      const saved = await legacy.getForSave?.(value);
      if (saved && key) await migrateBlob(key, saved.blob, saved.revision, contentHashOf(value));
      return saved ?? null;
    },

    async remove(value) {
      const key = storageKeyOf(value);
      const removed = key ? await host.removeKey(key).catch(() => false) : false;
      if (key) records.delete(key);
      // IndexedDB is the legacy engine's store: only it removes from there.
      return enabled ? removed : (await legacy.remove(value)) || removed;
    },

    async saveSource(asset) {
      if (!enabled || isLinked(asset)) return null;
      const key = storageKeyOf(asset);
      if (!key && !contentHashOf(asset)) return null;
      let record = await recordOf(asset);
      if (!record && key) {
        const saved = await legacy.getForSave?.(asset).catch(() => null);
        if (saved && await migrateBlob(key, saved.blob, saved.revision, contentHashOf(asset))) record = await recordOf(asset);
      }
      return record ? { hash: record.hash, size: record.size, type: record.type, revision: record.revision } : null;
    },

    rendition: {
      async get(originalHash) {
        /* Also with the flag off: the legacy viewer reuses the rendition the
           new engine kept instead of converting the original again. A
           rendition the legacy engine made (flag off) is in IndexedDB. */
        if (!originalHash || !HASH.test(originalHash)) return null;
        const kept = await readKey(renditionKey(originalHash), 'video/webm').catch(() => null);
        return kept ?? await legacy.get(renditionKey(originalHash)).catch(() => null);
      },
      async put(originalHash, blob) {
        if (!originalHash || !HASH.test(originalHash) || !blob) return false;
        // Flag off, the store is read-only: generated media goes where the legacy engine keeps its bytes.
        if (!enabled) return legacy.put(renditionKey(originalHash), blob, { type: blob.type }).catch((error) => { warn(`could not keep the rendition of ${originalHash}`, error); return false; });
        try { remember(await streamToStore(host, renditionKey(originalHash), blob, { type: blob.type || 'video/webm', origin: 'rendition' })); return true; }
        catch (error) { warn(`could not keep the rendition of ${originalHash}`, error); return false; }
      }
    },

    async convert(kind, original, ref) {
      const request = ref.link ? { link: ref.link, name: ref.name } : ref.hash && HASH.test(ref.hash) ? { hash: ref.hash, name: ref.name } : null;
      const byRef = kind === 'still' ? host.convertStill?.bind(host) : host.convertVideo?.bind(host);
      let result: MediaConversion | null = null;
      if (request && byRef) {
        result = await byRef(request);
        if (result.ok || !result.missing) return result;
      }
      if (!host.convertUpload || !original || !(original.size > 0)) {
        return result ?? { ok: false, error: kind === 'still' ? 'Image conversion is unavailable' : 'Video conversion is unavailable' };
      }
      // Main has neither the object nor the file: it converts the bytes this page has, and keeps none of them.
      const token = await host.beginPut({ key: null, type: original.type || '', size: original.size, origin: 'stream' });
      let sent = false;
      try {
        const blake3 = await hashBlob(original, (data, offset) => host.putChunk(token, offset, data));
        sent = true;
        return await host.convertUpload(token, { kind, name: ref.name, blake3 });
      } finally {
        if (!sent) await host.abortPut(token).catch(() => undefined);
      }
    },

    async putProjectRange(readToken, source, meta, onBytes) {
      if (!enabled || !host.putProjectRange || !readToken) return false;
      // An empty range has no bytes for main to read; the page sends it with its hash.
      if (!(source.length > 0)) return false;
      const key = meta?.storageKey || source.id;
      if (!key) return false;
      const token = await host.beginPut({ key, type: source.type || meta?.type || '', size: source.length, revision: source.revision ?? null, origin: 'project' });
      try {
        for (let at = 0; at < source.length;) {
          const length = Math.min(PROJECT_RANGE_BYTES, source.length - at);
          await host.putProjectRange(token, readToken, source.offset + at, length, at);
          at += length;
          onBytes?.(length);
        }
        /* With no recorded checksum (PMV3), main still compares the copy on
           disk with the bytes it read. */
        remember(await host.finishPut(token, source.sha256 ? { sha256: source.sha256 } : {}));
        return true;
      } catch (error) {
        await host.abortPut(token).catch(() => undefined);
        throw error;
      }
    },

    async migrate(assets, onProgress) {
      const counts = { copied: 0, present: 0, missing: 0, failed: 0 };
      const pending = assets.filter(asset => asset && !isLinked(asset) && storageKeyOf(asset));
      let completed = 0;
      let copying = false;
      for (const asset of pending) {
        try {
          const key = storageKeyOf(asset)!;
          const present = await recordOf(asset).catch(() => null);
          if (present) { remember(present); counts.present++; continue; }
          const saved = await legacy.getForSave?.(asset).catch(() => null);
          const blob = saved?.blob ?? await legacy.get(asset).catch(() => null);
          if (!blob) { counts.missing++; continue; }
          copying = true;
          onProgress?.(completed, pending.length);
          if (await migrateBlob(key, blob, saved?.revision ?? null, contentHashOf(asset),
            fraction => onProgress?.(completed + fraction, pending.length))) counts.copied++;
          else counts.failed++;
        } finally {
          completed++;
          if (copying) onProgress?.(completed, pending.length);
        }
      }
      return counts;
    }
  };
  return store;
}

/**
 * The storage an import gets (section 21): Copy into project unless the
 * project links files (or an agent asked for `storage: "link"`), and always
 * a copy for items without a file, 3D models and Blender sources, formats
 * that need conversion, agent workspace files, and camera media.
 */
export async function decideImport(host: MediaStoreBridge, input: {
  enabled: boolean;
  kind: string;
  sourcePath: string | null | undefined;
  /** `link` or `copy` from the caller (agent tools), else the project setting. */
  requested?: 'link' | 'copy';
  projectSetting?: 'link' | 'copy';
  converted?: boolean;
}): Promise<ImportDecision> {
  if (!input.enabled) return { storage: 'copied' };
  const wanted = input.requested ?? input.projectSetting ?? 'copy';
  if (wanted !== 'link') return { storage: 'copied', reason: input.requested ? 'requested' : 'setting' };
  if (!input.sourcePath) return { storage: 'copied', reason: 'notAFile' };
  if (input.kind === 'model') return { storage: 'copied', reason: 'model' };
  if (input.converted) return { storage: 'copied', reason: 'conversion' };
  const classified = await host.classify(input.sourcePath);
  if (classified.storage !== 'linked') return { storage: 'copied', ...(classified.reason ? { reason: classified.reason } : {}) };
  return { storage: 'linked', link: await host.link(input.sourcePath) };
}

/** What the engine needs to read each asset of a project (contract 14). */
export function bindingsFor(project: any, records: (key: string) => MediaStoreRecord | null): Record<string, MediaBinding> {
  const out: Record<string, MediaBinding> = {};
  for (const meta of Object.values<any>(project?.assets || {})) {
    if (!meta?.id) continue;
    if (isLinked(meta)) { out[meta.id] = { link: meta.link, ...(meta.rendition === true ? { rendition: true } : {}) }; continue; }
    const key = storageKeyOf(meta);
    const hash = typeof meta.contentHash === 'string' ? meta.contentHash : key ? records(key)?.hash : undefined;
    if (key || hash) out[meta.id] = { ...(key ? { key } : {}), ...(hash ? { hash } : {}), ...(meta.rendition === true ? { rendition: true } : {}) };
  }
  return out;
}

/**
 * Installs the wrapper over `PM.MediaStore` (after `core/media`), the
 * import decisions for `gl/raster`, and the engine bindings.
 */
export function installHostMediaStore(PM: Registry, bridge: { mediaStore?: MediaStoreBridge; media?: { sourcePath?(file: File): string | null }; remote?: unknown; nativeHidden?: boolean } | null | undefined): void {
  const host = bridge?.mediaStore;
  if (!host || (bridge?.remote && !bridge.nativeHidden) || !PM.MediaStore) return;
  let enabled = false;
  try { enabled = host.configSync().enabled === true; } catch { enabled = false; }
  const relinked = (meta: any, link: MediaLink) => {
    const project = PM.proj;
    if (!project?.assets?.[meta.id] || project.assets[meta.id] !== meta) return;
    // Engine-originated fact (section 6): a metadata write, not undoable, in the edit log.
    meta.link = link;
    meta.sourcePath = link.path;
    project.edits = compactEditLog([...(Array.isArray(project.edits) ? project.edits : []), {
      id: PM.uid?.('edit') ?? `edit-${Date.now()}`, revision: Number(project.revision) || 0, at: Date.now(), origin: 'engine',
      label: 'Relink media', summary: [`Relinked ${meta.name || meta.id} by content`], operations: [{ type: 'relink_media', asset: meta.id, path: link.path }]
    }]);
    PM.touch?.();
    PM.bus?.emit?.('assets');
  };
  const store = createHostMediaStore({
    host,
    legacy: PM.MediaStore,
    enabled,
    sourcePath: (file) => { try { return bridge?.media?.sourcePath?.(file) ?? null; } catch { return null; } },
    onRelinked: relinked
  });
  PM.MediaStore = store;
  PM.HostMedia = {
    enabled,
    store,
    /** Imports await the host's binding before exposing a playable asset:
     * immediate exports must not mistake the debounce window for offline media. */
    async flushBindings(project = PM.proj, onProgress?: (done: number, total: number) => void): Promise<void> {
      if (!enabled || !project?.id || !/^[A-Za-z0-9_-]{1,120}$/.test(project.id)) return;
      // Imports already have cached records. Keep their original immediate
      // bind; an extra IPC read here lets the viewer outrun the acknowledgement.
      const pending = Object.values<any>(project.assets || {}).filter(meta =>
        !isLinked(meta) && !store.recordFor(storageKeyOf(meta)));
      if (pending.length) await store.migrate(pending, onProgress);
      await host.bind(project.id, bindingsFor(project, store.recordFor));
    },
    decide: (input: Parameters<typeof decideImport>[1]) => decideImport(host, { ...input, enabled }),
    /** BLAKE3 of a Blob (the same digest as the store's and pm-media's). */
    hash: (blob: Blob) => hashBlob(blob),
    projectStorage: (): 'copy' | 'link' => PM.proj?.media?.whenImporting === 'link' ? 'link' : 'copy',
    setProjectStorage(value: 'copy' | 'link') {
      if (!PM.proj) return;
      const next = value === 'link' ? 'link' : 'copy';
      if ((PM.proj.media?.whenImporting ?? 'copy') === next) return;
      const apply = (target: 'copy' | 'link') => () => {
        const media = { ...(PM.proj.media || {}) };
        if (target === 'copy') delete media.whenImporting; else media.whenImporting = 'link';
        if (Object.keys(media).length) PM.proj.media = media; else delete PM.proj.media;
      };
      PM.hist?.do ? PM.hist.do('When importing', apply(next)) : apply(next)();
      PM.touch?.();
      PM.bus?.emit?.('project');
      PM.autosave?.();
    }
  };
  if (!enabled) return;
  // The engine reads each asset through a handle main mints from these.
  let timer: ReturnType<typeof setTimeout> | null = null;
  let migrated: string | null = null;
  let bound: { project: string; signature: string } | null = null;
  /** Binds the project's assets when their bindings differ from the last
   *  ones bound (or always, `force`). */
  const bindNow = (force: boolean): void => {
    const project = PM.proj;
    if (!project?.id || !/^[A-Za-z0-9_-]{1,120}$/.test(project.id)) return;
    const bindings = bindingsFor(project, store.recordFor);
    const signature = JSON.stringify(bindings);
    if (!force && bound && bound.project === project.id && bound.signature === signature) return;
    bound = { project: project.id, signature };
    void host.bind(project.id, bindings).catch(() => undefined);
  };
  // The engine resolves an asset again when its record changes (a replace,
  // its undo and redo, a relink), through these bindings: bound late, an
  // older binding answers the older file and the engine keeps drawing it.
  // So every batch the document mirror applies to the assets binds first
  // what it changed (other batches, a keystroke or a drag, cost nothing).
  // This subscribes before any engine session does (bootstrap installs the
  // media store before the engine connection), so the bind is sent to main
  // before the batch is posted to the engine.
  PM.EngineDoc?.subscribe?.((batch: { assets?: boolean }) => { if (batch?.assets !== false) bindNow(false); });
  const bind = () => {
    timer = null;
    const project = PM.proj;
    if (!project?.id || !/^[A-Za-z0-9_-]{1,120}$/.test(project.id)) return;
    bindNow(true);
    // Embedded media moves to the store on first open with the new engine;
    // IndexedDB keeps its copy (section 21).
    if (migrated !== project.id) {
      migrated = project.id;
      const posters = Object.values<any>(project.assets || {}).filter((meta) => meta?.poster && meta.storageKey)
        .map((meta) => ({ storageKey: `${meta.storageKey}:poster` }));
      void store.migrate([...Object.values(project.assets || {}), ...posters]).catch(() => undefined);
    }
  };
  const schedule = () => { if (!timer) timer = setTimeout(bind, 250); };
  /* The stamp follows the project's native-only features (linked media). */
  const stamp = () => {
    const project = PM.proj;
    if (!project?.id || !applyEngineStamp(project)) return;
    // Set or removed after the event's own announcement: announce it too.
    PM.doc?.touched?.([['requiresEngine']]);
    try { PM.Projects?.put?.(project); } catch { /* the next autosave keeps it */ }
  };
  for (const event of ['project', 'assets', 'projects:open']) PM.bus?.on?.(event, () => { stamp(); schedule(); });
  PM.bus?.on?.('layers', stamp);
  schedule();
}

export type { AssetStorage };
