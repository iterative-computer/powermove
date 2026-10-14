import { randomBytes } from 'node:crypto';
import * as nodeFs from 'node:fs/promises';
import * as syncFs from 'node:fs';
import path from 'node:path';
import type { App, IpcMain, IpcMainEvent, IpcMainInvokeEvent } from 'electron';

import { IpcValidationError, isRecord } from '../shared/guards';
import {
  IPC,
  LIMITS,
  type StoreDeleteRequest,
  type StoreErrorEvent,
  type StoreSetRequest,
  type StoreSnapshot
} from '../shared/ipc';
import { parseStoreKey, storeFileName } from '../shared/store-keys';
import { HistoryRecords, isHistoryDictionary, packStoredHistory, unpackStoredHistory } from '../shared/history-memory';

const WRITE_DELAY_MS = 150;
const HEAVY_KEY = /^(project|projectHistory|projectState|projectJournal|agentThreads|agentAttachment)\./;
const HEAVY_CACHE_BYTES = 64 * 1024 * 1024;
const QUIT_FLUSH_CEILING_MS = 5_000;
// Takes retain up to 24 complete editable snapshots; large tracked projects
// can exceed the ordinary settings limit without being corrupt.
const MAX_TAKES_BYTES = 24 * LIMITS.storeValueBytes;

export type StoreFileSystem = Pick<
  typeof nodeFs,
  'mkdir' | 'readdir' | 'readFile' | 'writeFile' | 'open' | 'rename' | 'unlink'
>;

export interface Store {
  load(): Promise<void>;
  snapshot(): StoreSnapshot;
  preload?(key: string): Promise<void>;
  preloadProject?(id: string): Promise<void>;
  /** Independent consumers hold a lease until their project session ends. */
  acquireProject?(id: string): () => void;
  get?(key: string): unknown;
  getSerialized?(key: string): string | null;
  getEncodedSerialized?(key: string): string | null;
  getEncodedSerializedAsync?(key: string): Promise<string | null>;
  bootstrapSerialized?(): Record<string, string>;
  snapshotSerialized?(): Record<string, string>;
  set(key: string, value: unknown): void;
  setSerialized?(key: string, serialized: string): void;
  delete(key: string): void;
  flushAll(): Promise<void>;
  onError(cb: (event: StoreErrorEvent) => void): () => void;
}

/** Test seams are optional; normal callers only pass the store directory. */
export interface StoreOptions {
  fs?: StoreFileSystem;
  syncFs?: Pick<typeof syncFs, 'readFileSync' | 'renameSync'>;
  heavyCacheBytes?: number;
  writeDelayMs?: number;
  now?: () => number;
  randomSuffix?: () => string;
}

type PendingOperation =
  | { kind: 'set'; serialized: string }
  | { kind: 'delete' };

class FileStore implements Store {
  private readonly historyRecords = new HistoryRecords();
  private readonly values = new Map<string, unknown>();
  private readonly keys = new Set<string>();
  private readonly cacheBytes = new Map<string, number>();
  private readonly projectLeases = new Map<string, number>();
  private readonly versions = new Map<string, number>();
  private readonly loading = new Map<string, Promise<unknown>>();
  // A failed write still owns the only current copy; keep it until a successful write.
  private readonly failedWrites = new Set<string>();
  private readonly listeners = new Set<(event: StoreErrorEvent) => void>();
  private readonly pending = new Map<string, PendingOperation>();
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();
  private readonly queues = new Map<string, Promise<void>>();

  constructor(
    private readonly directory: string,
    private readonly fs: StoreFileSystem,
    private readonly writeDelayMs: number,
    private readonly now: () => number,
    private readonly randomSuffix: () => string,
    private readonly synchronousFs: Pick<typeof syncFs, 'readFileSync' | 'renameSync'>,
    private readonly heavyCacheBytes: number
  ) {}

