import type { Worker, WorkerOptions } from 'node:worker_threads';

/** Native parsing is faster below this measured boundary. */
export const WORKER_DOCUMENT_BYTES = 64 * 1024 * 1024;

/** Keep this function capture-free: its production source also runs in Worker. */
export function assertProjectDocument(document: any): void {
  const project = document?.proj || document;
  if (!project || !Array.isArray(project.layers) || !Number.isFinite(project.w) || !Number.isFinite(project.h)) {
    throw new Error('This is not a Powermove project.');
  }
}

/** Infrastructure faults allow the existing native validation path to retry. */
export class ProjectDocumentWorkerError extends Error {
  constructor(message: string, cause?: unknown) {
    super(message, { cause });
    this.name = 'ProjectDocumentWorkerError';
  }
}

type ValidationReply = { ok: true } | { ok: false; error: { name: string; message: string } };
type ValidationWorker = Pick<Worker, 'once' | 'removeListener' | 'terminate'>;
type SpawnWorker = (source: string, options: WorkerOptions) => ValidationWorker;

function restoreError(error: { name: string; message: string }): Error {
  const result = error.name === 'SyntaxError' ? new SyntaxError(error.message)
    : error.name === 'RangeError' ? new RangeError(error.message)
    : error.name === 'TypeError' ? new TypeError(error.message) : new Error(error.message);
  result.name = error.name;
  return result;
}

/** Validate the entire owned document without cloning its parsed graph to main.
 * The optional factory is a lifecycle test seam; production loads Worker lazily. */
export async function validateProjectDocumentInWorker(bytes: Buffer, sha256: string, spawn?: SpawnWorker): Promise<void> {
  if (!(bytes.buffer instanceof ArrayBuffer) || bytes.byteOffset !== 0 || bytes.byteLength !== bytes.buffer.byteLength) {
    throw new ProjectDocumentWorkerError('Project validation needs an owned document buffer.');
  }
  const source = `
    const { workerData, parentPort } = require('node:worker_threads');
    const { createHash } = require('node:crypto');
    const assertProjectDocument = (${assertProjectDocument.toString()});
    try {
      const bytes = Buffer.from(workerData.buffer);
      if (createHash('sha256').update(bytes).digest('hex') !== workerData.sha256) {
        throw new Error('The project document checksum does not match.');
      }
      const document = JSON.parse(bytes.toString('utf8'));
      assertProjectDocument(document);
      parentPort.postMessage({ ok: true });
    } catch (error) {
      parentPort.postMessage({ ok: false, error: { name: error.name, message: error.message } });
    } finally { parentPort.close(); }
  `;
  let worker: ValidationWorker;
  try {
    const options: WorkerOptions = { eval: true, execArgv: [], name: 'Powermove project validation',
      workerData: { buffer: bytes.buffer, sha256 }, transferList: [bytes.buffer] };
    worker = spawn ? spawn(source, options) : new (await import('node:worker_threads')).Worker(source, options);
  } catch (error) { throw new ProjectDocumentWorkerError('The project validation worker could not start.', error); }

  let reply: ValidationReply | undefined, failure: ProjectDocumentWorkerError | undefined, stopping = false;
  let resolveReply!: (value: ValidationReply) => void, rejectReply!: (error: ProjectDocumentWorkerError) => void;
  let resolveExit!: () => void;
  const result = new Promise<ValidationReply>((resolve, reject) => { resolveReply = resolve; rejectReply = reject; });
  const exited = new Promise<void>(resolve => { resolveExit = resolve; });
  const fail = (message: string, cause?: unknown) => {
    failure ??= new ProjectDocumentWorkerError(message, cause);
    rejectReply(failure);
  };
  const onMessage = (value: unknown) => {
    const response = value as Partial<ValidationReply> | null;
    if (response?.ok === true) reply = { ok: true };
    else if (response?.ok === false && typeof response.error?.name === 'string' && typeof response.error.message === 'string') {
      reply = { ok: false, error: { name: response.error.name, message: response.error.message } };
    } else { fail('The project validation worker returned an invalid response.'); return; }
    resolveReply(reply);
  };
  const onError = (error: Error) => fail('The project validation worker failed.', error);
  const onExit = (code: number) => {
    resolveExit();
    if (!reply || !stopping && code !== 0) fail('The project validation worker exited unexpectedly.');
  };
  worker.once('message', onMessage);
  worker.once('error', onError);
  worker.once('exit', onExit);
  try { await result; } catch { /* failure is captured independently of document errors. */ }
  stopping = true;
  try { await worker.terminate(); }
  catch (error) { failure ??= new ProjectDocumentWorkerError('The project validation worker could not finish.', error); }
  await exited;
  worker.removeListener('message', onMessage);
  worker.removeListener('error', onError);
  worker.removeListener('exit', onExit);
  // Preserve expected parse/checksum/shape errors even if teardown also failed.
  if (reply?.ok === false) throw restoreError(reply.error);
  if (failure) throw failure;
}
