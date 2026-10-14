import { decodeProjectContainer, encodeProjectContainerBlob, encodeTextChunks, isProjectContainer, projectContainerIndex, type ProjectMediaRange } from '../../../../shared/project-container';
import { stringifyAsync } from './serialize-async';
import { bridge as hostBridge } from '../../kernel/bridge';
import { isIncrementalProject, readIncrementalIndex, type LinkedRecord, type SaveMedia } from '../../../../shared/project-incremental';
import { Sha256, sha256HexOf } from '../../../../shared/sha256';
import type { PowermoveBridge, FileSaveResult } from '../../../../shared/ipc';
import { applyEngineStamp } from '../../../../shared/engine-stamp';

type MediaStore = { get(asset: any): Promise<Blob | null>; put(id: string, blob: Blob, metadata: any, revision?: string): Promise<boolean>;
  getForSave?(asset: any): Promise<{ blob: Blob; revision: string } | null>;
  /** Host media store (new engine): bytes main copies into the save itself. */
  saveSource?(asset: any): Promise<{ hash: string; size: number; type: string; revision: string } | null> };

const isLinkedAsset = (asset: any) => asset?.storage === 'linked' && !!asset?.link && typeof asset.link.path === 'string';

/**
 * Export › Project and agent `save_project` (section 21): every linked
 * original is embedded, so the exported file is self-contained as before.
 * Embedded linked assets become copied assets in the exported document and
 * the engine stamp is recomputed, so the copy opens in either engine.
 */
export function embedLinkedAssets(document: any, embedded: ReadonlySet<string>): number {
  const assets = (document?.proj || document)?.assets;
  let changed = 0;
  for (const [id, asset] of Object.entries<any>(assets || {})) {
    if (!isLinkedAsset(asset) || !embedded.has(id)) continue;
    if (!asset.contentHash && asset.link.hash) asset.contentHash = asset.link.hash;
    if (!asset.sourcePath) asset.sourcePath = asset.link.path;
    delete asset.storage;
    delete asset.link;
    changed++;
  }
  applyEngineStamp(document?.proj || document);
  return changed;
}

/** Include media reachable only by undo/redo, such as a deleted image. */
function fileAssets(document: any): Record<string, any> {
  const assets: Record<string, any> = Object.create(null);
  for (const entry of document.history?.entries || []) {
    for (const patch of [...(entry.forward || []), ...(entry.backward || [])]) {
      if (!patch?.exists || !Array.isArray(patch.path)) continue;
      if (!patch.path.length) Object.assign(assets, patch.value?.assets || {});
      else if (patch.path[0] === 'assets' && patch.path.length === 1) Object.assign(assets, patch.value || {});
      else if (patch.path[0] === 'assets' && patch.path.length === 2 && patch.value?.id) assets[patch.path[1]] = patch.value;
    }
  }
  return Object.assign(assets, (document.proj || document).assets || {});
}

/** Pin immutable Blobs and their storage revisions before any native upload.
 * Only the main process decides which prior file ranges may be reused. */
export async function saveIncrementalProject(document: any, serialized: string, store: MediaStore,
  bridge: NonNullable<PowermoveBridge['projectSave']>, metadata: { name: string; projectId: string; saveAs: boolean },
  progress: (value: number) => void): Promise<FileSaveResult> {
  const assets = Object.entries<any>(fileAssets(document)).map(([id, asset]) => [id, { ...asset }] as const);
  const sources = new Map<string, Blob>(), media: SaveMedia[] = [], linked: LinkedRecord[] = [];
  const byId = new Map(assets);
  for (const [id, asset] of assets) {
    // Linked originals stay where they are: a record without bytes.
    if (isLinkedAsset(asset)) {
      linked.push({ id, hash: asset.link.hash, size: asset.link.size, path: asset.link.path, type: asset.type || '' });
      continue;
    }
    // Host media store: main copies the bytes from the store itself.
    const stored = store.saveSource ? await store.saveSource(asset) : null;
    if (stored) {
      media.push({ id, revision: stored.revision, type: stored.type || asset.type || '', length: stored.size, stored: stored.hash });
      continue;
    }
    const saved = store.getForSave ? await store.getForSave(asset) : null;
    const blob = saved?.blob ?? (store.getForSave ? null : await store.get(asset));
    if (!blob) continue;
    sources.set(id, blob);
    media.push({ id, revision: saved?.revision || crypto.randomUUID(), type: blob.type || asset.type || '', length: blob.size });
  }
  const documentBlob = new Blob(await encodeTextChunks(serialized) as BlobPart[]);
  progress(.3);
  const begin = await bridge.begin({ ...metadata, documentBytes: documentBlob.size, media, ...(linked.length ? { linked } : {}) });
  if (!begin.ok) return begin;
  try {
    // Stored media main could not find comes from the page after all.
    for (const id of begin.required) {
      if (sources.has(id)) continue;
      const saved = store.getForSave ? await store.getForSave(byId.get(id)) : await store.get(byId.get(id));
      const blob = saved instanceof Blob ? saved : saved?.blob;
      if (!blob || blob.size !== media.find(item => item.id === id)?.length) throw new Error('The save requested unavailable media.');
      sources.set(id, blob);
    }
    const total = documentBlob.size + begin.required.reduce((sum, id) => sum + (sources.get(id)?.size || 0), 0);
    let uploaded = 0;
    for (const id of [null, ...begin.required]) {
      const blob = id === null ? documentBlob : sources.get(id);
      if (!blob) throw new Error('The save requested unavailable media.');
      for (let offset = 0; offset < blob.size; offset += 1024 * 1024) {
        const data = new Uint8Array(await blob.slice(offset, offset + 1024 * 1024).arrayBuffer());
        await bridge.chunk(begin.token, id, data);
        uploaded += data.length; progress(.3 + .6 * uploaded / total);
      }
    }
    return await bridge.finish(begin.token);
  } finally { await bridge.abort(begin.token).catch(() => undefined); }
}

