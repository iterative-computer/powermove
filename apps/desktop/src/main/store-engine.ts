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
 */
import path from 'node:path';

import { isRecord } from '../shared/guards';
import { HistoryRecords, isHistoryDictionary, packStoredHistory } from '../shared/history-memory';
import { IPC, LIMITS } from '../shared/ipc';
import { parseStoreKey, storeFileName } from '../shared/store-keys';
import type { StoreFileSystem } from './storage';

// Takes retain up to 24 complete editable snapshots; large tracked projects
// can exceed the ordinary settings limit without being corrupt.
const MAX_TAKES_BYTES = 24 * LIMITS.storeValueBytes;

/** Main to engine. `id` orders replies; it increases for the store's lifetime. */
export type StoreRequest =
  | { op: 'init'; id: number; directory: string; writeDelayMs: number }
  | { op: 'load'; id: number }
  | { op: 'set'; id: number; key: string; value: unknown }
  | { op: 'setSerialized'; id: number; key: string; serialized: string }
  | { op: 'delete'; id: number; key: string }
  /** Re-persist bytes main already published, after a replacement engine starts. */
  | { op: 'restore'; id: number; key: string; bytes: Uint8Array | null }
  | { op: 'flush'; id: number };

/** Engine to main. */
export type StoreReply =
  | { type: 'ready'; id: number }
  | { type: 'loaded'; id: number; entries: Array<[string, Uint8Array]> }
  /** The value is valid and is the key's newest value; bytes are its encoded JSON (null: deleted). */
  | { type: 'committed'; id: number; key: string; bytes: Uint8Array | null }
  | { type: 'rejected'; id: number; key: string; channel: string; detail: string }
  /** The committed operation `id` reached disk. */
  | { type: 'written'; id: number; key: string }
  | { type: 'flushed'; id: number }
  | { type: 'failed'; id: number; error: string }
  /** `unsaved`: a write the engine committed did not reach disk. */
  | { type: 'error'; key: string; error: string; unsaved?: true };

export interface StoreEngineOptions {
  fs: StoreFileSystem;
  now: () => number;
  randomSuffix: () => string;
}

type PendingWrite =
  | { id: number; kind: 'set'; data: string | Uint8Array; checkpoint?: Checkpoint }
  | { id: number; kind: 'delete' };

/* What a project write says about checkpoint ids (projects.ts): a slot's own
   id (null: an envelope without one), or the ids a journal's entries are
   tagged with. */
