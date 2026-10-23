import { randomBytes } from 'node:crypto';
import * as nodeFs from 'node:fs/promises';
import path from 'node:path';
import type { App, IpcMain, IpcMainEvent, IpcMainInvokeEvent } from 'electron';

import { IpcValidationError, isRecord } from '../shared/guards';
import {
  IPC,
  type StoreDeleteRequest,
  type StoreErrorEvent,
  type StoreSetRequest,
  type StoreSnapshot
} from '../shared/ipc';
import { parseStoreKey, storeFileName } from '../shared/store-keys';
import { unpackStoredHistory } from '../shared/history-memory';
import { StoreEngine, storeByteLimit, type StoreEngineOptions, type StoreReply, type StoreRequest } from './store-engine';

const WRITE_DELAY_MS = 150;
const QUIT_FLUSH_CEILING_MS = 5_000;
/* A synchronous read of a key whose newest value is still being encoded waits
   for that one answer. Renderers are told about a write only after it is
   encoded, so this wait is rare; the ceiling bounds it if the worker hangs. */
const SYNC_READ_CEILING_MS = 5_000;
const LAZY_KEY = /^(project|projectHistory|projectState|projectJournal|agentThreads|agentAttachment)\./;

export type StoreFileSystem = Pick<
  typeof nodeFs,
  'mkdir' | 'readdir' | 'readFile' | 'writeFile' | 'open' | 'rename' | 'unlink'
>;

export interface Store {
  load(): Promise<void>;
  snapshot(): StoreSnapshot;
  get?(key: string): unknown;
  getSerialized?(key: string): string | null;
  getEncodedSerialized?(key: string): string | null;
  bootstrapSerialized?(): Record<string, string>;
  snapshotSerialized?(): Record<string, string>;
  /** Throws for an unknown key; the promise rejects if the value is refused. */
  /** `writer`: who asked (a window's webContents id; SAVE-1c), so a project is written only by the window that has it. */
  set(key: string, value: unknown, writer?: unknown): void | Promise<void>;
  setSerialized?(key: string, serialized: string, writer?: unknown): void | Promise<void>;
  /** One journal entry for a project (SAVE-1, project-durability.ts). */
  appendJournal?(key: string, serialized: string, writer?: unknown): Promise<void>;
  /** Which writer holds a project (null: none), from main's window registry (SAVE-1c). */
  setOwnership?(ownerOf: (projectId: string) => unknown): void;
  /** Whether a write of `key` by `writer` is refused now: a project another window has (SAVE-1c). */
  refuses?(key: string, writer: unknown): boolean;
  /** Keys whose newest write did not reach disk (a persist failure, or a refused journal entry) and none has since. */
  failingKeys?(): string[];
  delete(key: string): void | Promise<void>;
  flushAll(): Promise<void>;
  onError(cb: (event: StoreErrorEvent) => void): () => void;
}

/** How the store reaches its engine: the persistence worker thread, or this thread. */
export interface StoreChannel {
  /** Throws synchronously if the request cannot be copied or the thread is gone. */
  post(request: StoreRequest, transfer?: ArrayBuffer[]): void;
  onReply(listener: (reply: StoreReply) => void): void;
  /** A replacement engine started; it knows nothing until the store re-initialises it. */
  onRestart?(listener: () => void): void;
  /**
   * Delivers replies synchronously until `done()`; false if the deadline
   * passed first. Throws if the answer can never arrive while this thread
   * waits, so a read is refused instead of answered with an older value.
   */
  waitSync(done: () => boolean, timeoutMs: number): boolean;
}

/** Test seams are optional; normal callers only pass the store directory. */
export interface StoreOptions {
  /** Run encoding and writes on this persistence thread (persistence-thread.ts). */
  thread?: { storeChannel(): StoreChannel };
  writeDelayMs?: number;
  /* The remaining seams apply to an engine on this thread only. */
  fs?: StoreFileSystem;
  now?: () => number;
  randomSuffix?: () => string;
  /** A project journal's byte limit (tests). */
  journalLimitBytes?: number;
  /** One journal entry's byte limit, checked on this thread (tests; default the store's). */
  journalEntryBytes?: number;
}