  async load(): Promise<void> {
    await this.fs.mkdir(this.directory, { recursive: true });
    await this.removeStaleTemporaryFiles();

    const names = await this.fs.readdir(this.directory);
    const loadedProjectIds = new Set<string>();
    this.values.clear();
    this.keys.clear();
    this.cacheBytes.clear();
    this.historyRecords.clear();

    for (const name of names) {
      if (!name.endsWith('.json')) continue;

      const candidateKey = name.slice(0, -'.json'.length);
      const parsedKey = parseStoreKey(candidateKey);
      // Reconstructing the name prevents aliases and unchecked filenames from
      // ever reaching a filesystem path.
      if (parsedKey === null || storeFileName(parsedKey) !== name) continue;

      this.keys.add(candidateKey);
      if (parsedKey.kind === 'dynamic' && parsedKey.prefix === 'project') loadedProjectIds.add(parsedKey.id);
      if (HEAVY_KEY.test(candidateKey)) continue;

      const filePath = path.join(this.directory, name);
      let serialized: string;
      try {
        serialized = await this.fs.readFile(filePath, 'utf8');
      } catch (error) {
        this.emitError(candidateKey, `store file could not be read: ${errorText(error)}`);
        continue;
      }

      try {
        this.acceptSerialized(candidateKey, serialized);
      } catch (error) {
        this.keys.delete(candidateKey);
        await this.quarantine(name, candidateKey, error);
      }
    }

    this.reportProjectReconciliation(loadedProjectIds);
  }

  /** Snapshots are bootstrap metadata, never a full-store materialization. */
  snapshot(): StoreSnapshot {
    return Object.fromEntries(Object.entries(this.bootstrapSerialized()).map(([key, value]) => [key, JSON.parse(value)]));
  }

  get(key: string): unknown {
    if (!parseStoreKey(key)) return undefined;
    return unpackStoredHistory(key, structuredClone(this.readThrough(key)));
  }

  getSerialized(key: string): string | null {
    if (!parseStoreKey(key)) return null;
    const value = unpackStoredHistory(key, this.readThrough(key));
    return value === undefined ? null : JSON.stringify(value);
  }

  snapshotSerialized(): Record<string, string> { return this.bootstrapSerialized(); }

  getEncodedSerialized(key: string): string | null {
    if (!parseStoreKey(key)) return null;
    const value = this.readThrough(key);
    return value === undefined ? null : JSON.stringify(value);
  }

  bootstrapSerialized(): Record<string, string> {
    const snapshot: Record<string, string> = {};
    for (const [key, value] of this.values) if (!HEAVY_KEY.test(key)) snapshot[key] = JSON.stringify(value);
    const lazy = [...this.keys].filter(key => HEAVY_KEY.test(key));
    if (lazy.length) snapshot.__powermoveLazyKeys = JSON.stringify(lazy);
    return snapshot;
  }

  async preload(key: string): Promise<void> { await this.loadValue(key); }

  async getEncodedSerializedAsync(key: string): Promise<string | null> {
    const value = await this.loadValue(key);
    return value === undefined ? null : JSON.stringify(value);
  }

  private async loadValue(key: string): Promise<unknown> {
    if (!parseStoreKey(key) || !this.keys.has(key)) return undefined;
    if (this.values.has(key)) return this.readThrough(key);
    const active = this.loading.get(key);
    if (active) return active;
    const version = this.versions.get(key);
    const task = (async () => {
      let serialized: string;
      try { serialized = await this.fs.readFile(path.join(this.directory, this.fileNameFor(key, 'store:read')), 'utf8'); }
      catch (error) {
        if (this.keys.has(key) && this.versions.get(key) === version) this.emitError(key, `store file could not be read: ${errorText(error)}`);
        return;
      }
      // A synchronous read, set or delete supersedes this disk snapshot.
      if (!this.keys.has(key)) return undefined;
      if (this.values.has(key) || this.versions.get(key) !== version) return this.readThrough(key);
      try { return this.acceptSerialized(key, serialized); }
      catch (error) { this.quarantineHeavy(key, error); }
    })();
    this.loading.set(key, task);
    try { return await task; }
    finally { if (this.loading.get(key) === task) this.loading.delete(key); }
  }

