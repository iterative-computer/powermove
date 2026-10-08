import { createHash, randomUUID } from 'node:crypto';
import { mkdir, open, readFile, readdir, rename, unlink, type FileHandle } from 'node:fs/promises';
import path from 'node:path';
import { PROJECT_ID } from '../shared/ipc';
import { isProjectContainer, projectContainerIndex, type ProjectMediaRange } from '../shared/project-container';
import { FILE_CHUNK_BYTES } from './file-upload';
import { checkVersion, changedFile, copySnapshot, fileVersion, hashHandle, publishFile, syncDirectory, syncFile, temporarySibling, type FileVersion } from './durable-file';
import { isIncrementalProject, readIncrementalIndex, type SaveMedia } from '../shared/project-incremental';
import { nativeHash, ProjectSave, validateSaveMedia } from './project-save';

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

/** Main-process-only grants: renderer data can never invent an overwrite path. */
export class ProjectFiles {
  private readers = new Map<string, { handle: FileHandle; size: number; mtime: number; ctime: number; busy: boolean }>();
  private associations = new Map<string, Association>();
  private ready: Promise<void>;
  private queue: Promise<unknown> = Promise.resolve();
  private saving = new Set<string>();
  constructor(private readonly registryPath: string, private readonly backupDirectory: string) {
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
    this.queue = next.catch(() => undefined);
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
          // FlushFileBuffers requires a writable handle on Windows.
          const handle = await open(file, process.platform === 'win32' ? 'r+' : 'r');
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
      await publishFile(temporary, destination, previous, hash);
      this.associations.set(id, { path: destination, hash, generation });
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
  beginIncremental(id: string, media: SaveMedia[], documentBytes: number, selectedPath?: string): Promise<ProjectSave> {
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
          () => { this.saving.delete(destination); }, !selectedPath);
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
      try {
        const handle = await open(temporary, 'wx', 0o600);
        try { await handle.writeFile(writing()); await syncFile(handle); } finally { await handle.close(); }
        return await this.commit(id, destination, temporary, digest.digest('hex'), previous);
      } finally { await unlink(temporary).catch(() => undefined); }
    });
  }
  open(filePath: string): Promise<{ path: string; projectId: string; token: string; size: number; document: any; media: ProjectMediaRange[] }> {
    return this.serial(async () => {
      const handle = await open(filePath, 'r');
      try {
        const info = await handle.stat();
        if (!info.isFile() || !Number.isSafeInteger(info.size)) throw new Error('This is not a readable project file.');
        const readExactly = async (position: number, length: number) => {
          const buffer = Buffer.allocUnsafe(length);
          for (let offset = 0; offset < length;) {
            const { bytesRead } = await handle.read(buffer, offset, length - offset, position + offset);
            if (!bytesRead) throw new Error('The project container is truncated.');
            offset += bytesRead;
          }
          return buffer;
        };
        const prefix = await readExactly(0, Math.min(9, info.size));
        let document: any, media: ProjectMediaRange[] = [];
        if (isIncrementalProject(prefix)) {
          const index = await readIncrementalIndex(info.size, readExactly, nativeHash);
          const bytes = await readExactly(index.document.offset, index.document.length);
          if (await nativeHash(bytes) !== index.document.sha256) throw new Error('The project document checksum does not match.');
          document = JSON.parse(bytes.toString('utf8')); media = index.media;
          for (const item of index.media) {
            if ((await hashHandle(handle, item.offset, item.length)).digest('hex') !== item.sha256) throw new Error('The project media checksum does not match.');
          }
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
        const project = document?.proj || document;
        if (!project || !Array.isArray(project.layers) || !Number.isFinite(project.w) || !Number.isFinite(project.h)) throw new Error('This is not a Powermove project.');
        const digest = createHash('sha256');
        for await (const chunk of handle.createReadStream({ start: 0, autoClose: false, highWaterMark: FILE_CHUNK_BYTES })) digest.update(chunk);
        const after = await handle.stat();
        if (after.size !== info.size || after.mtimeMs !== info.mtimeMs || after.ctimeMs !== info.ctimeMs) throw new Error('The project changed while opening. Try opening it again.');
        const projectId = randomUUID(), token = randomUUID();
        this.associations.set(projectId, { path: filePath, hash: digest.digest('hex') });
        await this.persist();
        this.readers.set(token, { handle, size: info.size, mtime: info.mtimeMs, ctime: info.ctimeMs, busy: false });
        return { path: filePath, projectId, token, size: info.size, document, media };
      } catch (error) { await handle.close(); throw error; }
    });
  }

  async read(token: string, offset: number, length: number): Promise<Uint8Array> {
    const reader = this.readers.get(token);
    if (!reader || reader.busy || !Number.isSafeInteger(offset) || offset < 0 || offset > reader.size
      || !Number.isSafeInteger(length) || length < 1 || length > FILE_CHUNK_BYTES || length > reader.size - offset) throw new Error('Invalid project read range');
    reader.busy = true;
    try {
      const data = Buffer.allocUnsafe(length);
      const { bytesRead } = await reader.handle.read(data, 0, length, offset);
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