/** Embed available media; keep missing-media references editable on reopening. */
export async function packProjectFileBlob(snapshot: string | any, store: MediaStore, serialized?: string, onProgress?: (value: number) => void): Promise<Blob> {
  const document = typeof snapshot === 'string' ? JSON.parse(snapshot) : snapshot;
  const assets = fileAssets(document);
  const media: Array<{ id: string; type: string; data: Blob }> = [];
  const entries = Object.entries<any>(assets);
  let processed = 0;
  for (const [id, asset] of entries) {
    const blob = await store.get(asset);
    onProgress?.(++processed / entries.length);
    // Missing sources (including deleted assets retained by undo/redo) must not
    // prevent saving edits. Their metadata stays in the document and history.
    if (!blob) continue;
    media.push({ id, type: blob.type || asset.type || '', data: blob });
  }
  // Linked originals were embedded above: the file is self-contained.
  const embeddedLinks = embedLinkedAssets(document, new Set(media.map(item => item.id)));
  const documentJSON = serialized !== undefined && !embeddedLinks ? serialized : await stringifyAsync(document);
  const documentBytes = await encodeTextChunks(documentJSON);
  const packed = encodeProjectContainerBlob(documentBytes, media);
  return packed;
}

/** Byte API for callers that need a complete container; native Save streams the Blob. */
export async function packProjectFile(snapshot: string | any, store: MediaStore, serialized?: string): Promise<Uint8Array> {
  return new Uint8Array(await (await packProjectFileBlob(snapshot, store, serialized)).arrayBuffer());
}

export async function restoreProjectFileMedia(document: any, store: MediaStore): Promise<void> {
  if (document?.containerMedia) {
    const assets = fileAssets(document);
    for (const source of document.containerMedia as Array<{ id: string; type: string; data: Uint8Array | Blob; sha256?: string; revision?: string }>) {
      const asset = assets[source.id];
      if (!asset || !(source.data instanceof Uint8Array) && !(source.data instanceof Blob)) continue;
      const blob = source.data instanceof Blob ? source.data : new Blob([new Uint8Array(source.data)], { type: source.type });
      if (source.sha256) {
        const digest = new Sha256();
        for (let offset = 0; offset < blob.size; offset += 256 * 1024) {
          digest.update(new Uint8Array(await blob.slice(offset, offset + 256 * 1024).arrayBuffer()));
          await new Promise(resolve => setTimeout(resolve, 0));
        }
        if (digest.digest() !== source.sha256) throw new Error('The project media checksum does not match.');
      }
      if (!await store.put(source.id, blob, asset, source.revision)) {
        throw new Error(`Could not restore ${asset.name || source.id}. Check available disk space.`);
      }
    }
    return;
  }
  if (!document.media) return; // Older files still use the local media store.
  const assets = fileAssets(document);
  const entries: Array<{ id: string; asset: any; blob: Blob }> = [];
  for (const [id, source] of Object.entries<any>(document.media)) {
    const asset = assets[id];
    if (!asset) continue;
    if (!source || typeof source.type !== 'string' || typeof source.data !== 'string') {
      throw new Error(`Invalid saved media: ${asset.name || id}`);
    }
    // A repeated four-character capture can overflow the regex stack on video
    // files. This flat scan stays safe for the full supported project size.
    if (source.data.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(source.data)) {
      throw new Error(`Invalid saved media: ${asset.name || id}`);
    }
    const binary = atob(source.data);
    const data = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index++) data[index] = binary.charCodeAt(index);
    entries.push({ id, asset, blob: new Blob([data], { type: source.type }) });
  }
  for (const { id, asset, blob } of entries) {
    if (!await store.put(id, blob, asset)) throw new Error(`Could not restore ${asset.name || id}. Check available disk space.`);
  }
}