  async preloadProject(id: string): Promise<void> {
    // Serial reads bound the parse peak; attachments are fetched individually.
    const release = this.acquireProject(id);
    try {
      for (const prefix of ['project', 'projectHistory', 'projectState', 'projectJournal', 'agentThreads']) {
        await this.preload(`${prefix}.${id}`);
      }
    } finally { release(); }
  }

  acquireProject(id: string): () => void {
    if (!parseStoreKey(`project.${id}`)) throw new Error('Invalid project id');
    this.projectLeases.set(id, (this.projectLeases.get(id) ?? 0) + 1);
    let released = false;
    return () => {
      if (released) return;
      released = true;
      const remaining = (this.projectLeases.get(id) ?? 1) - 1;
      if (remaining) this.projectLeases.set(id, remaining);
      else this.projectLeases.delete(id);
      this.evict();
    };
  }

  private pinnedKeys(): Set<string> {
    const projects = new Set(this.projectLeases.keys());
    const add = (id: unknown): void => {
      if (typeof id === 'string' && parseStoreKey(`project.${id}`)) projects.add(id);
    };
    add(this.values.get('lastActiveProject'));
    for (const name of ['openWindows', 'windowTabs']) {
      const entries = this.values.get(name);
      if (!Array.isArray(entries)) continue;
      for (const entry of entries) {
        add(entry);
        if (isRecord(entry)) {
          add(entry.active);
          if (Array.isArray(entry.tabs)) for (const id of entry.tabs) add(id);
        }
      }
    }
    // Once current window metadata exists, the old single-window strip is stale.
    if (!this.values.has('openWindows') && !this.values.has('windowTabs')) {
      const tabs = this.values.get('openTabs');
      if (Array.isArray(tabs)) for (const id of tabs) add(id);
    }
    const pinned = new Set<string>();
    for (const id of projects) {
      for (const prefix of ['project', 'projectHistory', 'projectState', 'projectJournal', 'agentThreads']) {
        pinned.add(`${prefix}.${id}`);
      }
      // Attachment ids are independent of project ids; their owner is the
      // loaded thread archive. Pin the archived payloads without loading them.
      const archive = this.values.get(`agentThreads.${id}`);
      if (!isRecord(archive) || !Array.isArray(archive.threads)) continue;
      const attachments = (items: unknown): void => {
        if (!Array.isArray(items)) return;
        for (const item of items) if (isRecord(item) && item.payloadStored === true && typeof item.id === 'string') {
          const key = `agentAttachment.${item.id}`;
          if (parseStoreKey(key)) pinned.add(key);
        }
      };
      for (const thread of archive.threads) if (isRecord(thread)) {
        attachments(thread.attachments);
        if (Array.isArray(thread.conversation)) for (const message of thread.conversation) {
          if (isRecord(message)) attachments(message.attachments);
        }
      }
    }
    return pinned;
  }

  private readThrough(key: string): unknown {
    if (!this.keys.has(key)) return undefined;
    if (this.values.has(key)) {
      const bytes = this.cacheBytes.get(key);
      if (bytes !== undefined) { this.cacheBytes.delete(key); this.cacheBytes.set(key, bytes); }
      return this.values.get(key);
    }
    let serialized: string;
    try { serialized = this.synchronousFs.readFileSync(path.join(this.directory, this.fileNameFor(key, 'store:read')), 'utf8'); }
    catch (error) { this.emitError(key, `store file could not be read: ${errorText(error)}`); return undefined; }
    this.bumpVersion(key);
    try { return this.acceptSerialized(key, serialized); }
    catch (error) { this.quarantineHeavy(key, error); return undefined; }
  }

