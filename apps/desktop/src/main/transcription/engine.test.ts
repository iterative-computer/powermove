import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('electron', () => ({}));

import { TRANSCRIPTION_IPC, TRANSCRIPTION_MODEL_MISSING, type TranscribeResult } from '../../shared/transcription';
import { TranscriptCache } from './cache';
import { TranscriptionAbortedError, TranscriptionEngine, transcriptLanguage, validateRequest, type WorkerChannel } from './engine';
import { registerTranscriptionIpc } from './ipc';
import type { CatalogModel } from './catalog';
import { ModelStore } from './models';
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

const fakeModel = (id: string, extra: Partial<CatalogModel> = {}): CatalogModel => ({
  id, name: id, description: '', family: 'parakeet', languages: 'multi', languageCodes: ['en', 'de', 'fr'], detectsLanguage: true,
  timing: 'word', speed: 0.8, accuracy: 0.8, recommended: false, featured: false, wordTimestamps: true, license: '',
  files: [{ name: `${id}.gguf`, url: `https://example.com/${id}.gguf`, size: 10, sha256: 'a'.repeat(64) }],
  ...extra
});

/** A ModelStore over the real ModelStore.modelFor: `ready` in catalog order, the first one active. */
function fakeModels(ready: CatalogModel[] | boolean = true, language = 'auto'): ModelStore {
  const models = ready === true ? [fakeModel('parakeet')] : ready === false ? [] : ready;
  const store = {
    load: async () => undefined,
    status: () => ({ models: [], activeModelId: models[0]?.id ?? null }),
    activeModel: () => (models[0] ? { model: models[0], dir: `/models/${models[0].id}` } : null),
    language: () => language,
    modelDir: (id: string) => `/models/${id}`,
    options: { catalog: models },
    ready: new Set(models.map((model) => model.id))
  };
  (store as unknown as { modelFor: ModelStore['modelFor'] }).modelFor = ModelStore.prototype.modelFor.bind(store as unknown as ModelStore);
  return store as unknown as ModelStore;
}

