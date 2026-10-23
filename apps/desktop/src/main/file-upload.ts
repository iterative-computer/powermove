import { readWithRetry } from './read-retry';
import { mkdtemp, open, rename, rm, unlink, type FileHandle } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { copySnapshot, syncDirectory, syncFile, temporarySibling } from './durable-file';

export const FILE_CHUNK_BYTES = 1024 * 1024;

/** One acknowledged disk write at a time; project size never becomes a RAM budget. */
export class FileUpload {
  received = 0;
  private writing?: Promise<void>;
  private disposed = false;
  private claimed = false;
  private failure?: unknown;
  private publishing?: Promise<void>;
  private constructor(readonly size: number, private directory: string, private handle: FileHandle) {}

  static async create(size: number, root = tmpdir()): Promise<FileUpload> {
    if (!Number.isSafeInteger(size) || size < 1) throw new Error('Invalid save size');
    const directory = await mkdtemp(path.join(root, 'powermove-save-upload-'));
    try { return new FileUpload(size, directory, await open(path.join(directory, 'data'), 'wx+', 0o600)); }
    catch (error) { await rm(directory, { recursive: true, force: true }); throw error; }
  }

  write(data: Uint8Array): Promise<void> {
    if (this.disposed || this.claimed || this.failure || this.writing || !(data instanceof Uint8Array)
      || data.byteLength < 1 || data.byteLength > FILE_CHUNK_BYTES || data.byteLength > this.size - this.received) {
      throw new Error('Invalid save chunk');
    }
    const operation = (async () => {
      let offset = 0;
      while (offset < data.byteLength) {
        const { bytesWritten } = await this.handle.write(data, offset, data.byteLength - offset, this.received + offset);
        if (!bytesWritten) throw new Error('Could not write project data');
        offset += bytesWritten;
      }
      this.received += offset;
    })();
    this.writing = operation;
    return operation.catch(error => { this.failure = error; throw error; })
      .finally(() => { this.writing = undefined; });
  }

  /** Claim only complete uploads. An incomplete finish cannot consume the upload. */
  claim(): AsyncIterable<Uint8Array> {
    this.claimComplete();
    return this.read();
  }

  /** Export publication can clone the immutable spool instead of reading and
   * rewriting every byte through JavaScript. Project saves keep their stream
   * so their validated document, hashing, and backup path stay authoritative. */
  claimForExport(): (destination: string) => Promise<void> {
    this.claimComplete();
    let used = false;
    return destination => {
      if (this.disposed || used) throw new Error('The save upload is unavailable.');
      used = true;
      return this.publishing = this.publish(destination);
    };
  }

  private claimComplete(): void {
    if (this.disposed || this.claimed || this.writing || this.failure || this.received !== this.size) {
      throw new Error('The save upload is incomplete.');
    }
    this.claimed = true;
  }

  private async publish(destination: string): Promise<void> {
    const temporary = temporarySibling(destination);
    try {
      // copySnapshot falls back to an ordinary exclusive copy across volumes
      // and on filesystems without cloning. Never write through the target.
      await copySnapshot(path.join(this.directory, 'data'), temporary);
      const handle = await open(temporary, 'r+');
      try { await syncFile(handle); } finally { await handle.close(); }
      await rename(temporary, destination);
      await syncDirectory(path.dirname(destination));
    } catch (error) {
      // An exclusive-copy collision belongs to another writer, so it must not
      // become ours to remove during cleanup.
      if ((error as NodeJS.ErrnoException).code !== 'EEXIST') await unlink(temporary).catch(() => undefined);
      throw error;
    }
  }

  private async *read(): AsyncGenerator<Uint8Array> {
    const buffer = Buffer.allocUnsafe(FILE_CHUNK_BYTES);
    for (let offset = 0; offset < this.size;) {
      if (this.disposed) throw new Error('Save cancelled');
      const { bytesRead } = await readWithRetry(() => this.handle.read(buffer, 0, Math.min(buffer.length, this.size - offset), offset));
      if (!bytesRead) throw new Error('The save upload is truncated.');
      yield buffer.subarray(0, bytesRead);
      offset += bytesRead;
    }
  }

  async dispose(): Promise<void> {
    if (this.disposed) return;
    this.disposed = true;
    await this.writing?.catch(() => undefined);
    // Once publication starts, finish its durable outcome before removing its
    // source, even if the renderer disappears while the copy is in progress.
    await this.publishing?.catch(() => undefined);
    try { await this.handle.close(); }
    finally { await rm(this.directory, { recursive: true, force: true }); }
  }
}
