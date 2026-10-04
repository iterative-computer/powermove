/*
 * The transcription process. Main starts it with utilityProcess.fork (the
 * serve host uses child_process.fork) so model loading and inference never
 * run on the main or UI threads, and a native fault takes down only this
 * process. One job at a time; main queues the rest.
 *
 * The recognizer is sherpa-onnx's offline NeMo transducer (Parakeet TDT),
 * whose native addon ships inside the app and is signed with it.
 */
import { spawn, type ChildProcessByStdio } from 'node:child_process';
import type { Readable } from 'node:stream';

import { decodeArgs, decodeError, parseDuration, spanLength } from './audio';
import { Chunker, SAMPLE_RATE, appendAtSeam, wordsWithin, type AudioWindow } from './chunking';
import { tokensToWords, wordsToSegments } from './words';
import type { WorkerRequest, WorkerResponse } from './worker-protocol';
import type { TranscriptWord } from '../../shared/transcription';

interface SherpaResult { text: string; tokens: string[]; timestamps: number[]; durations?: number[]; ys_log_probs?: number[]; lang?: string }
interface SherpaStream { acceptWaveform(wave: { samples: Float32Array; sampleRate: number }): void }
interface SherpaRecognizer { createStream(): SherpaStream; decodeAsync(stream: SherpaStream): Promise<SherpaResult> }
interface SherpaModule { OfflineRecognizer: { createAsync(config: unknown): Promise<SherpaRecognizer> } }

type Port = { postMessage(message: unknown): void; on(event: 'message', listener: (event: { data: unknown }) => void): void };
const parentPort = (process as unknown as { parentPort?: Port }).parentPort;

function send(message: WorkerResponse): void {
  if (parentPort) parentPort.postMessage(message);
  else process.send?.(message);
}

let sherpa: SherpaModule | null = null;
let loaded: { key: string; recognizer: Promise<SherpaRecognizer> } | null = null;
const cancelled = new Set<string>();
let active: { id: string; ffmpeg: ChildProcessByStdio<null, Readable, Readable> | null } | null = null;

function recognizer(request: Extract<WorkerRequest, { type: 'transcribe' }>): Promise<SherpaRecognizer> {
  const key = `${request.modelDir}\n${request.threads}`;
  if (loaded?.key === key) return loaded.recognizer;
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  sherpa ??= require('sherpa-onnx-node') as SherpaModule;
  const dir = request.modelDir;
  const created = sherpa.OfflineRecognizer.createAsync({
    featConfig: { sampleRate: SAMPLE_RATE, featureDim: 80 },
    modelConfig: {
      transducer: { encoder: `${dir}/encoder.int8.onnx`, decoder: `${dir}/decoder.int8.onnx`, joiner: `${dir}/joiner.int8.onnx` },
      tokens: `${dir}/tokens.txt`,
      numThreads: request.threads,
      provider: 'cpu',
      modelType: 'nemo_transducer',
      debug: 0
    },
    decodingMethod: 'greedy_search'
  });
  loaded = { key, recognizer: created };
  created.catch(() => { if (loaded?.recognizer === created) loaded = null; });
  return created;
}

async function decodeWindow(model: SherpaRecognizer, window: AudioWindow, offset: number): Promise<TranscriptWord[]> {
  const stream = model.createStream();
  stream.acceptWaveform({ samples: window.samples, sampleRate: SAMPLE_RATE });
  const result = await model.decodeAsync(stream);
  const words = tokensToWords({
    tokens: result.tokens ?? [],
    timestamps: result.timestamps ?? [],
    ...(result.durations ? { durations: result.durations } : {}),
    ...(result.ys_log_probs ? { logProbs: result.ys_log_probs } : {})
  }, window.start / SAMPLE_RATE);
  return wordsWithin(words, window.ownFrom, window.ownTo).map((word) => ({ ...word, start: round(word.start + offset), end: round(word.end + offset) }));
}

function round(value: number): number { return Math.round(value * 1000) / 1000; }

class Cancelled extends Error {}

