import { createHash } from 'node:crypto';
import type { FileHandle } from 'node:fs/promises';
import { describe, expect, it, vi } from 'vitest';
import type { ProjectRecord } from '../shared/project-incremental';
import { FILE_CHUNK_BYTES } from './file-upload';
import { hashProjectFile } from './project-file-hash';

const digest = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');
const record = (bytes: Buffer, offset: number, length: number): ProjectRecord => ({ offset, length, sha256: digest(bytes.subarray(offset, offset + length)) });
function reader(bytes: Buffer, limit = FILE_CHUNK_BYTES) {
  const read = vi.fn(async (buffer: Buffer, offset: number, length: number, position: number) => {
    const bytesRead = Math.min(length, limit, Math.max(0, bytes.length - position));
    bytes.copy(buffer, offset, position, position + bytesRead);
    return { bytesRead, buffer };
  });
  return { handle: { read } as unknown as Pick<FileHandle, 'read'>, read };
}

describe('project file verification', () => {
  it('verifies unordered, empty and chunk-crossing media while hashing every file byte once', async () => {
    const bytes = Buffer.alloc(3 * FILE_CHUNK_BYTES + 17);
    for (let i = 0; i < bytes.length; i++) bytes[i] = i % 251;
    const media = [record(bytes, FILE_CHUNK_BYTES - 9, 33), record(bytes, 5, 7), record(bytes, 2 * FILE_CHUNK_BYTES - 2, FILE_CHUNK_BYTES), record(bytes, 27, 0)];
    const { handle, read } = reader(bytes);

    expect(await hashProjectFile(handle, bytes.length, media)).toBe(digest(bytes));
    expect(read).toHaveBeenCalledTimes(4);
    expect(read.mock.calls.every(([buffer, , length]) => buffer === read.mock.calls[0]![0] && length <= FILE_CHUNK_BYTES)).toBe(true);
    expect(media[0]!.offset).toBe(FILE_CHUNK_BYTES - 9);
  });

  it('handles short reads without skipping bytes inside or between media records', async () => {
    const bytes = Buffer.from(Array.from({ length: 257 }, (_, index) => index % 251));
    const { handle } = reader(bytes, 7);
    expect(await hashProjectFile(handle, bytes.length, [record(bytes, 15, 180), record(bytes, 210, 15)])).toBe(digest(bytes));
  });

  it.each([0, 1, 2])('rejects a mismatched checksum in media record %i', async corruptIndex => {
    const bytes = Buffer.alloc(FILE_CHUNK_BYTES + 50, 7);
    const media = [record(bytes, 5, 13), record(bytes, FILE_CHUNK_BYTES - 8, 30), record(bytes, 27, 0)];
    media[corruptIndex]!.sha256 = '0'.repeat(64);
    await expect(hashProjectFile(reader(bytes).handle, bytes.length, media)).rejects.toThrow('media checksum');
  });

  it('includes gaps and unreferenced bytes in the overwrite-protection hash', async () => {
    const bytes = Buffer.alloc(100, 7), media = [record(bytes, 5, 13), record(bytes, 50, 10)];
    const original = await hashProjectFile(reader(bytes).handle, bytes.length, media);
    bytes[80] = 9;
    const changed = await hashProjectFile(reader(bytes).handle, bytes.length, media);
    expect(changed).toBe(digest(bytes));
    expect(changed).not.toBe(original);
  });

  it('rejects a read ending before the expected file size', async () => {
    const bytes = Buffer.alloc(20, 7);
    await expect(hashProjectFile(reader(bytes).handle, bytes.length + 1, [record(bytes, 5, 3)])).rejects.toThrow('truncated');
  });

  it('retries a transient read deadlock at the same position', async () => {
    const bytes = Buffer.alloc(100, 7), { handle, read } = reader(bytes);
    read.mockRejectedValueOnce(Object.assign(new Error('read deadlock'), { code: 'EDEADLK' }));
    expect(await hashProjectFile(handle, bytes.length, [record(bytes, 5, 30)])).toBe(digest(bytes));
    expect(read.mock.calls.map(([, , , position]) => position)).toEqual([0, 0]);
  });

  it('does not start a positioned read after cancellation', async () => {
    const bytes = Buffer.alloc(100, 7), { handle, read } = reader(bytes), controller = new AbortController();
    const reason = new Error('Document validation failed'); controller.abort(reason);
    await expect(hashProjectFile(handle, bytes.length, [], controller.signal)).rejects.toBe(reason);
    expect(read).not.toHaveBeenCalled();
  });

  it('does not retry a transient pending read after document validation cancels hashing', async () => {
    const bytes = Buffer.alloc(100, 7), { handle, read } = reader(bytes), controller = new AbortController();
    const reason = new Error('Document validation failed');
    read.mockImplementationOnce(async () => {
      controller.abort(reason);
      throw Object.assign(new Error('read deadlock'), { code: 'EDEADLK' });
    });
    await expect(hashProjectFile(handle, bytes.length, [], controller.signal)).rejects.toBe(reason);
    expect(read).toHaveBeenCalledTimes(1);
  });
});
