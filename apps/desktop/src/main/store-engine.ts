/*
 * The store's serialization and persistence half. In the desktop app it runs
 * on the persistence worker thread (persistence-worker.ts), so parsing,
 * history packing, JSON.stringify, and file writes of multi-megabyte project
 * documents and undo histories never stall main's JavaScript thread. Main
 * keeps only the encoded bytes for its synchronous reads (storage.ts).
 *
 * Every request is answered in the order it arrived. A value is validated and
 * encoded before main publishes it; the disk write follows after the debounce,
 * through the same atomic sequence as before: write a sibling temporary file,
 * fsync it, close it, then rename it over the destination.
 *
 * Heavy keys (projects, histories, sessions, journals, agent threads and
 * attachments) are not read at load: load lists them, and main asks for one
 * with `read` when something needs it. A corrupt heavy file is quarantined on
 * that first read, not at startup.
 */
import * as nodeSyncFs from 'node:fs';
import path from 'node:path';

import { isRecord } from '../shared/guards';
import { HistoryRecords, isHistoryDictionary, packStoredHistory } from '../shared/history-memory';
import { IPC, LIMITS } from '../shared/ipc';
import { parseStoreKey, storeFileName } from '../shared/store-keys';
import type { StoreFileSystem } from './storage';

// Takes retain up to 24 complete editable snapshots; large tracked projects
// can exceed the ordinary settings limit without being corrupt.
const MAX_TAKES_BYTES = 24 * LIMITS.storeValueBytes;

/** Keys load lists by name only; main reads them on demand (storage.ts). */
export const HEAVY_KEY = /^(project|projectHistory|projectState|projectJournal|agentThreads|agentAttachment)\./;

/** Main to engine. `id` orders replies; it increases for the store's lifetime. */
export type StoreRequest =
  | { op: 'init'; id: number; directory: string; writeDelayMs: number }
  | { op: 'load'; id: number }
  | { op: 'set'; id: number; key: string; value: unknown }
  | { op: 'setSerialized'; id: number; key: string; serialized: string }
  | { op: 'delete'; id: number; key: string }
  /** Re-persist bytes main already published, after a replacement engine starts. */
  | { op: 'restore'; id: number; key: string; bytes: Uint8Array | null }
  /** Read a stored key: `sync` blocks this thread on the disk (main is waiting). */
  | { op: 'read'; id: number; key: string; sync: boolean }
  /** Main dropped a clean key from its cache; forget any copy kept here too. */
  | { op: 'evict'; key: string }
  | { op: 'flush'; id: number };

/** Engine to main. */
export type StoreReply =
  | { type: 'ready'; id: number }
  /** `lazy`: heavy keys present on disk, listed without reading them. */
  | { type: 'loaded'; id: number; entries: Array<[string, Uint8Array]>; lazy: string[] }
  /** A `read`: its encoded JSON, or why there is none ('missing': absent or quarantined). */
  | { type: 'read'; id: number; key: string; state: 'ok' | 'missing' | 'failed'; bytes: Uint8Array | null }
  /** The value is valid and is the key's newest value; bytes are its encoded JSON (null: deleted). */
  | { type: 'committed'; id: number; key: string; bytes: Uint8Array | null }
  | { type: 'rejected'; id: number; key: string; channel: string; detail: string }
  /** The committed operation `id` reached disk. */
  | { type: 'written'; id: number; key: string }
  | { type: 'flushed'; id: number }
  | { type: 'failed'; id: number; error: string }
  | { type: 'error'; key: string; error: string };

export interface StoreEngineOptions {
  fs: StoreFileSystem;
  /** Synchronous reads answer main while it waits, and quarantine without a gap. */
  syncFs?: Pick<typeof nodeSyncFs, 'readFileSync' | 'renameSync'>;
  now: () => number;
  randomSuffix: () => string;
}

type PendingWrite =
  | { id: number; kind: 'set'; data: string | Uint8Array }
  | { id: number; kind: 'delete' };

class Rejection extends Error {
  constructor(readonly channel: string, readonly detail: string) { super(detail); }
}

