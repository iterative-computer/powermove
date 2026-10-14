/*
 * Full-file BLAKE3 hashing off main's JavaScript thread.
 *
 * A file larger than one segment (16 MiB) is split into whole segments, each
 * a complete BLAKE3 subtree hashed by a pool worker, and a tail (the segment
 * holding the last byte, which may be the root) hashed after them with
 * their chaining values pushed in order. The digest is identical to a
 * single-threaded hash (`shared/blake3.ts`, the BLAKE3 specification, the
 * engine's `pm-media::hash`).
 *
 * Like `pm-media`'s background hasher, a file whose size or modification
 * time is still moving is waited on until it settles, and a file that
 * changes while it is hashed is hashed again.
 */
import type { Stats } from 'node:fs';
import { stat } from 'node:fs/promises';
import os from 'node:os';
import { Worker } from 'node:worker_threads';

import { CHUNK_LEN } from '../../shared/blake3';
import { runHashJob, type HashJob, type HashJobResult } from './hash-job';

/** Chunks per parallel segment: 16 MiB, a power of two. */
export const SEGMENT_CHUNKS = 16 * 1024;
export const SEGMENT_BYTES = SEGMENT_CHUNKS * CHUNK_LEN;

export type WorkerRequest = { type: 'run'; id: number; job: HashJob } | { type: 'cancel'; id: number };
export type WorkerReply =
  | { type: 'progress'; id: number; bytes: number }
  | { type: 'done'; id: number; result: HashJobResult }
  | { type: 'failed'; id: number; error: string; changed: boolean };

export interface HashExecutor {
  run(job: HashJob, progress?: (bytes: number) => void): Promise<HashJobResult>;
  readonly parallelism: number;
  close(): Promise<void>;
}

export class FileChanged extends Error {
  override name = 'FileChanged';
  constructor(file: string) { super(`“${file}” kept changing while it was being read.`); }
}

/** Runs jobs on this thread (tests, and the CLI-free fallback). */
export class InProcessExecutor implements HashExecutor {
  readonly parallelism = 1;
  run(job: HashJob, progress?: (bytes: number) => void): Promise<HashJobResult> {
    return runHashJob(job, progress).catch((error: unknown) => {
      throw (error as Error)?.name === 'FileChangedError' ? Object.assign(new Error((error as Error).message), { changed: true }) : error;
    });
  }
  async close(): Promise<void> {}
}

type Pending = { job: HashJob; resolve: (r: HashJobResult) => void; reject: (e: Error) => void; progress?: (bytes: number) => void };
type Slot = { worker: Worker; busy: number | null };

/** A small pool of worker threads, started on demand and stopped when idle. */
export class WorkerPool implements HashExecutor {
  private readonly slots: Slot[] = [];
  private readonly queue: Array<{ id: number } & Pending> = [];
  private readonly running = new Map<number, Pending & { slot: Slot }>();
  private nextId = 1;
  private idleTimer: ReturnType<typeof setTimeout> | null = null;
  private closed = false;

  constructor(private readonly entry: string, readonly parallelism = Math.max(1, Math.min(3, Math.floor(os.availableParallelism() / 4))),
    private readonly idleMs = 30_000) {}

  run(job: HashJob, progress?: (bytes: number) => void): Promise<HashJobResult> {
    if (this.closed) return Promise.reject(new Error('The media hasher is closed.'));
    return new Promise((resolve, reject) => {
      this.queue.push({ id: this.nextId++, job, resolve, reject, ...(progress ? { progress } : {}) });
      this.pump();
    });
  }

  async close(): Promise<void> {
    this.closed = true;
    if (this.idleTimer) clearTimeout(this.idleTimer);
    for (const item of this.queue.splice(0)) item.reject(new Error('The media hasher is closed.'));
    await Promise.all(this.slots.splice(0).map((slot) => slot.worker.terminate()));
  }

  private pump(): void {
    if (this.idleTimer) { clearTimeout(this.idleTimer); this.idleTimer = null; }
    while (this.queue.length) {
      let slot = this.slots.find((s) => s.busy === null);
      if (!slot && this.slots.length < this.parallelism) slot = this.spawn();
      if (!slot) return;
      const item = this.queue.shift()!;
      slot.busy = item.id;
      this.running.set(item.id, { ...item, slot });
      slot.worker.postMessage({ type: 'run', id: item.id, job: item.job } satisfies WorkerRequest);
    }
    if (!this.running.size && this.slots.length) {
      this.idleTimer = setTimeout(() => {
        this.idleTimer = null;
        if (this.running.size || this.queue.length) return;
        for (const slot of this.slots.splice(0)) void slot.worker.terminate();
      }, this.idleMs);
      this.idleTimer.unref?.();
    }
  }

