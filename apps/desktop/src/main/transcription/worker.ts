/*
 * The transcription process. Main starts it with utilityProcess.fork (the
 * serve host uses child_process.fork) so model loading and inference never
 * run on the main or UI threads, and a native fault takes down only this
 * process. One job at a time; main queues the rest.
 *
 * The recognizer is transcribe.cpp (ggml; Metal on Apple Silicon) through
 * its Node binding. The binding's koffi addon and the libtranscribe/ggml
 * dylibs ship unpacked inside the app and are signed with it.
 */
import { spawn, type ChildProcessByStdio } from 'node:child_process';
import type { Readable } from 'node:stream';
import type * as Transcribe from 'transcribe-cpp';

import { decodeArgs, decodeError, paddedSpan, parseDuration, spanLength } from './audio';
import { Chunker, LEAD_IN_SECONDS, SAMPLE_RATE, appendAtSeam, chunkingFor, speechBounds, withLeadIn, wordsWithin, type AudioWindow } from './chunking';
import { windowWords, type EngineResult } from './decode';
import { wordsToSegments } from './words';
import type { WorkerModel, WorkerRequest, WorkerResponse } from './worker-protocol';
import type { TranscriptWord } from '../../shared/transcription';

type Port = { postMessage(message: unknown): void; on(event: 'message', listener: (event: { data: unknown }) => void): void };
const parentPort = (process as unknown as { parentPort?: Port }).parentPort;

function send(message: WorkerResponse): void {
  if (parentPort) parentPort.postMessage(message);
  else process.send?.(message);
}

interface Loaded {
  key: string;
  model: Transcribe.TranscribeModel;
  session: Transcribe.Session;
  /** The model's own language tags (Nemotron says de-DE where we say de). */
  languages: string[];
}

let runtime: Promise<typeof Transcribe> | null = null;
let loaded: { key: string; promise: Promise<Loaded> } | null = null;
const cancelled = new Set<string>();
let active: { id: string; ffmpeg: ChildProcessByStdio<null, Readable, Readable> | null; abort: AbortController } | null = null;

function binding(library: string | undefined): Promise<typeof Transcribe> {
  /* dlopen cannot map a library inside the asar: point the binding at the unpacked copy. */
  if (library && !process.env['TRANSCRIBE_LIBRARY']) process.env['TRANSCRIBE_LIBRARY'] = library;
  runtime ??= import('transcribe-cpp');
  return runtime;
}

function load(model: WorkerModel, threads: number, library: string | undefined): Promise<Loaded> {
  const key = `${model.file}\n${threads}`;
  if (loaded?.key === key) return loaded.promise;
  const previous = loaded;
  const promise = (async () => {
    const old = await previous?.promise.catch(() => null);
    old?.model.dispose();
    const tc = await binding(library);
    const instance = await tc.TranscribeModel.load(model.file, { backend: 'auto' });
    try {
      const session = instance.createSession({ nThreads: threads });
      send({ type: 'loaded', file: model.file });
      return { key, model: instance, session, languages: instance.capabilities.languages ?? [] };
    } catch (error) {
      instance.dispose();
      throw error;
    }
  })();
  loaded = { key, promise };
  promise.catch(() => { if (loaded?.promise === promise) loaded = null; });
  return promise;
}

/** The model's own tag for a language: exact, else its first regional form. */
function modelLanguage(languages: string[], language: string | undefined): string | undefined {
  if (!language) return undefined;
  const lower = language.toLowerCase();
  return languages.find((tag) => tag.toLowerCase() === lower)
    ?? languages.find((tag) => tag.toLowerCase().startsWith(`${lower}-`));
}

class Cancelled extends Error {}

async function run(engine: Loaded, samples: Float32Array, language: string | undefined, signal: AbortSignal): Promise<EngineResult & { language: string }> {
  try {
    return await engine.session.run(samples, { timestamps: 'auto', signal, ...(language ? { language } : {}) });
  } catch (error) {
    if (signal.aborted) throw new Cancelled();
    /* A decoder that began repeating or ran long stops with what it heard so far. */
    const partial = (error as { partialResult?: EngineResult & { language: string } })?.partialResult;
    if (partial && /repeat|truncat/i.test((error as Error)?.message ?? '')) return partial;
    throw error;
  }
}