/**
 * The engine on the calling thread, for hosts without the persistence worker
 * (the `powermove serve` host) and for tests that substitute the filesystem.
 * Requests are copied as they would be for a worker, and encoding finishes
 * before `post` returns, except while the engine waits on the disk (loading,
 * or reading back a stored session before it has loaded). Nothing on this
 * thread can finish that wait, so a synchronous read behind it is refused.
 */
export class InlineStoreChannel implements StoreChannel {
  private readonly engine: StoreEngine;
  private readonly queue: StoreReply[] = [];
  private listener?: (reply: StoreReply) => void;
  private scheduled = false;

  constructor(options: StoreEngineOptions) {
    this.engine = new StoreEngine(options, reply => this.enqueue(reply));
  }

  post(request: StoreRequest): void {
    this.engine.handle(structuredClone(request));
  }

  onReply(listener: (reply: StoreReply) => void): void { this.listener = listener; }

  waitSync(done: () => boolean): boolean {
    while (!done() && this.queue.length) this.listener?.(this.queue.shift()!);
    if (done()) return true;
    throw new Error('The store cannot answer this read until a pending write finishes reading from disk.');
  }

  private enqueue(reply: StoreReply): void {
    this.queue.push(reply);
    if (this.scheduled) return;
    this.scheduled = true;
    queueMicrotask(() => {
      this.scheduled = false;
      while (this.queue.length) this.listener?.(this.queue.shift()!);
    });
  }
}

type Waiter = { resolve: (value: unknown) => void; reject: (error: unknown) => void };

/**
 * Main's side of the store. It holds each key's encoded JSON exactly as the
 * engine published it and answers the renderers' synchronous reads from
 * those bytes. Parsing, history packing, serialization, and file writes all
 * happen in the engine.
 */
class FileStore implements Store {
  private readonly committed = new Map<string, Buffer>();
  /** Keys whose newest set or delete the engine has not answered yet. */
  private readonly unsettled = new Map<string, { id: number; present: boolean }>();
  /** Keys whose newest committed value has not reached disk yet. */
  private readonly unwritten = new Map<string, number>();
  /** Keys whose last write failed to reach disk, until one of them reaches it. */
  private readonly failing = new Map<string, string>();
  /*
   * SAVE-1b: the one place that decides whether a project journal takes
   * entries. Any append that does not end in an acknowledged write (refused
   * here for its size, refused by the engine, not readable in a replacement
   * engine, lost when the worker stopped, or its journal write failing) leaves
   * an edit missing from the journal; a later entry accepted over it recovers
   * a document that never existed. So the journal is closed, at the request
   * id of the latest such edit, and every later entry is refused here (moving
   * that point), until a slot write posted after it has landed: its snapshot
   * holds every one of those edits. The engine is told in order (`journal`),
   * and a replacement engine is told before anything is replayed to it.
   */
  private readonly closedJournals = new Map<string, number>();
  /** Restore request id → the id of the write it replays (a replayed slot reopens by the original). */
  private readonly replayed = new Map<number, number>();
  /** Appends the engine has not answered (a refusal of one closes its journal; a refused set or delete does not). */
  private readonly appends = new Set<number>();
  /** Slot writes by (original) request id → who posted them, until they land. */
  private readonly slotWriters = new Map<number, unknown>();
  private ownerOf?: (projectId: string) => unknown;
  private readonly waiters = new Map<number, Waiter>();
  private readonly listeners = new Set<(event: StoreErrorEvent) => void>();
  private nextId = 1;

  constructor(
    private readonly directory: string,
    private readonly channel: StoreChannel,
    private readonly writeDelayMs: number,
    private readonly limits: { journalEntryBytes?: number } = {}
  ) {
    channel.onReply(reply => this.receive(reply));
    channel.onRestart?.(() => this.restarted());
    void this.request(id => ({ op: 'init', id, directory, writeDelayMs })).catch(() => undefined);
  }

  async load(): Promise<void> {
    await this.request(id => ({ op: 'load', id }));
  }

