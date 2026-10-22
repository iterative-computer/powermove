/*
 * Project fingerprints: SHA-256 of a project's JSON in UTF-8, stable across
 * reopen and independent of playhead, selection, and panel layout.
 *
 * The JSON is encoded in bounded pieces and hashed on a worker. For the large
 * private project (180 MB of JSON), the open and every autosave fingerprinted
 * it several times: a sliced serialization of about 5 s each, plus a 180 MB
 * TextEncoder call in one task. `fingerprintJSON` serializes in slices
 * (json-slices) and gives the same digest as `projectFingerprint` of
 * `JSON.stringify(value)`.
 */
import { Sha256 } from '../../../../shared/sha256';
import type { FingerprintReply, FingerprintRequest } from './fingerprint-digests';
import { nextTask, serializeInSlices } from './json-slices';

const PIECE = 1 << 20;
const encoder = new TextEncoder();

type Sink = { write(bytes: Uint8Array): void; finish(): Promise<string>; cancel(): void };

let worker: Worker | null = null;
let broken = false;
let jobs = 0;
const waiting = new Map<number, { resolve: (hex: string) => void; reject: (error: Error) => void }>();

function hashWorker(): Worker | null {
  if (broken || typeof Worker !== 'function') return null;
  if (worker) return worker;
  try {
    worker = new Worker(new URL('./fingerprint-worker.ts', import.meta.url), { type: 'module', name: 'project-fingerprint' });
  } catch { broken = true; return null; }
  worker.onmessage = ({ data }: MessageEvent<FingerprintReply>) => {
    const job = waiting.get(data.id);
    waiting.delete(data.id);
    job?.resolve(data.hex);
  };
  worker.onerror = event => {
    event.preventDefault?.();
    broken = true;
    worker?.terminate(); worker = null;
    const error = new Error('The fingerprint worker stopped');
    for (const job of waiting.values()) job.reject(error);
    waiting.clear();
  };
  return worker;
}

function workerSink(): Sink | null {
  const target = hashWorker();
  if (!target) return null;
  const id = ++jobs;
  let failed: Error | null = null;
  const post = (message: FingerprintRequest, transfer: Transferable[] = []) => {
    try { target.postMessage(message, transfer); } catch (error) { failed = error instanceof Error ? error : new Error(String(error)); }
  };
  return {
    write(bytes) { post({ id, bytes: bytes.buffer as ArrayBuffer }, [bytes.buffer as ArrayBuffer]); },
    finish() {
      return new Promise<string>((resolve, reject) => {
        if (failed || broken) { reject(failed ?? new Error('The fingerprint worker stopped')); return; }
        waiting.set(id, { resolve, reject });
        post({ id, end: true });
        if (failed) { waiting.delete(id); reject(failed); }
      });
    },
    cancel() { post({ id, cancel: true }); waiting.delete(id); },
  };
}

function threadSink(): Sink {
  const digest = new Sha256();
  return { write(bytes) { digest.update(bytes); }, finish: async () => digest.digest(), cancel() {} };
}

/** Feeds `produce`'s UTF-8 pieces to a digest; retries on this thread if the worker fails. */
async function fingerprint(produce: (write: (text: string) => void) => Promise<void>): Promise<string> {
  for (const make of [workerSink, threadSink]) {
    const sink = make();
    if (!sink) continue;
    try {
      // A piece is fresh bytes of its own: transferring it detaches nothing shared.
      await produce(text => sink.write(encoder.encode(text)));
      return await sink.finish();
    } catch (error) {
      sink.cancel();
      if (make === threadSink || !broken) throw error;
    }
  }
  throw new Error('No project fingerprint digest is available');
}

/** Pieces of `text` that never split a surrogate pair (TextEncoder would replace each half). */
async function piecesOf(text: string, write: (piece: string) => void): Promise<void> {
  for (let offset = 0; offset < text.length;) {
    let end = Math.min(text.length, offset + PIECE);
    const last = text.charCodeAt(end - 1);
    if (end < text.length && last >= 0xd800 && last <= 0xdbff) end++;
    write(text.slice(offset, end));
    offset = end;
    if (offset < text.length) await nextTask();
  }
}

/** The fingerprint of JSON text. */
export async function projectFingerprint(projectJSON: string): Promise<string> {
  return fingerprint(write => piecesOf(projectJSON, write));
}

/** `projectFingerprint(JSON.stringify(value))`, without building the string or a long task. */
export async function fingerprintJSON(value: unknown): Promise<string> {
  return fingerprint(write => serializeInSlices(value, write));
}
