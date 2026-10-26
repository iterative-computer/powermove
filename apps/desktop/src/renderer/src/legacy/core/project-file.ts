import { decodeProjectContainer, encodeProjectContainerBlob, encodeTextChunks, isProjectContainer, projectContainerIndex, type ProjectMediaRange } from '../../../../shared/project-container';
import { stringifyAsync } from './serialize-async';
import { parseAsync } from './parse-async';
import { bridge as hostBridge } from '../../kernel/bridge';
import { isIncrementalProject, readIncrementalIndex, type LinkedRecord, type ProjectRecord, type SaveMedia } from '../../../../shared/project-incremental';
import { Sha256, sha256HexOf } from '../../../../shared/sha256';
import type { PowermoveBridge, FileSaveResult } from '../../../../shared/ipc';

type MediaStore = { get(asset: any): Promise<Blob | null>; put(id: string, blob: Blob, metadata: any, revision?: string): Promise<boolean>;
  getForSave?(asset: any): Promise<{ blob: Blob; revision: string } | null>;
  /** Host media store (new engine): bytes main copies into the save itself. */
  saveSource?(asset: any): Promise<{ hash: string; size: number; type: string; revision: string } | null>;
  /** Host media store (new engine): main copies a range of the open project file into the store. */
  putProjectRange?(readToken: string, source: ProjectMediaRange, asset: any, onBytes?: (bytes: number) => void): Promise<boolean> };

import { embedLinkedAssets, fileAssets, isLinkedAsset } from '../../../../shared/project-assets';
export { embedLinkedAssets } from '../../../../shared/project-assets';

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
    const document = await parseAsync(new TextDecoder().decode(bytes));
    document.containerMedia = index.media.map(item => ({ ...item, data: file.slice(item.offset, item.offset + item.length, item.type) }));
    return document;
  }
  if (!isProjectContainer(prefix)) return parseAsync(await file.text());
  if (prefix.length < 9) throw new Error('The project container is truncated.');
  const headerLength = new DataView(prefix.buffer).getUint32(5, true);
  if (headerLength < 2 || headerLength > file.size - 9) throw new Error('The project container header is invalid.');
  const index = projectContainerIndex(await parseAsync(await file.slice(9, 9 + headerLength).text()), 9 + headerLength, file.size);
  index.document.containerMedia = index.media.map(item => ({ ...item, data: file.slice(item.offset, item.offset + item.length, item.type) }));
  return index.document;
}

/** Large document graphs must not cross Electron's context bridge. Decode the
 * validated file range through bounded reads, retaining project and Undo data. */
export async function readProjectFileDocument(range: ProjectRecord, size: number,
  read: (offset: number, length: number) => Promise<Uint8Array>,
  onProgress?: (completed: number, total: number) => void): Promise<any> {
  if (!range || !Number.isSafeInteger(size) || !Number.isSafeInteger(range.offset) || range.offset < 0
      || !Number.isSafeInteger(range.length) || range.length < 2 || range.offset > size || range.length > size - range.offset
      || typeof range.sha256 !== 'string' || !/^[a-f0-9]{64}$/.test(range.sha256)) throw new Error('The project document range is invalid.');
  const parts: string[] = [], decoder = new TextDecoder(), digest = new Sha256();
  for (let offset = 0; offset < range.length;) {
    const length = Math.min(1024 * 1024, range.length - offset);
    const chunk = await read(range.offset + offset, length);
    if (!(chunk instanceof Uint8Array) || !chunk.length || chunk.length > length) throw new Error('The project document is truncated.');
    digest.update(chunk);
    parts.push(decoder.decode(chunk, { stream: true }));
    offset += chunk.length;
    onProgress?.(offset, range.length);
  }
  parts.push(decoder.decode());
  if (digest.digest() !== range.sha256) throw new Error('The project document checksum does not match.');
  return parseAsync(parts.join(''));
}

/** Stage native ranges on disk, then commit file-backed Blobs to the media store. */
/** Runs `work` (restoring a project streamed by main's project reader), then
 *  closes the reader. Closing is cleanup: main expires a reader 120 s after
 *  its last completed read if it remains idle, so a failed close neither
 *  fails an open that read every byte nor hides the restore's own error. */