async function transcribe(request: Extract<WorkerRequest, { type: 'transcribe' }>): Promise<void> {
  const { id } = request;
  const offset = request.start && request.start > 0 ? request.start : 0;
  const model = await recognizer(request);
  if (cancelled.has(id)) throw new Cancelled();
  const ffmpeg = spawn(request.ffmpeg, decodeArgs(request.file, { start: request.start, end: request.end }), { stdio: ['ignore', 'pipe', 'pipe'] });
  active = { id, ffmpeg };
  let stderr = '';
  /* The span to decode: the requested end until ffmpeg's banner gives the
     real duration, which wins when the request runs past the media. */
  let total = request.end !== undefined ? request.end - offset : null;
  let measured = false;
  ffmpeg.stderr.on('data', (chunk: Buffer) => {
    stderr = (stderr + chunk.toString()).slice(-16_000);
    if (!measured) {
      const duration = parseDuration(stderr);
      if (duration !== null) {
        measured = true;
        total = spanLength(duration, offset, request.end);
      }
    }
  });
  const exited = new Promise<number | null>((resolve, reject) => {
    ffmpeg.once('error', reject);
    ffmpeg.once('close', (code) => resolve(code));
  });

  const chunker = new Chunker();
  const words: TranscriptWord[] = [];
  let decoded = 0;
  let lastProgress = -1;
  const progress = () => {
    if (!total || total <= 0) return;
    const value = Math.min(0.99, decoded / total);
    if (value - lastProgress < 0.01) return;
    lastProgress = value;
    send({ type: 'progress', id, progress: value });
  };
  /* Decoding is slower than ffmpeg: pause its output while a window decodes so memory stays flat. */
  let carry = Buffer.alloc(0);
  let queue: Promise<void> = Promise.resolve();
  let failure: unknown = null;
  const decode = (windows: AudioWindow[]) => {
    if (!windows.length) return;
    ffmpeg.stdout.pause();
    queue = queue.then(async () => {
      for (const window of windows) {
        if (cancelled.has(id)) throw new Cancelled();
        appendAtSeam(words, await decodeWindow(model, window, offset));
        decoded = window.ownTo === Infinity ? decoded : window.ownTo;
        progress();
      }
    }).catch((error: unknown) => {
      failure ??= error;
      ffmpeg.kill('SIGKILL');
    }).finally(() => {
      if (!failure) ffmpeg.stdout.resume();
    });
  };
  ffmpeg.stdout.on('data', (chunk: Buffer) => {
    const bytes = carry.length ? Buffer.concat([carry, chunk]) : chunk;
    const whole = bytes.length - (bytes.length % 4);
    carry = Buffer.from(bytes.subarray(whole));
    if (!whole) return;
    const copy = new Float32Array(whole / 4);
    new Uint8Array(copy.buffer).set(bytes.subarray(0, whole));
    decode(chunker.push(copy));
  });
  const code = await exited;
  active = null;
  await queue;
  if (cancelled.has(id)) throw new Cancelled();
  if (failure) throw failure;
  if (code !== 0) throw new Error(decodeError(stderr));
  const rest = chunker.finish();
  if (rest && rest.samples.length >= SAMPLE_RATE / 10) appendAtSeam(words, await decodeWindow(model, rest, offset));
  if (cancelled.has(id)) throw new Cancelled();
  const duration = round(chunker.received);
  send({ type: 'done', id, duration, segments: wordsToSegments(words) });
}

function handle(message: unknown): void {
  const request = message as WorkerRequest;
  if (!request || typeof request !== 'object') return;
  if (request.type === 'cancel') {
    cancelled.add(request.id);
    if (active?.id === request.id) active.ffmpeg?.kill('SIGKILL');
    return;
  }
  if (request.type === 'warm') {
    void recognizer({ type: 'transcribe', id: '', file: '', modelDir: request.modelDir, threads: request.threads, ffmpeg: '' }).catch(() => undefined);
    return;
  }
  if (request.type !== 'transcribe') return;
  void transcribe(request).then(undefined, (error: unknown) => {
    if (error instanceof Cancelled || cancelled.has(request.id)) send({ type: 'cancelled', id: request.id });
    else send({ type: 'error', id: request.id, message: error instanceof Error ? error.message : String(error) });
  }).finally(() => {
    cancelled.delete(request.id);
    if (active?.id === request.id) active = null;
  });
}

if (parentPort) parentPort.on('message', (event) => handle(event.data));
else process.on('message', handle);
send({ type: 'ready' });
