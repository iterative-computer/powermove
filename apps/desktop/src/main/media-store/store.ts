/*
 * The host-owned media store (plan section 21, "Where bytes live").
 *
 * With "Use New Engine" on, project media bytes live here instead of in the
 * renderer's IndexedDB, and the engine reads them through capability
 * handles that main mints (contract 14). Layout under `<userData>/Media
 * Store`:
 *
 *   objects/<2 hex>/<64 hex>   immutable copies named by their BLAKE3 hash
 *   keys/<sha256(key)>.json    a renderer storage key → the object it names
 *   tmp/                       partial writes, swept at launch
 *
 * Objects are content-addressed, so the same bytes imported twice, in two
 * projects, or under two keys are stored once. Every object is hashed from
 * disk after it is written and fsynced, and committed only when that hash
 * is the name it gets (and, for streamed bytes, the hash the sender
 * computed): what the store hands back is what was put in.
 *
 * Nothing here deletes an object. Removing a key forgets the mapping only.
 * Linked files are never written, moved, or deleted by the store; it only
 * reads and hashes them.
 */
import { createHash, randomUUID } from 'node:crypto';
import { constants } from 'node:fs';
import { chmod, copyFile, mkdir, open, readdir, readFile, rename, rm, stat, unlink, type FileHandle } from 'node:fs/promises';
import path from 'node:path';

import { syncDirectory, syncFile } from '../durable-file';
import { hashFile, type FileIdentity, type HashExecutor } from './hasher';
import type { MediaLink, MediaStoreRecord, MediaRecordSource } from '../../shared/media-store';
import { HASH_HEX, validStorageKey } from '../../shared/media-store';

export const STORE_DIRECTORY = 'Media Store';
/** Bytes per streamed chunk (renderer → main and main → renderer). */
export const STREAM_CHUNK_BYTES = 4 * 1024 * 1024;
const MAX_STREAM_BYTES = 256 * 1024 ** 3;
const MAX_TYPE = 200;
const MAX_REVISION = 200;

export class MediaStoreError extends Error {
  override name = 'MediaStoreError';
  constructor(message: string, readonly code: 'notFound' | 'invalid' | 'mismatch' | 'changed' | 'io' = 'invalid') { super(message); }
}

type Stream = {
  owner: number;
  key: string | null;
  type: string;
  size: number;
  revision: string | null;
  source: MediaRecordSource;
  handle: FileHandle;
  file: string;
  received: number;
  queue: Promise<void>;
  sha256: ReturnType<typeof createHash>;
  /** Every chunk so far was read by main itself (a project file range). */
  readByMain: boolean;
};

export type MediaStoreOptions = {
  root: string;
  executor: HashExecutor;
  now?: () => number;
  log?: (message: string) => void;
};

/** SHA-256 of a file read back from disk (libuv reads; native hashing). */
async function sha256OfFile(file: string): Promise<string> {
  const digest = createHash('sha256');
  const handle = await open(file, 'r');
  try {
    for await (const chunk of handle.createReadStream({ autoClose: false, highWaterMark: STREAM_CHUNK_BYTES })) digest.update(chunk as Buffer);
  } finally { await handle.close(); }
  return digest.digest('hex');
}

const keyFile = (key: string) => `${createHash('sha256').update(key).digest('hex')}.json`;

function validRecord(value: unknown, key: string): value is MediaStoreRecord {
  const record = value as MediaStoreRecord;
  return !!record && typeof record === 'object' && record.key === key && HASH_HEX.test(record.hash)
    && Number.isSafeInteger(record.size) && record.size >= 0 && typeof record.type === 'string' && record.type.length <= MAX_TYPE
    && typeof record.revision === 'string' && record.revision.length > 0 && record.revision.length <= MAX_REVISION
    && Number.isFinite(record.at);
}

export class MediaStore {
  readonly root: string;
  private readonly objects: string;
  private readonly keys: string;
  private readonly tmp: string;
  private readonly now: () => number;
  private readonly streams = new Map<string, Stream>();
  private ready: Promise<void> | null = null;

  constructor(private readonly options: MediaStoreOptions) {
    this.root = options.root;
    this.objects = path.join(this.root, 'objects');
    this.keys = path.join(this.root, 'keys');
    this.tmp = path.join(this.root, 'tmp');
    this.now = options.now ?? Date.now;
  }

