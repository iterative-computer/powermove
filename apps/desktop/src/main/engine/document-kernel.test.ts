import { EventEmitter } from 'node:events';
import { mkdtemp, open, readFile, readdir, rm } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import os from 'node:os';
import path from 'node:path';
import { expect, it, vi } from 'vitest';
import type { IpcMain } from 'electron';
import { DOCUMENT_KERNEL_IPC as IPC } from '../../shared/document-kernel';
import { canonicalStringify } from '../../shared/canonical-json';
import { installDocumentKernelIpc } from './document-kernel';

class Ipc extends EventEmitter {
  handlers = new Map<string, (...args: any[]) => any>();
  handle(name: string, fn: (...args: any[]) => any) { this.handlers.set(name, fn); }
  removeHandler(name: string) { this.handlers.delete(name); }
  invoke(name: string, event: any, ...args: any[]) { return this.handlers.get(name)!(event, ...args); }
}
function owner(id: number) {
  let destroyed = false;
  const sender = Object.assign(new EventEmitter(), { id, isDestroyed: () => destroyed });
  return {
    sender,
    navigate(isMainFrame = true, isSameDocument = false) { sender.emit('did-start-navigation', { isMainFrame, isSameDocument }); },
    crash() { sender.emit('render-process-gone'); },
    destroy() { destroyed = true; sender.emit('destroyed'); },
  };
}

it('streams exactly the supplied bytes, independently hashes them and removes staging on success and failure', async () => {
  const cache = await mkdtemp(path.join(os.tmpdir(), 'pm-kernel-test-'));
  const ipc = new Ipc(), event = owner(7);
  let receipt: unknown, fail = false;
  const call = vi.fn(async (id: number, request: string) => {
    expect(id).toBe(7);
    const { op, args } = JSON.parse(request);
    if (op === 'load') {
      const body = await readFile(args.path);
      expect(body.toString()).toBe('{"a":"🦀","z":2}');
      expect(args.hash).toBe(createHash('sha256').update(body).digest('hex'));
      // The utility checks the bytes and answers the tree's canonical hash.
      expect(args.source).toBe('text');
      if (fail) throw new Error('reader failed');
      receipt = { pending: false, revision: args.revision, hash: 'c'.repeat(64), integrity: 'checked' };
      return JSON.stringify({ ok: true, value: { pending: true } });
    }
    return JSON.stringify({ ok: true, value: op === 'poll' ? receipt : null });
  });
  const stop = installDocumentKernelIpc({ ipc: ipc as unknown as IpcMain, cache, trusted: () => true, call });
  try {
    for (const failure of [false, true]) {
      fail = failure;
      const token = await ipc.invoke(IPC.begin, event);
      const bytes = new TextEncoder().encode('{"a":"🦀","z":2}');
      await ipc.invoke(IPC.write, event, token, bytes.subarray(0, 9));
      await ipc.invoke(IPC.write, event, token, bytes.subarray(9));
      const committed = ipc.invoke(IPC.commit, event, token, 'document', 4);
      if (fail) await expect(committed).rejects.toThrow('reader failed');
      else expect(await committed).toMatchObject({ pending: false, revision: 4 });
      expect(await readdir(cache)).toEqual([]);
    }
  } finally { await stop(); await rm(cache, { recursive: true, force: true }); }
});

it('fences owners, bounds chunks, refuses page-supplied file loads, and releases aborted streams', async () => {
  const cache = await mkdtemp(path.join(os.tmpdir(), 'pm-kernel-test-'));
  const ipc = new Ipc(), one = owner(1), two = owner(2), call = vi.fn(async () => '{"ok":true,"value":null}');
  const stop = installDocumentKernelIpc({ ipc: ipc as unknown as IpcMain, cache, trusted: () => true, call });
  try {
    const pending = ipc.invoke(IPC.begin, one);
    await expect(ipc.invoke(IPC.begin, one)).rejects.toThrow('already open');
    const token = await pending;
    await expect(ipc.invoke(IPC.write, two, token, new Uint8Array([1]))).rejects.toThrow('closed');
    await expect(ipc.invoke(IPC.write, one, token, new Uint8Array(1024 * 1024 + 1))).rejects.toThrow('Invalid document chunk');
    const event = { ...one, returnValue: undefined as unknown };
    ipc.emit(IPC.call, event, '{"op":"load","args":{"path":"/arbitrary"}}');
    await vi.waitFor(() => expect(event.returnValue).toBeDefined());
    expect(JSON.parse(event.returnValue as string).ok).toBe(false);
    expect(call).not.toHaveBeenCalled();
    await ipc.invoke(IPC.abort, one, token);
    expect(await readdir(cache)).toEqual([]);
  } finally { await stop(); await rm(cache, { recursive: true, force: true }); }
});

