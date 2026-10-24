import path from 'node:path';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const native = vi.hoisted(() => ({ load: vi.fn() }));
vi.mock('transcribe-cpp', () => ({ TranscribeModel: { load: native.load } }));

import { CATALOG } from './catalog';
import type { TranscriptCache } from './cache';
import { TranscriptionEngine, type WorkerChannel } from './engine';
import type { ModelStore } from './models';
import type { WorkerRequest, WorkerResponse } from './worker-protocol';

const parentPort = Object.getOwnPropertyDescriptor(process, 'parentPort');
const engines: TranscriptionEngine[] = [];
beforeEach(() => { vi.resetModules(); native.load.mockReset(); });
afterEach(() => {
  for (const engine of engines.splice(0)) engine.dispose();
  if (parentPort) Object.defineProperty(process, 'parentPort', parentPort);
  else delete (process as unknown as { parentPort?: unknown }).parentPort;
  vi.useRealTimers();
});

/** The actual worker handler and engine exchange their normal messages. Only
 * native model loading is replaced; no model files or processes are used. */
async function worker() {
  let receive!: (event: { data: WorkerRequest }) => void;
  let listener: ((message: WorkerResponse) => void) | undefined;
  const sent: WorkerResponse[] = [];
  const kill = vi.fn();
  Object.defineProperty(process, 'parentPort', { configurable: true, value: {
    on: (_event: string, callback: typeof receive) => { receive = callback; },
    postMessage: (message: WorkerResponse) => { sent.push(message); listener?.(message); }
  } });
  await import('./worker');
  const channel: WorkerChannel = {
    post: (message) => receive({ data: message }),
    onMessage: (callback) => { listener = callback; }, onExit: () => undefined, kill
  };
  return { channel, sent, kill };
}

function engine(channel: WorkerChannel) {
  const model = CATALOG[0]!;
  const instance = new TranscriptionEngine({
    models: { load: async () => undefined, activeModel: () => ({ model, dir: '/models/speech' }) } as unknown as ModelStore,
    cache: {} as TranscriptCache, ffmpeg: '/unused/ffmpeg', requestModel: () => undefined,
    spawnWorker: () => channel
    // Keep the production 180-second idle limit, rather than a short fixture limit.
  });
  engines.push(instance);
  return { instance, modelFile: path.join('/models/speech', model.files[0]!.name) };
}

it('releases a failed background warm-up as soon as native loading fails', async () => {
  const setup = await worker();
  vi.useFakeTimers();
  native.load.mockRejectedValue(new Error('invalid gguf model file'));
  const { instance } = engine(setup.channel);
  const start = Date.now();
  let settled: { value: boolean; elapsedMs: number } | undefined;
  void instance.warm({ release: true }).then((value) => { settled = { value, elapsedMs: Date.now() - start }; });
  await vi.advanceTimersByTimeAsync(0);
  expect(native.load).toHaveBeenCalledOnce();
  // The reproduction observes the old pending promise through its complete
  // production idle period, and the fixed path through its immediate reply.
  if (!settled) await vi.advanceTimersByTimeAsync(180_000);
  expect(settled).toEqual({ value: false, elapsedMs: 0 });
  expect(setup.kill).toHaveBeenCalledOnce();
});

it('keeps a load in progress and releases it immediately when its native failure arrives', async () => {
  const setup = await worker();
  vi.useFakeTimers();
  native.load.mockImplementation(() => new Promise((_resolve, reject) => {
    setTimeout(() => reject(new Error('invalid gguf model file')), 50);
  }));
  const { instance } = engine(setup.channel);
  let settled: boolean | undefined;
  void instance.warm({ release: true }).then((value) => { settled = value; });
  await vi.advanceTimersByTimeAsync(49);
  expect(settled).toBeUndefined();
  expect(setup.kill).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1);
  expect(settled).toBe(false);
  expect(setup.kill).toHaveBeenCalledOnce();
  expect(vi.getTimerCount()).toBe(0);
});

it('shares a failing native load while resolving every same-model warm-up and releasing once', async () => {
  const setup = await worker();
  vi.useFakeTimers();
  native.load.mockRejectedValue(new Error('invalid gguf model file'));
  const { instance } = engine(setup.channel);
  const first = instance.warm(), second = instance.warm();
  await vi.advanceTimersByTimeAsync(0);
  expect(await Promise.all([first, second])).toEqual([false, false]);
  expect(native.load).toHaveBeenCalledOnce();
  expect(setup.sent.filter((message) => message.type === 'warm-error')).toHaveLength(2);
  expect(setup.kill).toHaveBeenCalledOnce();
});

it('releases a partial native model if session creation fails', async () => {
  const setup = await worker();
  vi.useFakeTimers();
  const resources = new Set(['native-model']);
  const dispose = vi.fn(() => resources.clear());
  native.load.mockResolvedValue({
    createSession: () => { throw new Error('native session failed'); },
    dispose, capabilities: { languages: [] }
  });
  const { instance } = engine(setup.channel);
  const pending = instance.warm({ release: true });
  await vi.advanceTimersByTimeAsync(0);
  expect(await pending).toBe(false);
  expect(dispose).toHaveBeenCalledOnce();
  expect(resources.size).toBe(0);
  expect(setup.kill).toHaveBeenCalledOnce();
});

it('reports separate warm and transcription failures when both await the same native load', async () => {
  const setup = await worker();
  vi.useFakeTimers();
  native.load.mockRejectedValue(new Error('invalid gguf model file'));
  const model = { file: '/models/speech.gguf', timing: 'word' as const };
  setup.channel.post({ type: 'warm', id: 'warm-attempt', model, threads: 2 });
  setup.channel.post({ type: 'transcribe', id: 'caption-job', model, threads: 2, file: '/unused/audio.wav', ffmpeg: '/unused/ffmpeg' });
  await vi.advanceTimersByTimeAsync(0);
  expect(native.load).toHaveBeenCalledOnce();
  expect(setup.sent).toEqual([
    { type: 'ready' },
    { type: 'warm-error', id: 'warm-attempt', file: model.file },
    { type: 'error', id: 'caption-job', message: 'The speech model could not be loaded. Delete it in Settings › Transcription and download it again.' }
  ]);
});

it('retries a failed native model load rather than retaining its rejected promise', async () => {
  const setup = await worker();
  vi.useFakeTimers();
  const createSession = vi.fn(() => ({ run: vi.fn() }));
  native.load.mockRejectedValueOnce(new Error('invalid gguf model file')).mockResolvedValue({
    createSession, dispose: vi.fn(), capabilities: { languages: [] }
  });
  const model = { file: '/models/speech.gguf', timing: 'word' as const };
  setup.channel.post({ type: 'warm', id: 'first', model, threads: 2 });
  await vi.advanceTimersByTimeAsync(0);
  setup.channel.post({ type: 'warm', id: 'retry', model, threads: 2 });
  await vi.advanceTimersByTimeAsync(0);
  expect(native.load).toHaveBeenCalledTimes(2);
  expect(createSession).toHaveBeenCalledExactlyOnceWith({ nThreads: 2 });
  expect(setup.sent).toEqual([
    { type: 'ready' }, { type: 'warm-error', id: 'first', file: model.file },
    { type: 'loaded', file: model.file }
  ]);
});
