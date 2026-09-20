import { decodeProjectContainer, encodeProjectContainerBlob, encodeTextChunks, isProjectContainer, projectContainerIndex, type ProjectMediaRange } from '../../../../shared/project-container';
import { stringifyAsync } from './serialize-async';

type MediaStore = { get(asset: any): Promise<Blob | null>; put(id: string, blob: Blob, metadata: any): Promise<boolean> };

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

/** Embed available media; keep missing-media references editable on reopening. */
export async function packProjectFileBlob(snapshot: string | any, store: MediaStore, serialized?: string, onProgress?: (value: number) => void): Promise<Blob> {
  const document = typeof snapshot === 'string' ? JSON.parse(snapshot) : snapshot;
  const assets = fileAssets(document);
  const media: Array<{ id: string; type: string; data: Blob }> = [];
  const documentJSON = serialized ?? await stringifyAsync(document);
  const documentBytes = await encodeTextChunks(documentJSON);
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
    for (const source of document.containerMedia as Array<{ id: string; type: string; data: Uint8Array | Blob }>) {
      const asset = assets[source.id];
      if (!asset || !(source.data instanceof Uint8Array) && !(source.data instanceof Blob)) continue;
      if (!await store.put(source.id, source.data instanceof Blob ? source.data : new Blob([new Uint8Array(source.data)], { type: source.type }), asset)) {
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
  read: (offset: number, length: number) => Promise<Uint8Array>): Promise<void> {
  const assets = fileAssets(document);
  const sources = media.filter(source => assets[source.id]);
  if (!sources.length) { await restoreProjectFileMedia(document, store); return; }
  /* A browser served by `powermove serve` stages in memory: OPFS needs a secure
     context, and current Chromium keeps IndexedDB blobs as references to the
     staged file, which is gone once staging is cleaned up. */
  if ((typeof window !== 'undefined' && (window as any).powermove?.remote) || typeof navigator.storage?.getDirectory !== 'function') {
    for (const source of sources) {
      const chunks: Uint8Array[] = [];
      for (let offset = 0; offset < source.length;) {
        const length = Math.min(1024 * 1024, source.length - offset);
        const chunk = await read(source.offset + offset, length);
        if (!(chunk instanceof Uint8Array) || !chunk.length || chunk.length > length) throw new Error('The project media is truncated.');
        chunks.push(chunk);
        offset += chunk.length;
      }
      if (!await store.put(source.id, new Blob(chunks as BlobPart[], { type: source.type }), assets[source.id])) {
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
    try {
      writer = await file.createWritable();
      for (let offset = 0; offset < source.length;) {
        const length = Math.min(1024 * 1024, source.length - offset);
        const chunk = await read(source.offset + offset, length);
        if (!(chunk instanceof Uint8Array) || !chunk.length || chunk.length > length) throw new Error('The project media is truncated.');
        await writer.write(new Uint8Array(chunk));
        offset += chunk.length;
      }
      await writer.close(); writer = undefined;
      const blob = (await file.getFile()).slice(0, source.length, source.type);
      if (!await store.put(source.id, blob, assets[source.id])) {
        throw new Error(`Could not restore ${assets[source.id].name || source.id}. Check available disk space.`);
      }
    } finally {
      await writer?.abort().catch(() => undefined);
      await root.removeEntry(name);
    }
  }
}