export function storeByteLimit(key: string): number {
  if (key === 'takes') return MAX_TAKES_BYTES;
  const parsed = parseStoreKey(key);
  // Project snapshots and sessions (including undo/redo history) must support
  // the same size as project files, including during autosave and save completion.
  const large = parsed?.kind === 'dynamic'
    && (parsed.prefix === 'project' || parsed.prefix === 'projectState' || parsed.prefix === 'projectHistory'
      /* A document host worker's undo history (docs/engine/document-host.md). */
      || parsed.prefix === 'documentHost');
  return large ? LIMITS.fileSaveBytes : LIMITS.storeValueBytes;
}

const encoder = new TextEncoder();

export class StoreEngine {
  private readonly pending = new Map<string, PendingWrite>();
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();
  private readonly queues = new Map<string, Promise<void>>();
  /* The newest encoded session per projectState key (null: none), so a
     metadata-only update keeps the stored undo history. Sessions are heavy,
     so load reads none: a key missing here is unknown, not empty, and a
     metadata-only update reads its stored history from disk first. */
  private readonly sessions = new Map<string, string | null>();
  /** Bumped by every set, delete and restore, so an older disk read cannot win. */
  private readonly generations = new Map<string, number>();
  private readonly syncFs: Pick<typeof nodeSyncFs, 'readFileSync' | 'renameSync'>;
  private readonly backlog: StoreRequest[] = [];
  private busy = false;
  private directory = '';
  private writeDelayMs = 150;

  constructor(private readonly options: StoreEngineOptions, private readonly reply: (message: StoreReply, transfer?: ArrayBuffer[]) => void) {
    this.syncFs = options.syncFs ?? nodeSyncFs;
  }

  /** Requests run in arrival order; most finish synchronously inside this call. */
  handle(request: StoreRequest): void {
    if (this.busy) { this.backlog.push(request); return; }
    const running = this.run(request);
    if (!running) return;
    this.busy = true;
    void running.finally(() => {
      this.busy = false;
      while (!this.busy && this.backlog.length) this.handle(this.backlog.shift()!);
    });
  }

  private run(request: StoreRequest): Promise<void> | undefined {
    switch (request.op) {
      case 'init':
        this.directory = request.directory;
        this.writeDelayMs = request.writeDelayMs;
        this.reply({ type: 'ready', id: request.id });
        return undefined;
      case 'load':
        return this.load().then(
          ({ entries, lazy }) => this.reply({ type: 'loaded', id: request.id, entries, lazy }, entries.map(([, bytes]) => bytes.buffer as ArrayBuffer)),
          error => this.reply({ type: 'failed', id: request.id, error: errorText(error) })
        );
      case 'set':
        return this.encode(request.id, request.key, () => this.encodeValue(request.key, request.value));
      case 'setSerialized':
        return this.encode(request.id, request.key, () => this.encodeSerialized(request.key, request.serialized));
      case 'delete':
        this.bump(request.key);
        if (request.key.startsWith('projectState.')) this.sessions.set(request.key, null);
        this.reply({ type: 'committed', id: request.id, key: request.key, bytes: null });
        this.schedule(request.key, { id: request.id, kind: 'delete' });
        return undefined;
      case 'restore':
        this.bump(request.key);
        if (request.key.startsWith('projectState.')) this.sessions.set(request.key, request.bytes && Buffer.from(request.bytes).toString('utf8'));
        this.schedule(request.key, request.bytes ? { id: request.id, kind: 'set', data: request.bytes } : { id: request.id, kind: 'delete' });
        return undefined;
      case 'read':
        // An asynchronous read does not hold up later requests; a set or
        // delete that overtakes it bumps the generation and wins.
        if (request.sync) this.readNow(request.id, request.key);
        else void this.readLater(request.id, request.key);
        return undefined;
      case 'evict':
        if (request.key.startsWith('projectState.')) this.sessions.delete(request.key);
        return undefined;
      case 'flush':
        // Later requests keep encoding while earlier writes finish.
        void this.flushAll().then(() => this.reply({ type: 'flushed', id: request.id }));
        return undefined;
    }
  }

