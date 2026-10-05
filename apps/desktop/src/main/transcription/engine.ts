import { availableParallelism } from 'node:os';
import path from 'node:path';

import type { TranscribeRequest, Transcript, TranscriptionStatus } from '../../shared/transcription';
import { TranscriptionModelMissingError, type TranscriptionService } from './service';
import { transcriptKey, type TranscriptCache } from './cache';
import { decodeLanguage, modelFile, type CatalogModel } from './catalog';
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
  /** Packaged app: the unpacked libtranscribe.dylib for the worker to load. */
  runtime?: string;
}

export class TranscriptionAbortedError extends Error {
  readonly name = 'AbortError';
  constructor() { super('Transcription was cancelled.'); }
}

type JobResult = { duration: number; segments: Transcript['segments']; language?: string };

/** One caller waiting on a job, with its own progress and cancel. */
interface Subscriber {
  onProgress?: (progress: number) => void;
  resolve(value: Transcript): void;
  reject(error: unknown): void;
}

interface Job {
  id: string;
  request: TranscribeRequest;
  model: CatalogModel;
  modelDir: string;
  /** Tag handed to the model; undefined lets it detect. */
  decodeLanguage?: string;
  key: string;
  /** Everyone waiting on this file and span; the job is cancelled when the last one leaves. */
  subscribers: Set<Subscriber>;
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
  if (request.wordTimestamps !== undefined && typeof request.wordTimestamps !== 'boolean') throw new Error('Invalid transcription request.');
  return {
    path: request.path,
    ...(start !== undefined ? { start } : {}),
    ...(end !== undefined ? { end } : {}),
    ...(request.language ? { language: request.language } : {}),
    ...(request.requestId ? { requestId: request.requestId } : {}),
    ...(request.wordTimestamps ? { wordTimestamps: true } : {})
  };
}

/** A model's identity for the transcript cache: a re-pinned file misses. */
export function modelKey(model: Pick<CatalogModel, 'id' | 'files'>): string {
  return `${model.id}@${model.files[0]?.sha256 ?? ''}`;
}

let sequence = 0;