async function decodeWindow(engine: Loaded, request: TranscribeJob, window: AudioWindow, offset: number, signal: AbortSignal): Promise<{ words: TranscriptWord[]; language: string }> {
  const sound = speechBounds(window.samples);
  if (!sound) return { words: [], language: '' };
  const result = await run(engine, withLeadIn(window.samples), modelLanguage(engine.languages, request.language), signal);
  const start = window.start / SAMPLE_RATE;
  const words = windowWords(result, request.model.timing, {
    offset: start - LEAD_IN_SECONDS,
    soundFrom: start + sound.from / SAMPLE_RATE,
    soundTo: start + sound.to / SAMPLE_RATE
  }, request.model.lag);
  /* Word-timed windows overlap: keep the words this window owns. The first
     window owns everything before it too (a word timed a little early into
     the lead-in is still the first word). Other windows meet at the cut. */
  const owned = request.model.timing === 'word'
    ? wordsWithin(words, window.ownFrom > 0 ? window.ownFrom : -Infinity, window.ownTo)
    : words;
  return {
    words: owned.map((word) => ({ ...word, start: round(Math.max(0, word.start + offset)), end: round(Math.max(0, word.end + offset)) })),
    language: result.language ?? ''
  };
}

function round(value: number): number { return Math.round(value * 1000) / 1000; }

type TranscribeJob = Extract<WorkerRequest, { type: 'transcribe' }>;

async function transcribe(request: TranscribeJob): Promise<void> {
  const { id } = request;
  const abort = new AbortController();
  active = { id, ffmpeg: null, abort };
  /* Decoded with a little audio either side; the span's own words are kept at the end. */
  const span = paddedSpan({ start: request.start, end: request.end });
  const offset = span.start;
  const engine = await load(request.model, request.threads, request.runtime);
  if (cancelled.has(id)) throw new Cancelled();
  const ffmpeg = spawn(request.ffmpeg, decodeArgs(request.file, { start: span.start, end: span.end }), { stdio: ['ignore', 'pipe', 'pipe'] });
  active.ffmpeg = ffmpeg;
  let stderr = '';
  /* The span to decode: the requested end until ffmpeg's banner gives the
     real duration, which wins when the request runs past the media. */
  let total = span.end !== undefined ? span.end - offset : null;
  let measured = false;
  ffmpeg.stderr.on('data', (chunk: Buffer) => {
    stderr = (stderr + chunk.toString()).slice(-16_000);
    if (!measured) {
      const duration = parseDuration(stderr);
      if (duration !== null) {
        measured = true;
        total = spanLength(duration, offset, span.end);
      }
    }
  });
  const exited = new Promise<number | null>((resolve, reject) => {
    ffmpeg.once('error', reject);
    ffmpeg.once('close', (code) => resolve(code));
  });

  const seams = request.model.timing === 'word';
  const chunker = new Chunker(chunkingFor(request.model.timing));
  const words: TranscriptWord[] = [];
  let language = '';
  const keep = (decoded: { words: TranscriptWord[]; language: string }) => {
    if (seams) appendAtSeam(words, decoded.words);
    else words.push(...decoded.words);
    if (!language && decoded.words.length) language = decoded.language;
  };
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
        keep(await decodeWindow(engine, request, window, offset, abort.signal));
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
  active.ffmpeg = null;
  await queue;
  if (cancelled.has(id)) throw new Cancelled();
  if (failure) throw failure;
  if (code !== 0) throw new Error(decodeError(stderr));
  const rest = chunker.finish();
  if (rest && rest.samples.length >= SAMPLE_RATE / 10) keep(await decodeWindow(engine, request, rest, offset, abort.signal));
  if (cancelled.has(id)) throw new Cancelled();
  const duration = round(Math.max(0, Math.min(span.keepTo, offset + chunker.received) - span.keepFrom));
  const kept = span.keepFrom > 0 || span.keepTo !== Infinity ? wordsWithin(words, span.keepFrom > 0 ? span.keepFrom : -Infinity, span.keepTo) : words;
  send({ type: 'done', id, duration, segments: wordsToSegments(kept), ...(language ? { language } : {}) });
}

function handle(message: unknown): void {
  const request = message as WorkerRequest;
  if (!request || typeof request !== 'object') return;
  if (request.type === 'cancel') {
    cancelled.add(request.id);
    if (active?.id === request.id) {
      active.abort.abort();
      active.ffmpeg?.kill('SIGKILL');
    }
    return;
  }
  if (request.type === 'warm') {
    void load(request.model, request.threads, request.runtime).catch(() => undefined);
    return;
  }
  if (request.type !== 'transcribe') return;
  void transcribe(request).then(undefined, (error: unknown) => {
    if (error instanceof Cancelled || cancelled.has(request.id)) send({ type: 'cancelled', id: request.id });
    else send({ type: 'error', id: request.id, message: describe(error) });
  }).finally(() => {
    cancelled.delete(request.id);
    if (active?.id === request.id) active = null;
  });
}

function describe(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  const name = error instanceof Error ? error.name : '';
  if (/ModelLoad|gguf|load/i.test(name) || /failed to load|invalid gguf|model file/i.test(message)) return 'The speech model could not be loaded. Delete it in Settings › Transcription and download it again.';
  return message;
}

if (parentPort) parentPort.on('message', (event) => handle(event.data));
else process.on('message', handle);
send({ type: 'ready' });