const segments = [{ text: 'Hello there.', start: 0.5, end: 1.2, words: [{ text: 'Hello', start: 0.5, end: 0.8 }, { text: 'there.', start: 0.8, end: 1.2 }] }];
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function engine(options: { ready?: boolean | CatalogModel[]; language?: string } = {}) {
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
    expect(validateRequest({ path: '/a.wav', wordTimestamps: true })).toEqual({ path: '/a.wav', wordTimestamps: true });
    expect(() => validateRequest({ path: '/a.wav', wordTimestamps: 'yes' as never })).toThrow();
  });

  it('runs a job in the worker, reports progress, and answers repeats from the cache', async () => {
    const { instance, workers } = engine({ language: 'de' });
    const progress: number[] = [];
    const pending = instance.transcribe({ path: media, start: 2, end: 9 }, (value) => progress.push(value));
    await until(() => workers.length === 1 && workers[0]!.posted.length === 1);
    const job = workers[0]!.last();
    expect(job).toMatchObject({ type: 'transcribe', file: media, start: 2, end: 9, model: { file: '/models/parakeet/parakeet.gguf', timing: 'word' }, language: 'de', threads: 2, ffmpeg: '/bin/ffmpeg' });
    workers[0]!.reply({ type: 'progress', id: job.id, progress: 0.5 });
    workers[0]!.reply({ type: 'done', id: job.id, duration: 7, segments });
    const transcript = await pending;
    expect(transcript).toEqual({ modelId: 'parakeet', duration: 7, segments, language: 'de' });
    expect(progress).toEqual([0.5, 1]);

    const again = await instance.transcribe({ path: media, start: 2, end: 9, language: 'de' });
    expect(again).toEqual(transcript);
    expect(workers[0]!.posted).toHaveLength(1);
    // A different span is a different transcript.
    void instance.transcribe({ path: media, start: 3 }).catch(() => undefined);
    await until(() => workers[0]!.posted.length === 2);
    instance.dispose();
  });

  it('keys the cache by the language a model is told, and passes it and the lag to the worker', async () => {
    const canary = fakeModel('canary', { family: 'canary', detectsLanguage: false, timing: 'none', wordTimestamps: false, lag: [0.1, 0.05] });
    const { instance, workers } = engine({ ready: [canary] });
    const english = instance.transcribe({ path: media });
    await until(() => workers[0]?.posted.length === 1);
    expect(workers[0]!.last()).toMatchObject({ model: { file: '/models/canary/canary.gguf', timing: 'none', lag: [0.1, 0.05] }, language: 'en' });
    workers[0]!.reply({ type: 'done', id: workers[0]!.last().id, duration: 1, segments });
    await english;
    // German is a different transcript for a model that must be told: not a cache hit.
    const german = instance.transcribe({ path: media, language: 'de' });
    await until(() => workers[0]!.posted.length === 2);
    expect(workers[0]!.last()).toMatchObject({ language: 'de' });
    workers[0]!.reply({ type: 'done', id: workers[0]!.last().id, duration: 1, segments, language: 'de' });
    expect(await german).toMatchObject({ modelId: 'canary', language: 'de' });
    instance.dispose();
  });

  it('uses a downloaded word-timed model when the request needs word timing and the active one lacks it', async () => {
    const cohere = fakeModel('cohere', { family: 'cohere', detectsLanguage: false, timing: 'none', wordTimestamps: false });
    const english = fakeModel('unified', { languages: 'en', languageCodes: ['en'], detectsLanguage: false });
    const multi = fakeModel('nemotron');
    const { instance, workers } = engine({ ready: [cohere, english, multi], language: 'de' });
    const pending = instance.transcribe({ path: media, wordTimestamps: true });
    await until(() => workers[0]?.posted.length === 1);
    // German is the setting: the timed model that lists German wins over the first timed one.
    expect(workers[0]!.last().model.file).toBe('/models/nemotron/nemotron.gguf');
    workers[0]!.reply({ type: 'done', id: workers[0]!.last().id, duration: 1, segments });
    expect(await pending).toMatchObject({ modelId: 'nemotron', language: 'de' });
    instance.dispose();

    const untimed = engine({ ready: [cohere] });
    await expect(untimed.instance.transcribe({ path: media, wordTimestamps: true })).rejects.toMatchObject({
      code: TRANSCRIPTION_MODEL_MISSING, message: 'Captions need a speech model that times each word.'
    });
    // Without the requirement the active model runs as usual.
    void untimed.instance.transcribe({ path: media }).catch(() => undefined);
    await until(() => untimed.workers[0]?.posted.length === 1);
    expect(untimed.workers[0]!.last().model.timing).toBe('none');
    untimed.instance.dispose();
  });

  it('labels a transcript with the language the model detected', async () => {
    const { instance, workers } = engine();
    const pending = instance.transcribe({ path: media });
    await until(() => workers[0]?.posted.length === 1);
    expect(workers[0]!.last().language).toBeUndefined();
    workers[0]!.reply({ type: 'done', id: workers[0]!.last().id, duration: 1, segments, language: 'de-DE' });
    expect(await pending).toMatchObject({ language: 'de' });
    instance.dispose();
  });

  it('runs one job at a time and shares identical requests', async () => {
    const { instance, workers } = engine();
    const other = path.join(dir, 'other.wav');
    await writeFile(other, 'other');
    const first = instance.transcribe({ path: media });
    await until(() => workers[0]?.posted.length === 1);
    const twin = instance.transcribe({ path: media });
    const second = instance.transcribe({ path: other });
    for (let tries = 0; tries < 20; tries++) await flush();
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
    await until(() => workers[0]!.killed);
    expect(workers[0]!.killed).toBe(true);
    // The next job gets a fresh worker.
    void instance.transcribe({ path: media }).catch(() => undefined);
    await until(() => workers.length === 2);
    expect(workers[1]!.posted).toHaveLength(1);
    instance.dispose();
  });

  it('shares one job between callers that each pass their own signal', async () => {
    const { instance, workers } = engine();
    const a = new AbortController();
    const b = new AbortController();
    const progressA: number[] = [];
    const progressB: number[] = [];
    const first = instance.transcribe({ path: media }, (value) => progressA.push(value), a.signal);
    const second = instance.transcribe({ path: media }, (value) => progressB.push(value), b.signal);
    await until(() => workers[0]?.posted.length === 1);
    await flush();
    expect(workers[0]!.posted).toHaveLength(1);
    workers[0]!.reply({ type: 'progress', id: workers[0]!.last().id, progress: 0.4 });
    // One caller leaving does not stop the job the other still waits on.
    a.abort();
    await expect(first).rejects.toBeInstanceOf(TranscriptionAbortedError);
    expect(workers[0]!.posted.some((message) => message.type === 'cancel')).toBe(false);
    workers[0]!.reply({ type: 'done', id: workers[0]!.last().id, duration: 1, segments });
    expect((await second).segments).toEqual(segments);
    expect(progressA).toEqual([0.4]);
    expect(progressB).toEqual([0.4, 1]);
    instance.dispose();
  });

  it('cancels a job once every caller has left', async () => {
    const { instance, workers } = engine();
    const a = new AbortController();
    const b = new AbortController();
    const first = instance.transcribe({ path: media }, undefined, a.signal);
    const second = instance.transcribe({ path: media }, undefined, b.signal);
    await until(() => workers[0]?.posted.length === 1);
    a.abort();
    b.abort();
    await expect(first).rejects.toBeInstanceOf(TranscriptionAbortedError);
    await expect(second).rejects.toBeInstanceOf(TranscriptionAbortedError);
    expect(workers[0]!.posted.filter((message) => message.type === 'cancel')).toHaveLength(1);
    instance.dispose();
  });

  it('honours a cancel that lands while the request is being looked up', async () => {
    const { instance, workers } = engine();
    const controller = new AbortController();
    const pending = instance.transcribe({ path: media }, undefined, controller.signal);
    // Abort before the engine has reached the queue (it is awaiting the model store and cache).
    controller.abort();
    await expect(pending).rejects.toBeInstanceOf(TranscriptionAbortedError);
    await flush();
    expect(workers).toHaveLength(0);
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

describe('transcriptLanguage', () => {
  const v3 = { languages: 'multi' as const, languageCodes: ['en', 'de', 'fr'] };
  const v2 = { languages: 'en' as const, languageCodes: ['en'] };

  it('labels only languages the model can produce', () => {
    // German was chosen while V3 was active; V2 can only ever produce English.
    expect(transcriptLanguage(v2, undefined, 'de')).toBe('en');
    expect(transcriptLanguage(v2, 'fr', 'auto')).toBe('en');
    expect(transcriptLanguage(v3, undefined, 'de')).toBe('de');
    expect(transcriptLanguage(v3, 'fr', 'de')).toBe('fr');
    expect(transcriptLanguage(v3, 'fr-CA', 'auto')).toBe('fr-CA');
    // Outside the model's languages: the hint falls back to the setting, else unlabelled.
    expect(transcriptLanguage(v3, 'ja', 'de')).toBe('de');
    expect(transcriptLanguage(v3, 'ja', 'auto')).toBeUndefined();
    expect(transcriptLanguage(v3, undefined, 'auto')).toBeUndefined();
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