  /** Creates the folders (owner-only) and removes partial writes a crash left. */
  init(): Promise<void> {
    this.ready ??= (async () => {
      for (const directory of [this.root, this.objects, this.keys]) await mkdir(directory, { recursive: true, mode: 0o700 });
      await rm(this.tmp, { recursive: true, force: true });
      await mkdir(this.tmp, { recursive: true, mode: 0o700 });
    })();
    return this.ready;
  }

  objectPath(hash: string): string {
    if (!HASH_HEX.test(hash)) throw new MediaStoreError('A content hash is 64 hex digits.');
    return path.join(this.objects, hash.slice(0, 2), hash);
  }

  async hasObject(hash: string, size?: number): Promise<boolean> {
    try {
      const info = await stat(this.objectPath(hash));
      return info.isFile() && (size === undefined || info.size === size);
    } catch { return false; }
  }

  /** The record a key names, if its object is present. */
  async record(key: string): Promise<MediaStoreRecord | null> {
    if (!validStorageKey(key)) throw new MediaStoreError('Invalid media storage key.');
    await this.init();
    let value: unknown;
    try { value = JSON.parse(await readFile(path.join(this.keys, keyFile(key)), 'utf8')); }
    catch { return null; }
    if (!validRecord(value, key)) return null;
    return await this.hasObject(value.hash, value.size) ? value : null;
  }

  /** Every key record (diagnostics and tests). */
  async records(): Promise<MediaStoreRecord[]> {
    await this.init();
    const out: MediaStoreRecord[] = [];
    for (const name of await readdir(this.keys)) {
      if (!name.endsWith('.json')) continue;
      try {
        const value = JSON.parse(await readFile(path.join(this.keys, name), 'utf8')) as MediaStoreRecord;
        if (value && typeof value.key === 'string' && validRecord(value, value.key)) out.push(value);
      } catch { /* a damaged record names nothing */ }
    }
    return out;
  }

  /** Forgets a key. The object stays (other keys and projects may use it). */
  async removeKey(key: string): Promise<boolean> {
    if (!validStorageKey(key)) throw new MediaStoreError('Invalid media storage key.');
    await this.init();
    try { await unlink(path.join(this.keys, keyFile(key))); return true; }
    catch { return false; }
  }

  /**
   * Copies a file into the store (Copy into project). On APFS the copy is a
   * clone, so it costs no space until either side changes. The copy is
   * hashed from disk; a source that changes while it is copied is copied
   * again.
   */
  async putFile(source: string, options: { key?: string | null; type?: string; revision?: string | null; origin?: MediaRecordSource; progress?: (done: number, total: number) => void } = {}): Promise<MediaStoreRecord & { identity: FileIdentity }> {
    if (!path.isAbsolute(source)) throw new MediaStoreError('A media file path must be absolute.');
    if (options.key != null && !validStorageKey(options.key)) throw new MediaStoreError('Invalid media storage key.');
    await this.init();
    for (let attempt = 1; ; attempt++) {
      const before = await stat(source);
      if (!before.isFile()) throw new MediaStoreError(`“${source}” is not a file.`);
      const temporary = path.join(this.tmp, `${randomUUID()}.part`);
      try {
        await copyFile(source, temporary, constants.COPYFILE_EXCL | constants.COPYFILE_FICLONE);
        const after = await stat(source);
        if (after.size !== before.size || after.mtimeMs !== before.mtimeMs || after.ino !== before.ino) {
          if (attempt < 3) { await rm(temporary, { force: true }); continue; }
          throw new MediaStoreError(`“${path.basename(source)}” kept changing while it was copied.`, 'changed');
        }
        const handle = await open(temporary, 'r+');
        try { await syncFile(handle); } finally { await handle.close(); }
        const identity = await hashFile(this.options.executor, temporary, { settleMs: 0, ...(options.progress ? { progress: options.progress } : {}) });
        if (identity.size !== before.size) throw new MediaStoreError('The copy is incomplete.', 'mismatch');
        await this.commitObject(temporary, identity.hash, identity.size);
        const record = await this.writeKey(options.key ?? null, identity.hash, identity.size, options.type ?? '', options.revision ?? null, options.origin ?? 'import');
        return { ...record, identity: { ...identity, mtimeMs: before.mtimeMs, dev: before.dev, ino: before.ino } };
      } finally {
        await rm(temporary, { force: true });
      }
    }
  }