  /** Encodes synchronously; a metadata-only session update reads its stored history synchronously first. */
  private encode(id: number, key: string, produce: () => string | Promise<string>): Promise<void> | undefined {
    const publish = (serialized: string) => {
      this.bump(key);
      if (key.startsWith('projectState.')) this.sessions.set(key, serialized);
      const bytes = encoder.encode(serialized);
      this.reply({ type: 'committed', id, key, bytes }, [bytes.buffer as ArrayBuffer]);
      this.schedule(key, { id, kind: 'set', data: serialized });
    };
    const refuse = (error: unknown) => {
      const rejection = error instanceof Rejection ? error : new Rejection(IPC.storeSet, errorText(error));
      this.reply({ type: 'rejected', id, key, channel: rejection.channel, detail: rejection.detail });
    };
    try {
      this.fileNameFor(key);
      const produced = produce();
      if (typeof produced === 'string') { publish(produced); return undefined; }
      return produced.then(publish, refuse);
    } catch (error) {
      refuse(error);
      return undefined;
    }
  }

  private encodeValue(key: string, value: unknown): string | Promise<string> {
    // Old renderers send session metadata without history. Such a partial
    // update must never erase the last recoverable undo stack.
    if (key.startsWith('projectState.') && isRecord(value) && !Object.hasOwn(value, 'history')) {
      const known = this.sessions.get(key);
      if (known !== undefined) return this.pack(key, withHistory(value, known));
      // Not read yet (or evicted): its stored history comes from disk first.
      // A file that cannot be read refuses the write rather than drop history.
      return this.pack(key, withHistory(value, this.readStoredSync(key)));
    }
    return this.pack(key, value);
  }

  private pack(key: string, value: unknown): string {
    let serialized: string | undefined;
    try {
      serialized = JSON.stringify(packStored(key, value));
    } catch (error) {
      throw new Rejection(IPC.storeSet, `value is not JSON-serialisable: ${errorText(error)}`);
    }
    if (serialized === undefined) throw new Rejection(IPC.storeSet, 'value is not JSON-serialisable');
    const byteLength = Buffer.byteLength(serialized, 'utf8');
    const byteLimit = storeByteLimit(key);
    if (byteLength > byteLimit) throw new Rejection(IPC.storeSet, `serialised value is ${byteLength} bytes (maximum ${byteLimit})`);
    return serialized;
  }

  private encodeSerialized(key: string, serialized: string): string | Promise<string> {
    if (Buffer.byteLength(serialized, 'utf8') > storeByteLimit(key)) throw new Rejection(IPC.storeSetSerialized, 'serialized value exceeds its size limit');
    let value: unknown;
    try { value = JSON.parse(serialized); }
    catch { throw new Rejection(IPC.storeSetSerialized, 'invalid JSON'); }
    if (key.startsWith('projectState.')) return this.encodeValue(key, value);
    const packed = packStored(key, value);
    return packed === value ? serialized : JSON.stringify(packed);
  }

  private readStoredSync(key: string): string | null {
    try {
      return this.syncFs.readFileSync(path.join(this.directory, this.fileNameFor(key)), 'utf8');
    } catch (error) {
      if (isErrorCode(error, 'ENOENT')) return null;
      throw new Rejection(IPC.storeSet, `store file could not be read: ${errorText(error)}`);
    }
  }

  private bump(key: string): void { this.generations.set(key, (this.generations.get(key) ?? 0) + 1); }

  private readNow(id: number, key: string): void {
    const generation = this.generations.get(key);
    let serialized: string;
    try { serialized = this.syncFs.readFileSync(path.join(this.directory, this.fileNameFor(key)), 'utf8'); }
    catch (error) { this.readFailed(id, key, error); return; }
    this.accept(id, key, serialized, generation);
  }

  private async readLater(id: number, key: string): Promise<void> {
    const generation = this.generations.get(key);
    let serialized: string;
    try { serialized = await this.options.fs.readFile(path.join(this.directory, this.fileNameFor(key)), 'utf8'); }
    catch (error) {
      if (this.generations.get(key) !== generation) this.reply({ type: 'read', id, key, state: 'failed', bytes: null });
      else this.readFailed(id, key, error);
      return;
    }
    this.accept(id, key, serialized, generation);
  }

  private readFailed(id: number, key: string, error: unknown): void {
    if (isErrorCode(error, 'ENOENT')) { this.reply({ type: 'read', id, key, state: 'missing', bytes: null }); return; }
    if (!(error instanceof Rejection)) this.emitError(key, `store file could not be read: ${errorText(error)}`);
    this.reply({ type: 'read', id, key, state: 'failed', bytes: null });
  }

