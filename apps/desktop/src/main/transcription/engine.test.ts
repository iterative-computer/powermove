import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('electron', () => ({}));

import { TRANSCRIPTION_IPC, TRANSCRIPTION_MODEL_MISSING, type TranscribeResult } from '../../shared/transcription';
import { TranscriptCache } from './cache';
import { TranscriptionAbortedError, TranscriptionEngine, validateRequest, type WorkerChannel } from './engine';
import { registerTranscriptionIpc } from './ipc';
import type { ModelStore } from './models';
import type { WorkerRequest, WorkerResponse } from './worker-protocol';

let dir = '';
let media = '';

beforeEach(async () => {
  dir = await mkdtemp(path.join(os.tmpdir(), 'pm-engine-'));
  media = path.join(dir, 'clip.wav');
  await writeFile(media, 'not really audio');
});
afterEach(async () => { await rm(dir, { recursive: true, force: true }); });

class FakeWorker implements WorkerChannel {
  posted: WorkerRequest[] = [];
  killed = false;
  private listener: ((message: WorkerResponse) => void) | null = null;
  private exit: ((code: number | null) => void) | null = null;
  post(message: WorkerRequest) { this.posted.push(message); }
  onMessage(listener: (message: WorkerResponse) => void) { this.listener = listener; }
  onExit(listener: (code: number | null) => void) { this.exit = listener; }
  kill() { this.killed = true; }
  reply(message: WorkerResponse) { this.listener?.(message); }
  crash(code = 9) { this.exit?.(code); }
  last() { return this.posted.at(-1) as Extract<WorkerRequest, { type: 'transcribe' }>; }
}

function fakeModels(ready = true, language = 'auto'): ModelStore {
  const model = { id: 'parakeet', languages: 'multi' };
  return {
    load: async () => undefined,
    status: () => ({ models: [], activeModelId: ready ? 'parakeet' : null }),
    activeModel: () => (ready ? { model, dir: '/models/parakeet' } : null),
    language: () => language
  } as unknown as ModelStore;
}

const segments = [{ text: 'Hello there.', start: 0.5, end: 1.2, words: [{ text: 'Hello', start: 0.5, end: 0.8 }, { text: 'there.', start: 0.8, end: 1.2 }] }];
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function engine(options: { ready?: boolean; language?: string } = {}) {
  const workers: FakeWorker[] = [];
  const requestModel = vi.fn();
  const instance = new TranscriptionEngine({
    models: fakeModels(options.ready ?? true, options.language),
    cache: new TranscriptCache(path.join(dir, 'cache')),
    ffmpeg: '/bin/ffmpeg',
    spawnWorker: () => { const worker = new FakeWorker(); workers.push(worker); return worker; },
    requestModel,
    cancelGraceMs: 30,
    idleMs: 50,
    threads: 2
  });
  return { instance, workers, requestModel };
}

async function until(check: () => boolean) {
  for (let tries = 0; tries < 200 && !check(); tries++) await flush();
}

