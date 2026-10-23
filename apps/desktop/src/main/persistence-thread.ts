/*
 * Main's handle on the persistence worker thread (persistence-worker.ts). The
 * store's encoding and writes (store-engine.ts) and the Save document check
 * (project-document.ts) run there, off main's JavaScript thread.
 *
 * If the worker exits unexpectedly, a replacement starts and the store
 * re-sends every value main published that had not reached disk yet, so a
 * worker fault loses no committed edit. Requests in flight at that moment
 * fail and are reported to their callers.
 */
import { MessageChannel, receiveMessageOnPort, Worker, type MessagePort } from 'node:worker_threads';

import type { ProjectDocumentValidator } from './project-document';
import type { StoreReply, StoreRequest } from './store-engine';
import type { StoreChannel } from './storage';

export type PersistenceRequest =
  | { to: 'store'; request: StoreRequest }
  | { to: 'project'; id: number; file: string; offset: number; length: number };

export type PersistenceReply =
  | { from: 'store'; reply: StoreReply }
  | { from: 'project'; id: number; error?: string };

/** Store requests answered by exactly one reply; `restore` is answered only by `written`. */
const ANSWERED = new Set<StoreRequest['op']>(['init', 'load', 'set', 'setSerialized', 'delete', 'append', 'flush']);
const FINAL = new Set<StoreReply['type']>(['ready', 'loaded', 'committed', 'rejected', 'flushed', 'failed']);
const MAX_RESTARTS = 5;

export interface PersistenceThreadOptions {
  /** Called when the worker stops unexpectedly (before any replacement starts). */
  onFault?: (detail: string) => void;
}

export class PersistenceThread {
  private worker!: Worker;
  private port!: MessagePort;
  private counter!: Int32Array;
  private storeListener?: (reply: StoreReply) => void;
  private restartListener?: () => void;
  private readonly validations = new Map<number, { resolve: () => void; reject: (error: Error) => void }>();
  private nextValidation = 1;
  private outstanding = 0;
  private restarts = 0;
  private closing = false;
  private failed?: string;

  constructor(private readonly entry: string, private readonly options: PersistenceThreadOptions = {}) {
    this.spawn();
  }

  /** The store's transport; one store per thread. */
  storeChannel(): StoreChannel {
    return {
      post: (request, transfer) => this.send({ to: 'store', request }, transfer, ANSWERED.has(request.op)),
      onReply: (listener) => { this.storeListener = listener; },
      onRestart: (listener) => { this.restartListener = listener; },
      waitSync: (done, timeoutMs) => this.waitSync(done, timeoutMs)
    };
  }

  readonly validateProjectDocument: ProjectDocumentValidator = (file, offset, length) => new Promise<void>((resolve, reject) => {
    const id = this.nextValidation++;
    this.validations.set(id, { resolve, reject });
    try { this.send({ to: 'project', id, file, offset, length }, [], true); }
    catch (error) { this.validations.delete(id); reject(error); }
  });

  async close(): Promise<void> {
    this.closing = true;
    await this.worker.terminate();
  }

  /** Test seam: the worker's thread id, to tell a replacement apart. */
  get threadId(): number { return this.worker.threadId; }

  private spawn(): void {
    const signal = new SharedArrayBuffer(4);
    const { port1, port2 } = new MessageChannel();
    const worker = new Worker(this.entry, { workerData: { port: port2, signal }, transferList: [port2], name: 'Powermove persistence' });
    this.worker = worker;
    this.port = port1;
    this.counter = new Int32Array(signal);
    this.outstanding = 0;
    port1.on('message', (message: PersistenceReply) => this.dispatch(message));
    port1.unref();
    worker.unref();
    worker.on('error', (error) => { console.error('The persistence worker failed.', error); });
    worker.on('exit', (code) => { this.exited(worker, port1, code); });
  }

  private send(message: PersistenceRequest, transfer: ArrayBuffer[] = [], answered: boolean): void {
    if (this.failed) throw new Error(this.failed);
    this.port.postMessage(message, transfer);
    if (answered && this.outstanding++ === 0) this.port.ref();
  }

  private settled(): void {
    if (this.outstanding > 0 && --this.outstanding === 0) this.port.unref();
  }

  private dispatch(message: PersistenceReply): void {
    if (message.from === 'store') {
      if (FINAL.has(message.reply.type)) this.settled();
      this.storeListener?.(message.reply);
      return;
    }
    this.settled();
    const validation = this.validations.get(message.id);
    if (!validation) return;
    this.validations.delete(message.id);
    if (message.error === undefined) validation.resolve();
    else validation.reject(new Error(message.error));
  }

  /** Delivers replies on this thread until `done()`; false once `timeoutMs` passes first. */
  private waitSync(done: () => boolean, timeoutMs: number): boolean {
    const deadline = performance.now() + timeoutMs;
    while (!done()) {
      const seen = Atomics.load(this.counter, 0);
      const received = receiveMessageOnPort(this.port);
      if (received) { this.dispatch(received.message as PersistenceReply); continue; }
      const remaining = deadline - performance.now();
      if (remaining <= 0) return false;
      Atomics.wait(this.counter, 0, seen, remaining);
    }
    return true;
  }

  private exited(worker: Worker, port: MessagePort, code: number): void {
    if (worker !== this.worker) return;
    // Replies already queued still count: deliver them before giving up on the rest.
    for (let received = receiveMessageOnPort(port); received; received = receiveMessageOnPort(port)) this.dispatch(received.message as PersistenceReply);
    port.close();
    if (this.closing) return;
    const detail = `The persistence worker stopped (exit code ${code}).`;
    this.options.onFault?.(detail);
    for (const validation of this.validations.values()) validation.reject(new Error(`${detail} The project was not saved.`));
    this.validations.clear();
    if (++this.restarts > MAX_RESTARTS) {
      this.failed = `${detail} Saving is unavailable until Powermove restarts.`;
      this.restartListener?.();
      return;
    }
    this.spawn();
    this.restartListener?.();
  }
}