type Checkpoint = { slot: string | null } | { journal: string[] };

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
     metadata-only update keeps the stored undo history. Load fills it from
     disk; afterwards every key is either here or has no file. Only an engine
     that has not loaded (a replacement after a worker fault) reads a session
     back from disk, asynchronously. */
  private readonly sessions = new Map<string, string | null>();
  private loaded = false;
  /** projectState keys whose file existed at load but could not be read. */
  private readonly unread = new Set<string>();
  /** Per project.<id> slot, the newest write's request id and whether it reached disk (null: not yet). */
  private readonly slotWrites = new Map<string, { id: number; ok: boolean | null }>();
  /** Per project.<id> slot, the checkpoint id of the slot on disk (null: none; absent: unknown). */
  private readonly diskCheckpoints = new Map<string, string | null>();
  private readonly backlog: StoreRequest[] = [];
  private busy = false;
  private directory = '';
  private writeDelayMs = 150;

  constructor(private readonly options: StoreEngineOptions, private readonly reply: (message: StoreReply, transfer?: ArrayBuffer[]) => void) {}

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
          entries => this.reply({ type: 'loaded', id: request.id, entries }, entries.map(([, bytes]) => bytes.buffer as ArrayBuffer)),
          error => this.reply({ type: 'failed', id: request.id, error: errorText(error) })
        );
      case 'set':
        return this.encode(request.id, request.key, () => this.encodeValue(request.key, request.value));
      case 'setSerialized':
        return this.encode(request.id, request.key, () => this.encodeSerialized(request.key, request.serialized));
      case 'delete':
        if (request.key.startsWith('projectState.')) this.sessions.set(request.key, null);
        if (request.key.startsWith('project.')) this.noteSlotWrite(request.key, request.id, null);
        this.reply({ type: 'committed', id: request.id, key: request.key, bytes: null });
        this.schedule(request.key, { id: request.id, kind: 'delete' });
        return undefined;
      case 'restore': {
        if (request.key.startsWith('projectState.')) this.sessions.set(request.key, request.bytes && Buffer.from(request.bytes).toString('utf8'));
        if (request.key.startsWith('project.')) this.noteSlotWrite(request.key, request.id, null);
        // A slot's checkpoint id is at its head; a journal is small.
        const text = request.bytes && (request.key.startsWith('project.') ? Buffer.from(request.bytes.subarray(0, 512)).toString('utf8')
          : request.key.startsWith('projectJournal.') ? Buffer.from(request.bytes).toString('utf8') : null);
        this.schedule(request.key, request.bytes ? { id: request.id, kind: 'set', data: request.bytes, ...(text ? { checkpoint: checkpointOf(request.key, text) } : {}) } : { id: request.id, kind: 'delete' });
        return undefined;
      }
      case 'flush':
        // Later requests keep encoding while earlier writes finish.
        void this.flushAll().then(() => this.reply({ type: 'flushed', id: request.id }));
        return undefined;
    }
  }

  /** Encodes synchronously unless a legacy session needs its stored history read first. */
  private encode(id: number, key: string, produce: () => string | Promise<string>): Promise<void> | undefined {
    const publish = (serialized: string) => {
      if (key.startsWith('projectState.')) this.sessions.set(key, serialized);
      const bytes = encoder.encode(serialized);
      this.reply({ type: 'committed', id, key, bytes }, [bytes.buffer as ArrayBuffer]);
      if (key.startsWith('project.')) this.noteSlotWrite(key, id, null);
      this.schedule(key, { id, kind: 'set', data: serialized, checkpoint: checkpointOf(key, serialized) });
    };
    const refuse = (error: unknown) => {
      const rejection = error instanceof Rejection ? error : new Rejection(IPC.storeSet, errorText(error));
      // The window deletes the slot's journal right after posting it.
      if (key.startsWith('project.')) this.noteSlotWrite(key, id, false);
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
      if (this.loaded && !this.unread.has(key)) return this.pack(key, value);
      return this.readStored(key).then(stored => this.pack(key, withHistory(value, stored)));
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

  private async readStored(key: string): Promise<string | null> {
    try {
      return await this.options.fs.readFile(path.join(this.directory, this.fileNameFor(key)), 'utf8');
    } catch (error) {
      if (isErrorCode(error, 'ENOENT')) return null;
      throw error;
    }
  }

  private async load(): Promise<Array<[string, Uint8Array]>> {
    const { fs } = this.options;
    await fs.mkdir(this.directory, { recursive: true });
    await this.removeStaleTemporaryFiles();

    const names = await fs.readdir(this.directory);
    const entries: Array<[string, Uint8Array]> = [];
    const loadedProjectIds = new Set<string>();
    let registry: unknown;
    // Mirrors main, which replaces everything it held with what is on disk.
    this.sessions.clear();
    this.unread.clear();

    for (const name of names) {
      if (!name.endsWith('.json')) continue;

      const candidateKey = name.slice(0, -'.json'.length);
      const parsedKey = parseStoreKey(candidateKey);
      // Reconstructing the name prevents aliases and unchecked filenames from
      // ever reaching a filesystem path.
      if (parsedKey === null || storeFileName(parsedKey) !== name) continue;

      let serialized: string;
      try {
        serialized = await fs.readFile(path.join(this.directory, name), 'utf8');
      } catch (error) {
        this.emitError(candidateKey, `store file could not be read: ${errorText(error)}`);
        if (candidateKey.startsWith('projectState.')) this.unread.add(candidateKey);
        continue;
      }

      try {
        const value: unknown = JSON.parse(serialized);
        const packed = packStored(candidateKey, value);
        const before = candidateKey.startsWith('projectState.') && isRecord(value) ? value.history : value;
        const after = candidateKey.startsWith('projectState.') && isRecord(packed) ? packed.history : packed;
        // Finish lossless migration through the normal atomic writer. Otherwise
        // inactive legacy histories re-create the same huge parse peak each boot.
        if (isHistoryDictionary(after) && !isHistoryDictionary(before)) {
          serialized = JSON.stringify(packed);
          this.schedule(candidateKey, { id: 0, kind: 'set', data: serialized });
        }
        if (candidateKey.startsWith('projectState.')) this.sessions.set(candidateKey, serialized);
        if (candidateKey.startsWith('project.')) this.diskCheckpoints.set(candidateKey, isRecord(value) && typeof value.cp === 'string' ? value.cp : null);
        entries.push([candidateKey, encoder.encode(serialized)]);
        if (candidateKey === 'projects') registry = value;
        if (parsedKey.kind === 'dynamic' && parsedKey.prefix === 'project') loadedProjectIds.add(parsedKey.id);
      } catch (error) {
        await this.quarantine(name, candidateKey, error);
      }
    }

    this.reportProjectReconciliation(registry, loadedProjectIds);
    this.loaded = true;
    return entries;
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

    let previous = this.queues.get(key) ?? Promise.resolve();
    /* A project's recovery journal is relative to its slot: deleting it after
       a checkpoint, or writing entries made after one, must reach disk only
       once that checkpoint has. Otherwise a crash in between (the slot is a
       large write, the journal a small one) recovered the older slot with no
       journal, or with entries meant for the newer one. */
    const slot = key.startsWith('projectJournal.') ? `project.${key.slice('projectJournal.'.length)}` : null;
    if (slot) {
      if (this.pending.has(slot)) this.enqueuePending(slot);
      const checkpoint = this.queues.get(slot);
      if (checkpoint) { const own = previous; previous = Promise.all([own, checkpoint]).then(() => undefined); }
    }
    const isSlot = key.startsWith('project.');
    const current = previous
      .then(async () => {
        if (slot && !this.journalApplies(slot, operation)) return false;
        await this.persist(key, operation);
        if (isSlot) {
          this.settleSlotWrite(key, operation.id, true);
          this.diskCheckpoints.set(key, operation.kind === 'set' && operation.checkpoint && 'slot' in operation.checkpoint ? operation.checkpoint.slot : null);
        }
        return true;
      })
      .then(persisted => { if (persisted && operation.id) this.reply({ type: 'written', id: operation.id, key }); })
      .catch((error: unknown) => {
        if (isSlot) this.settleSlotWrite(key, operation.id, false);
        this.reply({ type: 'error', key, error: errorText(error), unsaved: true });
      });
    this.queues.set(key, current);
    void current.finally(() => {
      if (this.queues.get(key) === current) this.queues.delete(key);
    });
  }

  private noteSlotWrite(key: string, id: number, ok: boolean | null): void {
    const newest = this.slotWrites.get(key);
    if (!newest || id >= newest.id) this.slotWrites.set(key, { id, ok });
  }

  /** An earlier slot write settling never changes what is known of a newer one. */
  private settleSlotWrite(key: string, id: number, ok: boolean): void {
    const newest = this.slotWrites.get(key);
    if (newest && newest.id === id) newest.ok = ok;
  }

  /*
   * Whether a journal operation describes the slot on disk. It runs only after
   * every slot write posted before it has settled (enqueuePending), so:
   * - a deletion follows a checkpoint, and goes ahead only if that checkpoint,
   *   the newest slot write posted before it, reached disk. When it was
   *   refused or failed, the journal on disk is the only record of the edits
   *   since the older slot and stays. A failure is cleared only by a newer
   *   slot write reaching disk, never by an older one still in flight
   *   finishing after it.
   * - entries tagged with checkpoint ids (projects.ts) are written only when
   *   every tag names the slot on disk: entries meant for a slot that never
   *   landed would replace the journal of the one that did. The window writes
   *   them again, against the older slot, once it hears of the failure.
   *   Untagged entries (older builds) and an unknown slot (a replacement
   *   engine that has not loaded) are written, as before.
   */
  private journalApplies(slot: string, operation: PendingWrite): boolean {
    if (operation.kind === 'delete') {
      const newest = this.slotWrites.get(slot);
      return !newest || newest.id > operation.id || newest.ok !== false;
    }
    const tags = operation.checkpoint && 'journal' in operation.checkpoint ? operation.checkpoint.journal : [];
    if (!tags.length || !this.diskCheckpoints.has(slot)) return true;
    const disk = this.diskCheckpoints.get(slot);
    return tags.every(tag => tag === disk);
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

/** The checkpoint ids a project slot or journal write carries (see Checkpoint). */
function checkpointOf(key: string, serialized: string): Checkpoint | undefined {
  if (key.startsWith('project.')) {
    // The envelope is {"v":…,"cp":…,"proj":…}; reading its head avoids a second parse of the slot.
    const head = /^\{"v":(?:"(?:[^"\\]|\\.)*"|[^,{}"]*),"cp":"((?:[^"\\]|\\.)*)"/.exec(serialized.slice(0, 512));
    return { slot: head ? JSON.parse(`"${head[1]}"`) as string : null };
  }
  if (!key.startsWith('projectJournal.')) return undefined;
  try {
    const entries: unknown = JSON.parse(serialized);
    if (!Array.isArray(entries)) return undefined;
    return { journal: [...new Set(entries.flatMap(entry => isRecord(entry) && typeof entry.cp === 'string' ? [entry.cp] : []))] };
  } catch { return undefined; }
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