  /** Starts a streamed put (bytes from the renderer, such as converted
   *  media or IndexedDB migration). */
  async beginStream(owner: number, request: { key: string | null; type: string; size: number; revision?: string | null; origin?: MediaRecordSource }): Promise<string> {
    if (request.key !== null && !validStorageKey(request.key)) throw new MediaStoreError('Invalid media storage key.');
    if (!Number.isSafeInteger(request.size) || request.size < 0 || request.size > MAX_STREAM_BYTES) throw new MediaStoreError('Invalid media size.');
    if (typeof request.type !== 'string' || request.type.length > MAX_TYPE) throw new MediaStoreError('Invalid media type.');
    if (request.revision != null && (typeof request.revision !== 'string' || !request.revision || request.revision.length > MAX_REVISION)) throw new MediaStoreError('Invalid media revision.');
    await this.init();
    const file = path.join(this.tmp, `${randomUUID()}.part`);
    const handle = await open(file, 'wx', 0o600);
    const token = randomUUID().replaceAll('-', '');
    this.streams.set(token, {
      owner, key: request.key, type: request.type, size: request.size, revision: request.revision ?? null,
      source: request.origin ?? 'stream', handle, file, received: 0, queue: Promise.resolve(), sha256: createHash('sha256'), readByMain: true
    });
    return token;
  }

  /** Appends bytes to an upload. `from` is `main` only for bytes main read
   *  itself (a range of a project file the window opened). */
  async chunk(owner: number, token: string, offset: number, data: Uint8Array, from: 'page' | 'main' = 'page'): Promise<void> {
    const stream = this.streams.get(token);
    if (!stream || stream.owner !== owner) throw new MediaStoreError('Unknown media upload.', 'notFound');
    if (!(data instanceof Uint8Array) || data.byteLength < 1 || data.byteLength > STREAM_CHUNK_BYTES || offset !== stream.received || offset + data.byteLength > stream.size) {
      throw new MediaStoreError('Invalid media upload chunk.');
    }
    if (from !== 'main') stream.readByMain = false;
    stream.received += data.byteLength;
    stream.sha256.update(data);
    stream.queue = stream.queue.then(async () => {
      let written = 0;
      while (written < data.byteLength) {
        const { bytesWritten } = await stream.handle.write(data, written, data.byteLength - written, offset + written);
        written += bytesWritten;
      }
    });
    await stream.queue;
  }

  /**
   * Finishes a streamed put. The object is committed only when the bytes on
   * disk are the bytes that were sent:
   *
   *  - `blake3` is the sender's hash of the bytes it read from its source
   *    (the renderer's uploads); the copy on disk must hash to it.
   *  - `sha256` is the checksum a project file records for a media range
   *    (main's own copies out of an open project, `putProjectRange`). The
   *    bytes received, and the copy read back from disk, must both match it.
   *  - Neither, only for bytes main read itself from a project file that
   *    records no checksum (PMV3): the copy read back from disk must match
   *    the bytes received.
   */
  async finishStream(owner: number, token: string, expected: { blake3?: string; sha256?: string }): Promise<MediaStoreRecord & { sha256: string }> {
    const stream = this.streams.get(token);
    if (!stream || stream.owner !== owner) throw new MediaStoreError('Unknown media upload.', 'notFound');
    this.streams.delete(token);
    try {
      await stream.queue;
      if (stream.received !== stream.size) throw new MediaStoreError('The media upload is incomplete.', 'mismatch');
      await syncFile(stream.handle);
      await stream.handle.close();
      const blake3 = expected?.blake3;
      const sha256 = expected?.sha256;
      const byBlake3 = typeof blake3 === 'string' && HASH_HEX.test(blake3);
      const bySha256 = typeof sha256 === 'string' && HASH_HEX.test(sha256);
      if (!byBlake3 && !bySha256 && !stream.readByMain) throw new MediaStoreError('A media upload names its BLAKE3 hash or its SHA-256 checksum.');
      const received = stream.sha256.digest('hex');
      if (bySha256 && received !== sha256) throw new MediaStoreError('The project media checksum does not match.', 'mismatch');
      if (!byBlake3 && await sha256OfFile(stream.file) !== received) {
        throw new MediaStoreError('The stored copy does not match the bytes that were read.', 'mismatch');
      }
      const identity = await hashFile(this.options.executor, stream.file, { settleMs: 0 });
      if ((byBlake3 && identity.hash !== blake3) || identity.size !== stream.size) {
        throw new MediaStoreError('The stored copy does not match the original bytes.', 'mismatch');
      }
      await this.commitObject(stream.file, identity.hash, identity.size);
      const record = await this.writeKey(stream.key, identity.hash, identity.size, stream.type, stream.revision, stream.source);
      return { ...record, sha256: received };
    } finally {
      await stream.handle.close().catch(() => undefined);
      await rm(stream.file, { force: true });
    }
  }