  private acceptSerialized(key: string, serialized: string): unknown {
    const value: unknown = JSON.parse(serialized);
    const packed = packStoredHistory(key, value, this.historyRecords);
    this.values.set(key, packed);
    const before = key.startsWith('projectState.') && isRecord(value) ? value.history : value;
    const after = key.startsWith('projectState.') && isRecord(packed) ? packed.history : packed;
    if (isHistoryDictionary(after) && !isHistoryDictionary(before)) {
      this.schedule(key, { kind: 'set', serialized: JSON.stringify(packed) });
    }
    this.touchCache(key, Buffer.byteLength(serialized));
    return packed;
  }

  private touchCache(key: string, bytes: number): void {
    if (!HEAVY_KEY.test(key)) { this.evict(); return; }
    this.cacheBytes.delete(key);
    // Account conservatively for V8 strings, rather than retaining serialized copies.
    this.cacheBytes.set(key, bytes * 2);
    this.evict();
  }

  private evict(): void {
    const pinned = this.pinnedKeys();
    // Unknown consumers still need a warm project AND history, even when
    // either exceeds the budget alone. Keep the MRU of each heavy family.
    const latest = new Map<string, string>();
    let bytes = 0;
    for (const [key, size] of this.cacheBytes) {
      latest.set(key.slice(0, key.indexOf('.')), key);
      if (!pinned.has(key)) bytes += size;
    }
    for (const [key, size] of this.cacheBytes) {
      if (bytes <= this.heavyCacheBytes) break;
      if (pinned.has(key) || latest.get(key.slice(0, key.indexOf('.'))) === key) continue;
      if (this.pending.has(key) || this.timers.has(key) || this.queues.has(key) || this.failedWrites.has(key)) continue;
      this.values.delete(key);
      this.cacheBytes.delete(key);
      bytes -= size;
    }
    // HistoryRecords contains only WeakRefs; evicting one history must not clear
    // the interning index still shared by other cached histories.
  }

  private bumpVersion(key: string): void { this.versions.set(key, (this.versions.get(key) ?? 0) + 1); }

  private quarantineHeavy(key: string, cause: unknown): void {
    this.keys.delete(key);
    this.values.delete(key);
    this.cacheBytes.delete(key);
    this.bumpVersion(key);
    const name = this.fileNameFor(key, 'store:read');
    const destination = `${name}.corrupt-${this.now()}`;
    let detail = `invalid store file quarantined as ${destination}: ${errorText(cause)}`;
    // No await between detecting corruption and moving the file: a new write
    // cannot replace it in that gap and accidentally be quarantined instead.
    try { this.synchronousFs.renameSync(path.join(this.directory, name), path.join(this.directory, destination)); }
    catch (error) { detail = `invalid store file could not be quarantined: ${errorText(cause)}; ${errorText(error)}`; }
    this.emitError(key, detail);
  }