  snapshot(): StoreSnapshot {
    this.settle();
    return Object.fromEntries([...this.committed].map(([key, bytes]) => [key, unpackStoredHistory(key, JSON.parse(bytes.toString('utf8')))]));
  }

  get(key: string): unknown {
    if (!parseStoreKey(key)) return undefined;
    const encoded = this.encoded(key);
    return encoded === null ? undefined : unpackStoredHistory(key, JSON.parse(encoded));
  }

  getSerialized(key: string): string | null {
    if (!parseStoreKey(key)) return null;
    return this.serialized(key);
  }

  snapshotSerialized(): Record<string, string> {
    this.settle();
    return Object.fromEntries([...this.committed.keys()].map(key => [key, this.serialized(key)!]));
  }

  getEncodedSerialized(key: string): string | null {
    if (!parseStoreKey(key)) return null;
    return this.encoded(key);
  }

  bootstrapSerialized(): Record<string, string> {
    // Lazy keys are listed by name only, so only the others must be settled.
    this.settle(key => !LAZY_KEY.test(key));
    const keys = new Set(this.committed.keys());
    for (const [key, { present }] of this.unsettled) {
      if (present) keys.add(key); else keys.delete(key);
    }
    const lazy: string[] = [], snapshot: Record<string, string> = {};
    for (const key of keys) {
      if (LAZY_KEY.test(key)) { lazy.push(key); continue; }
      // Only an engine that missed the read ceiling leaves a key unanswered here.
      const bytes = this.committed.get(key);
      if (bytes) snapshot[key] = bytes.toString('utf8');
    }
    snapshot.__powermoveLazyKeys = JSON.stringify(lazy);
    return snapshot;
  }

  set(key: string, value: unknown, writer?: unknown): Promise<void> {
    this.fileNameFor(key, IPC.storeSet);
    if (this.refuses(key, writer)) return this.refused(IPC.storeSet, 'the project is open in another window');
    return this.mutate(key, true, IPC.storeSet, id => ({ op: 'set', id, key, value }), writer);
  }

  setSerialized(key: string, serialized: string, writer?: unknown): Promise<void> {
    this.fileNameFor(key, IPC.storeSetSerialized);
    if (this.refuses(key, writer)) return this.refused(IPC.storeSetSerialized, 'the project is open in another window');
    // A string has at least as many UTF-8 bytes as characters; the engine checks
    // the exact byte length without scanning the string on this thread.
    if (typeof serialized !== 'string' || serialized.length > storeByteLimit(key)) throw new IpcValidationError(IPC.storeSetSerialized, 'serialized value exceeds its size limit');
    return this.mutate(key, true, IPC.storeSetSerialized, id => ({ op: 'setSerialized', id, key, serialized }), writer);
  }

  setOwnership(ownerOf: (projectId: string) => unknown): void { this.ownerOf = ownerOf; }

  /*
   * SAVE-1c: a project is written only by the window that has it. A slot from
   * another window (Rename… on the Projects screen of a project open
   * elsewhere) is that window's stored copy: it lacks the owner's newest
   * edits, among them any the journal refused, and landing it would both
   * prune entries it does not hold and reopen a closed journal under a
   * document the owner never had. The window renames through the owner
   * instead (window:rename-project); this refusal is the backstop. A project
   * no window has is written by anyone, as before. A main-internal write (no
   * writer) is never refused.
   */
  refuses(key: string, writer: unknown): boolean {
    if (!key.startsWith('project.') || writer === undefined || !this.ownerOf) return false;
    const owner = this.ownerOf(key.slice('project.'.length));
    return owner !== null && owner !== undefined && owner !== writer;
  }

  private refused(channel: string, detail: string): Promise<void> {
    const refused = Promise.reject(new IpcValidationError(channel, detail));
    refused.catch(() => undefined);
    return refused;
  }

