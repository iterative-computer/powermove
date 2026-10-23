import { readWithRetry } from './read-retry';
import { assertProjectDocument, ProjectDocumentError, ProjectDocumentWorkerError, validateProjectDocumentInWorker, WORKER_DOCUMENT_BYTES } from './project-document-validator';
import { createHash, randomUUID } from 'node:crypto';
import { mkdir, open, readFile, readdir, rename, unlink, type FileHandle } from 'node:fs/promises';
import path from 'node:path';
import { PROJECT_ID } from '../shared/ipc';
import { isProjectContainer, projectContainerIndex, type ProjectMediaRange } from '../shared/project-container';
import { FILE_CHUNK_BYTES } from './file-upload';
import { checkVersion, changedFile, copySnapshot, fileVersion, publishFile, RecoveryFileError, syncDirectory, syncFile, temporarySibling, type FileVersion } from './durable-file';
import { isIncrementalProject, readIncrementalIndex, type LinkedRecord, type ProjectRecord, type SaveMedia } from '../shared/project-incremental';
import { nativeHash, ProjectSave, validateSaveMedia } from './project-save';
import { hashProjectFile } from './project-file-hash';
import { CURRENT_READER_VERSION, NEWER_PROJECT_MESSAGE, needsNewerReader } from '../shared/reader-version';
import { validateProjectDocument, type ProjectDocumentValidator } from './project-document';

export interface ProjectFilesOptions {
  /** Checks a saved document before it is published; defaults to this thread. */
  validateDocument?: ProjectDocumentValidator;
  /** The host media store's file for a content hash (section 21). */
  storedFile?(hash: string, length: number): Promise<string | null>;
}

/** Replace only after every byte has reached disk; a failed write preserves the original. */
export async function atomicWrite(filePath: string, data: Uint8Array | Iterable<Uint8Array> | AsyncIterable<Uint8Array>): Promise<void> {
  const temporary = path.join(path.dirname(filePath), `.${path.basename(filePath)}.${randomUUID()}.tmp`);
  try {
    const handle = await open(temporary, 'wx', 0o600);
    try { await handle.writeFile(data); await syncFile(handle); } finally { await handle.close(); }
    await rename(temporary, filePath);
    await syncDirectory(path.dirname(filePath));
  } catch (error) {
    await unlink(temporary).catch(() => undefined);
    throw error;
  }
}

type Association = { path: string; hash: string; generation?: number };
type OpenedProject = { path: string; projectId: string; token: string; size: number; media: ProjectMediaRange[] };
type FullProject = OpenedProject & { document: any; documentRange?: ProjectRecord };
type RangeProject = OpenedProject & { document?: never; documentRange: ProjectRecord };
type HashResult = { ok: true; hash: string } | { ok: false; error: unknown };