  set(key: string, value: unknown): void {
    this.fileNameFor(key, IPC.storeSet);

    // Old renderers send session metadata without history. Such a partial
    // update must never erase the last recoverable undo stack.
    const previous = key.startsWith('projectState.') && isRecord(value) && !Object.hasOwn(value, 'history')
      ? this.readThrough(key) : this.values.get(key);
    if (key.startsWith('projectState.') && isRecord(value) && !Object.hasOwn(value, 'history') && isRecord(previous) && previous.history !== undefined) {
      value = { ...value, history: previous.history };
    }
    let cloned: unknown;
    let serialized: string | undefined;
    try {
      cloned = packStoredHistory(key, structuredClone(value), this.historyRecords);
      serialized = JSON.stringify(cloned);
    } catch (error) {
      throw new IpcValidationError(IPC.storeSet, `value is not JSON-serialisable: ${errorText(error)}`);
    }

    if (serialized === undefined) {
      throw new IpcValidationError(IPC.storeSet, 'value is not JSON-serialisable');
    }
    const byteLength = Buffer.byteLength(serialized, 'utf8');
    const parsedKey = parseStoreKey(key);
    // Project snapshots and sessions (including undo/redo history) must support
    // the same size as project files, including during autosave and save completion.
    const isProject = parsedKey?.kind === 'dynamic'
      && (parsedKey.prefix === 'project' || parsedKey.prefix === 'projectState' || parsedKey.prefix === 'projectHistory');
    const byteLimit = key === 'takes' ? MAX_TAKES_BYTES
      : isProject ? LIMITS.fileSaveBytes : LIMITS.storeValueBytes;
    if (byteLength > byteLimit) {
      throw new IpcValidationError(
        IPC.storeSet,
        `serialised value is ${byteLength} bytes (maximum ${byteLimit})`
      );
    }

    this.bumpVersion(key);
    this.keys.add(key);
    this.values.set(key, cloned);
    this.schedule(key, { kind: 'set', serialized });
    this.touchCache(key, byteLength);
  }

  setSerialized(key: string, serialized: string): void {
    this.fileNameFor(key, IPC.storeSetSerialized);
    const parsed = parseStoreKey(key);
    const large = parsed?.kind === 'dynamic' && ['project', 'projectState', 'projectHistory'].includes(parsed.prefix);
    const limit = key === 'takes' ? MAX_TAKES_BYTES : large ? LIMITS.fileSaveBytes : LIMITS.storeValueBytes;
    if (typeof serialized !== 'string' || Buffer.byteLength(serialized) > limit) throw new IpcValidationError(IPC.storeSetSerialized, 'serialized value exceeds its size limit');
    let value: unknown;
    try { value = JSON.parse(serialized); }
    catch { throw new IpcValidationError(IPC.storeSetSerialized, 'invalid JSON'); }
    if (key.startsWith('projectState.')) { this.set(key, value); return; }
    const packed = packStoredHistory(key, value, this.historyRecords);
    this.bumpVersion(key);
    this.keys.add(key);
    this.values.set(key, packed);
    this.schedule(key, { kind: 'set', serialized: packed === value ? serialized : JSON.stringify(packed) });
    this.touchCache(key, Buffer.byteLength(serialized));
  }

  delete(key: string): void {
    this.fileNameFor(key, IPC.storeDelete);
    this.bumpVersion(key);
    this.keys.delete(key);
    this.cacheBytes.delete(key);
    this.values.delete(key);
    this.schedule(key, { kind: 'delete' });
    this.evict();
  }

  async flushAll(): Promise<void> {
    // Scheduling a pending operation appends it to that key's existing chain.
    for (const key of [...this.pending.keys()]) this.enqueuePending(key);

    // Writes can finish while this array is assembled, but every promise here
    // represents the tail of its key's queue at the flush boundary.
    await Promise.all([...this.queues.values()]);
  }

  onError(cb: (event: StoreErrorEvent) => void): () => void {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }

  private schedule(key: string, operation: PendingOperation): void {
    this.pending.set(key, operation);
    const previousTimer = this.timers.get(key);
    if (previousTimer !== undefined) clearTimeout(previousTimer);
    this.timers.set(
      key,
      setTimeout(() => this.enqueuePending(key), this.writeDelayMs)
    );
  }

  private enqueuePending(key: string): void {
    const timer = this.timers.get(key);
    if (timer !== undefined) clearTimeout(timer);
    this.timers.delete(key);

    const operation = this.pending.get(key);
    if (operation === undefined) return;
    this.pending.delete(key);

    const previous = this.queues.get(key) ?? Promise.resolve();
    const current = previous
      .then(async () => { await this.persist(key, operation); this.failedWrites.delete(key); })
      .catch((error: unknown) => {
        this.failedWrites.add(key);
        this.emitError(key, errorText(error));
      });
    this.queues.set(key, current);
    void current.finally(() => {
      if (this.queues.get(key) === current) this.queues.delete(key);
      this.evict();
    });
  }