export class TranscriptionEngine implements TranscriptionService {
  private worker: WorkerChannel | null = null;
  private readonly queue: Job[] = [];
  private current: Job | null = null;
  private idleTimer: ReturnType<typeof setTimeout> | null = null;
  /** Queued and running jobs by transcript key, so identical requests share one. */
  private readonly jobs = new Map<string, Job>();

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
    const setting = this.options.models.language();
    const active = this.options.models.modelFor({ ...(request.wordTimestamps ? { wordTimestamps: true } : {}), language: request.language ?? setting });
    if (!active) {
      throw new TranscriptionModelMissingError(request.wordTimestamps && this.options.models.activeModel()
        ? 'Captions need a speech model that times each word.'
        : undefined);
    }
    const spoken = decodeLanguage(active.model, request.language ?? setting);
    const key = await transcriptKey(request.path, modelKey(active.model), { ...request, ...(spoken ? { language: spoken } : {}) }).catch((error: unknown) => {
      const code = (error as NodeJS.ErrnoException)?.code;
      if (code === 'ENOENT') throw new Error('The media file could not be found.');
      throw error;
    });
    const language = transcriptLanguage(active.model, request.language, setting);
    const finish = (transcript: Transcript): Transcript => ({ ...transcript, ...(language ? { language } : {}) });
    const cached = await this.options.cache.get(key);
    if (cached) {
      onProgress?.(1);
      return finish(cached);
    }
    return finish(await new Promise<Transcript>((resolve, reject) => {
      /* The caller may have cancelled while the cache was read: its abort event has already fired. */
      if (signal?.aborted) {
        reject(new TranscriptionAbortedError());
        return;
      }
      const subscriber: Subscriber = { resolve, reject, ...(onProgress ? { onProgress } : {}) };
      /* Identical requests (same file, model and span) share one job; each caller can still cancel its own wait. */
      let job = this.jobs.get(key);
      if (!job) {
        job = {
          id: `t${++sequence}`,
          request,
          model: active.model,
          modelDir: active.dir,
          ...(spoken ? { decodeLanguage: spoken } : {}),
          key,
          subscribers: new Set(),
          started: false
        };
        this.jobs.set(key, job);
        this.queue.push(job);
      }
      job.subscribers.add(subscriber);
      const joined = job;
      signal?.addEventListener('abort', () => this.leave(joined, subscriber), { once: true });
      this.pump();
    }));
  }

  /** Stops the process and fails anything still queued (app quit). */
  dispose(): void {
    const jobs = [...this.queue, ...(this.current ? [this.current] : [])];
    this.queue.length = 0;
    this.current = null;
    for (const job of jobs) this.settle(job, new TranscriptionAbortedError());
    this.stopWorker();
  }

  /** Resolves or fails every caller waiting on the job. */
  private settle(job: Job, outcome: Transcript | Error): void {
    if (job.cancelTimer) clearTimeout(job.cancelTimer);
    if (this.jobs.get(job.key) === job) this.jobs.delete(job.key);
    const subscribers = [...job.subscribers];
    job.subscribers.clear();
    for (const subscriber of subscribers) {
      if (outcome instanceof Error) subscriber.reject(outcome);
      else subscriber.resolve(structuredClone(outcome));
    }
  }

  /** One caller cancelled. The job itself stops only once nobody is waiting on it. */
  private leave(job: Job, subscriber: Subscriber): void {
    if (!job.subscribers.delete(subscriber)) return;
    subscriber.reject(new TranscriptionAbortedError());
    if (job.subscribers.size) return;
    if (this.jobs.get(job.key) === job) this.jobs.delete(job.key);
    const index = this.queue.indexOf(job);
    if (index >= 0) {
      this.queue.splice(index, 1);
      return;
    }
    if (this.current !== job) return;
    this.worker?.post({ type: 'cancel', id: job.id });
    /* A long native decode cannot be interrupted: past the grace period, end the process. */
    job.cancelTimer = setTimeout(() => {
      if (this.current !== job) return;
      this.current = null;
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
        this.settle(job, new Error(`The transcription process stopped unexpectedly${code !== null ? ` (code ${code})` : ''}. Try again.`));
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
      model: { file: path.join(job.modelDir, modelFile(job.model).name), timing: job.model.timing, ...(job.model.lag ? { lag: job.model.lag } : {}) },
      ...(job.decodeLanguage ? { language: job.decodeLanguage } : {}),
      threads,
      ffmpeg: this.options.ffmpeg,
      ...(this.options.runtime ? { runtime: this.options.runtime } : {})
    });
  }

  private receive(worker: WorkerChannel, message: WorkerResponse): void {
    if (worker !== this.worker || message.type === 'ready') return;
    const job = this.current;
    if (!job || job.id !== message.id) return;
    if (message.type === 'progress') {
      for (const subscriber of job.subscribers) subscriber.onProgress?.(message.progress);
      return;
    }
    this.current = null;
    if (job.cancelTimer) clearTimeout(job.cancelTimer);
    if (message.type === 'done') {
      void this.complete(job, { duration: message.duration, segments: message.segments, ...(message.language ? { language: message.language } : {}) });
    } else {
      this.settle(job, message.type === 'cancelled' ? new TranscriptionAbortedError() : new Error(message.message));
    }
    this.pump();
  }

  /** Caches the transcript before answering, so a request arriving meanwhile still joins this job. */
  private async complete(job: Job, result: JobResult): Promise<void> {
    /* The model's own detection labels the transcript when nothing else will (finish() overrides it with a chosen language). */
    const detected = detectedLanguage(job.model, result.language);
    const transcript: Transcript = { modelId: job.model.id, ...(detected ? { language: detected } : {}), duration: result.duration, segments: result.segments };
    await this.options.cache.set(job.key, transcript).catch((error: unknown) => console.warn('[transcription] cache write failed', error));
    for (const subscriber of job.subscribers) subscriber.onProgress?.(1);
    this.settle(job, transcript);
  }
}

/**
 * The language a transcript is labelled with. Only a language the model can
 * actually produce: an English-only model is always English, a multilingual
 * one takes the caller's hint or the saved setting when it lists that
 * language, and otherwise leaves it unset (detected, unlabelled).
 */
/** A detected tag the model lists, as its base language (Nemotron's de-DE is de). */
export function detectedLanguage(model: Pick<CatalogModel, 'languageCodes'>, detected: string | undefined): string | undefined {
  const base = detected?.split(/[-_]/)[0]?.toLowerCase();
  return base && model.languageCodes.includes(base) ? base : undefined;
}

export function transcriptLanguage(model: Pick<CatalogModel, 'languages' | 'languageCodes'>, requested: string | undefined, setting: string): string | undefined {
  if (model.languages === 'en') return 'en';
  const supported = (tag: string | undefined): tag is string => !!tag && tag !== 'auto' && model.languageCodes.includes(tag.split('-')[0]!.toLowerCase());
  if (supported(requested)) return requested;
  if (supported(setting)) return setting;
  return undefined;
}
