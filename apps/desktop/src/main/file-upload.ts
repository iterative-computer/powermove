import { mkdtemp, open, rm, type FileHandle } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

export const FILE_CHUNK_BYTES = 1024 * 1024;

/** One acknowledged disk write at a time; project size never becomes a RAM budget. */
export class FileUpload {
  received = 0;
  private writing?: Promise<void>;
  private disposed = false;
  private claimed = false;
  private failure?: unknown;
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
    if (this.disposed || this.claimed || this.writing || this.failure || this.received !== this.size) {
      throw new Error('The save upload is incomplete.');
    }
    this.claimed = true;
    return this.read();
  }

  private async *read(): AsyncGenerator<Uint8Array> {
    const buffer = Buffer.allocUnsafe(FILE_CHUNK_BYTES);
    for (let offset = 0; offset < this.size;) {
      if (this.disposed) throw new Error('Save cancelled');
      const { bytesRead } = await this.handle.read(buffer, 0, Math.min(buffer.length, this.size - offset), offset);
      if (!bytesRead) throw new Error('The save upload is truncated.');
      yield buffer.subarray(0, bytesRead);
      offset += bytesRead;
    }
  }

  async dispose(): Promise<void> {
    if (this.disposed) return;
    this.disposed = true;
    await this.writing?.catch(() => undefined);
    try { await this.handle.close(); }
    finally { await rm(this.directory, { recursive: true, force: true }); }
  }
}
