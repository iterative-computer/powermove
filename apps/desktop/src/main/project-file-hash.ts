import { createHash } from 'node:crypto';
import type { FileHandle } from 'node:fs/promises';
import type { ProjectRecord } from '../shared/project-incremental';
import { FILE_CHUNK_BYTES } from './file-upload';
import { readWithRetry } from './read-retry';

/** Hash the file and verify its already validated, non-overlapping media ranges
 * in one bounded read. Unreferenced bytes still protect the overwrite grant. */
export async function hashProjectFile(handle: Pick<FileHandle, 'read'>, size: number, media: readonly ProjectRecord[] = [], signal?: AbortSignal): Promise<string> {
  const hash = createHash('sha256');
  const emptyHash = createHash('sha256').digest('hex');
  for (const record of media) {
    if (!record.length && record.sha256 !== emptyHash) throw new Error('The project media checksum does not match.');
  }
  const ranges = media.filter(record => record.length).sort((a, b) => a.offset - b.offset);
  let index = 0, mediaHash = ranges.length ? createHash('sha256') : undefined;
  const buffer = Buffer.allocUnsafe(Math.min(FILE_CHUNK_BYTES, size));
  for (let position = 0; position < size;) {
    signal?.throwIfAborted();
    const { bytesRead } = await readWithRetry(() => {
      signal?.throwIfAborted();
      return handle.read(buffer, 0, Math.min(buffer.length, size - position), position);
    });
    if (!bytesRead) throw new Error('The project container is truncated.');
    const end = position + bytesRead;
    hash.update(buffer.subarray(0, bytesRead));
    while (index < ranges.length) {
      const record = ranges[index]!;
      if (record.offset >= end) break;
      const start = Math.max(position, record.offset), stop = Math.min(end, record.offset + record.length);
      if (stop > start) mediaHash!.update(buffer.subarray(start - position, stop - position));
      if (record.offset + record.length > end) break;
      if (mediaHash!.digest('hex') !== record.sha256) throw new Error('The project media checksum does not match.');
      index += 1;
      mediaHash = index < ranges.length ? createHash('sha256') : undefined;
    }
    position = end;
  }
  if (index !== ranges.length) throw new Error('The project container is truncated.');
  return hash.digest('hex');
}