it('computes streaming consistency hashes outside the renderer without creating a file', async () => {
  const cache = await mkdtemp(path.join(os.tmpdir(), 'pm-kernel-test-'));
  const ipc = new Ipc(), event = owner(7);
  const stop = installDocumentKernelIpc({ ipc: ipc as unknown as IpcMain, cache, trusted: () => true, call: async () => '' });
  try {
    const token = await ipc.invoke(IPC.digestBegin, event, 'v');
    await ipc.invoke(IPC.digestWrite, event, token, new TextEncoder().encode('{"x":1}'));
    expect(await ipc.invoke(IPC.digestEnd, event, token)).toBe(createHash('sha256').update('v{"x":1}').digest('hex'));
    expect(await readdir(cache)).toEqual([]);
    await expect(Promise.resolve().then(() => ipc.invoke(IPC.digestEnd, event, token))).rejects.toThrow('closed');
  } finally { await stop(); await rm(cache, { recursive: true, force: true }); }
});


it('hashes bounded postorder batches independently and rejects oversized or forward work', async () => {
  const cache = await mkdtemp(path.join(os.tmpdir(), 'pm-kernel-test-'));
  const ipc = new Ipc(), event = owner(7);
  const stop = installDocumentKernelIpc({ ipc: ipc as unknown as IpcMain, cache, trusted: () => true, call: async () => '' });
  const hash = (value: any, prefix = '') => createHash('sha256').update(prefix).update(canonicalStringify(value)!).digest('hex');
  try {
    const scalar = hash({ z: '🦀', a: 1 }, 'v');
    const array = hash(['a', [scalar, scalar]]);
    expect(await ipc.invoke(IPC.digestBatch, event, [
      { value: { z: '🦀', a: 1 }, prefix: 'v' }, { array: [0, scalar] }, { object: [['child', 1]] }
    ])).toEqual([scalar, array, hash(['o', [['child', array]]])]);
    // These small leaves fit the renderer's byte budget even though they
    // contain more than 32k empty-container nodes in total.
    const dense = Array.from({ length: 8 }, () => ({ value: Array.from({ length: 10_000 }, () => ({})), prefix: 'v' }));
    expect(Buffer.byteLength(JSON.stringify(dense))).toBeLessThan(256 * 1024);
    expect(await ipc.invoke(IPC.digestBatch, event, dense)).toEqual(dense.map(item => hash(item.value, 'v')));
    expect(() => ipc.invoke(IPC.digestBatch, event, [{ array: [0] }])).toThrow(/earlier instruction/);
    expect(() => ipc.invoke(IPC.digestBatch, event, [{ value: '\0'.repeat(50_000), prefix: 'v' }])).toThrow(/too large/);
    expect(() => ipc.invoke(IPC.digestBatch, event, [{ value: new Array(1_000_000_000) }])).toThrow(/too large/);
    // A tiny request can otherwise expand into more than 256 KiB of hashes.
    expect(() => ipc.invoke(IPC.digestBatch, event, [{ value: 1 }, { array: Array(4096).fill(0) }])).toThrow(/too large/);
    expect(() => ipc.invoke(IPC.digestBatch, event, Array.from({ length: 129 }, () => ({ value: 1 })))).toThrow(/Invalid digest batch/);
    event.destroy();
    expect(() => ipc.invoke(IPC.digestBatch, event, [{ value: 1 }])).toThrow(/Unauthorized/);
  } finally { await stop(); await rm(cache, { recursive: true, force: true }); }
});

