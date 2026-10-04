import { availableParallelism } from 'node:os';
import path from 'node:path';

import type { TranscribeRequest, Transcript, TranscriptionStatus } from '../../shared/transcription';
import { TranscriptionModelMissingError, type TranscriptionService } from './service';
import { transcriptKey, type TranscriptCache } from './cache';
import type { ModelStore } from './models';
import type { WorkerRequest, WorkerResponse } from './worker-protocol';

/*
 * The real TranscriptionService: checks the request, answers from the cache,
 * otherwise queues the job for the transcription process. Jobs run one at a
 * time (the model already uses several cores); the process is started on
 * first use and stopped after a quiet spell so its model memory goes back.
 */

export interface WorkerChannel {
  post(message: WorkerRequest): void;
  onMessage(listener: (message: WorkerResponse) => void): void;
  onExit(listener: (code: number | null) => void): void;
  kill(): void;
}

export interface EngineOptions {
  models: ModelStore;
  cache: TranscriptCache;
  ffmpeg: string;
  spawnWorker: () => WorkerChannel;
  requestModel: (reason: string) => void;
  /** Stop the idle process after this long. */
  idleMs?: number;
  /** After a cancel, kill the process if the job has not stopped by then. */
  cancelGraceMs?: number;
  threads?: number;
}

export class TranscriptionAbortedError extends Error {
  readonly name = 'AbortError';
  constructor() { super('Transcription was cancelled.'); }
}

interface Job {
  id: string;
  request: TranscribeRequest;
  modelId: string;
  modelDir: string;
  key: string | null;
  onProgress?: (progress: number) => void;
  signal?: AbortSignal;
  resolve(value: { duration: number; segments: Transcript['segments'] }): void;
  reject(error: unknown): void;
  started: boolean;
  cancelTimer?: ReturnType<typeof setTimeout>;
}

const MAX_SECONDS = 24 * 60 * 60;

export function validateRequest(request: TranscribeRequest): TranscribeRequest {
  if (!request || typeof request.path !== 'string' || !path.isAbsolute(request.path) || request.path.length > 4096) {
    throw new Error('Transcription needs an absolute path to a media file.');
  }
  const time = (value: unknown, name: string): number | undefined => {
    if (value === undefined || value === null) return undefined;
    if (typeof value !== 'number' || !Number.isFinite(value) || value < 0 || value > MAX_SECONDS) throw new Error(`Transcription ${name} must be a time in seconds.`);
    return value;
  };
  const start = time(request.start, 'start');
  const end = time(request.end, 'end');
  if (start !== undefined && end !== undefined && end <= start) throw new Error('Transcription end must come after its start.');
  if (request.language !== undefined && (typeof request.language !== 'string' || !/^[a-z]{2,3}(?:-[A-Za-z0-9]{2,8}){0,2}$/.test(request.language))) {
    throw new Error('Transcription language must be a language tag such as “en”.');
  }
  if (request.requestId !== undefined && (typeof request.requestId !== 'string' || request.requestId.length > 128)) throw new Error('Invalid transcription request id.');
  return {
    path: request.path,
    ...(start !== undefined ? { start } : {}),
    ...(end !== undefined ? { end } : {}),
    ...(request.language ? { language: request.language } : {}),
    ...(request.requestId ? { requestId: request.requestId } : {})
  };
}

let sequence = 0;

export class TranscriptionEngine implements TranscriptionService {
  private worker: WorkerChannel | null = null;
  private readonly queue: Job[] = [];
  private current: Job | null = null;
  private idleTimer: ReturnType<typeof setTimeout> | null = null;
  /** Identical requests in flight share one job. */
  private readonly inflight = new Map<string, Promise<Transcript>>();

  constructor(private readonly options: EngineOptions) {}

  async status(): Promise<TranscriptionStatus> {
    await this.options.models.load();
    return this.options.models.status();
  }

  requestModel(reason: string): void {
    this.options.requestModel(reason);
  }

