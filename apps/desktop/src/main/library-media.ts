import { createHash, randomUUID, type Hash } from 'node:crypto';
import { constants } from 'node:fs';
import { lstat, mkdir, open, readFile, readdir, rename, unlink, writeFile, type FileHandle } from 'node:fs/promises';
import path from 'node:path';
import { Readable } from 'node:stream';
import type { IpcMain, IpcMainInvokeEvent } from 'electron';
import { LIBRARY_MEDIA_CHUNK, LIBRARY_MEDIA_IPC, type ImportedArtifact, type LibraryMediaInput, type LibraryMediaLink, type LibraryMediaRecord } from '../shared/library-media';
import { PROJECT_ID } from '../shared/ipc';
import { hashHandle, readExactly, sameFile, syncDirectory } from './durable-file';
async function writeLibraryJSON(file: string, value: unknown): Promise<void> {
  await mkdir(path.dirname(file), { recursive: true });
  const temporary = `${file}.${randomUUID()}.tmp`;
  try {
    await writeFile(temporary, JSON.stringify(value), { mode: 0o600, flag: 'wx' });
    const handle = await open(temporary, 'r');
    try { await handle.sync(); } finally { await handle.close(); }
    await rename(temporary, file);
    await syncDirectory(path.dirname(file));
  } finally { await unlink(temporary).catch(() => undefined); }
}

type Record = { key: string; revision: string; size: number; type: string; file?: string; sourcePath?: string; sha256: string; mtime: number };
type Upload = { owner: number; input: LibraryMediaInput; file: string; handle: FileHandle; hash: Hash; received: number; queue: Promise<void> };
const KEY = /^[A-Za-z0-9:._-]{1,256}$/;
const FILE = /^[a-f0-9]{64}\.(?:png|jpg|webp|gif|mp4|mov|webm|mp3|wav|ogg|flac|bin)$/;
const extensions: { [type: string]: string } = { 'image/png': 'png', 'image/jpeg': 'jpg', 'image/webp': 'webp', 'image/gif': 'gif',
  'video/mp4': 'mp4', 'video/quicktime': 'mov', 'video/webm': 'webm', 'audio/webm': 'webm', 'audio/mp4': 'mp4',
  'audio/mpeg': 'mp3', 'audio/wav': 'wav', 'audio/ogg': 'ogg', 'audio/flac': 'flac' };
const indexName = (key: string) => createHash('sha256').update(key).digest('hex') + '.json';
const validKey = (key: unknown): key is string => typeof key === 'string' && KEY.test(key);
const validPath = (value: unknown): value is string => typeof value === 'string' && path.isAbsolute(value) && !value.includes('\0') && value.length <= 16_384;
const artifactKey = (reference: ImportedArtifact): string => `artifact:${reference.projectId}:${createHash('sha256').update(reference.path).digest('hex')}`;

/** One durable copy of a payload, shared by all projects that import it.
 * Renderer requests name keys and capabilities, never filesystem paths. */