/** Pause a real FileHandle write; no timing-dependent large-file I/O needed. */
async function holdFirstWrite(cache: string) {
  const probePath = path.join(cache, 'probe');
  const probe = await open(probePath, 'w');
  const prototype = Object.getPrototypeOf(probe);
  const original = prototype.write;
  await probe.close(); await rm(probePath);
  let release!: () => void, started!: () => void, first = true;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const pending = new Promise<void>(resolve => { started = resolve; });
  const spy = vi.spyOn(prototype, 'write').mockImplementation(async function(this: any, ...args: any[]) {
    if (first) { first = false; started(); await gate; }
    return original.apply(this, args);
  });
  return { pending, release, restore: () => spy.mockRestore() };
}

it('bounds pending staging writes and releases capacity after awaited streaming', async () => {
  const cache = await mkdtemp(path.join(os.tmpdir(), 'pm-kernel-test-'));
  const ipc = new Ipc(), event = owner(7);
  const held = await holdFirstWrite(cache);
  const stop = installDocumentKernelIpc({ ipc: ipc as unknown as IpcMain, cache, trusted: () => true, call: async () => '{"ok":true,"value":null}' });
  try {
    const token = await ipc.invoke(IPC.begin, event);
    const first = ipc.invoke(IPC.write, event, token, new Uint8Array(1024 * 1024));
    await held.pending;
    const second = ipc.invoke(IPC.write, event, token, new Uint8Array(1024 * 1024));
    await expect(ipc.invoke(IPC.write, event, token, new Uint8Array([1]))).rejects.toThrow(/backpressure/);
    held.release();
    await Promise.all([first, second]);
    await ipc.invoke(IPC.write, event, token, new Uint8Array([1]));
    await ipc.invoke(IPC.abort, event, token);
    expect(await readdir(cache)).toEqual([]);
  } finally { held.release(); held.restore(); await stop(); await rm(cache, { recursive: true, force: true }); }
});

it('cancels queued writes on owner teardown and removes files after pending I/O finishes', async () => {
  const cache = await mkdtemp(path.join(os.tmpdir(), 'pm-kernel-test-'));
  const ipc = new Ipc(), event = owner(7), call = vi.fn(async () => '{"ok":true,"value":null}');
  const held = await holdFirstWrite(cache);
  const stop = installDocumentKernelIpc({ ipc: ipc as unknown as IpcMain, cache, trusted: () => true, call });
  try {
    const token = await ipc.invoke(IPC.begin, event);
    const first = ipc.invoke(IPC.write, event, token, new Uint8Array([1]));
    await held.pending;
    const second = ipc.invoke(IPC.write, event, token, new Uint8Array([2]));
    const results = Promise.allSettled([first, second]);
    event.destroy();
    held.release();
    expect((await results).map(result => result.status)).toEqual(['rejected', 'rejected']);
    await vi.waitFor(async () => expect(await readdir(cache)).toEqual([]));
    expect(call).toHaveBeenCalledWith(7, '{"op":"close"}');
  } finally { held.release(); held.restore(); await stop(); await rm(cache, { recursive: true, force: true }); }
});

it('holds the stream reservation until aborted pending writes are cleaned up', async () => {
  const cache = await mkdtemp(path.join(os.tmpdir(), 'pm-kernel-test-'));
  const ipc = new Ipc(), event = owner(7), held = await holdFirstWrite(cache);
  const stop = installDocumentKernelIpc({ ipc: ipc as unknown as IpcMain, cache, trusted: () => true, call: async () => '' });
  try {
    const token = await ipc.invoke(IPC.begin, event);
    const writing = ipc.invoke(IPC.write, event, token, new Uint8Array([1]));
    const finished = Promise.allSettled([writing]);
    await held.pending;
    const aborting = ipc.invoke(IPC.abort, event, token);
    await expect(ipc.invoke(IPC.begin, event)).rejects.toThrow(/already open/);
    held.release();
    expect((await finished)[0].status).toBe('rejected');
    await aborting;
    const next = await ipc.invoke(IPC.begin, event);
    await ipc.invoke(IPC.abort, event, next);
    expect(await readdir(cache)).toEqual([]);
  } finally { held.release(); held.restore(); await stop(); await rm(cache, { recursive: true, force: true }); }
});