  appendJournal(key: string, serialized: string, _writer?: unknown): Promise<void> {
    this.fileNameFor(key, IPC.storeAppendJournal);
    const refuse = (detail: string): Promise<void> => {
      // This edit is not in the journal: the close point moves past it.
      this.closeJournal(key, this.nextId++);
      const refused = Promise.reject(new IpcValidationError(IPC.storeAppendJournal, detail));
      refused.catch(() => undefined);
      return refused;
    };
    if (this.closedJournals.has(key)) return refuse('the project journal is closed after an entry that was not saved, until the project is saved');
    if (typeof serialized !== 'string' || serialized.length > this.entryLimit(key)) return refuse('journal entry exceeds its size limit');
    const written = this.mutate(key, true, IPC.storeAppendJournal, id => { this.appends.add(id); return { op: 'append', id, key, serialized }; });
    return written;
  }

  private entryLimit(key: string): number { return this.limits.journalEntryBytes ?? storeByteLimit(key); }

  private closeJournal(key: string, after: number, post = true): void {
    const was = this.closedJournals.get(key);
    this.closedJournals.set(key, Math.max(was ?? 0, after));
    if (was !== undefined || !post) return;
    try { this.channel.post({ op: 'journal', id: this.nextId++, key, open: false }); } catch { /* the worker is gone; its replacement is told */ }
  }

  /** A slot write landed: a journal closed before it was posted holds nothing it lacks. */
  private slotLanded(slotKey: string, id: number, writer: unknown): void {
    const key = `projectJournal.${slotKey.slice('project.'.length)}`;
    const after = this.closedJournals.get(key);
    if (after === undefined || id <= after) return;
    // Only the window that has the project holds the edits the journal lacks.
    if (writer !== undefined && this.ownerOf) {
      const owner = this.ownerOf(slotKey.slice('project.'.length));
      if (owner !== null && owner !== undefined && owner !== writer) return;
    }
    this.closedJournals.delete(key);
    try { this.channel.post({ op: 'journal', id: this.nextId++, key, open: true }); } catch { /* reopened when the replacement starts */ }
  }

  delete(key: string): Promise<void> {
    this.fileNameFor(key, IPC.storeDelete);
    return this.mutate(key, false, IPC.storeDelete, id => ({ op: 'delete', id, key }));
  }

  async flushAll(): Promise<void> {
    await this.request(id => ({ op: 'flush', id }));
  }

  failingKeys(): string[] { return [...new Set([...this.failing.keys(), ...this.closedJournals.keys()])]; }

  onError(cb: (event: StoreErrorEvent) => void): () => void {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }

  private mutate(key: string, present: boolean, channel: string, build: (id: number) => StoreRequest, writer?: unknown): Promise<void> {
    let posted: { id: number; settled: Promise<unknown> };
    try {
      posted = this.post(build);
      if (key.startsWith('project.')) this.slotWriters.set(posted.id, writer);
    } catch (error) {
      // Values reach the engine as a structured clone, which refuses what
      // JSON cannot represent either (functions, symbols, host objects).
      if (error instanceof Error && error.name === 'DataCloneError') throw new IpcValidationError(channel, `value is not JSON-serialisable: ${error.message}`);
      // The storage worker is gone for good; report it like a failed write.
      if (key.startsWith('projectJournal.')) this.closeJournal(key, this.nextId++, false);
      this.emitError(key, errorText(error));
      const failed = Promise.reject(error);
      failed.catch(() => undefined);
      return failed;
    }
    this.unsettled.set(key, { id: posted.id, present });
    const done = posted.settled.then(() => undefined);
    // Fire-and-forget callers rely on onError; awaiting callers get the rejection.
    done.catch(() => undefined);
    return done;
  }

  private request(build: (id: number) => StoreRequest): Promise<unknown> {
    return this.post(build).settled;
  }

  private post(build: (id: number) => StoreRequest): { id: number; settled: Promise<unknown> } {
    const id = this.nextId++;
    const settled = new Promise<unknown>((resolve, reject) => this.waiters.set(id, { resolve, reject }));
    try {
      this.channel.post(build(id));
    } catch (error) {
      this.waiters.delete(id);
      throw error;
    }
    return { id, settled };
  }