/** Main-process-only grants: renderer data can never invent an overwrite path. */
export class ProjectFiles {
  private readers = new Map<string, { handle: FileHandle; size: number; mtime: number; ctime: number; busy: boolean }>();
  private associations = new Map<string, Association>();
  private ready: Promise<void>;
  private queue: Promise<unknown> = Promise.resolve();
  private saving = new Set<string>();
  private readonly validateDocument: ProjectDocumentValidator;
  private readonly storedFile: ProjectFilesOptions['storedFile'];
  constructor(private readonly registryPath: string, private readonly backupDirectory: string, options: ProjectFilesOptions = {}) {
    this.validateDocument = options.validateDocument ?? validateProjectDocument;
    this.storedFile = options.storedFile;
    this.ready = this.load();
  }
  private async load(): Promise<void> {
    try {
      const data = JSON.parse(await readFile(this.registryPath, 'utf8'));
      for (const [id, value] of Object.entries<any>(data)) {
        if (PROJECT_ID.test(id) && typeof value?.path === 'string' && path.isAbsolute(value.path)
          && typeof value.hash === 'string') this.associations.set(id, value);
      }
    } catch (error: any) {
      if (error.code !== 'ENOENT') console.warn('Project file associations unavailable; Save As will be used.', error);
    }
    // A file commit can outlive a failed registry write. Reconcile only an
    // exact matching committed file; never grant an overwrite from stale intent.
    const directory = path.dirname(this.registryPath), prefix = path.basename(this.registryPath) + '.pending-';
    for (const name of await readdir(directory).catch(() => [])) {
      if (!name.startsWith(prefix) || !name.endsWith('.json')) continue;
      const journal = path.join(directory, name);
      try {
        const value = JSON.parse(await readFile(journal, 'utf8'));
        if (typeof value.id === 'string' && PROJECT_ID.test(value.id) && typeof value.path === 'string' && path.isAbsolute(value.path)
          && typeof value.hash === 'string' && Number.isSafeInteger(value.generation) && value.generation > (this.associations.get(value.id)?.generation || 0)
          && (await fileVersion(value.path))?.hash === value.hash) {
          this.associations.set(value.id, { path: value.path, hash: value.hash, generation: value.generation });
          await this.persist();
        }
        await unlink(journal);
      } catch (error) { console.warn('Project save metadata needs recovery.', error); }
    }
  }
  private async persist(): Promise<void> {
    await mkdir(path.dirname(this.registryPath), { recursive: true });
    await atomicWrite(this.registryPath, Buffer.from(JSON.stringify(Object.fromEntries(this.associations))));
  }
  async destination(id: string): Promise<string | undefined> {
    await this.ready;
    return this.associations.get(id)?.path;
  }
  private serial<T>(operation: () => Promise<T>): Promise<T> {
    const next = this.queue.then(async () => { await this.ready; return operation(); });
    // The queue needs completion, not the operation's value. Passing a
    // successful open result through catch keeps its entire project/history
    // alive until the next file operation, even after the reader is closed.
    this.queue = next.then(() => undefined, () => undefined);
    return next;
  }
  async backups(id: string): Promise<string[]> {
    if (!PROJECT_ID.test(id)) throw new Error('Invalid project id');
    const directory = path.join(this.backupDirectory, id);
    try {
      return (await readdir(directory, { withFileTypes: true }))
        .filter(entry => entry.isFile() && /^\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}-\d{3}Z\.pmv(?:-\d+)?$/.test(entry.name))
        .map(entry => entry.name)
        .sort((a, b) => b.localeCompare(a))
        .map(name => path.resolve(directory, name));
    } catch (error: any) {
      if (error.code === 'ENOENT') return [];
      throw error;
    }
  }
  private async backup(id: string, destination: string): Promise<string> {
    const directory = path.join(this.backupDirectory, id);
    await mkdir(directory, { recursive: true });
    const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
    let collision = 0;
    while (true) {
      const name = `${timestamp}.pmv${collision ? `-${collision}` : ''}`;
      try {
        const file = path.join(directory, name);
        await copySnapshot(destination, file);
        try {
          const handle = await open(file, 'r');
          try { await syncFile(handle); } finally { await handle.close(); }
          await syncDirectory(directory);
        } catch (error) {
          await unlink(file).catch(() => undefined);
          throw error;
        }
        return file;
      } catch (error: any) {
        if (error.code !== 'EEXIST') throw error;
        collision++;
      }
    }
  }
  private async prune(id: string): Promise<void> {
    try {
      const backups = await this.backups(id);
      await Promise.all(backups.slice(5).map(filePath => unlink(filePath)));
    } catch (error) {
      console.warn(`Could not prune project backups for ${id}.`, error);
    }
  }
  private async commit(id: string, destination: string, temporary: string, hash: string, previous?: FileVersion): Promise<string> {
    await checkVersion(destination, previous);
    const journal = `${this.registryPath}.pending-${randomUUID()}.json`;
    const generation = (this.associations.get(id)?.generation || 0) + 1;
    let backup: string | undefined;
    try {
      if (previous) backup = await this.backup(id, destination);
      await checkVersion(destination, previous);
      await mkdir(path.dirname(this.registryPath), { recursive: true });
      await atomicWrite(journal, Buffer.from(JSON.stringify({ id, path: destination, hash, generation })));
      await publishFile(temporary, destination, previous, hash, () => {
        // Finalization can fail after publication. Retry against the bytes that
        // reached the destination while the renderer keeps its edits dirty.
        this.associations.set(id, { path: destination, hash, generation });
      });
      // The user's file is durably committed. A failed private registry write
      // must not report that their file failed; retain the journal for restart.
      try { await this.persist(); await unlink(journal); }
      catch (error) { console.warn('Project saved; its file association will be repaired on restart.', error); }
      await this.prune(id);
      return destination;
    } catch (error) {
      if (backup) {
        const current = await fileVersion(destination).catch(() => undefined);
        if (previous && current?.hash === previous.hash) await unlink(backup).catch(() => undefined);
        else {
          // A failed/uncertain publication may still need its old version.
          // Keep it separately so it cannot evict successful backup generations.
          await rename(backup, backup + '.recovery.pmv').catch(() => undefined);
        }
      }
      throw error;
    }
  }
  beginIncremental(id: string, media: SaveMedia[], documentBytes: number, selectedPath?: string, linked?: LinkedRecord[]): Promise<ProjectSave> {
    return this.serial(async () => {
      if (!PROJECT_ID.test(id)) throw new Error('Invalid project id');
      validateSaveMedia(media);
      const known = this.associations.get(id), destination = selectedPath || known?.path;
      if (!destination) throw new Error('Choose a destination with Save As.');
      if (this.saving.has(destination)) throw new Error('A save to this project file is already in progress.');
      const previous = await fileVersion(destination);
      if (!selectedPath && known && previous?.hash !== known.hash) throw changedFile();
      this.saving.add(destination);
      try {
        return await ProjectSave.create(destination, previous, media, documentBytes,
          (temporary, hash) => this.serial(() => this.commit(id, destination, temporary, hash, previous)),
          () => { this.saving.delete(destination); }, !selectedPath, this.validateDocument,
          { ...(this.storedFile ? { storedFile: this.storedFile } : {}), ...(linked ? { linked } : {}) });
      } catch (error) { this.saving.delete(destination); throw error; }
    });
  }
  save(id: string, data: Uint8Array | Iterable<Uint8Array> | AsyncIterable<Uint8Array>, selectedPath?: string): Promise<string> {
    return this.serial(async () => {
      if (!PROJECT_ID.test(id)) throw new Error('Invalid project id');
      const known = this.associations.get(id);
      const destination = selectedPath || known?.path;
      if (!destination) throw new Error('Choose a destination with Save As.');
      if (this.saving.has(destination)) throw new Error('A save to this project file is already in progress.');
      const previous = await fileVersion(destination);
      if (!selectedPath && known && previous?.hash !== known.hash) throw changedFile();
      const digest = createHash('sha256');
      async function* writing() {
        for await (const chunk of data instanceof Uint8Array ? [data] : data) {
          digest.update(chunk); yield chunk;
        }
      }
      const temporary = temporarySibling(destination);
      let retainTemporary = false;
      try {
        const handle = await open(temporary, 'wx', 0o600);
        try { await handle.writeFile(writing()); await syncFile(handle); } finally { await handle.close(); }
        return await this.commit(id, destination, temporary, digest.digest('hex'), previous);
      } catch (error) {
        retainTemporary = error instanceof RecoveryFileError && error.recoveryPath === temporary;
        throw error;
      } finally { if (!retainTemporary) await unlink(temporary).catch(() => undefined); }
    });
  }
  open(filePath: string): Promise<FullProject>;
  open(filePath: string, options: { allowDocumentRange?: boolean }): Promise<FullProject | RangeProject>;
  open(filePath: string, options: { allowDocumentRange?: boolean } = {}): Promise<FullProject | RangeProject> {
    return this.serial(async () => {
      const handle = await open(filePath, 'r');
      let hashing: Promise<HashResult> | undefined;
      let hashingAbort: AbortController | undefined;
      try {
        const info = await handle.stat();
        if (!info.isFile() || !Number.isSafeInteger(info.size)) throw new Error('This is not a readable project file.');
        const readExactly = async (position: number, length: number, owned = false) => {
          const buffer = owned ? Buffer.allocUnsafeSlow(length) : Buffer.allocUnsafe(length);
          for (let offset = 0; offset < length;) {
            const { bytesRead } = await readWithRetry(() => handle.read(buffer, offset, length - offset, position + offset));
            if (!bytesRead) throw new Error('The project container is truncated.');
            offset += bytesRead;
          }
          return buffer;
        };
        const prefix = await readExactly(0, Math.min(9, info.size));
        let document: any, rangeValidated = false, rangeReaderVersion = 0, media: ProjectMediaRange[] = [], documentRange: ProjectRecord | undefined, mediaToVerify: ProjectRecord[] = [];
        if (isIncrementalProject(prefix)) {
          const index = await readIncrementalIndex(info.size, readExactly, nativeHash);
          const workerRange = options.allowDocumentRange && index.document.length >= WORKER_DOCUMENT_BYTES;
          let bytes = await readExactly(index.document.offset, index.document.length, workerRange);
          if (workerRange) {
            const validation = validateProjectDocumentInWorker(bytes, index.document.sha256);
            // Capture both outcomes immediately: document validation must win
            // over a concurrent media/read error, and no rejected hash escapes.
            hashingAbort = new AbortController();
            hashing = Promise.resolve().then(() => hashProjectFile(handle, info.size, index.media, hashingAbort!.signal))
              .then(hash => ({ ok: true as const, hash }), error => ({ ok: false as const, error }));
            try {
              rangeReaderVersion = (await validation).requiredReaderVersion;
              rangeValidated = true;
            } catch (error) {
              // A newer build may change the document's shape; say why it cannot open.
              if (error instanceof ProjectDocumentError && error.requiredReaderVersion > CURRENT_READER_VERSION) throw new Error(`${NEWER_PROJECT_MESSAGE}.`);
              if (!(error instanceof ProjectDocumentWorkerError)) throw error;
              // Transfer detaches the owned bytes even if the worker later
              // fails. Retry the original checksum/parser on the same handle.
              if (bytes.byteLength !== index.document.length) bytes = await readExactly(index.document.offset, index.document.length);
              if (await nativeHash(bytes) !== index.document.sha256) throw new Error('The project document checksum does not match.');
              document = JSON.parse(bytes.toString('utf8'));
            }
          } else {
            if (await nativeHash(bytes) !== index.document.sha256) throw new Error('The project document checksum does not match.');
            document = JSON.parse(bytes.toString('utf8'));
          }
          media = index.media;
          mediaToVerify = index.media;
          documentRange = { offset: index.document.offset, length: index.document.length, sha256: index.document.sha256 };
        } else if (isProjectContainer(prefix)) {
          if (prefix.length < 9) throw new Error('The project container is truncated.');
          const headerLength = prefix.readUInt32LE(5);
          if (headerLength < 2 || headerLength > info.size - 9) throw new Error('The project container header is invalid.');
          const header = JSON.parse((await readExactly(9, headerLength)).toString('utf8'));
          ({ document, media } = projectContainerIndex(header, 9 + headerLength, info.size));
        } else {
          // Legacy JSON stores its media inside the document itself. Keep that
          // format readable alongside PMV3 and incremental PMV4.
          document = JSON.parse((await readExactly(0, info.size)).toString('utf8'));
        }
        // A newer build may change the document's shape; say why it cannot open.
        // A worker-validated range is parsed only there; it reports the stamps.
        const newer = rangeValidated ? rangeReaderVersion > CURRENT_READER_VERSION : needsNewerReader(document);
        if (!rangeValidated) {
          try { assertProjectDocument(document); }
          catch (error) { throw newer ? new Error(`${NEWER_PROJECT_MESSAGE}.`) : error; }
        }
        const hashed: HashResult = hashing ? await hashing : { ok: true, hash: await hashProjectFile(handle, info.size, mediaToVerify) };
        if (!hashed.ok) throw hashed.error;
        const hash = hashed.hash;
        const after = await handle.stat();
        if (after.size !== info.size || after.mtimeMs !== info.mtimeMs || after.ctimeMs !== info.ctimeMs) throw new Error('The project changed while opening. Try opening it again.');
        const projectId = randomUUID(), token = randomUUID();
        // A project written for a newer reader opens read-only: without an
        // association, no save can target this file except one the person
        // chooses again in a Save dialog.
        if (!newer) {
          this.associations.set(projectId, { path: filePath, hash });
          await this.persist();
        }
        this.readers.set(token, { handle, size: info.size, mtime: info.mtimeMs, ctime: info.ctimeMs, busy: false });
        const opened = { path: filePath, projectId, token, size: info.size };
        return rangeValidated ? { ...opened, media, documentRange: documentRange! }
          : { ...opened, document, media, ...(documentRange ? { documentRange } : {}) };
      } catch (error) { hashingAbort?.abort(); await hashing; await handle.close(); throw error; }
    });
  }

  async read(token: string, offset: number, length: number): Promise<Uint8Array> {
    const reader = this.readers.get(token);
    if (!reader || reader.busy || !Number.isSafeInteger(offset) || offset < 0 || offset > reader.size
      || !Number.isSafeInteger(length) || length < 1 || length > FILE_CHUNK_BYTES || length > reader.size - offset) throw new Error('Invalid project read range');
    reader.busy = true;
    try {
      const data = Buffer.allocUnsafe(length);
      const { bytesRead } = await readWithRetry(() => reader.handle.read(data, 0, length, offset));
      if (!bytesRead) throw new Error('The project container is truncated.');
      return data.subarray(0, bytesRead);
    } finally { reader.busy = false; }
  }

  async close(token: string, verify = true): Promise<void> {
    const reader = this.readers.get(token);
    if (!reader) return;
    this.readers.delete(token);
    try {
      if (verify) {
        const info = await reader.handle.stat();
        if (info.size !== reader.size || info.mtimeMs !== reader.mtime || info.ctimeMs !== reader.ctime) throw new Error('The project changed while opening. Try opening it again.');
      }
    } finally { await reader.handle.close(); }
  }
}