  /** Publishes a value read from disk unless a newer set, delete or restore overtook it. */
  private accept(id: number, key: string, serialized: string, generation: number | undefined): void {
    // Main ignores a superseded read too; here it must not touch the session or the file.
    if (this.generations.get(key) !== generation) { this.reply({ type: 'read', id, key, state: 'failed', bytes: null }); return; }
    let encoded: string;
    try {
      encoded = this.migrate(key, serialized);
    } catch (error) {
      this.quarantineNow(key, error);
      this.reply({ type: 'read', id, key, state: 'missing', bytes: null });
      return;
    }
    if (key.startsWith('projectState.')) this.sessions.set(key, encoded);
    const bytes = encoder.encode(encoded);
    this.reply({ type: 'read', id, key, state: 'ok', bytes }, [bytes.buffer as ArrayBuffer]);
  }

  /** Parses a stored value; a legacy history is packed and rewritten through the atomic writer. */
  private migrate(key: string, serialized: string): string {
    const value: unknown = JSON.parse(serialized);
    const packed = packStored(key, value);
    const before = key.startsWith('projectState.') && isRecord(value) ? value.history : value;
    const after = key.startsWith('projectState.') && isRecord(packed) ? packed.history : packed;
    // Finish lossless migration through the normal atomic writer. Otherwise
    // inactive legacy histories re-create the same huge parse peak each boot.
    if (isHistoryDictionary(after) && !isHistoryDictionary(before)) {
      const migrated = JSON.stringify(packed);
      this.schedule(key, { id: 0, kind: 'set', data: migrated });
      return migrated;
    }
    return serialized;
  }

  /** No await between finding the corruption and moving the file: a new write cannot slip in and be quarantined instead. */
  private quarantineNow(key: string, cause: unknown): void {
    this.bump(key);
    if (key.startsWith('projectState.')) this.sessions.set(key, null);
    const name = this.fileNameFor(key);
    const destination = `${name}.corrupt-${this.options.now()}`;
    let detail = `invalid store file quarantined as ${destination}: ${errorText(cause)}`;
    try { this.syncFs.renameSync(path.join(this.directory, name), path.join(this.directory, destination)); }
    catch (error) { detail = `invalid store file could not be quarantined: ${errorText(cause)}; ${errorText(error)}`; }
    this.emitError(key, detail);
  }

  private async load(): Promise<{ entries: Array<[string, Uint8Array]>; lazy: string[] }> {
    const { fs } = this.options;
    await fs.mkdir(this.directory, { recursive: true });
    await this.removeStaleTemporaryFiles();

    const names = await fs.readdir(this.directory);
    const entries: Array<[string, Uint8Array]> = [];
    const lazy: string[] = [];
    const loadedProjectIds = new Set<string>();
    let registry: unknown;
    // Mirrors main, which replaces everything it held with what is on disk.
    this.sessions.clear();

    for (const name of names) {
      if (!name.endsWith('.json')) continue;

      const candidateKey = name.slice(0, -'.json'.length);
      const parsedKey = parseStoreKey(candidateKey);
      // Reconstructing the name prevents aliases and unchecked filenames from
      // ever reaching a filesystem path.
      if (parsedKey === null || storeFileName(parsedKey) !== name) continue;
      if (parsedKey.kind === 'dynamic' && parsedKey.prefix === 'project') loadedProjectIds.add(parsedKey.id);
      if (HEAVY_KEY.test(candidateKey)) { lazy.push(candidateKey); continue; }

      let serialized: string;
      try {
        serialized = await fs.readFile(path.join(this.directory, name), 'utf8');
      } catch (error) {
        this.emitError(candidateKey, `store file could not be read: ${errorText(error)}`);
        continue;
      }

      try {
        serialized = this.migrate(candidateKey, serialized);
        entries.push([candidateKey, encoder.encode(serialized)]);
        if (candidateKey === 'projects') registry = JSON.parse(serialized);
      } catch (error) {
        await this.quarantine(name, candidateKey, error);
      }
    }

    this.reportProjectReconciliation(registry, loadedProjectIds);
    return { entries, lazy };
  }

  async flushAll(): Promise<void> {
    // Scheduling a pending operation appends it to that key's existing chain.
    for (const key of [...this.pending.keys()]) this.enqueuePending(key);

    // Writes can finish while this array is assembled, but every promise here
    // represents the tail of its key's queue at the flush boundary.
    await Promise.all([...this.queues.values()]);
  }