it('rejects an ingestion response completed after its owner was destroyed', async () => {
  const cache = await mkdtemp(path.join(os.tmpdir(), 'pm-kernel-test-'));
  const ipc = new Ipc(), event = owner(7);
  let release!: () => void, started!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const pending = new Promise<void>(resolve => { started = resolve; });
  const stop = installDocumentKernelIpc({ ipc: ipc as unknown as IpcMain, cache, trusted: () => true, call: async (_owner, request) => {
    const { op, args } = JSON.parse(request);
    if (op === 'load') { started(); await gate; return JSON.stringify({ ok: true, value: { pending: false, revision: args.revision, hash: args.hash, integrity: 'checked' } }); }
    return '{"ok":true,"value":null}';
  } });
  try {
    const token = await ipc.invoke(IPC.begin, event);
    await ipc.invoke(IPC.write, event, token, new TextEncoder().encode('{}'));
    const committed = ipc.invoke(IPC.commit, event, token, 'document', 4);
    const result = Promise.allSettled([committed]);
    await pending; event.destroy(); release();
    expect((await result)[0]).toMatchObject({ status: 'rejected', reason: expect.objectContaining({ message: 'The document owner closed' }) });
    await vi.waitFor(async () => expect(await readdir(cache)).toEqual([]));
  } finally { release(); await stop(); await rm(cache, { recursive: true, force: true }); }
});


it('waits for already-started stream creation to remove its private cache on disposal', async () => {
  const cache = await mkdtemp(path.join(os.tmpdir(), 'pm-kernel-test-'));
  const ipc = new Ipc(), event = owner(7);
  const stop = installDocumentKernelIpc({ ipc: ipc as unknown as IpcMain, cache, trusted: () => true, call: async () => '' });
  try {
    const beginning = ipc.invoke(IPC.begin, event);
    const result = Promise.allSettled([beginning]);
    await stop();
    expect((await result)[0]).toMatchObject({ status: 'rejected', reason: expect.objectContaining({ message: 'The document owner closed' }) });
    expect(await readdir(cache)).toEqual([]);
  } finally { await stop(); await rm(cache, { recursive: true, force: true }); }
});

it('cleans only a reloaded owner on main-frame navigation and admits its new page after close', async () => {
  const cache = await mkdtemp(path.join(os.tmpdir(), 'pm-kernel-test-'));
  const ipc = new Ipc(), one = owner(7), two = owner(8);
  let releaseClose!: () => void;
  const closing = new Promise<void>(resolve => { releaseClose = resolve; });
  const call = vi.fn(async (ownerId: number, request: string) => {
    if (JSON.parse(request).op === 'close' && ownerId === 7) await closing;
    return '{"ok":true,"value":null}';
  });
  const stop = installDocumentKernelIpc({ ipc: ipc as unknown as IpcMain, cache, trusted: () => true, call });
  try {
    const old = await ipc.invoke(IPC.begin, one);
    const other = await ipc.invoke(IPC.begin, two);
    const digest = ipc.invoke(IPC.digestBegin, one, 'v');
    one.navigate(false, false); one.navigate(true, true);
    await ipc.invoke(IPC.write, one, old, new Uint8Array([1]));
    one.navigate();
    const next = ipc.invoke(IPC.begin, one);
    let admitted = false;
    void next.then(() => { admitted = true; });
    await vi.waitFor(() => expect(call).toHaveBeenCalledWith(7, '{"op":"close"}'));
    expect(admitted).toBe(false);
    await expect(ipc.invoke(IPC.write, one, old, new Uint8Array([2]))).rejects.toThrow('closed');
    await expect(Promise.resolve().then(() => ipc.invoke(IPC.digestEnd, one, digest))).rejects.toThrow('closed');
    await ipc.invoke(IPC.write, two, other, new Uint8Array([3]));
    releaseClose();
    const fresh = await next;
    expect(fresh).not.toBe(old);
    await ipc.invoke(IPC.abort, one, fresh);
    await ipc.invoke(IPC.abort, two, other);
    expect(await readdir(cache)).toEqual([]);
  } finally { releaseClose(); await stop(); await rm(cache, { recursive: true, force: true }); }
});