  private receive(reply: StoreReply): void {
    if (reply.type === 'error') { this.emitError(reply.key, reply.error, reply.unsaved === true); return; }
    if (reply.type === 'written') {
      if (this.unwritten.get(reply.key) === reply.id) this.unwritten.delete(reply.key);
      this.failing.delete(reply.key);
      const original = this.replayed.get(reply.id) ?? reply.id;
      this.replayed.delete(reply.id);
      const writer = this.slotWriters.get(original);
      this.slotWriters.delete(original);
      if (reply.key.startsWith('project.') && original) this.slotLanded(reply.key, original, writer);
      return;
    }
    const waiter = this.waiters.get(reply.id);
    this.waiters.delete(reply.id);
    switch (reply.type) {
      case 'loaded':
        this.committed.clear();
        for (const [key, bytes] of reply.entries) this.committed.set(key, asBuffer(bytes));
        waiter?.resolve(undefined);
        return;
      case 'committed':
        this.appends.delete(reply.id);
        if (reply.bytes) this.committed.set(reply.key, asBuffer(reply.bytes));
        else this.committed.delete(reply.key);
        this.unwritten.set(reply.key, reply.id);
        if (this.unsettled.get(reply.key)?.id === reply.id) this.unsettled.delete(reply.key);
        waiter?.resolve(undefined);
        return;
      case 'rejected': {
        if (this.unsettled.get(reply.key)?.id === reply.id) this.unsettled.delete(reply.key);
        if (this.appends.delete(reply.id)) this.closeJournal(reply.key, reply.id);
        const error = new IpcValidationError(reply.channel, reply.detail);
        this.emitError(reply.key, error.message);
        waiter?.reject(error);
        return;
      }
      case 'failed':
        waiter?.reject(new Error(reply.error));
        return;
      case 'ready':
      case 'flushed':
        waiter?.resolve(undefined);
        return;
    }
  }

  /** A replacement engine: everything not yet on disk is written again. */
  private restarted(): void {
    const stopped = new Error('The storage worker stopped before this change was saved.');
    // Entries the stopped worker never answered are lost: their journals close at them.
    for (const [key, { id }] of this.unsettled) if (key.startsWith('projectJournal.')) this.closeJournal(key, id, false);
    this.appends.clear();
    for (const [key] of this.unsettled) this.emitError(key, stopped.message);
    for (const waiter of this.waiters.values()) waiter.reject(stopped);
    this.waiters.clear();
    this.unsettled.clear();
    try {
      void this.request(id => ({ op: 'init', id, directory: this.directory, writeDelayMs: this.writeDelayMs })).catch(() => undefined);
      // Closed journals first: nothing replayed reopens one unless it was posted after the close point.
      for (const key of this.closedJournals.keys()) this.channel.post({ op: 'journal', id: this.nextId++, key, open: false });
      // In the order they were first made (their ids), not the map's: a project's
      // slot and journal are replayed as they happened (project-durability.ts).
      for (const [key, latest] of [...this.unwritten].sort(([, a], [, b]) => a - b)) {
        const id = this.nextId++;
        const bytes = this.committed.get(key);
        // Replayed before (an earlier replacement): still the write it first was.
        const original = this.replayed.get(latest) ?? latest;
        this.replayed.delete(latest);
        this.channel.post({ op: 'restore', id, key, bytes: bytes ? new Uint8Array(bytes) : null });
        this.unwritten.set(key, id);
        this.replayed.set(id, original);
      }
    } catch (error) {
      for (const key of this.unwritten.keys()) this.emitError(key, `not saved: ${errorText(error)}`, true);
    }
  }

  private settle(pending: (key: string) => boolean = () => true): void {
    const waiting = () => [...this.unsettled.keys()].some(pending);
    if (!waiting()) return;
    if (!this.channel.waitSync(() => !waiting(), SYNC_READ_CEILING_MS)) {
      console.warn('A store read used the last saved values: the storage worker did not answer in time.');
    }
  }

  private encoded(key: string): string | null {
    this.settle(candidate => candidate === key);
    return this.committed.get(key)?.toString('utf8') ?? null;
  }

  /** The value as the renderer stores it, with undo history unpacked. */
  private serialized(key: string): string | null {
    const encoded = this.encoded(key);
    if (encoded === null || !/^project(History|State)\./.test(key)) return encoded;
    return JSON.stringify(unpackStoredHistory(key, JSON.parse(encoded)));
  }