describe('TranscriptionEngine', () => {
  it('rejects with the model-missing code when nothing is ready', async () => {
    const { instance } = engine({ ready: false });
    await expect(instance.transcribe({ path: media })).rejects.toMatchObject({ code: TRANSCRIPTION_MODEL_MISSING });
  });

  it('validates requests at the boundary', () => {
    expect(() => validateRequest({ path: 'relative.wav' })).toThrow(/absolute path/);
    expect(() => validateRequest({ path: '/a.wav', start: 5, end: 2 })).toThrow(/after its start/);
    expect(() => validateRequest({ path: '/a.wav', start: Number.NaN })).toThrow();
    expect(() => validateRequest({ path: '/a.wav', language: 'English' })).toThrow();
    expect(validateRequest({ path: '/a.wav', start: 1, end: 2, language: 'de', requestId: 'r1' })).toEqual({ path: '/a.wav', start: 1, end: 2, language: 'de', requestId: 'r1' });
  });

  it('runs a job in the worker, reports progress, and answers repeats from the cache', async () => {
    const { instance, workers } = engine({ language: 'de' });
    const progress: number[] = [];
    const pending = instance.transcribe({ path: media, start: 2, end: 9 }, (value) => progress.push(value));
    await until(() => workers.length === 1 && workers[0]!.posted.length === 1);
    const job = workers[0]!.last();
    expect(job).toMatchObject({ type: 'transcribe', file: media, start: 2, end: 9, modelDir: '/models/parakeet', threads: 2, ffmpeg: '/bin/ffmpeg' });
    workers[0]!.reply({ type: 'progress', id: job.id, progress: 0.5 });
    workers[0]!.reply({ type: 'done', id: job.id, duration: 7, segments });
    const transcript = await pending;
    expect(transcript).toEqual({ modelId: 'parakeet', duration: 7, segments, language: 'de' });
    expect(progress).toEqual([0.5, 1]);

    const again = await instance.transcribe({ path: media, start: 2, end: 9, language: 'fr' });
    expect(again).toEqual({ ...transcript, language: 'fr' });
    expect(workers[0]!.posted).toHaveLength(1);
    // A different span is a different transcript.
    void instance.transcribe({ path: media, start: 3 }).catch(() => undefined);
    await until(() => workers[0]!.posted.length === 2);
    instance.dispose();
  });

  it('runs one job at a time and shares identical requests', async () => {
    const { instance, workers } = engine();
    const other = path.join(dir, 'other.wav');
    await writeFile(other, 'other');
    const first = instance.transcribe({ path: media });
    const twin = instance.transcribe({ path: media });
    const second = instance.transcribe({ path: other });
    await until(() => workers[0]?.posted.length === 1);
    await flush();
    expect(workers[0]!.posted).toHaveLength(1);
    workers[0]!.reply({ type: 'done', id: workers[0]!.last().id, duration: 1, segments });
    await until(() => workers[0]!.posted.length === 2);
    expect(workers[0]!.last().file).toBe(other);
    workers[0]!.reply({ type: 'done', id: workers[0]!.last().id, duration: 2, segments: [] });
    expect((await first).duration).toBe(1);
    expect((await twin).duration).toBe(1);
    expect((await second).duration).toBe(2);
  });

  it('cancels a running job, killing the worker if it does not stop', async () => {
    const { instance, workers } = engine();
    const controller = new AbortController();
    const pending = instance.transcribe({ path: media }, undefined, controller.signal);
    await until(() => workers[0]?.posted.length === 1);
    controller.abort();
    expect(workers[0]!.posted.at(-1)).toMatchObject({ type: 'cancel' });
    await expect(pending).rejects.toBeInstanceOf(TranscriptionAbortedError);
    expect(workers[0]!.killed).toBe(true);
    // The next job gets a fresh worker.
    void instance.transcribe({ path: media }).catch(() => undefined);
    await until(() => workers.length === 2);
    expect(workers[1]!.posted).toHaveLength(1);
    instance.dispose();
  });

  it('reports a crashed worker and keeps going', async () => {
    const { instance, workers } = engine();
    const pending = instance.transcribe({ path: media });
    await until(() => workers[0]?.posted.length === 1);
    workers[0]!.crash(11);
    await expect(pending).rejects.toThrow(/stopped unexpectedly \(code 11\)/);
    const retry = instance.transcribe({ path: media });
    await until(() => workers.length === 2 && workers[1]!.posted.length === 1);
    workers[1]!.reply({ type: 'error', id: workers[1]!.last().id, message: 'This file has no audio to transcribe.' });
    await expect(retry).rejects.toThrow('This file has no audio to transcribe.');
  });

  it('stops the idle worker', async () => {
    const { instance, workers } = engine();
    const pending = instance.transcribe({ path: media });
    await until(() => workers[0]?.posted.length === 1);
    workers[0]!.reply({ type: 'done', id: workers[0]!.last().id, duration: 1, segments });
    await pending;
    await new Promise((resolve) => setTimeout(resolve, 80));
    expect(workers[0]!.killed).toBe(true);
  });
});