  private async persist(key: string, operation: PendingOperation): Promise<void> {
    const finalPath = path.join(this.directory, this.fileNameFor(key, 'store:write'));
    if (operation.kind === 'delete') {
      try {
        await this.fs.unlink(finalPath);
      } catch (error) {
        if (!isErrorCode(error, 'ENOENT')) throw error;
      }
      return;
    }

    const temporaryPath = `${finalPath}.tmp-${this.randomSuffix()}`;
    try {
      // Atomic replacement sequence: write a sibling temporary file, open and
      // fsync that file, close it, then rename it over the destination.
      await this.fs.writeFile(temporaryPath, operation.serialized, { encoding: 'utf8', flag: 'wx' });
      const handle = await this.fs.open(temporaryPath, 'r');
      try {
        await handle.sync();
      } finally {
        await handle.close();
      }
      await this.fs.rename(temporaryPath, finalPath);
    } catch (error) {
      try {
        await this.fs.unlink(temporaryPath);
      } catch (cleanupError) {
        if (!isErrorCode(cleanupError, 'ENOENT')) {
          this.emitError(key, `temporary-file cleanup failed: ${errorText(cleanupError)}`);
        }
      }
      throw error;
    }
  }

  private async removeStaleTemporaryFiles(): Promise<void> {
    const names = await this.fs.readdir(this.directory);
    await Promise.all(
      names
        .filter((name) => name.includes('.tmp-'))
        .map(async (name) => {
          try {
            await this.fs.unlink(path.join(this.directory, name));
          } catch (error) {
            if (!isErrorCode(error, 'ENOENT')) {
              this.emitError(name, `stale temporary-file cleanup failed: ${errorText(error)}`);
            }
          }
        })
    );
  }

  private async quarantine(name: string, key: string, cause: unknown): Promise<void> {
    const source = path.join(this.directory, name);
    const destination = path.join(this.directory, `${name}.corrupt-${this.now()}`);
    let detail = `invalid store file quarantined as ${path.basename(destination)}: ${errorText(cause)}`;
    try {
      await this.fs.rename(source, destination);
    } catch (error) {
      detail = `invalid store file could not be quarantined: ${errorText(cause)}; ${errorText(error)}`;
    }
    this.emitError(key, detail);
  }

  private reportProjectReconciliation(projectFileIds: ReadonlySet<string>): void {
    const registry = this.values.get('projects');
    const registryIds = new Set<string>();
    if (Array.isArray(registry)) {
      for (const entry of registry) {
        if (isRecord(entry) && typeof entry['id'] === 'string') registryIds.add(entry['id']);
      }
    }

    for (const id of registryIds) {
      if (!projectFileIds.has(id)) {
        this.emitError(`project.${id}`, 'project registry entry has no corresponding project file');
      }
    }
    for (const id of projectFileIds) {
      if (!registryIds.has(id)) {
        this.emitError(`project.${id}`, 'project file has no corresponding registry entry');
      }
    }
  }

  private fileNameFor(key: string, channel: string): string {
    if (typeof key !== 'string') throw new IpcValidationError(channel, 'key must be a string');
    const parsed = parseStoreKey(key);
    if (parsed === null) throw new IpcValidationError(channel, `unknown store key: ${key}`);
    return storeFileName(parsed);
  }