  private fileNameFor(key: string, channel: string): string {
    if (typeof key !== 'string') throw new IpcValidationError(channel, 'key must be a string');
    const parsed = parseStoreKey(key);
    if (parsed === null) throw new IpcValidationError(channel, `unknown store key: ${key}`);
    return storeFileName(parsed);
  }

  private emitError(key: string, error: string, unsaved = false): void {
    if (unsaved) {
      this.failing.set(key, error);
      // A journal write that failed, or a replay that could not be applied: entries may be missing.
      if (key.startsWith('projectJournal.')) this.closeJournal(key, this.nextId - 1);
    }
    const event: StoreErrorEvent = unsaved ? { key, error, unsaved } : { key, error };
    for (const listener of this.listeners) {
      try {
        listener(event);
      } catch {
        // A reporting callback must not stop persistence or other listeners.
      }
    }
  }
}

export function createStore(dir: string, options: StoreOptions = {}): Store {
  if (options.thread && (options.fs || options.now || options.randomSuffix || options.journalLimitBytes)) {
    throw new Error('Filesystem seams apply only to a store engine on this thread.');
  }
  const channel = options.thread?.storeChannel() ?? new InlineStoreChannel({
    fs: options.fs ?? nodeFs,
    now: options.now ?? Date.now,
    randomSuffix: options.randomSuffix ?? (() => `${process.pid}-${randomBytes(8).toString('hex')}`),
    ...(options.journalLimitBytes ? { journalLimitBytes: options.journalLimitBytes } : {})
  });
  return new FileStore(path.resolve(dir), channel, options.writeDelayMs ?? WRITE_DELAY_MS, options.journalEntryBytes ? { journalEntryBytes: options.journalEntryBytes } : {});
}

function asBuffer(bytes: Uint8Array): Buffer {
  return Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}

type StoreIpcEvent = IpcMainEvent | IpcMainInvokeEvent;

export interface StoreIpcContext {
  isTrustedSender(event: StoreIpcEvent): boolean;
  /**
   * Tells the other windows which keys a window just wrote. Each renderer keeps
   * its own cache of the store, so without this a second window would answer
   * reads from a snapshot taken before the first window's edits and overwrite
   * them on its next write.
   */
  broadcastChange?(keys: string[], sender: unknown): void;
  /** The writer id (webContents id) of the window that has a project, or null (SAVE-1c). */
  ownerOf?(projectId: string): unknown;
}