export class LibraryMedia {
  private uploads = new Map<string, Upload>();
  private grants = new Map<string, { owner: number; record: Record }>();
  private queue: Promise<unknown> = Promise.resolve();
  constructor(readonly root: string, private readonly artifactSource?: (reference: ImportedArtifact) => Promise<string>,
    private readonly sourceAvailable: (source: string) => Promise<boolean> = async () => true) {}
  async initialize(): Promise<void> {
    for (const folder of [this.root, path.join(this.root, 'Index'), path.join(this.root, '.incoming')]) {
      await mkdir(folder, { recursive: true });
      if (!(await lstat(folder)).isDirectory()) throw new Error('The media library contains an unsafe folder link.');
    }
    // Only main's incomplete uploads live here; no editors exist at startup.
    for (const file of await readdir(path.join(this.root, '.incoming'))) {
      if (/^[a-f0-9-]{36}\.part$/.test(file)) await unlink(path.join(this.root, '.incoming', file));
    }
    for (const file of await readdir(path.join(this.root, 'Index'))) {
      if (/^[a-f0-9]{64}\.json\.[a-f0-9-]{36}\.tmp$/.test(file)) await unlink(path.join(this.root, 'Index', file));
    }
  }
  private serial<T>(action: () => Promise<T>): Promise<T> {
    const task = this.queue.then(action);
    this.queue = task.then(() => undefined, () => undefined);
    return task;
  }
  private async record(key: string, metadataOnly = false): Promise<Record | null> {
    if (!validKey(key)) throw new Error('Invalid media key.');
    let record: Record;
    try {
      const file = path.join(this.root, 'Index', indexName(key));
      const handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW);
      try { record = JSON.parse(await handle.readFile('utf8')); } finally { await handle.close(); }
    } catch (error: any) { if (error.code === 'ENOENT') return null; throw error; }
    if (record.key !== key || !(record.sourcePath !== undefined
      ? validPath(record.sourcePath) && record.file === undefined : typeof record.file === 'string' && FILE.test(record.file))
      || !/^[a-f0-9]{64}$/.test(record.sha256) || typeof record.revision !== 'string' || typeof record.type !== 'string'
      || !Number.isSafeInteger(record.size) || record.size < 0 || !Number.isFinite(record.mtime)) throw new Error('A media record is damaged.');
    try { if (!metadataOnly) await this.validateFile(record); }
    catch (error) {
      if (record.sourcePath) throw new Error('The source file is missing or changed. Use Locate File to relink it.', { cause: error });
      throw error;
    }
    return record;
  }
  private pathFor(record: Record): string { return record.sourcePath ?? path.join(this.root, record.file!); }
  private async validateFile(record: Record): Promise<void> {
    // Metadata probes must not silently materialize an offloaded cloud file.
    // The renderer's existing cloud prompt owns that download decision.
    if (record.sourcePath && !await this.sourceAvailable(record.sourcePath)) throw new Error('The source file needs to be downloaded.');
    const file = this.pathFor(record), metadata = await lstat(file);
    if (!metadata.isFile() || metadata.size !== record.size) throw new Error('A media file was changed or removed.');
    if (metadata.mtimeMs !== record.mtime) {
      const handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW);
      try {
        if (!sameFile(metadata, await handle.stat()) || (await hashHandle(handle)).digest('hex') !== record.sha256
          || !sameFile(metadata, await handle.stat())) throw new Error('A media file was changed outside Powermove.');
      } finally { await handle.close(); }
    }
  }
  private grant(owner: number, record: Record): LibraryMediaRecord {
    for (const [token, grant] of this.grants) {
      if (grant.owner === owner && grant.record.key === record.key && grant.record.revision === record.revision
        && grant.record.sha256 === record.sha256 && grant.record.type === record.type
        && this.pathFor(grant.record) === this.pathFor(record)) {
        return this.publicRecord(record, token);
      }
    }
    const token = randomUUID();
    this.grants.set(token, { owner, record });
    return this.publicRecord(record, token);
  }
  private publicRecord(record: Record, token: string): LibraryMediaRecord {
    return { revision: record.revision, size: record.size, type: record.type, sha256: record.sha256,
      ...(record.sourcePath ? { sourcePath: record.sourcePath } : {}), url: `app://powermove/_media/${token}` };
  }
  async get(owner: number, key: string, metadataOnly = false): Promise<LibraryMediaRecord | null> {
    if (typeof metadataOnly !== 'boolean') throw new Error('Invalid media request.');
    const record = await this.record(key, metadataOnly);
    return record ? this.grant(owner, record) : null;
  }
  async fileFor(key: string): Promise<string | null> {
    const record = await this.record(key);
    return record ? this.pathFor(record) : null;
  }
  /** Persist only a link. A cache migration must match the complete previous
   * payload and revision before relinquishing its owned copy. */
  link(owner: number, input: LibraryMediaLink): Promise<LibraryMediaRecord> {
    return this.serial(async () => {
      if (!input || !validKey(input.key) || !validPath(input.sourcePath) || !Number.isSafeInteger(input.size) || input.size < 0
        || input.size > 64 * 1024 ** 3 || typeof input.type !== 'string' || !/^[\w.+-]+\/[\w.+-]+$/.test(input.type)
        || input.revision !== undefined && (typeof input.revision !== 'string' || input.revision.length > 256)
        || input.matchExisting !== undefined && typeof input.matchExisting !== 'boolean') throw new Error('Invalid media link.');
      const previous = input.matchExisting ? await this.record(input.key) : null;
      if (input.matchExisting && (!previous || previous.revision !== input.revision)) throw new Error('The source does not match the stored media.');
      const sourcePath = path.normalize(input.sourcePath);
      if (!await this.sourceAvailable(sourcePath)) throw new Error('Download the source file before linking it.');
      const metadata = await lstat(sourcePath);
      if (!metadata.isFile() || metadata.size !== input.size) throw new Error('The selected source file changed. Import it again.');
      const handle = await open(sourcePath, constants.O_RDONLY | constants.O_NOFOLLOW);
      let sha256: string;
      try {
        if (!sameFile(metadata, await handle.stat())) throw new Error('The selected source file changed.');
        sha256 = (await hashHandle(handle)).digest('hex');
        if (!sameFile(metadata, await handle.stat()) || !sameFile(metadata, await lstat(sourcePath))) throw new Error('The selected source file changed.');
      } finally { await handle.close(); }
      if (previous && (previous.sha256 !== sha256 || previous.size !== input.size)) throw new Error('The source does not match the stored media.');
      const record: Record = { key: input.key, sourcePath, size: input.size, type: input.type,
        revision: input.revision || randomUUID(), sha256, mtime: metadata.mtimeMs };
      await writeLibraryJSON(path.join(this.root, 'Index', indexName(record.key)), record);
      for (const grant of this.grants.values()) {
        if (grant.record.key === record.key && grant.record.sha256 === record.sha256) grant.record = record;
      }
      return this.grant(owner, record);
    });
  }
  /** Upgrade references are read from saved projects, never guessed by name.
   * A missing/changed original keeps its owned copy. I/O failures still retry. */
  async optimizeSource(key: string, sourcePath: string): Promise<void> {
    if (!validKey(key) || !validPath(sourcePath)) return;
    const relative = path.relative(path.dirname(this.root), sourcePath);
    // Working artifacts and caches inside the app profile are not originals.
    if (relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative)) return;
    const record = await this.record(key, true);
    if (!record || record.sourcePath) return;
    try {
      await this.link(0, { key, sourcePath, size: record.size, type: record.type, revision: record.revision, matchExisting: true });
    } catch (error: any) {
      if (['ENOENT', 'EACCES', 'EPERM', 'ELOOP'].includes(error.code)
        || /does not match|selected source file changed|Download the source/.test(error.message)) return;
      throw error;
    } finally { await this.release(0); }
  }
  /** Remove only hash-named owned copies proven redundant with a live link.
   * Unrelated unindexed payloads retain the ordinary grace period. */
  collectLinkedDuplicates(): Promise<number> {
    return this.serial(async () => {
      const referenced = new Set<string>(), linked = new Map<string, Record>(), sources = new Set<string>();
      for (const entry of await readdir(path.join(this.root, 'Index'))) {
        if (!/^[a-f0-9]{64}\.json$/.test(entry)) continue;
        const value = JSON.parse(await readFile(path.join(this.root, 'Index', entry), 'utf8')) as Record;
        const record = await this.record(value.key, true);
        if (record?.file) referenced.add(record.file);
        else if (record?.sourcePath) { linked.set(record.sha256, record); sources.add(path.resolve(record.sourcePath)); }
      }
      for (const grant of this.grants.values()) if (grant.record.file) referenced.add(grant.record.file);
      let freed = 0;
      for (const file of await readdir(this.root)) {
        if (!FILE.test(file) || referenced.has(file)) continue;
        const record = linked.get(file.slice(0, 64));
        if (!record || !await this.sourceAvailable(record.sourcePath!)) continue;
        let source: FileHandle | undefined, owned: FileHandle | undefined;
        try {
          source = await open(record.sourcePath!, constants.O_RDONLY | constants.O_NOFOLLOW);
          const before = await source.stat();
          if (!before.isFile() || before.size !== record.size || (await hashHandle(source)).digest('hex') !== record.sha256) continue;
          const target = path.join(this.root, file);
          if (sources.has(target)) continue;
          const metadata = await lstat(target);
          if (!metadata.isFile() || metadata.size !== record.size) continue;
          owned = await open(target, constants.O_RDONLY | constants.O_NOFOLLOW);
          if (!sameFile(metadata, await owned.stat()) || (await hashHandle(owned)).digest('hex') !== record.sha256
            || !sameFile(metadata, await lstat(target)) || !sameFile(before, await source.stat())
            || !sameFile(before, await lstat(record.sourcePath!))) continue;
          await unlink(target); freed += metadata.size;
        } catch (error: any) {
          if (!['ENOENT', 'EACCES', 'EPERM', 'ELOOP'].includes(error.code)) throw error;
        } finally { await source?.close(); await owned?.close(); }
      }
      return freed;
    });
  }
  async begin(owner: number, raw: LibraryMediaInput): Promise<string> {
    if (!raw || !validKey(raw.key) || !Number.isSafeInteger(raw.size) || raw.size < 0 || raw.size > 64 * 1024 ** 3
      || typeof raw.type !== 'string' || !/^[\w.+-]+\/[\w.+-]+$/.test(raw.type)
      || raw.revision !== undefined && (typeof raw.revision !== 'string' || raw.revision.length > 256)
      || raw.onlyIfMissing !== undefined && typeof raw.onlyIfMissing !== 'boolean') throw new Error('Invalid media upload.');
    if (raw.artifact && (typeof raw.artifact.projectId !== 'string' || !PROJECT_ID.test(raw.artifact.projectId)
      || typeof raw.artifact.path !== 'string' || raw.artifact.path.length > 4096)) throw new Error('Invalid artifact reference.');
    if ([...this.uploads.values()].filter(job => job.owner === owner).length >= 8) throw new Error('Too many media imports are in progress.');
    const token = randomUUID(), file = path.join(this.root, '.incoming', `${token}.part`);
    const handle = await open(file, 'wx', 0o600);
    this.uploads.set(token, { owner, input: { key: raw.key, size: raw.size, type: raw.type, revision: raw.revision, onlyIfMissing: raw.onlyIfMissing, artifact: raw.artifact },
      file, handle, hash: createHash('sha256'), received: 0, queue: Promise.resolve() });
    return token;
  }
  async chunk(owner: number, token: string, offset: number, data: Uint8Array): Promise<void> {
    const job = this.uploads.get(token);
    if (!job || job.owner !== owner || !(data instanceof Uint8Array) || !data.length || data.length > LIBRARY_MEDIA_CHUNK
      || offset !== job.received || offset + data.length > job.input.size) throw new Error('Invalid media upload chunk.');
    job.received += data.length;
    job.queue = job.queue.then(async () => {
      for (let at = 0; at < data.length;) {
        const written = await job.handle.write(data, at, data.length - at, offset + at);
        if (!written.bytesWritten) throw new Error('Could not write the imported media.');
        at += written.bytesWritten;
      }
      job.hash.update(data);
    });
    await job.queue;
  }
  finish(owner: number, token: string): Promise<LibraryMediaRecord> {
    return this.serial(async () => {
      const job = this.uploads.get(token);
      if (!job || job.owner !== owner) throw new Error('Unknown media upload.');
      this.uploads.delete(token);
      try {
        await job.queue;
        if (job.received !== job.input.size) throw new Error('The media import is incomplete.');
        await job.handle.sync();
        await job.handle.close();
        const existing = job.input.onlyIfMissing ? await this.record(job.input.key) : null;
        if (existing) return this.grant(owner, existing);
        const sha256 = job.hash.digest('hex');
        const file = `${sha256}.${extensions[job.input.type] ?? 'bin'}`;
        const target = path.join(this.root, file);
        try {
          const handle = await open(target, constants.O_RDONLY | constants.O_NOFOLLOW);
          try { if ((await hashHandle(handle)).digest('hex') !== sha256) throw new Error('An existing media file is damaged.'); }
          finally { await handle.close(); }
        } catch (error: any) {
          if (error.code !== 'ENOENT') throw error;
          await rename(job.file, target);
          await syncDirectory(this.root);
        }
        const record: Record = { key: job.input.key, revision: job.input.revision || randomUUID(), type: job.input.type,
          size: job.input.size, file, sha256, mtime: (await lstat(target)).mtimeMs };
        await writeLibraryJSON(path.join(this.root, 'Index', indexName(record.key)), record);
        if (job.input.artifact && this.artifactSource) {
          // The imported copy is already durable. Failure to archive must keep
          // the source, and must not turn a successful import into data loss.
          try { await this.archiveArtifact(job.input.artifact, record); }
          catch { console.warn('[media-library] The original AI artifact was kept.'); }
        }
        return this.grant(owner, record);
      } finally { await job.handle.close().catch(() => undefined); await unlink(job.file).catch(() => undefined); }
    });
  }
  private async archiveArtifact(reference: ImportedArtifact, record: Record): Promise<void> {
    const source = await this.artifactSource!(reference);
    const before = await lstat(source);
    if (!before.isFile() || before.size !== record.size) return;
    const handle = await open(source, constants.O_RDONLY | constants.O_NOFOLLOW);
    try {
      if (!sameFile(before, await handle.stat()) || (await hashHandle(handle)).digest('hex') !== record.sha256
        || !sameFile(before, await handle.stat())) return;
    } finally { await handle.close(); }
    const pinned = { ...record, key: artifactKey(reference) };
    await writeLibraryJSON(path.join(this.root, 'Index', indexName(pinned.key)), pinned);
    if (sameFile(before, await lstat(source))) await unlink(source);
  }
  async archivedArtifact(reference: ImportedArtifact): Promise<{ path: string; mime: string } | null> {
    const record = await this.record(artifactKey(reference));
    return record ? { path: this.pathFor(record), mime: record.type } : null;
  }
  forgetArtifacts(projectId: string): Promise<void> {
    return this.serial(async () => {
      for (const name of await readdir(path.join(this.root, 'Index'))) {
        if (!/^[a-f0-9]{64}\.json$/.test(name)) continue;
        const file = path.join(this.root, 'Index', name);
        const record = JSON.parse(await readFile(file, 'utf8')) as Record;
        if (record.key.startsWith(`artifact:${projectId}:`)) await unlink(file);
      }
    });
  }
  async abort(owner: number, token: string): Promise<void> {
    const job = this.uploads.get(token);
    if (!job || job.owner !== owner) return;
    this.uploads.delete(token);
    await job.queue.catch(() => undefined);
    await job.handle.close();
    await unlink(job.file).catch(() => undefined);
  }
  async release(owner: number): Promise<void> {
    for (const [token, grant] of this.grants) if (grant.owner === owner) this.grants.delete(token);
    for (const [token, job] of this.uploads) if (job.owner === owner) await this.abort(owner, token);
  }
  remove(key: string): Promise<boolean> {
    return this.serial(async () => {
      if (!validKey(key)) throw new Error('Invalid media key.');
      await unlink(path.join(this.root, 'Index', indexName(key))).catch((error: any) => { if (error.code !== 'ENOENT') throw error; });
      return true;
    });
  }
  /** Payloads no key references are disposable after a seven-day grace period. */
  collectUnused(now = Date.now(), dryRun = false): Promise<number> {
    return this.serial(async () => {
      const referenced = new Set<string>();
      for (const entry of await readdir(path.join(this.root, 'Index'))) {
        const metadata = JSON.parse(await readFile(path.join(this.root, 'Index', entry), 'utf8')) as Record;
        if (metadata.sourcePath !== undefined && validPath(metadata.sourcePath) && metadata.file === undefined) continue;
        if (typeof metadata.file !== 'string' || !FILE.test(metadata.file)) throw new Error('Media cleanup stopped because its index needs repair.');
        referenced.add(metadata.file);
      }
      let bytes = 0;
      for (const file of await readdir(this.root)) {
        if (!FILE.test(file) || referenced.has(file)) continue;
        const metadata = await lstat(path.join(this.root, file));
        if (metadata.isFile() && metadata.mtimeMs < now - 7 * 86400_000) {
          if (!dryRun) await unlink(path.join(this.root, file));
          bytes += metadata.size;
        }
      }
      return bytes;
    });
  }
  async serve(token: string, request: Request): Promise<Response> {
    const grant = this.grants.get(token);
    if (!grant) return new Response('Not found', { status: 404 });
    const file = this.pathFor(grant.record);
    try { await this.validateFile(grant.record); }
    catch { return new Response('Media unavailable. Use Locate File to relink the source.', { status: 404 }); }
    const headers: { [key: string]: string } = { 'Content-Type': grant.record.type, 'Cache-Control': 'no-store', 'Access-Control-Allow-Origin': '*',
      'Content-Security-Policy': "default-src 'none'; sandbox", 'X-Content-Type-Options': 'nosniff', 'Accept-Ranges': 'bytes' };
    let start = 0, end = grant.record.size - 1, status = 200;
    const range = request.headers.get('Range');
    if (range) {
      const match = /^bytes=(\d+)-(\d+)$/.exec(range);
      start = Number(match?.[1]); end = Number(match?.[2]);
      if (!match || !Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start < 0 || end < start
        || end >= grant.record.size || end - start + 1 > LIBRARY_MEDIA_CHUNK) {
        return new Response(null, { status: 416, headers: { ...headers, 'Content-Range': `bytes */${grant.record.size}` } });
      }
      status = 206; headers['Content-Range'] = `bytes ${start}-${end}/${grant.record.size}`;
    }
    headers['Content-Length'] = String(Math.max(0, end - start + 1));
    if (request.method === 'HEAD' || !grant.record.size) return new Response(null, { status, headers });
    request.signal.throwIfAborted();
    const handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW);
    try {
      const info = await handle.stat();
      if (!info.isFile() || info.size !== grant.record.size) throw new Error('The project media changed while reading.');
      if (range) {
        // Save ranges are at most one MiB. Finish the filesystem read before
        // handing bytes to Chromium, avoiding its custom-protocol stream /
        // Blob cancellation path while keeping allocation bounded.
        try {
          const bytes = await readExactly(handle, start, end - start + 1);
          request.signal.throwIfAborted();
          return new Response(new Uint8Array(bytes), { status, headers });
        } finally { await handle.close(); }
      }
      const stream = handle.createReadStream({ start, end, autoClose: true, signal: request.signal, highWaterMark: 64 * 1024 });
      return new Response(Readable.toWeb(stream, { strategy: { highWaterMark: 64 * 1024, size: chunk => chunk.byteLength } }) as any, { status, headers });
    } catch (error) { await handle.close(); throw error; }
  }
}