it('fences a pre-reload ingestion reply and its cancellation before a fresh owner load', async () => {
  const cache = await mkdtemp(path.join(os.tmpdir(), 'pm-kernel-test-'));
  const ipc = new Ipc(), event = owner(7);
  let releaseLoad!: () => void, started!: () => void;
  const hold = new Promise<void>(resolve => { releaseLoad = resolve; });
  const loading = new Promise<void>(resolve => { started = resolve; });
  const ops: string[] = [];
  let loads = 0;
  const call = vi.fn(async (_owner: number, request: string) => {
    const { op, args } = JSON.parse(request);
    ops.push(op);
    if (op === 'load') {
      if (++loads === 1) { started(); await hold; }
      return JSON.stringify({ ok: true, value: { pending: false, revision: args.revision, hash: args.hash, integrity: 'checked' } });
    }
    return '{"ok":true,"value":null}';
  });
  const stop = installDocumentKernelIpc({ ipc: ipc as unknown as IpcMain, cache, trusted: () => true, call });
  try {
    const old = await ipc.invoke(IPC.begin, event);
    await ipc.invoke(IPC.write, event, old, new TextEncoder().encode('{"old":1}'));
    const committed = ipc.invoke(IPC.commit, event, old, 'document', 1);
    const outcome = Promise.allSettled([committed]);
    await loading;
    event.crash();
    const fresh = ipc.invoke(IPC.begin, event);
    releaseLoad();
    expect((await outcome)[0]).toMatchObject({ status: 'rejected', reason: expect.objectContaining({ message: 'The document owner closed' }) });
    const token = await fresh;
    await ipc.invoke(IPC.write, event, token, new TextEncoder().encode('{"new":2}'));
    expect(await ipc.invoke(IPC.commit, event, token, 'document', 2)).toMatchObject({ revision: 2 });
    expect(ops).toEqual(['load', 'close', 'load']);
    expect(await readdir(cache)).toEqual([]);
  } finally { releaseLoad(); await stop(); await rm(cache, { recursive: true, force: true }); }
});

it('waits for a destroyed owner’s held native close before shutdown finishes', async () => {
  const cache = await mkdtemp(path.join(os.tmpdir(), 'pm-kernel-test-'));
  const ipc = new Ipc(), event = owner(7);
  let releaseClose!: () => void;
  const closing = new Promise<void>(resolve => { releaseClose = resolve; });
  const call = vi.fn(async (_owner: number, request: string) => {
    if (JSON.parse(request).op === 'close') await closing;
    return '{"ok":true,"value":null}';
  });
  const stop = installDocumentKernelIpc({ ipc: ipc as unknown as IpcMain, cache, trusted: () => true, call });
  let stopping: Promise<void> | undefined;
  try {
    const token = await ipc.invoke(IPC.begin, event);
    await ipc.invoke(IPC.abort, event, token);
    event.destroy();
    await vi.waitFor(() => expect(call).toHaveBeenCalledWith(7, '{"op":"close"}'));
    let finished = false;
    stopping = stop().then(() => { finished = true; });
    await Promise.resolve();
    expect(finished).toBe(false);
    releaseClose();
    await stopping;
    expect(finished).toBe(true);
  } finally {
    releaseClose();
    await (stopping ?? stop());
    await rm(cache, { recursive: true, force: true });
  }
});

it('retries only typed busy loads with the same canonical file and preserves its exact bytes', async () => {
  const cache = await mkdtemp(path.join(os.tmpdir(), 'pm-kernel-test-'));
  const ipc = new Ipc(), event = owner(7);
  const canonical = '{"a":"🦀","z":2}';
  const seen: string[] = [];
  let attempts = 0;
  const call = vi.fn(async (_owner: number, request: string) => {
    const { op, args } = JSON.parse(request);
    if (op === 'load') {
      seen.push(args.path);
      expect(await readFile(args.path, 'utf8')).toBe(canonical);
      expect(args.hash).toBe(createHash('sha256').update(canonical).digest('hex'));
      if (++attempts < 3) return '{"ok":false,"code":"busy","error":"reader busy"}';
      return JSON.stringify({ ok: true, value: { pending: false, revision: args.revision, hash: args.hash, integrity: 'checked' } });
    }
    return '{"ok":true,"value":null}';
  });
  const stop = installDocumentKernelIpc({ ipc: ipc as unknown as IpcMain, cache, trusted: () => true, call });
  try {
    const token = await ipc.invoke(IPC.begin, event);
    await ipc.invoke(IPC.write, event, token, new TextEncoder().encode(canonical));
    expect(await ipc.invoke(IPC.commit, event, token, 'document', 4)).toMatchObject({ revision: 4 });
    expect(seen).toHaveLength(3);
    expect(new Set(seen).size).toBe(1);
    expect(await readdir(cache)).toEqual([]);
  } finally { await stop(); await rm(cache, { recursive: true, force: true }); }
});

