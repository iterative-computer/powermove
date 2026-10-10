import { createHash, type Hash } from 'node:crypto';
import { chmod, open, unlink, type FileHandle } from 'node:fs/promises';
import { INCREMENTAL_MAGIC, isIncrementalProject, projectFooter, readIncrementalIndex, type IncrementalMedia, type ProjectIndex, type ProjectRecord, type SaveMedia } from '../shared/project-incremental';
import { checkVersion, copySnapshot, hashHandle, readExactly, syncFile, temporarySibling, type FileVersion } from './durable-file';
import { FILE_CHUNK_BYTES } from './file-upload';

type Incoming = { id: string | null; length: number; received: number; offset: number; digest: Hash; source?: SaveMedia };
export const nativeHash = async (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');
export function validateSaveMedia(media: unknown): asserts media is SaveMedia[] {
  if (!Array.isArray(media) || media.length > 100_000) throw new Error('Invalid project media');
  const ids = new Set<string>();
  for (const item of media) {
    if (!item || typeof item.id !== 'string' || !item.id || item.id.length > 1000 || ids.has(item.id)
      || typeof item.revision !== 'string' || !item.revision || item.revision.length > 200
      || typeof item.type !== 'string' || item.type.length > 1000 || !Number.isSafeInteger(item.length) || item.length < 0) throw new Error('Invalid project media');
    ids.add(item.id);
  }
}

/** One bounded upload writes directly into a destination-side transaction file.
 * Reused ranges come only from the main process's verified previous container. */
export class ProjectSave {
  readonly required: string[] = [];
  private records: IncrementalMedia[] = [];
  private incoming: Incoming[] = [];
  private document?: ProjectRecord;
  private offset = 0;
  private digest = createHash('sha256');
  private handle!: FileHandle;
  private clonedMode?: number;
  private busy?: Promise<unknown>;
  private ended = false;
  private failed = false;
  private readonly temporary: string;
  private constructor(readonly destination: string, private commit: (temporary: string, hash: string) => Promise<string>, private release: () => void) {
    this.temporary = temporarySibling(destination);
  }
  static async create(destination: string, previous: FileVersion | undefined, media: SaveMedia[], documentBytes: number,
    commit: (temporary: string, hash: string) => Promise<string>, release: () => void, reusePrevious = true): Promise<ProjectSave> {
    validateSaveMedia(media);
    if (!Number.isSafeInteger(documentBytes) || documentBytes < 2 || documentBytes > 0xffffffff) throw new Error('Invalid project document size');
    const save = new ProjectSave(destination, commit, release);
    let source: FileHandle | undefined;
    try {
      let index: ProjectIndex | undefined;
      if (previous && reusePrevious) {
        source = await open(destination, 'r');
        if (isIncrementalProject(await readExactly(source, 0, Math.min(5, previous.info.size)))) {
          index = await readIncrementalIndex(previous.info.size, (offset, length) => readExactly(source!, offset, length), nativeHash);
        }
      }
      const available = new Map(index?.media.map(item => [item.id, item]));
      for (const item of media) {
        const saved = available.get(item.id);
        if (saved && saved.revision === item.revision && saved.length === item.length) save.records.push({ ...saved, type: item.type });
        else save.required.push(item.id);
      }
      const liveBytes = save.records.reduce((sum, item) => sum + item.length, 0);
      const deadBytes = (previous?.info.size ?? 0) - liveBytes;
      // Compact only when meaningful dead data accumulates. No unbounded log
      // growth; Undo-reachable media remains in the incoming manifest.
      const compact = !index || deadBytes > Math.max(16 * 1024 * 1024, liveBytes / 2);
      if (!compact && previous) {
        await copySnapshot(destination, save.temporary);
        // A clone inherits read-only permissions. Only the transaction copy
        // needs write access; restore the user's mode before publishing it.
        save.clonedMode = previous.info.mode & 0o777;
        await chmod(save.temporary, save.clonedMode | 0o600);
        save.handle = await open(save.temporary, 'r+');
        save.offset = previous.info.size;
        save.digest = previous.digest.copy();
      } else {
        save.handle = await open(save.temporary, 'wx+', 0o600);
        await save.append(INCREMENTAL_MAGIC);
        for (const item of save.records) {
          const newOffset = save.offset;
          const digest = createHash('sha256');
          for (let offset = 0; offset < item.length;) {
            const bytes = await readExactly(source!, item.offset + offset, Math.min(FILE_CHUNK_BYTES, item.length - offset));
            digest.update(bytes); await save.append(bytes); offset += bytes.length;
          }
          if (digest.digest('hex') !== item.sha256) throw new Error('The saved media checksum does not match.');
          item.offset = newOffset;
        }
      }
      await checkVersion(destination, previous);
      const required = new Set(save.required);
      save.incoming = [{ id: null, length: documentBytes, received: 0, offset: save.offset, digest: createHash('sha256') },
        ...media.filter(item => required.has(item.id)).map(item => ({ id: item.id, length: item.length, received: 0, offset: 0, digest: createHash('sha256'), source: item }))];
      return save;
    } catch (error) { await save.dispose(); throw error; }
    finally { await source?.close(); }
  }
  private async append(bytes: Uint8Array): Promise<void> {
    if (!Number.isSafeInteger(this.offset + bytes.length)) throw new Error('The project exceeds the filesystem numeric range.');
    for (let offset = 0; offset < bytes.length;) {
      const { bytesWritten } = await this.handle.write(bytes, offset, bytes.length - offset, this.offset + offset);
      if (!bytesWritten) throw new Error('Could not write project data');
      offset += bytesWritten;
    }
    this.digest.update(bytes); this.offset += bytes.length;
  }
  private finishRecords(): void {
    while (this.incoming.length && this.incoming[0]!.received === this.incoming[0]!.length) {
      const item = this.incoming.shift()!;
      const record = { offset: item.length ? item.offset : this.offset, length: item.length, sha256: item.digest.digest('hex') };
      if (item.source) this.records.push({ ...item.source, ...record });
      else this.document = record;
      if (this.incoming.length) this.incoming[0]!.offset = this.offset;
    }
  }
  write(id: string | null, data: Uint8Array): Promise<void> {
    if (this.ended || this.failed || this.busy) throw new Error('A project write is already in progress or has ended.');
    this.finishRecords();
    const target = this.incoming[0];
    if (!target || target.id !== id || !(data instanceof Uint8Array) || !data.length || data.length > FILE_CHUNK_BYTES || data.length > target.length - target.received) throw new Error('Invalid project save chunk');
    const work = (async () => { await this.append(data); target.digest.update(data); target.received += data.length; this.finishRecords(); })();
    this.busy = work;
    return work.catch(error => { this.failed = true; throw error; }).finally(() => { this.busy = undefined; });
  }
  finish(): Promise<string> {
    if (this.ended || this.failed || this.busy) throw new Error('The project save is incomplete.');
    this.finishRecords();
    if (this.incoming.length || !this.document) throw new Error('The project save is incomplete.');
    const work = (async () => {
      const document = JSON.parse((await readExactly(this.handle, this.document!.offset, this.document!.length)).toString('utf8'));
      const project = document?.proj || document;
      if (!project || !Array.isArray(project.layers) || !Number.isFinite(project.w) || !Number.isFinite(project.h)) throw new Error('This is not a Powermove project.');
      const index: ProjectIndex = { version: 4, document: this.document!, media: this.records };
      const bytes = Buffer.from(JSON.stringify(index));
      const footer = projectFooter(this.offset, bytes.length, await nativeHash(bytes));
      await this.append(bytes); await this.append(footer);
      if (this.clonedMode !== undefined) await this.handle.chmod(this.clonedMode);
      await syncFile(this.handle); await this.handle.close();
      return await this.commit(this.temporary, this.digest.digest('hex'));
    })();
    this.busy = work;
    return work.finally(async () => { this.busy = undefined; await this.dispose(); });
  }
  async dispose(): Promise<void> {
    if (this.ended) return;
    this.ended = true;
    try { await this.busy?.catch(() => undefined); await this.handle?.close().catch(() => undefined); }
    finally { await unlink(this.temporary).catch(() => undefined); this.release(); }
  }
}