export function registerLibraryMedia(ipc: Pick<IpcMain, 'handle'>, media: LibraryMedia, trusted: (event: IpcMainInvokeEvent) => boolean): void {
  const tracked = new WeakSet<object>();
  const owner = (event: IpcMainInvokeEvent): number => {
    if (!trusted(event)) throw new Error('Unauthorized media request.');
    if (!tracked.has(event.sender)) {
      tracked.add(event.sender);
      const id = event.sender.id;
      event.sender.once('destroyed', () => { void media.release(id).catch(() => undefined); });
    }
    return event.sender.id;
  };
  ipc.handle(LIBRARY_MEDIA_IPC.begin, (event, input) => media.begin(owner(event), input));
  ipc.handle(LIBRARY_MEDIA_IPC.chunk, (event, token, offset, data) => media.chunk(owner(event), token, offset, data));
  ipc.handle(LIBRARY_MEDIA_IPC.finish, (event, token) => media.finish(owner(event), token));
  ipc.handle(LIBRARY_MEDIA_IPC.abort, (event, token) => media.abort(owner(event), token));
  ipc.handle(LIBRARY_MEDIA_IPC.get, (event, key, metadataOnly) => media.get(owner(event), key, metadataOnly));
  ipc.handle(LIBRARY_MEDIA_IPC.remove, (event, key) => { owner(event); return media.remove(key); });
  ipc.handle(LIBRARY_MEDIA_IPC.link, (event, input) => media.link(owner(event), input));
}