it('passes an unchanged binary payload only to apply and addFont', async () => {
  const cache = await mkdtemp(path.join(os.tmpdir(), 'pm-kernel-test-'));
  const ipc = new Ipc(), event = owner(7);
  const binary = new Uint8Array([0, 255, 42]);
  const call = vi.fn(async () => '{"ok":true,"value":null}');
  const stop = installDocumentKernelIpc({ ipc: ipc as unknown as IpcMain, cache, trusted: () => true, call });
  try {
    for (const op of ['apply', 'addFont']) {
      const request = JSON.stringify({ op, args: { revision: 1 } });
      const message = { sender: event.sender, returnValue: undefined as unknown };
      ipc.emit(IPC.call, message, request, binary);
      await vi.waitFor(() => expect(message.returnValue).toBeDefined());
      expect(call).toHaveBeenLastCalledWith(7, request, binary);
      expect(request).not.toContain('[0,255,42]');
    }
    const bad = { sender: event.sender, returnValue: undefined as unknown };
    ipc.emit(IPC.call, bad, '{"op":"query"}', binary);
    await vi.waitFor(() => expect(bad.returnValue).toBeDefined());
    expect(JSON.parse(bad.returnValue as string)).toMatchObject({ ok: false });
    expect(call).toHaveBeenCalledTimes(2);
  } finally { await stop(); await rm(cache, { recursive: true, force: true }); }
});

it('expires abandoned stream and digest tokens but not a write in flight', async () => {
  const cache = await mkdtemp(path.join(os.tmpdir(), 'pm-kernel-test-'));
  const ipc = new Ipc(), event = owner(7), held = await holdFirstWrite(cache);
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
  const stop = installDocumentKernelIpc({ ipc: ipc as unknown as IpcMain, cache, trusted: () => true, call: async () => '' });
  try {
    const abandoned = await ipc.invoke(IPC.begin, event);
    const digest = ipc.invoke(IPC.digestBegin, event, 'v');
    await vi.advanceTimersByTimeAsync(120_001);
    await new Promise<void>(resolve => setImmediate(resolve));
    await expect(ipc.invoke(IPC.write, event, abandoned, new Uint8Array([1]))).rejects.toThrow('closed');
    expect(() => ipc.invoke(IPC.digestEnd, event, digest)).toThrow('closed');
    let left = await readdir(cache);
    for (let i = 0; left.length && i < 50; i++) { await new Promise<void>(resolve => setImmediate(resolve)); left = await readdir(cache); }
    expect(left).toEqual([]);

    let active: string | undefined;
    for (let i = 0; !active && i < 50; i++) {
      try { active = await ipc.invoke(IPC.begin, event); }
      catch (error) {
        if (!(error instanceof Error) || !error.message.includes('already open')) throw error;
        await new Promise<void>(resolve => setImmediate(resolve));
      }
    }
    expect(active).toBeDefined();
    const writing = ipc.invoke(IPC.write, event, active, new Uint8Array([42]));
    await held.pending;
    await vi.advanceTimersByTimeAsync(120_001);
    held.release();
    await writing;
    await ipc.invoke(IPC.write, event, active, new Uint8Array([43]));
    await ipc.invoke(IPC.abort, event, active);
    expect(await readdir(cache)).toEqual([]);
  } finally { held.release(); held.restore(); await stop(); vi.useRealTimers(); await rm(cache, { recursive: true, force: true }); }
});