export function unpackProjectFile(input: string | Uint8Array | ArrayBuffer): any {
  const decoded = decodeProjectContainer(input);
  if (decoded.media.length) decoded.document.containerMedia = decoded.media;
  return decoded.document;
}

/** Browser file imports keep source media as file-backed slices. */
export async function unpackProjectFileBlob(file: Blob): Promise<any> {
  const prefix = new Uint8Array(await file.slice(0, 9).arrayBuffer());
  if (isIncrementalProject(prefix)) {
    const index = await readIncrementalIndex(file.size, async (offset, length) => new Uint8Array(await file.slice(offset, offset + length).arrayBuffer()));
    const bytes = new Uint8Array(await file.slice(index.document.offset, index.document.offset + index.document.length).arrayBuffer());
    if (await sha256HexOf(bytes) !== index.document.sha256) throw new Error('The project document checksum does not match.');
    const document = JSON.parse(new TextDecoder().decode(bytes));
    document.containerMedia = index.media.map(item => ({ ...item, data: file.slice(item.offset, item.offset + item.length, item.type) }));
    return document;
  }
  if (!isProjectContainer(prefix)) return JSON.parse(await file.text());
  if (prefix.length < 9) throw new Error('The project container is truncated.');
  const headerLength = new DataView(prefix.buffer).getUint32(5, true);
  if (headerLength < 2 || headerLength > file.size - 9) throw new Error('The project container header is invalid.');
  const index = projectContainerIndex(JSON.parse(await file.slice(9, 9 + headerLength).text()), 9 + headerLength, file.size);
  index.document.containerMedia = index.media.map(item => ({ ...item, data: file.slice(item.offset, item.offset + item.length, item.type) }));
  return index.document;
}

/** Stage native ranges on disk, then commit file-backed Blobs to the media store. */
export async function restoreProjectFileStream(document: any, media: ProjectMediaRange[], store: MediaStore,
  read: (offset: number, length: number) => Promise<Uint8Array>,
  onProgress?: (completed: number, total: number) => void): Promise<void> {
  const assets = fileAssets(document);
  const sources = media.filter(source => assets[source.id]);
  if (!sources.length) { await restoreProjectFileMedia(document, store); return; }
  const total = sources.reduce((sum, source) => sum + source.length, 0);
  let completed = 0;
  const advance = (bytes: number) => { completed += bytes; onProgress?.(completed, total); };
  /* A browser served by `powermove serve` stages in memory: OPFS needs a secure
     context, and current Chromium keeps IndexedDB blobs as references to the
     staged file, which is gone once staging is cleaned up. */
  if ((typeof window !== 'undefined' && (hostBridge() as any)?.remote) || typeof navigator.storage?.getDirectory !== 'function') {
    for (const source of sources) {
      const chunks: Uint8Array[] = [];
      const digest = source.sha256 ? new Sha256() : null;
      for (let offset = 0; offset < source.length;) {
        const length = Math.min(1024 * 1024, source.length - offset);
        const chunk = await read(source.offset + offset, length);
        if (!(chunk instanceof Uint8Array) || !chunk.length || chunk.length > length) throw new Error('The project media is truncated.');
        chunks.push(chunk);
        digest?.update(chunk);
        offset += chunk.length;
        advance(chunk.length);
      }
      if (digest && digest.digest() !== source.sha256) throw new Error('The project media checksum does not match.');
      if (!await store.put(source.id, new Blob(chunks as BlobPart[], { type: source.type }), assets[source.id], source.revision)) {
        throw new Error(`Could not restore ${assets[source.id].name || source.id}. Check available disk space.`);
      }
    }
    return;
  }
  const root = await navigator.storage.getDirectory();
  for (const source of sources) {
    // crypto.randomUUID needs a secure context; the served app is plain http.
    const name = 'project-import-' + (crypto.randomUUID?.() ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`);
    const file = await root.getFileHandle(name, { create: true });
    let writer: FileSystemWritableFileStream | undefined;
    const digest = source.sha256 ? new Sha256() : null;
    try {
      writer = await file.createWritable();
      for (let offset = 0; offset < source.length;) {
        const length = Math.min(1024 * 1024, source.length - offset);
        const chunk = await read(source.offset + offset, length);
        if (!(chunk instanceof Uint8Array) || !chunk.length || chunk.length > length) throw new Error('The project media is truncated.');
        await writer.write(new Uint8Array(chunk));
        digest?.update(chunk);
        offset += chunk.length;
        advance(chunk.length);
      }
      await writer.close(); writer = undefined;
      if (digest && digest.digest() !== source.sha256) throw new Error('The project media checksum does not match.');
      const blob = (await file.getFile()).slice(0, source.length, source.type);
      if (!await store.put(source.id, blob, assets[source.id], source.revision)) {
        throw new Error(`Could not restore ${assets[source.id].name || source.id}. Check available disk space.`);
      }
    } finally {
      await writer?.abort().catch(() => undefined);
      await root.removeEntry(name);
    }
  }
}