  private schedule(key: string, operation: PendingWrite): void {
    this.pending.set(key, operation);
    const previousTimer = this.timers.get(key);
    if (previousTimer !== undefined) clearTimeout(previousTimer);
    this.timers.set(key, setTimeout(() => this.enqueuePending(key), this.writeDelayMs));
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
      .then(() => { if (operation.id) this.reply({ type: 'written', id: operation.id, key }); })
      .catch((error: unknown) => {
        this.emitError(key, errorText(error));
      });
    this.queues.set(key, current);
    void current.finally(() => {
      if (this.queues.get(key) === current) this.queues.delete(key);
    });
  }

  private async persist(key: string, operation: PendingWrite): Promise<void> {
    const { fs } = this.options;
    const finalPath = path.join(this.directory, this.fileNameFor(key));
    if (operation.kind === 'delete') {
      try {
        await fs.unlink(finalPath);
      } catch (error) {
        if (!isErrorCode(error, 'ENOENT')) throw error;
      }
      return;
    }

    const temporaryPath = `${finalPath}.tmp-${this.options.randomSuffix()}`;
    try {
      // Atomic replacement sequence: write a sibling temporary file, open and
      // fsync that file, close it, then rename it over the destination.
      await fs.writeFile(temporaryPath, operation.data, { encoding: 'utf8', flag: 'wx' });
      const handle = await fs.open(temporaryPath, 'r');
      try {
        await handle.sync();
      } finally {
        await handle.close();
      }
      await fs.rename(temporaryPath, finalPath);
    } catch (error) {
      try {
        await fs.unlink(temporaryPath);
      } catch (cleanupError) {
        if (!isErrorCode(cleanupError, 'ENOENT')) {
          this.emitError(key, `temporary-file cleanup failed: ${errorText(cleanupError)}`);
        }
      }
      throw error;
    }
  }

  private async removeStaleTemporaryFiles(): Promise<void> {
    const names = await this.options.fs.readdir(this.directory);
    await Promise.all(
      names
        .filter((name) => name.includes('.tmp-'))
        .map(async (name) => {
          try {
            await this.options.fs.unlink(path.join(this.directory, name));
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
    const destination = path.join(this.directory, `${name}.corrupt-${this.options.now()}`);
    let detail = `invalid store file quarantined as ${path.basename(destination)}: ${errorText(cause)}`;
    try {
      await this.options.fs.rename(source, destination);
    } catch (error) {
      detail = `invalid store file could not be quarantined: ${errorText(cause)}; ${errorText(error)}`;
    }
    this.emitError(key, detail);
  }

  private reportProjectReconciliation(registry: unknown, projectFileIds: ReadonlySet<string>): void {
    const registryIds = new Set<string>();
    if (Array.isArray(registry)) {
      for (const entry of registry) {
        if (isRecord(entry) && typeof entry['id'] === 'string') registryIds.add(entry['id']);
      }
    }
    for (const id of registryIds) {
      if (!projectFileIds.has(id)) this.emitError(`project.${id}`, 'project registry entry has no corresponding project file');
    }
    for (const id of projectFileIds) {
      if (!registryIds.has(id)) this.emitError(`project.${id}`, 'project file has no corresponding registry entry');
    }
  }

  private fileNameFor(key: string): string {
    const parsed = typeof key === 'string' ? parseStoreKey(key) : null;
    if (parsed === null) throw new Rejection('store:write', `unknown store key: ${String(key)}`);
    return storeFileName(parsed);
  }

  private emitError(key: string, error: string): void {
    this.reply({ type: 'error', key, error });
  }
}

/* Every value arrives here freshly parsed or cloned, so records interned for
   an earlier value can never match a later one: each value is packed with its
   own interning table, which still shares equal records within its history. */
function packStored(key: string, value: unknown): unknown {
  return packStoredHistory(key, value, new HistoryRecords());
}

function withHistory(value: Record<string, unknown>, stored: string | null): unknown {
  if (stored === null) return value;
  let previous: unknown;
  try { previous = JSON.parse(stored); } catch { return value; }
  return isRecord(previous) && previous.history !== undefined ? { ...value, history: previous.history } : value;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function isErrorCode(error: unknown, code: string): boolean {
  return isRecord(error) && error['code'] === code;
}