export async function withProjectReader<T>(work: () => Promise<T>, close: () => Promise<unknown>): Promise<T> {
  try { return await work(); } finally { await close().catch(() => undefined); }
}

export async function restoreProjectFileStream(document: any, media: ProjectMediaRange[], store: MediaStore,
  read: (offset: number, length: number) => Promise<Uint8Array>,
  onProgress?: (completed: number, total: number) => void, readToken?: string): Promise<void> {
  const assets = fileAssets(document);
  let sources = media.filter(source => assets[source.id]);
  if (!sources.length) { await restoreProjectFileMedia(document, store); return; }
  const total = sources.reduce((sum, source) => sum + source.length, 0);
  let completed = 0;
  const advance = (bytes: number) => { completed += bytes; onProgress?.(completed, total); };
  /* New engine: main copies each range from the open file into the media
     store itself, verified against the file's checksum, so the bytes never
     pass through this page (section 21). */
  if (readToken && store.putProjectRange) {
    /* A few copies at a time: each one ends with main syncing and hashing
       its file, which leaves the reader idle (a project with fifty embedded
       stills spent 1.7 s here one by one). */
    const copiedBy = new Map<ProjectMediaRange, boolean>();
    let next = 0, refused: unknown = null;
    const copy = async () => {
      while (next < sources.length && refused === null) {
        const source = sources[next++]!;
        let copied = 0;
        const stored = await store.putProjectRange!(readToken, source, assets[source.id], bytes => { copied += bytes; advance(bytes); })
          .catch((error: unknown) => {
            // A checksum mismatch is the file's fault: the page would find the same bytes.
            if (/checksum/i.test(String((error as Error)?.message))) refused ??= error;
            return false;
          });
        if (!stored) completed -= copied;
        copiedBy.set(source, stored);
      }
    };
    await Promise.all(Array.from({ length: Math.min(PROJECT_RANGE_COPIES, sources.length) }, copy));
    if (refused !== null) throw refused;
    const remaining = sources.filter(source => !copiedBy.get(source));
    if (!remaining.length) return;
    sources = remaining;
  }
  /* Each range becomes a Blob built from its bytes. Staging in OPFS does not
     work: Chromium keeps an IndexedDB blob that came from an OPFS file as a
     reference to that file, so removing the staging file broke the media
     just stored (the asset was then re-imported from its source path with a
     new revision, or went missing). Parts are folded into a Blob every
     64 MiB, so the page never holds a large file as one array; the browser
     owns the folded Blob's bytes. */
  for (const source of sources) {
    const digest = source.sha256 ? new Sha256() : null;
    let folded = new Blob([], { type: source.type });
    let parts: Uint8Array[] = [];
    let partBytes = 0;
    for (let offset = 0; offset < source.length;) {
      const length = Math.min(1024 * 1024, source.length - offset);
      const chunk = await read(source.offset + offset, length);
      if (!(chunk instanceof Uint8Array) || !chunk.length || chunk.length > length) throw new Error('The project media is truncated.');
      parts.push(chunk);
      partBytes += chunk.length;
      digest?.update(chunk);
      offset += chunk.length;
      advance(chunk.length);
      if (partBytes >= RESTORE_FOLD_BYTES) {
        folded = new Blob([folded, ...parts as BlobPart[]], { type: source.type });
        parts = [];
        partBytes = 0;
      }
    }
    if (digest && digest.digest() !== source.sha256) throw new Error('The project media checksum does not match.');
    const blob = new Blob([folded, ...parts as BlobPart[]], { type: source.type });
    if (!await store.put(source.id, blob, assets[source.id], source.revision)) {
      throw new Error(`Could not restore ${assets[source.id].name || source.id}. Check available disk space.`);
    }
  }
}

/** Embedded media ranges main copies into the media store at once. */
const PROJECT_RANGE_COPIES = 8;
/** Bytes of restored media the page holds before folding them into a Blob. */
const RESTORE_FOLD_BYTES = 64 * 1024 * 1024;