  async abortStream(owner: number, token: string): Promise<void> {
    const stream = this.streams.get(token);
    if (!stream || stream.owner !== owner) return;
    this.streams.delete(token);
    await stream.queue.catch(() => undefined);
    await stream.handle.close().catch(() => undefined);
    await rm(stream.file, { force: true });
  }

  /** Drops a window's unfinished uploads (the window went away). */
  async abandon(owner: number): Promise<void> {
    for (const [token, stream] of [...this.streams]) if (stream.owner === owner) await this.abortStream(owner, token);
  }

  /** Hashes a file where it is (Link to files). Nothing is written. */
  async identify(file: string, progress?: (done: number, total: number) => void): Promise<FileIdentity> {
    if (!path.isAbsolute(file)) throw new MediaStoreError('A media file path must be absolute.');
    return hashFile(this.options.executor, file, { settleMs: 1000, ...(progress ? { progress } : {}) });
  }

  /**
   * Checks that a linked file is still the content it was linked as. The
   * same file (volume and file number, when recorded) with an unchanged size
   * and modification time is enough; anything else, such as another file
   * copied over the path with its dates kept, is hashed again.
   */
  async verifyLink(link: MediaLink): Promise<{ status: 'ok'; path: string; identity: FileIdentity | null } | { status: 'missing' | 'changed' }> {
    let info;
    try { info = await stat(link.path); } catch { return { status: 'missing' }; }
    if (!info.isFile()) return { status: 'missing' };
    if (info.size !== link.size) return { status: 'changed' };
    const sameFile = (link.ino === undefined || info.ino === link.ino) && (link.dev === undefined || info.dev === link.dev);
    if (link.mtimeMs !== undefined && info.mtimeMs === link.mtimeMs && sameFile) return { status: 'ok', path: link.path, identity: null };
    try {
      const identity = await hashFile(this.options.executor, link.path, { settleMs: 0 });
      return identity.hash === link.hash ? { status: 'ok', path: link.path, identity } : { status: 'changed' };
    } catch { return { status: 'missing' }; }
  }

  /** Moves a verified temporary file into `objects/` under its hash. */
  private async commitObject(temporary: string, hash: string, size: number): Promise<void> {
    const target = this.objectPath(hash);
    if (await this.hasObject(hash, size)) return;
    await mkdir(path.dirname(target), { recursive: true, mode: 0o700 });
    await chmod(temporary, 0o400);
    await rename(temporary, target);
    await syncDirectory(path.dirname(target));
  }

  private async writeKey(key: string | null, hash: string, size: number, type: string, revision: string | null, source: MediaRecordSource): Promise<MediaStoreRecord> {
    const record: MediaStoreRecord = { key: key ?? `blake3:${hash}`, hash, size, type: type.slice(0, MAX_TYPE), revision: revision || hash, at: this.now(), source };
    if (key === null) return record;
    const file = path.join(this.keys, keyFile(key));
    const temporary = path.join(this.tmp, `${randomUUID()}.json`);
    const handle = await open(temporary, 'wx', 0o600);
    try {
      await handle.writeFile(JSON.stringify(record));
      await syncFile(handle);
    } finally { await handle.close(); }
    await rename(temporary, file);
    await syncDirectory(this.keys);
    return record;
  }
}
