/*
 * A media hash worker thread (hasher.ts starts a small pool). Each message
 * is one job from hash-job.ts; replies carry the result or the error, and
 * progress in bytes at most every 64 MiB.
 */
import { parentPort } from 'node:worker_threads';

import { runHashJob, type HashJob } from './hash-job';
import type { WorkerReply, WorkerRequest } from './hasher';

const port = parentPort!;
const cancelled = new Set<number>();
const PROGRESS_BYTES = 64 * 1024 * 1024;

port.on('message', (message: WorkerRequest) => {
  if (message.type === 'cancel') { cancelled.add(message.id); return; }
  const { id, job } = message as { id: number; job: HashJob };
  let pending = 0;
  const progress = (bytes: number) => {
    pending += bytes;
    if (pending >= PROGRESS_BYTES) { port.postMessage({ type: 'progress', id, bytes: pending } satisfies WorkerReply); pending = 0; }
  };
  runHashJob(job, progress, () => cancelled.has(id)).then(
    (result) => {
      if (pending) port.postMessage({ type: 'progress', id, bytes: pending } satisfies WorkerReply);
      port.postMessage({ type: 'done', id, result } satisfies WorkerReply);
    },
    (error: unknown) => port.postMessage({ type: 'failed', id, error: error instanceof Error ? error.message : String(error), changed: (error as Error)?.name === 'FileChangedError' } satisfies WorkerReply)
  ).finally(() => cancelled.delete(id));
});