describe('transcription IPC', () => {
  function register(service: Partial<TranscriptionEngine>, trusted = true) {
    const handlers = new Map<string, (event: unknown, ...args: unknown[]) => Promise<unknown>>();
    const models = {
      load: async () => undefined,
      status: () => ({ models: [], activeModelId: null }),
      download: vi.fn(async () => undefined),
      cancel: vi.fn(async () => undefined),
      remove: vi.fn(async () => undefined),
      setActive: vi.fn(async () => undefined),
      setLanguage: vi.fn(async () => undefined),
      root: '/models'
    };
    const reveal = vi.fn();
    registerTranscriptionIpc({ handle: (channel: string, handler: never) => handlers.set(channel, handler) } as never, {
      service: service as TranscriptionEngine,
      models: models as unknown as ModelStore,
      isTrustedSender: () => trusted,
      reveal
    });
    const sent: unknown[][] = [];
    const sender = { id: 7, isDestroyed: () => false, send: (...args: unknown[]) => sent.push(args), once: vi.fn(), removeListener: vi.fn() };
    const call = (channel: string, ...args: unknown[]) => handlers.get(channel)!({ sender }, ...args);
    return { call, models, reveal, sent };
  }

  it('refuses untrusted senders and malformed payloads', async () => {
    const untrusted = register({}, false);
    await expect(untrusted.call(TRANSCRIPTION_IPC.status)).rejects.toThrow(/unauthorized/);
    const { call, models } = register({ status: async () => ({ models: [], activeModelId: null }) });
    await expect(call(TRANSCRIPTION_IPC.download, '../../etc')).rejects.toThrow(/model id/);
    await expect(call(TRANSCRIPTION_IPC.setLanguage, 'Klingon!')).rejects.toThrow(/language/);
    await call(TRANSCRIPTION_IPC.download, 'parakeet-tdt-0.6b-v3');
    expect(models.download).toHaveBeenCalledWith('parakeet-tdt-0.6b-v3');
    await call(TRANSCRIPTION_IPC.setLanguage, 'auto');
    expect(models.setLanguage).toHaveBeenCalledWith('auto');
  });

  it('keeps the model-missing code across the boundary and streams progress', async () => {
    const missing = register({ transcribe: async () => { throw Object.assign(new Error('No model'), { code: TRANSCRIPTION_MODEL_MISSING }); } });
    expect(await missing.call(TRANSCRIPTION_IPC.transcribe, { path: media })).toEqual({ ok: false, code: TRANSCRIPTION_MODEL_MISSING, message: 'No model' });
    expect(await missing.call(TRANSCRIPTION_IPC.transcribe, { path: 'x' })).toMatchObject({ ok: false });

    const transcript = { modelId: 'm', duration: 1, segments };
    const ok = register({
      transcribe: async (_request, onProgress) => { onProgress?.(0.25); return transcript; }
    });
    const result = await ok.call(TRANSCRIPTION_IPC.transcribe, { path: media, requestId: 'r1' }) as TranscribeResult;
    expect(result).toEqual({ ok: true, transcript });
    expect(ok.sent).toEqual([[TRANSCRIPTION_IPC.progress, { requestId: 'r1', progress: 0.25 }]]);
  });

  it('cancels a transcription by request id', async () => {
    let seen: AbortSignal | undefined;
    const { call } = register({
      transcribe: (_request, _progress, signal) => new Promise((_resolve, reject) => {
        seen = signal;
        signal?.addEventListener('abort', () => reject(new TranscriptionAbortedError()));
      })
    });
    const pending = call(TRANSCRIPTION_IPC.transcribe, { path: media, requestId: 'r2' });
    await until(() => !!seen);
    await call(TRANSCRIPTION_IPC.cancelTranscribe, 'r2');
    expect(await pending).toMatchObject({ ok: false, code: 'aborted' });
  });
});