  async transcribe(raw: TranscribeRequest, onProgress?: (progress: number) => void, signal?: AbortSignal): Promise<Transcript> {
    const request = validateRequest(raw);
    if (signal?.aborted) throw new TranscriptionAbortedError();
    await this.options.models.load();
    const active = this.options.models.activeModel();
    if (!active) throw new TranscriptionModelMissingError();
    const key = await transcriptKey(request.path, active.model.id, request).catch((error: unknown) => {
      const code = (error as NodeJS.ErrnoException)?.code;
      if (code === 'ENOENT') throw new Error('The media file could not be found.');
      throw error;
    });
    const language = request.language
      ?? (this.options.models.language() !== 'auto' ? this.options.models.language() : active.model.languages === 'en' ? 'en' : undefined);
    const finish = (transcript: Transcript): Transcript => ({ ...transcript, ...(language ? { language } : {}) });
    const cached = await this.options.cache.get(key);
    if (cached) {
      onProgress?.(1);
      return finish(cached);
    }
    /* A second caller for the same file and span waits on the first job. */
    const shared = this.inflight.get(key);
    if (shared && !signal) return finish(await shared);
    const run = new Promise<{ duration: number; segments: Transcript['segments'] }>((resolve, reject) => {
      const job: Job = {
        id: `t${++sequence}`,
        request,
        modelId: active.model.id,
        modelDir: active.dir,
        key,
        ...(onProgress ? { onProgress } : {}),
        ...(signal ? { signal } : {}),
        resolve,
        reject,
        started: false
      };
      signal?.addEventListener('abort', () => this.abort(job), { once: true });
      this.queue.push(job);
      this.pump();
    }).then(async ({ duration, segments }) => {
      const transcript: Transcript = { modelId: active.model.id, duration, segments };
      await this.options.cache.set(key, transcript).catch((error: unknown) => console.warn('[transcription] cache write failed', error));
      return transcript;
    });
    if (!signal) {
      this.inflight.set(key, run);
      void run.catch(() => undefined).finally(() => { if (this.inflight.get(key) === run) this.inflight.delete(key); });
    }
    return finish(await run);
  }

  /** Stops the process and fails anything still queued (app quit). */
  dispose(): void {
    for (const job of [...this.queue]) job.reject(new TranscriptionAbortedError());
    this.queue.length = 0;
    this.current?.reject(new TranscriptionAbortedError());
    this.current = null;
    this.stopWorker();
  }

  private abort(job: Job): void {
    const index = this.queue.indexOf(job);
    if (index >= 0) {
      this.queue.splice(index, 1);
      job.reject(new TranscriptionAbortedError());
      return;
    }
    if (this.current !== job) return;
    this.worker?.post({ type: 'cancel', id: job.id });
    /* A long native decode cannot be interrupted: past the grace period, end the process. */
    job.cancelTimer = setTimeout(() => {
      if (this.current !== job) return;
      this.current = null;
      job.reject(new TranscriptionAbortedError());
      this.stopWorker();
      this.pump();
    }, this.options.cancelGraceMs ?? 3000);
  }

  private ensureWorker(): WorkerChannel {
    if (this.worker) return this.worker;
    const worker = this.options.spawnWorker();
    this.worker = worker;
    worker.onMessage((message) => this.receive(worker, message));
    worker.onExit((code) => {
      if (this.worker !== worker) return;
      this.worker = null;
      const job = this.current;
      if (job) {
        this.current = null;
        if (job.cancelTimer) clearTimeout(job.cancelTimer);
        job.reject(job.signal?.aborted ? new TranscriptionAbortedError()
          : new Error(`The transcription process stopped unexpectedly${code !== null ? ` (code ${code})` : ''}. Try again.`));
        this.pump();
      }
    });
    return worker;
  }

  private stopWorker(): void {
    const worker = this.worker;
    this.worker = null;
    if (this.idleTimer) clearTimeout(this.idleTimer);
    this.idleTimer = null;
    worker?.kill();
  }

  private pump(): void {
    if (this.current) return;
    const job = this.queue.shift();
    if (!job) {
      if (this.idleTimer) clearTimeout(this.idleTimer);
      this.idleTimer = setTimeout(() => this.stopWorker(), this.options.idleMs ?? 3 * 60_000);
      return;
    }
    if (this.idleTimer) clearTimeout(this.idleTimer);
    this.idleTimer = null;
    this.current = job;
    job.started = true;
    const threads = this.options.threads ?? Math.max(1, Math.min(6, availableParallelism() - 2));
    this.ensureWorker().post({
      type: 'transcribe',
      id: job.id,
      file: job.request.path,
      ...(job.request.start !== undefined ? { start: job.request.start } : {}),
      ...(job.request.end !== undefined ? { end: job.request.end } : {}),
      modelDir: job.modelDir,
      threads,
      ffmpeg: this.options.ffmpeg
    });
  }

  private receive(worker: WorkerChannel, message: WorkerResponse): void {
    if (worker !== this.worker || message.type === 'ready') return;
    const job = this.current;
    if (!job || job.id !== message.id) return;
    if (message.type === 'progress') {
      job.onProgress?.(message.progress);
      return;
    }
    this.current = null;
    if (job.cancelTimer) clearTimeout(job.cancelTimer);
    if (message.type === 'done') {
      job.onProgress?.(1);
      job.resolve({ duration: message.duration, segments: message.segments });
    } else if (message.type === 'cancelled') {
      job.reject(new TranscriptionAbortedError());
    } else {
      job.reject(new Error(message.message));
    }
    this.pump();
  }
}