export function registerStoreIpc(
  ipcMain: Pick<IpcMain, 'handle' | 'on'>,
  store: Store,
  ctx: StoreIpcContext
): void {
  const requireTrusted = (event: StoreIpcEvent, channel: string): void => {
    if (!ctx.isTrustedSender(event)) throw new IpcValidationError(channel, 'untrusted sender');
  };
  /* A write main accepted can still fail to reach disk (ENOSPC, EIO, a
     storage worker that stopped). Its writer is told, as it is of a refused
     value: without this a window kept treating such a project checkpoint as
     written, never wrote it again, and kept throwing nothing. */
  type Writer = { send(channel: string, payload: unknown): void; isDestroyed?(): boolean };
  /** A window's identity as a writer (its webContents id), for main's ownership rule (SAVE-1c). */
  const writerOf = (event: StoreIpcEvent): unknown => (event.sender as { id?: unknown }).id;
  if (ctx.ownerOf) store.setOwnership?.(ctx.ownerOf);
  const writers = new Map<string, Writer>();
  /* Only a write main accepts makes its window the key's writer: a refused
     one (another window's project, SAVE-1c) must not take over where the
     owner's later persist failures are sent (review recheck 6, R6h). It is
     recorded before the write is posted, so a failure reported while posting
     reaches it too. */
  const wrote = (event: StoreIpcEvent, key: string): boolean => {
    if (store.refuses?.(key, writerOf(event))) return false;
    writers.set(key, event.sender as unknown as Writer);
    return true;
  };
  store.onError(event => {
    if (!event.unsaved) return;
    const writer = writers.get(event.key);
    if (!writer || writer.isDestroyed?.()) return;
    try { writer.send(IPC.storeError, { key: event.key, error: event.error, unsaved: true } satisfies StoreErrorEvent); } catch { /* the window is closing */ }
  });
  /* Writes arrive in bursts (a rename touches the registry and the slot), so
     the keys are coalesced into one message per turn of the event loop. A key
     is announced once its value is encoded, so a sibling's refill reads it
     without waiting on the storage worker. */
  const pendingChanges = new Map<unknown, Set<string>>();
  const announce = (key: string, sender: unknown): void => {
    if (!ctx.broadcastChange) return;
    const existing = pendingChanges.get(sender);
    if (existing) {
      existing.add(key);
      return;
    }
    pendingChanges.set(sender, new Set([key]));
    setImmediate(() => {
      const keys = pendingChanges.get(sender);
      pendingChanges.delete(sender);
      if (keys?.size) ctx.broadcastChange?.([...keys], sender);
    });
  };
  /* Fire-and-forget writes learn about a refused value the way they always
     have: a storeError message to the window that sent it. */
  const settleWrite = (event: IpcMainEvent, payload: unknown, key: string, written: void | Promise<void>): void => {
    void Promise.resolve(written).then(
      () => announce(key, event.sender),
      (error: unknown) => sendStoreError(event, payload, error)
    );
  };

  ipcMain.handle(IPC.storeSnapshot, (event) => {
    requireTrusted(event, IPC.storeSnapshot);
    return { ...store.snapshot(), __powermoveAsyncStore: true };
  });

  ipcMain.handle(IPC.storeSetSerialized, async (event, payload: unknown) => {
    requireTrusted(event, IPC.storeSetSerialized);
    if (!isRecord(payload) || typeof payload.key !== 'string' || typeof payload.serialized !== 'string') throw new IpcValidationError(IPC.storeSetSerialized, 'expected key and serialized JSON');
    wrote(event, payload.key);
    if (store.setSerialized) await store.setSerialized(payload.key, payload.serialized, writerOf(event));
    else await store.set(payload.key, JSON.parse(payload.serialized), writerOf(event));
    announce(payload.key, event.sender);
  });

  // JSON values cross IPC and contextBridge without recursively copying and
  // freezing each history object. Keep one string per key so the entire store
  // does not have to fit in V8's maximum single-string length.
  ipcMain.on(IPC.storeSnapshotSerializedSync, (event) => {
    event.returnValue = ctx.isTrustedSender(event)
      ? { ...(store.snapshotSerialized?.() ?? Object.fromEntries(Object.entries(store.snapshot()).map(([key, value]) => [key, JSON.stringify(value)]))), __powermoveAsyncStore: 'true' }
      : {};
  });

  ipcMain.on(IPC.storeBootstrapSync, (event) => {
    event.returnValue = ctx.isTrustedSender(event)
      ? { ...(store.bootstrapSerialized?.() ?? store.snapshotSerialized?.() ?? {}), __powermoveAsyncStore: 'true' }
      : {};
  });

  ipcMain.on(IPC.storeGetEncodedSync, (event, payload: unknown) => {
    if (!ctx.isTrustedSender(event) || !isRecord(payload) || typeof payload.key !== 'string') {
      event.returnValue = null; return;
    }
    event.returnValue = store.getEncodedSerialized?.(payload.key) ?? store.getSerialized?.(payload.key) ?? null;
  });

  ipcMain.on(IPC.storeSnapshotSync, (event) => {
    if (!ctx.isTrustedSender(event)) {
      event.returnValue = {};
      return;
    }
    event.returnValue = { ...store.snapshot(), __powermoveAsyncStore: true };
  });

  ipcMain.on(IPC.storeSet, (event, payload: unknown) => {
    if (!ctx.isTrustedSender(event)) return;
    try {
      if (!isRecord(payload) || typeof payload['key'] !== 'string' || !('value' in payload)) {
        throw new IpcValidationError(IPC.storeSet, 'expected { key, value }');
      }
      const request = payload as unknown as StoreSetRequest;
      wrote(event, request.key);
      settleWrite(event, payload, request.key, store.set(request.key, request.value, writerOf(event)));
    } catch (error) {
      if (error instanceof IpcValidationError) {
        sendValidationError(event, payload, error);
        return;
      }
      sendStoreError(event, payload, error);
    }
  });

  ipcMain.on(IPC.storeDelete, (event, payload: unknown) => {
    if (!ctx.isTrustedSender(event)) return;
    try {
      if (!isRecord(payload) || typeof payload['key'] !== 'string') {
        throw new IpcValidationError(IPC.storeDelete, 'expected { key }');
      }
      const request = payload as unknown as StoreDeleteRequest;
      wrote(event, request.key);
      settleWrite(event, payload, request.key, store.delete(request.key));
    } catch (error) {
      if (error instanceof IpcValidationError) {
        sendValidationError(event, payload, error);
        return;
      }
      sendStoreError(event, payload, error);
    }
  });

  // Refills one key a sibling window invalidated. Synchronous because the
  // renderer's PM.store.get is synchronous everywhere it is called from.
  ipcMain.on(IPC.storeGetSync, (event, payload: unknown) => {
    if (!ctx.isTrustedSender(event) || !isRecord(payload) || typeof payload['key'] !== 'string') {
      event.returnValue = null;
      return;
    }
    if (store.getSerialized) event.returnValue = store.getSerialized(payload['key'] as string);
    else {
      const value = store.snapshot()[payload['key'] as string];
      event.returnValue = value === undefined ? null : JSON.stringify(value);
    }
  });

  ipcMain.handle(IPC.storeAppendJournal, async (event, payload: unknown) => {
    requireTrusted(event, IPC.storeAppendJournal);
    if (!isRecord(payload) || typeof payload.key !== 'string' || typeof payload.serialized !== 'string') throw new IpcValidationError(IPC.storeAppendJournal, 'expected key and serialized JSON');
    if (!store.appendJournal) throw new IpcValidationError(IPC.storeAppendJournal, 'this store keeps no project journals');
    wrote(event, payload.key);
    await store.appendJournal(payload.key, payload.serialized, writerOf(event));
    announce(payload.key, event.sender);
  });

  ipcMain.handle(IPC.storeFlush, async (event) => {
    requireTrusted(event, IPC.storeFlush);
    await store.flushAll();
    return { failing: store.failingKeys?.() ?? [] };
  });
}