  private spawn(): Slot {
    const worker = new Worker(this.entry, { name: 'Powermove media hash' });
    worker.unref();
    const slot: Slot = { worker, busy: null };
    worker.on('message', (reply: WorkerReply) => {
      const item = this.running.get(reply.id);
      if (!item) return;
      if (reply.type === 'progress') { item.progress?.(reply.bytes); return; }
      this.running.delete(reply.id);
      slot.busy = null;
      if (reply.type === 'done') item.resolve(reply.result);
      else item.reject(Object.assign(new Error(reply.error), { changed: reply.changed }));
      this.pump();
    });
    const fail = (error: Error) => {
      const index = this.slots.indexOf(slot);
      if (index >= 0) this.slots.splice(index, 1);
      if (slot.busy !== null) {
        const item = this.running.get(slot.busy);
        this.running.delete(slot.busy);
        item?.reject(error);
      }
      if (!this.closed) this.pump();
    };
    worker.on('error', fail);
    worker.on('exit', (code) => { if (this.slots.includes(slot)) fail(new Error(`The media hash worker stopped (exit ${code}).`)); });
    this.slots.push(slot);
    return slot;
  }
}

export type FileIdentity = {
  /** BLAKE3 of the whole file, 64 hex digits. */
  hash: string;
  size: number;
  mtimeMs: number;
  dev: number;
  ino: number;
};

export type HashFileOptions = {
  progress?: (done: number, total: number) => void;
  /** How long size and mtime must stay put before hashing (default 2 s). */
  settleMs?: number;
  /** How long to wait for a moving file before giving up (default 30 s). */
  settleTimeoutMs?: number;
  now?: () => number;
  sleep?: (ms: number) => Promise<void>;
  /** Attempts when the file changes while hashed (default 3). */
  attempts?: number;
};

const defaultSleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** Waits until the file's size and mtime have been still for `settleMs`. */
async function settled(file: string, options: HashFileOptions): Promise<Stats> {
  const now = options.now ?? Date.now;
  const sleep = options.sleep ?? defaultSleep;
  const quiet = options.settleMs ?? 2000;
  const deadline = now() + (options.settleTimeoutMs ?? 30_000);
  let last = await stat(file);
  let stillSince = now();
  for (;;) {
    if (!last.isFile()) throw new Error(`“${file}” is not a file.`);
    // A file last written long enough ago is already settled.
    if (now() - last.mtimeMs >= quiet || now() - stillSince >= quiet) return last;
    if (now() > deadline) throw new FileChanged(file);
    await sleep(Math.min(250, quiet));
    const next = await stat(file);
    if (next.size !== last.size || next.mtimeMs !== last.mtimeMs) stillSince = now();
    last = next;
  }
}

/** The file's BLAKE3 digest and the stat it was taken against. */
export async function hashFile(executor: HashExecutor, file: string, options: HashFileOptions = {}): Promise<FileIdentity> {
  const attempts = options.attempts ?? 3;
  for (let attempt = 1; ; attempt++) {
    const before = await settled(file, options);
    const size = before.size;
    let done = 0;
    const progress = (bytes: number) => { done += bytes; options.progress?.(done, size); };
    try {
      const segments = size > SEGMENT_BYTES ? Math.floor((size - 1) / SEGMENT_BYTES) : 0;
      const results = await Promise.all(Array.from({ length: segments }, (_, index) =>
        executor.run({ op: 'subtree', path: file, firstChunk: index * SEGMENT_CHUNKS, chunks: SEGMENT_CHUNKS }, progress)));
      const subtrees = new Uint32Array(segments * 8);
      results.forEach((result, index) => {
        if (result.op !== 'subtree') throw new Error('A subtree job returned the wrong result.');
        subtrees.set(result.chainingValue, index * 8);
      });
      const tail = await executor.run({ op: 'tail', path: file, subtrees, subtreeChunks: SEGMENT_CHUNKS, offset: segments * SEGMENT_BYTES, end: size }, progress);
      if (tail.op !== 'tail' || tail.bytes !== size) throw new Error('The tail job returned the wrong result.');
      const after = await stat(file);
      if (after.size !== before.size || after.mtimeMs !== before.mtimeMs || after.ino !== before.ino) throw Object.assign(new FileChanged(file), { changed: true });
      return { hash: tail.hex, size, mtimeMs: before.mtimeMs, dev: before.dev, ino: before.ino };
    } catch (error) {
      if (!(error as { changed?: boolean })?.changed || attempt >= attempts) throw error instanceof FileChanged || !(error as { changed?: boolean })?.changed ? error : new FileChanged(file);
    }
  }
}