  private emitError(key: string, error: string): void {
    const event: StoreErrorEvent = { key, error };
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
  return new FileStore(
    path.resolve(dir),
    options.fs ?? nodeFs,
    options.writeDelayMs ?? WRITE_DELAY_MS,
    options.now ?? Date.now,
    options.randomSuffix ?? (() => `${process.pid}-${randomBytes(8).toString('hex')}`),
    options.syncFs ?? syncFs,
    options.heavyCacheBytes ?? HEAVY_CACHE_BYTES
  );
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
}

export function registerStoreIpc(
  ipcMain: Pick<IpcMain, 'handle' | 'on'>,
  store: Store,
  ctx: StoreIpcContext
): void {
  const requireTrusted = (event: StoreIpcEvent, channel: string): void => {
    if (!ctx.isTrustedSender(event)) throw new IpcValidationError(channel, 'untrusted sender');
  };
  /* Writes arrive in bursts (a rename touches the registry and the slot), so
     the keys are coalesced into one message per turn of the event loop. */
  const pendingChanges = new Map<unknown, Set<string>>();
  const announce = (key: string, sender: unknown): void => {
    if (!ctx.broadcastChange) return;
    const existing = pendingChanges.get(sender);
    if (existing) {
      existing.add(key);
      return;
    }
    pendingChanges.set(sender, new Set([key]));
    queueMicrotask(() => {
      const keys = pendingChanges.get(sender);
      pendingChanges.delete(sender);
      if (keys?.size) ctx.broadcastChange?.([...keys], sender);
    });
  };

  ipcMain.handle(IPC.storeSnapshot, (event) => {
    requireTrusted(event, IPC.storeSnapshot);
    return { ...store.snapshot(), __powermoveAsyncStore: true };
  });

  ipcMain.handle(IPC.storeSetSerialized, (event, payload: unknown) => {
    requireTrusted(event, IPC.storeSetSerialized);
    if (!isRecord(payload) || typeof payload.key !== 'string' || typeof payload.serialized !== 'string') throw new IpcValidationError(IPC.storeSetSerialized, 'expected key and serialized JSON');
    if (store.setSerialized) store.setSerialized(payload.key, payload.serialized);
    else store.set(payload.key, JSON.parse(payload.serialized));
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

  ipcMain.handle(IPC.storeGetEncoded, async (event, payload: unknown) => {
    if (!ctx.isTrustedSender(event) || !isRecord(payload) || typeof payload.key !== 'string') {
      throw new IpcValidationError(IPC.storeGetEncoded, 'expected a trusted store key');
    }
    if (store.getEncodedSerializedAsync) return store.getEncodedSerializedAsync(payload.key);
    await store.preload?.(payload.key);
    return store.getEncodedSerialized?.(payload.key) ?? store.getSerialized?.(payload.key) ?? null;
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
      store.set(request.key, request.value);
      announce(request.key, event.sender);
    } catch (error) {
      if (error instanceof IpcValidationError) {
        sendValidationError(event, payload, error);
        return;
      }
      throw error;
    }
  });

  ipcMain.on(IPC.storeDelete, (event, payload: unknown) => {
    if (!ctx.isTrustedSender(event)) return;
    try {
      if (!isRecord(payload) || typeof payload['key'] !== 'string') {
        throw new IpcValidationError(IPC.storeDelete, 'expected { key }');
      }
      const request = payload as unknown as StoreDeleteRequest;
      store.delete(request.key);
      announce(request.key, event.sender);
    } catch (error) {
      if (error instanceof IpcValidationError) {
        sendValidationError(event, payload, error);
        return;
      }
      throw error;
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
      const value = store.get ? store.get(payload['key'] as string) : store.snapshot()[payload['key'] as string];
      event.returnValue = value === undefined ? null : JSON.stringify(value);
    }
  });

  ipcMain.handle(IPC.storeFlush, async (event) => {
    requireTrusted(event, IPC.storeFlush);
    await store.flushAll();
  });
}

function sendValidationError(
  event: IpcMainEvent,
  payload: unknown,
  error: IpcValidationError
): void {
  const key = isRecord(payload) && typeof payload['key'] === 'string' ? payload['key'] : '';
  event.sender.send(IPC.storeError, { key, error: error.message } satisfies StoreErrorEvent);
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

function isErrorCode(error: unknown, code: string): boolean {
  return isRecord(error) && error['code'] === code;
}