function sendValidationError(
  event: IpcMainEvent,
  payload: unknown,
  error: IpcValidationError
): void {
  sendStoreError(event, payload, error);
}

function sendStoreError(event: IpcMainEvent, payload: unknown, error: unknown): void {
  // The window may have closed while its value was being encoded.
  if ((event.sender as { isDestroyed?: () => boolean }).isDestroyed?.()) return;
  const key = isRecord(payload) && typeof payload['key'] === 'string' ? payload['key'] : '';
  event.sender.send(IPC.storeError, { key, error: errorText(error) } satisfies StoreErrorEvent);
}

/**
 * Installs the two-pass quit barrier. The integrator owns and acquires the
 * application's single-instance lock before installing this helper.
 */
export function installQuitFlush(app: Pick<App, 'on' | 'quit'>, store: Store, prepare?: () => Promise<void>): { isPrepared(): boolean } {
  let flushing = false;
  let flushed = false;

  app.on('before-quit', (event) => {
    if (flushed) return;
    event.preventDefault();
    if (flushing) return;
    flushing = true;

    const finish = () => withCeiling(store.flushAll(), QUIT_FLUSH_CEILING_MS).finally(() => {
      flushed = true; flushing = false; app.quit();
    });
    if (prepare) {
      void prepare().then(finish).catch(error => {
        flushing = false;
        console.error('Could not prepare editor state for quitting; keeping the app open',error);
      });
    } else void finish();
  });
  return {isPrepared:() => flushed};
}

async function withCeiling(operation: Promise<void>, milliseconds: number): Promise<void> {
  let timeout: ReturnType<typeof setTimeout> | undefined;
  const ceiling = new Promise<void>((resolve) => {
    timeout = setTimeout(resolve, milliseconds);
  });
  try {
    await Promise.race([operation.catch(() => undefined), ceiling]);
  } finally {
    if (timeout !== undefined) clearTimeout(timeout);
  }
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
