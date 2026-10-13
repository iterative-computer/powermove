/*
 * The persistence worker thread. Main starts it (persistence-thread.ts) so
 * autosave and Save serialization run here instead of on main's JavaScript
 * thread, where they delayed input, IPC, and frame forwarding by hundreds of
 * milliseconds for large projects (docs/engine/spikes/S1.md).
 *
 * After every reply the worker bumps a shared counter and wakes main, which
 * lets main wait synchronously for one answer when a synchronous store read
 * arrives while that key's newest value is still being encoded.
 */
import * as fs from 'node:fs/promises';
import { randomBytes } from 'node:crypto';
import { workerData, type MessagePort } from 'node:worker_threads';

import { validateProjectDocument } from './project-document';
import { StoreEngine } from './store-engine';
import type { PersistenceReply, PersistenceRequest } from './persistence-thread';

const { port, signal } = workerData as { port: MessagePort; signal: SharedArrayBuffer };
const counter = new Int32Array(signal);

function post(message: PersistenceReply, transfer: ArrayBuffer[] = []): void {
  port.postMessage(message, transfer);
  Atomics.add(counter, 0, 1);
  Atomics.notify(counter, 0);
}

const store = new StoreEngine({
  fs,
  now: Date.now,
  randomSuffix: () => `${process.pid}-${randomBytes(8).toString('hex')}`
}, (reply, transfer) => post({ from: 'store', reply }, transfer));

port.on('message', (message: PersistenceRequest) => {
  if (message.to === 'store') store.handle(message.request);
  else if (message.to === 'project') {
    validateProjectDocument(message.file, message.offset, message.length).then(
      () => post({ from: 'project', id: message.id }),
      (error: unknown) => post({ from: 'project', id: message.id, error: error instanceof Error ? error.message : String(error) })
    );
  }
});
