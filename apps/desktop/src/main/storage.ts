import { randomBytes } from 'node:crypto';
import * as nodeFs from 'node:fs/promises';
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

const WRITE_DELAY_MS = 150;
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
  set(key: string, value: unknown): void;
  setSerialized?(key: string, serialized: string): void;
  delete(key: string): void;
  flushAll(): Promise<void>;
  onError(cb: (event: StoreErrorEvent) => void): () => void;
}

/** Test seams are optional; normal callers only pass the store directory. */
export interface StoreOptions {
  fs?: StoreFileSystem;
  writeDelayMs?: number;
  now?: () => number;
  randomSuffix?: () => string;
}

type PendingOperation =
  | { kind: 'set'; serialized: string }
  | { kind: 'delete' };

class FileStore implements Store {
  private readonly values = new Map<string, unknown>();
  private readonly listeners = new Set<(event: StoreErrorEvent) => void>();
  private readonly pending = new Map<string, PendingOperation>();
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();
  private readonly queues = new Map<string, Promise<void>>();

  constructor(
    private readonly directory: string,
    private readonly fs: StoreFileSystem,
    private readonly writeDelayMs: number,
    private readonly now: () => number,
    private readonly randomSuffix: () => string
  ) {}

  async load(): Promise<void> {
    await this.fs.mkdir(this.directory, { recursive: true });
    await this.removeStaleTemporaryFiles();

    const names = await this.fs.readdir(this.directory);
    const loadedProjectIds = new Set<string>();
    this.values.clear();

    for (const name of names) {
      if (!name.endsWith('.json')) continue;

      const candidateKey = name.slice(0, -'.json'.length);
      const parsedKey = parseStoreKey(candidateKey);
      // Reconstructing the name prevents aliases and unchecked filenames from
      // ever reaching a filesystem path.
      if (parsedKey === null || storeFileName(parsedKey) !== name) continue;

      const filePath = path.join(this.directory, name);
      let serialized: string;
      try {
        serialized = await this.fs.readFile(filePath, 'utf8');
      } catch (error) {
        this.emitError(candidateKey, `store file could not be read: ${errorText(error)}`);
        continue;
      }

      try {
        const value: unknown = JSON.parse(serialized);
        this.values.set(candidateKey, value);
        if (parsedKey.kind === 'dynamic' && parsedKey.prefix === 'project') {
          loadedProjectIds.add(parsedKey.id);
        }
      } catch (error) {
        await this.quarantine(name, candidateKey, error);
      }
    }

    this.reportProjectReconciliation(loadedProjectIds);
  }

  snapshot(): StoreSnapshot {
    return structuredClone(Object.fromEntries(this.values));
  }

  set(key: string, value: unknown): void {
    this.fileNameFor(key, IPC.storeSet);

    // Old renderers send session metadata without history. Such a partial
    // update must never erase the last recoverable undo stack.
    const previous = this.values.get(key);
    if (key.startsWith('projectState.') && isRecord(value) && !Object.hasOwn(value, 'history') && isRecord(previous) && previous.history !== undefined) {
      value = { ...value, history: previous.history };
    }
    let cloned: unknown;
    let serialized: string | undefined;
    try {
      cloned = structuredClone(value);
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

    this.values.set(key, cloned);
    this.schedule(key, { kind: 'set', serialized });
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
    this.values.set(key, value);
    this.schedule(key, { kind: 'set', serialized });
  }

  delete(key: string): void {
    this.fileNameFor(key, IPC.storeDelete);
    this.values.delete(key);
    this.schedule(key, { kind: 'delete' });
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
      .then(() => this.persist(key, operation))
      .catch((error: unknown) => {
        this.emitError(key, errorText(error));
      });
    this.queues.set(key, current);
    void current.finally(() => {
      if (this.queues.get(key) === current) this.queues.delete(key);
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
    options.randomSuffix ?? (() => `${process.pid}-${randomBytes(8).toString('hex')}`)
  );
}

type StoreIpcEvent = IpcMainEvent | IpcMainInvokeEvent;

export function registerStoreIpc(
  ipcMain: Pick<IpcMain, 'handle' | 'on'>,
  store: Store,
  ctx: { isTrustedSender(event: StoreIpcEvent): boolean }
): void {
  const requireTrusted = (event: StoreIpcEvent, channel: string): void => {
    if (!ctx.isTrustedSender(event)) throw new IpcValidationError(channel, 'untrusted sender');
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
    } catch (error) {
      if (error instanceof IpcValidationError) {
        sendValidationError(event, payload, error);
        return;
      }
      throw error;
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
