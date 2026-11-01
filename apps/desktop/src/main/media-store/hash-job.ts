/*
 * The hashing work itself, shared by the hash worker threads
 * (hash-worker.ts) and the in-process executor tests use. A job reads one
 * file range and returns either a digest (the tail, which may be the root)
 * or a subtree chaining value (a power-of-two run of whole chunks that does
 * not hold the file's last byte). `hasher.ts` splits files and assembles.
 */
import { open } from 'node:fs/promises';

import { Blake3, CHUNK_LEN, toHex } from '../../shared/blake3';

/** Reads go through one 4 MiB buffer at offset 0, so its words are aligned. */
export const READ_BYTES = 4 * 1024 * 1024;

export type HashJob =
  /** The chaining value of `chunks` whole chunks starting at chunk `firstChunk`. */
  | { op: 'subtree'; path: string; firstChunk: number; chunks: number }
  /** The digest: push `subtrees` (8 words each, `subtreeChunks` chunks each)
   *  in order, then hash bytes from `offset` to `end`. */
  | { op: 'tail'; path: string; subtrees: Uint32Array; subtreeChunks: number; offset: number; end: number };

export type HashJobResult =
  | { op: 'subtree'; chainingValue: Uint32Array }
  | { op: 'tail'; hex: string; bytes: number };

export type JobProgress = (bytes: number) => void;

/** Thrown when a read comes up short: the file shrank while it was read. */
export class FileChangedError extends Error {
  override name = 'FileChangedError';
  constructor() { super('The file changed while it was being hashed.'); }
}

async function feed(path: string, hasher: Blake3, start: number, end: number, progress: JobProgress, cancelled: () => boolean): Promise<void> {
  const handle = await open(path, 'r');
  try {
    const buffer = new Uint8Array(new ArrayBuffer(READ_BYTES));
    for (let at = start; at < end;) {
      if (cancelled()) throw new Error('cancelled');
      const want = Math.min(READ_BYTES, end - at);
      let filled = 0;
      while (filled < want) {
        const { bytesRead } = await handle.read(buffer, filled, want - filled, at + filled);
        if (bytesRead === 0) throw new FileChangedError();
        filled += bytesRead;
      }
      hasher.update(buffer.subarray(0, want));
      at += want;
      progress(want);
    }
  } finally {
    await handle.close();
  }
}

export async function runHashJob(job: HashJob, progress: JobProgress = () => {}, cancelled: () => boolean = () => false): Promise<HashJobResult> {
  if (job.op === 'subtree') {
    const hasher = new Blake3(job.firstChunk);
    const start = job.firstChunk * CHUNK_LEN;
    await feed(job.path, hasher, start, start + job.chunks * CHUNK_LEN, progress, cancelled);
    return { op: 'subtree', chainingValue: hasher.subtreeChainingValue() };
  }
  const hasher = new Blake3();
  const count = job.subtrees.length / 8;
  if (!Number.isInteger(count)) throw new Error('Subtree chaining values are 8 words each.');
  for (let index = 0; index < count; index++) hasher.pushSubtree(job.subtrees.subarray(index * 8, index * 8 + 8), job.subtreeChunks);
  if (hasher.length !== job.offset) throw new Error('The tail does not start where the subtrees end.');
  await feed(job.path, hasher, job.offset, job.end, progress, cancelled);
  return { op: 'tail', hex: toHex(hasher.digest()), bytes: hasher.length };
}
