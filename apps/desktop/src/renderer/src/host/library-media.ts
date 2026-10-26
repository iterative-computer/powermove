import { mediaRangeSource } from './media-range';
import type { MediaReadSource } from '../../../shared/media-source';
import { LIBRARY_MEDIA_CHUNK, type LibraryMediaBridge, type LibraryMediaRecord } from '../../../shared/library-media';
import { Sha256 } from '../../../shared/sha256';

export type LegacyMediaRecord = { id: string; blob: Blob; revision?: string; at?: number; size?: number };
export interface LegacyMedia {
  record(value: unknown): Promise<LegacyMediaRecord | null>;
  keys(): Promise<string[]>;
  removeIfUnchanged(record: LegacyMediaRecord): Promise<boolean>;
}
const keyOf = (value: any): string | null => typeof value === 'string' ? value : value?.storageKey || value?.id || null;

/** Desktop media uses streamed files in the chosen library. IndexedDB is read
 * only for migration, and each old record is removed only after a durable put. */
export function attachLibraryMedia(host: LibraryMediaBridge, legacy: LegacyMedia) {
  const checksum = async (blob: Blob): Promise<string> => {
    const digest = new Sha256();
    for (let offset = 0; offset < blob.size; offset += LIBRARY_MEDIA_CHUNK) digest.update(new Uint8Array(await blob.slice(offset, offset + LIBRARY_MEDIA_CHUNK).arrayBuffer()));
    return digest.digest();
  };
  const retireLegacy = async (old: LegacyMediaRecord, saved: LibraryMediaRecord) => {
    // Revision labels from early migrations can differ even when the payload
    // is identical. Only a full checksum can authorize dropping that copy.
    if (saved.sha256 && old.blob.size === saved.size && await checksum(old.blob) === saved.sha256) await legacy.removeIfUnchanged(old);
  };
  const put = async (key: string, blob: Blob, revision?: string, onlyIfMissing = false): Promise<LibraryMediaRecord> => {
    const token = await host.begin({ key, size: blob.size, type: blob.type || 'application/octet-stream', revision: revision || crypto.randomUUID(), onlyIfMissing,
      ...((blob as any).__powermoveArtifact ? { artifact: (blob as any).__powermoveArtifact } : {}) });
    try {
      for (let offset = 0; offset < blob.size; offset += LIBRARY_MEDIA_CHUNK) {
        await host.chunk(token, offset, new Uint8Array(await blob.slice(offset, offset + LIBRARY_MEDIA_CHUNK).arrayBuffer()));
      }
      return await host.finish(token);
    } finally { await host.abort(token).catch(() => undefined); }
  };
  const migrateRecord = async (record: LegacyMediaRecord): Promise<LibraryMediaRecord> => {
    const revision = record.revision || crypto.randomUUID();
    const saved = await put(record.id, record.blob, revision, true);
    await retireLegacy(record, saved);
    return saved;
  };
  const find = async (value: unknown): Promise<LibraryMediaRecord | null> => {
    const key = keyOf(value);
    if (!key) return null;
    let record = await host.get(key);
    if (!record && typeof value === 'object' && value && keyOf((value as any).id) !== key && (value as any).id) record = await host.get((value as any).id);
    if (record) {
      const sourcePath = value && typeof value === 'object' && 'sourcePath' in value && typeof value.sourcePath === 'string' ? value.sourcePath : null;
      if (host.link && !record.sourcePath && sourcePath) {
        try {
          record = await host.link({ key, sourcePath, size: record.size, type: record.type,
            revision: record.revision, matchExisting: true });
        } catch { /* A moved/changed original must never replace a valid stored copy. */ }
      }
      try { const old = await legacy.record(value); if (old?.blob instanceof Blob) await retireLegacy(old, record); }
      catch { /* Browser storage failure must not hide a durable native source. */ }
      return record;
    }
    const old = await legacy.record(value);
    return old?.blob instanceof Blob ? migrateRecord(old) : host.get(key);
  };
  const blobFor = async (record: LibraryMediaRecord, signal?: AbortSignal): Promise<Blob> => {
    const response = await fetch(record.url, { cache: 'no-store', signal });
    if (!response.ok) throw new Error('The project media could not be read from your library.');
    const blob = await response.blob();
    if (blob.size !== record.size) throw new Error('The project media is incomplete.');
    return blob;
  };
  let migration: Promise<{ moved: number; remaining: number }> | null = null;
  return {
    async put(id: string, blob: Blob, meta: any = {}, savedRevision?: string): Promise<boolean> {
      const key = keyOf({ storageKey: meta.storageKey, id });
      if (!key || !blob) return false;
      try {
        if (meta.sourcePath && host.link) {
          await host.link({ key, sourcePath: meta.sourcePath, size: blob.size, type: blob.type || meta.type || 'application/octet-stream', revision: savedRevision });
        } else await put(key, blob, savedRevision);
        return true;
      }
      catch (error) { console.warn('[media-library] Import was not saved.', error); return false; }
    },
    async get(value: unknown, { signal }: { signal?: AbortSignal } = {}): Promise<Blob | null> {
      const record = await find(value);
      signal?.throwIfAborted();
      return record ? blobFor(record, signal) : null;
    },
    async getForSave(value: any): Promise<{ source?: MediaReadSource; revision: string; linked?: boolean } | null> {
      const key = keyOf(value);
      const metadata = key ? await host.get(key, true) : null;
      // A missing original must not stop the user saving unrelated edits.
      if (metadata?.sourcePath && value?.sourcePath === metadata.sourcePath) return { revision: metadata.revision, linked: true };
      const record = await find(value);
      // Ordinary native saves retain the original path already present in the
      // document. Portable exports call get() and deliberately embed all bytes.
      if (record?.sourcePath && value?.sourcePath === record.sourcePath) return { revision: record.revision, linked: true };
      return record ? { source: mediaRangeSource(record), revision: record.revision } : null;
    },
    async remove(value: unknown): Promise<boolean> {
      const key = keyOf(value);
      return key ? host.remove(key) : false;
    },
    async removeLegacyDuplicates(): Promise<number> {
      let removed = 0;
      for (const key of await legacy.keys()) {
        const saved = await host.get(key).catch(() => null);
        if (!saved) continue;
        const old = await legacy.record(key);
        if (old?.blob instanceof Blob && saved.sha256 && old.blob.size === saved.size && await checksum(old.blob) === saved.sha256) {
          if (await legacy.removeIfUnchanged(old)) removed++;
        }
      }
      return removed;
    },
    async optimizeLegacy(onProgress?: (done: number, total: number) => void): Promise<void> {
      const keys = await legacy.keys();
      for (let index = 0; index < keys.length; index++) {
        const key = keys[index]!, old = await legacy.record(key);
        if (old?.blob instanceof Blob) {
          const metadata = await host.get(key, true);
          if (metadata) {
            // An unavailable source or a different native revision may leave
            // this browser record as the only copy. Preserve it in that case.
            const saved = await host.get(key).catch(() => null);
            if (saved) await retireLegacy(old, saved);
          } else await migrateRecord(old);
        }
        onProgress?.(index + 1, keys.length);
        await new Promise<void>(resolve => setTimeout(resolve, 0));
      }
    },
    migrateLegacy(onProgress?: (done: number, total: number) => void): Promise<{ moved: number; remaining: number }> {
      if (migration) return migration;
      migration = (async () => {
        const keys = await legacy.keys();
        let moved = 0;
        for (let index = 0; index < keys.length; index++) {
          const old = await legacy.record(keys[index]);
          if (old?.blob instanceof Blob) { await migrateRecord(old); moved++; }
          onProgress?.(index + 1, keys.length);
        }
        return { moved, remaining: (await legacy.keys()).length };
      })().finally(() => { migration = null; });
      return migration;
    }
  };
}
