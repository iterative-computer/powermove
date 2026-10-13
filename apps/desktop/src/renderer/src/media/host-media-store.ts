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
import type { AssetStorage, ClassifyResult, MediaBinding, MediaLink, MediaReadOpen, MediaRecordSource, MediaStoreBridge, MediaStoreRecord } from '../../../shared/media-store';
import { compactEditLog } from '../core/edit-log';
import { applyEngineStamp } from '../../../shared/engine-stamp';

/** Bytes per IPC chunk (main caps reads and writes at 4 MiB). */
export const CHUNK_BYTES = 4 * 1024 * 1024;
/** Hashing runs in slices with a yield between, so it never holds the
 *  renderer's thread for more than a couple of milliseconds. */
const HASH_SLICE = 1024 * 1024;
/** Chunks read from main are folded into a Blob every 64 MiB, so a large
 *  file is never held in the page's own memory all at once. */
const FOLD_BYTES = 64 * 1024 * 1024;

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
  options: { type?: string; revision?: string | null; origin?: MediaRecordSource } = {}): Promise<MediaStoreRecord & { sha256: string }> {
  const token = await host.beginPut({ key, type: options.type ?? blob.type ?? '', size: blob.size, revision: options.revision ?? null, origin: options.origin ?? 'stream' });
  try {
    const hash = await hashBlob(blob, (data, offset) => host.putChunk(token, offset, data));
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
  /** Desktop Save: bytes main copies straight from the store. */
  saveSource(asset: any): Promise<{ hash: string; size: number; type: string; revision: string } | null>;
  /** Browser renditions (playback proxies) kept as generated media. */
  rendition: { get(key: string): Promise<Blob | null>; put(key: string, blob: Blob): Promise<boolean> };
  /** Copies IndexedDB media under these assets into the store (verified). */
  migrate(assets: any[]): Promise<{ copied: number; present: number; missing: number; failed: number }>;
  readonly host: MediaStoreBridge;
};

export const renditionKey = (key: string) => `${key}:rendition`;

export function createHostMediaStore(options: HostMediaStoreOptions): HostMediaStore {
  const { host, legacy, enabled } = options;
  const records = new Map<string, MediaStoreRecord>();
  const migrations = new Map<string, Promise<boolean>>();
  const warn = options.warn ?? ((message, error) => console.warn(`[media-store] ${message}`, error));

  const readKey = async (key: string, fallbackType: string): Promise<Blob | null> => {
    const opened = await host.open({ key });
    if (!opened.ok) return null;
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
  const migrateBlob = (key: string, blob: Blob, revision: string | null): Promise<boolean> => {
    let job = migrations.get(key);
    if (!job) {
      job = (async () => {
        const existing = await host.record(key);
        if (existing && existing.size === blob.size) { remember(existing); return true; }
        const record = await streamToStore(host, key, blob, { type: blob.type, revision, origin: 'indexeddb' });
        remember(record);
        return true;
      })().catch((error) => { warn(`could not copy ${key} from IndexedDB`, error); return false; })
        .finally(() => migrations.delete(key));
      migrations.set(key, job);
    }
    return job;
  };

  const storeGet = async (value: any): Promise<Blob | null> => {
    if (isLinked(value)) return readLink(value);
    const key = storageKeyOf(value);
    if (!key) return null;
    const type = (value && typeof value === 'object' && (value.type || mimeOf(value.name))) || '';
    let blob = await readKey(key, type);
    if (!blob && value && typeof value === 'object' && value.id && value.id !== key) blob = await readKey(value.id, type);
    if (!blob && value && typeof value === 'object' && typeof value.contentHash === 'string') blob = await readHash(value.contentHash, type);
    return blob;
  };

  const store: HostMediaStore = {
    enabled,
    keepsOriginals: enabled,
    host,
    recordFor: (key) => (key ? records.get(key) ?? null : null),

    async put(id, blob, meta = {}, savedRevision) {
      if (!enabled) return legacy.put(id, blob, meta, savedRevision);
      const key = meta?.storageKey || id;
      if (!key || !blob) return false;
      try {
        const path = typeof File !== 'undefined' && blob instanceof File ? options.sourcePath(blob) : null;
        if (path) {
          try {
            const record = await host.putFile({ path, key, type: blob.type || meta?.type || '', revision: savedRevision ?? null });
            if (record.size === blob.size) { remember(record); return true; }
          } catch (error) { warn(`copying ${path} failed; sending its bytes instead`, error); }
        }
        remember(await streamToStore(host, key, blob, { type: blob.type || meta?.type || '', revision: savedRevision ?? null, origin: meta?.origin ?? 'stream' }));
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
        if (key) void migrateBlob(key, old, saved?.revision ?? null);
      }
      return old;
    },

    async getForSave(value) {
      if (!enabled) {
        if (isLinked(value)) return null;
        const saved = await legacy.getForSave?.(value);
        if (saved) return saved;
        const key = storageKeyOf(value);
        const record = key ? await host.record(key).catch(() => null) : null;
        const blob = record ? await readKey(record.key, record.type) : null;
        return blob && record ? { blob, revision: record.revision } : null;
      }
      // Linked originals are not stored in the desktop project file.
      if (isLinked(value)) return null;
      const key = storageKeyOf(value);
      const record = key ? await host.record(key) : null;
      if (record) {
        const blob = await readKey(record.key, record.type);
        return blob ? { blob, revision: record.revision } : null;
      }
      const saved = await legacy.getForSave?.(value);
      if (saved && key) await migrateBlob(key, saved.blob, saved.revision);
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
      if (!key) return null;
      let record = await host.record(key);
      if (!record) {
        const saved = await legacy.getForSave?.(asset).catch(() => null);
        if (saved && await migrateBlob(key, saved.blob, saved.revision)) record = await host.record(key);
      }
      return record ? { hash: record.hash, size: record.size, type: record.type, revision: record.revision } : null;
    },

    rendition: {
      async get(key) {
        if (!enabled || !key) return null;
        return readKey(renditionKey(key), 'video/webm').catch(() => null);
      },
      async put(key, blob) {
        if (!enabled || !key) return false;
        try { remember(await streamToStore(host, renditionKey(key), blob, { type: blob.type || 'video/webm', origin: 'rendition' })); return true; }
        catch (error) { warn(`could not keep the rendition of ${key}`, error); return false; }
      }
    },

    async migrate(assets) {
      const counts = { copied: 0, present: 0, missing: 0, failed: 0 };
      for (const asset of assets) {
        if (!asset || isLinked(asset)) continue;
        const key = storageKeyOf(asset);
        if (!key) continue;
        if (await host.record(key).catch(() => null)) { counts.present++; continue; }
        const saved = await legacy.getForSave?.(asset).catch(() => null);
        const blob = saved?.blob ?? await legacy.get(asset).catch(() => null);
        if (!blob) { counts.missing++; continue; }
        if (await migrateBlob(key, blob, saved?.revision ?? null)) counts.copied++;
        else counts.failed++;
      }
      return counts;
    }
  };
  return store;
}

const MIME: Record<string, string> = {
  mp4: 'video/mp4', m4v: 'video/mp4', mov: 'video/quicktime', webm: 'video/webm', mkv: 'video/x-matroska',
  png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp', svg: 'image/svg+xml',
  wav: 'audio/wav', mp3: 'audio/mpeg', m4a: 'audio/mp4', aac: 'audio/aac', ogg: 'audio/ogg', flac: 'audio/flac'
};
export function mimeOf(name: unknown): string {
  const match = /\.([A-Za-z0-9]{1,8})$/.exec(String(name || ''));
  return match ? MIME[match[1]!.toLowerCase()] ?? '' : '';
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
    if (isLinked(meta)) { out[meta.id] = { link: meta.link }; continue; }
    const key = storageKeyOf(meta);
    const hash = typeof meta.contentHash === 'string' ? meta.contentHash : key ? records(key)?.hash : undefined;
    if (key || hash) out[meta.id] = { ...(key ? { key } : {}), ...(hash ? { hash } : {}) };
  }
  return out;
}

/**
 * Installs the wrapper over `PM.MediaStore` (after `core/media`), the
 * import decisions for `gl/raster`, and the engine bindings.
 */
export function installHostMediaStore(PM: Registry, bridge: { mediaStore?: MediaStoreBridge; media?: { sourcePath?(file: File): string | null }; remote?: unknown } | null | undefined): void {
  const host = bridge?.mediaStore;
  if (!host || bridge?.remote || !PM.MediaStore) return;
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
    decide: (input: Parameters<typeof decideImport>[1]) => decideImport(host, { ...input, enabled }),
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
  const bind = () => {
    timer = null;
    const project = PM.proj;
    if (!project?.id || !/^[A-Za-z0-9_-]{1,120}$/.test(project.id)) return;
    void host.bind(project.id, bindingsFor(project, store.recordFor)).catch(() => undefined);
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
    try { PM.Projects?.put?.(project); } catch { /* the next autosave keeps it */ }
  };
  for (const event of ['project', 'assets', 'projects:open']) PM.bus?.on?.(event, () => { stamp(); schedule(); });
  PM.bus?.on?.('layers', stamp);
  schedule();
}

export type { AssetStorage };
