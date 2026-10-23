/*
 * SAVE-1: main owns a project's durability (docs/engine/host-processes.md,
 * shared/project-journal.ts). The window only posts, in order, whole-project
 * snapshots (writes of `project.<id>`) and journal entries (appends to
 * `projectJournal.<id>`); this engine decides what reaches disk and when.
 *
 * Per project it keeps:
 * - `seq`: one sequence for slot writes and journal entries, in arrival order;
 * - `durable`: the slot fully on disk (its seq and cp);
 * - `entries`: the journal against that slot, every entry numbered after it.
 *
 * Rules:
 * - A slot write gets the next seq and a new cp, written into its envelope.
 *   Slot writes go to disk one at a time; a newer one not yet started
 *   replaces an older one (its snapshot holds every edit the older held).
 * - Only a slot write this engine completed changes `durable`, and only then
 *   does the journal shrink: to the entries numbered after that slot. A slot
 *   write that fails, or is refused, changes nothing.
 * - An entry gets the next seq and durable's cp, and is appended. The journal
 *   file is rewritten whole (atomically) from `entries`, independently of the
 *   slot writes: whatever slot a crash leaves, the entries after it are on
 *   disk once their journal write has landed.
 * - A journal entry that is not written durably leaves a gap: accepting a
 *   later, smaller one would recover a document that never existed. Main's
 *   FileStore owns that rule (SAVE-1b, storage.ts): any append that does not
 *   end in an acknowledged write closes the project's journal there, and only
 *   a slot posted after the latest such edit, once it has landed, reopens it
 *   (`open`). This engine closes the journal itself the moment it refuses an
 *   entry or cannot read the project for one, so appends already on their way
 *   are refused too, and it reopens only when main says so.
 * - Before the journal shrinks after a slot lands, the slot file and its
 *   directory are synced (one barrier per checkpoint, `barrier`), so a power
 *   loss cannot leave the shrunk journal beside the older slot.
 * - Nothing the window sends can delete or replace the journal: writes and
 *   deletions of `projectJournal.<id>` are refused. Deleting the slot (the
 *   project is deleted) deletes the journal after it.
 * - A replacement engine (after a worker fault) reads a project's slot head and
 *   journal from disk before it touches the project, and main replays what had
 *   not reached disk in its original order (storage.ts).
 */
import path from 'node:path';

import { isRecord } from '../shared/guards';
import { superseded, type JournalEntry } from '../shared/project-journal';

export interface DurabilityFileSystem {
  readFile(path: string, encoding: 'utf8'): Promise<string>;
  open(path: string, flags: string): Promise<{ read(buffer: Uint8Array, offset: number, length: number, position: number): Promise<{ bytesRead: number }>; close(): Promise<void> }>;
}

/** cp null: a bare project, not an envelope (written as before: unnumbered, its journal dropped once it lands). */
type Slot = { id: number; seq: number; cp: string | null; data: string | Uint8Array } | { id: number; seq: number; deleted: true };

type State = {
  seq: number;
  /** The slot on disk (undefined: not read yet; null: none). */
  durable: { seq: number | null; cp: string | null } | null | undefined;
  /** The journal against `durable` (undefined: not read yet). */
  entries: JournalEntry[] | undefined;
  /** The newest slot write not started yet. */
  slot?: Slot;
  slotTask?: Promise<void>;
  journalTask?: Promise<void>;
  /** The journal differs from the file; the request ids its next write acknowledges. */
  journalDirty: boolean;
  acks: number[];
  /** A journal write failed: retried at the next append, slot write or flush, not at once. */
  journalHeld: boolean;
  /** Entries are refused (an earlier one was not written), until main reopens the journal. */
  closed?: boolean;
  timer?: ReturnType<typeof setTimeout>;
  reading?: Promise<void>;
  /** What the running writes will do when they finish (for the model check's state key). */
  writingSlot?: { id: number; seq: number; cp: string | null; deleted: boolean };
  writingAcks?: number[];
};

export interface DurabilityHost {
  fs: DurabilityFileSystem;
  directory(): string;
  writeDelayMs(): number;
  /** Whether this engine read the store directory (otherwise it is a replacement). */
  loaded(): boolean;
  randomId(): string;
  /** Atomic write (null: delete) of a store key's file. */
  persist(key: string, data: string | Uint8Array | null): Promise<void>;
  /** Syncs a landed file and its directory to stable storage (an ordering barrier). */
  barrier(key: string): Promise<void>;

  written(id: number, key: string): void;
  failed(key: string, error: unknown): void;
  later(run: () => void, ms: number): ReturnType<typeof setTimeout>;
  cancel(timer: ReturnType<typeof setTimeout>): void;
}

const SLOT = 'project.', JOURNAL = 'projectJournal.';
const HEAD = /^\{"cp":"((?:[^"\\]|\\.)*)","seq":(\d+),/;
const encoder = new TextEncoder();

export const isSlotKey = (key: string) => key.startsWith(SLOT);
export const isJournalKey = (key: string) => key.startsWith(JOURNAL);
export const projectOf = (key: string) => isSlotKey(key) ? key.slice(SLOT.length) : isJournalKey(key) ? key.slice(JOURNAL.length) : null;

/** A slot envelope's cp and seq, from its head (written by `stamp`). */
export function slotHead(head: string): { seq: number | null; cp: string | null } {
  const match = HEAD.exec(head);
  return match ? { cp: JSON.parse(`"${match[1]}"`) as string, seq: Number(match[2]) } : { seq: null, cp: null };
}

/** The envelope with this write's cp and seq first. */
function stamp(serialized: string, cp: string, seq: number): string {
  const head = `{"cp":${JSON.stringify(cp)},"seq":${seq}`;
  return serialized === '{}' ? `${head}}` : `${head},${serialized.slice(1)}`;
}

export class ProjectDurability {
  private readonly states = new Map<string, State>();

  constructor(private readonly host: DurabilityHost) {}

  /** From the files a load read. */
  loadedSlot(id: string, value: unknown): void {
    const state = this.state(id);
    const seq = isRecord(value) && typeof value.seq === 'number' ? value.seq : null;
    state.durable = { seq, cp: isRecord(value) && typeof value.cp === 'string' ? value.cp : null };
    state.seq = Math.max(state.seq, seq ?? 0);
  }

  loadedJournal(id: string, value: unknown): void {
    const state = this.state(id);
    state.entries = Array.isArray(value) ? value.filter(isRecord) as JournalEntry[] : [];
    for (const entry of state.entries) if (typeof entry.seq === 'number') state.seq = Math.max(state.seq, entry.seq);
  }

  /** After a load: projects with a journal and no slot, or neither, are known too. */
  loadFinished(): void {
    for (const state of this.states.values()) {
      if (state.durable === undefined) state.durable = null;
      if (state.entries === undefined) state.entries = [];
    }
  }

  /** Before a replacement engine touches a project, what is on disk. */
  ready(key: string): Promise<void> | undefined {
    const id = projectOf(key);
    if (id === null || this.host.loaded()) return undefined;
    const state = this.state(id);
    if (state.durable !== undefined && state.entries !== undefined) return undefined;
    // A failed read is tried again by the next request.
    state.reading ??= this.read(id, state).catch(error => { state.reading = undefined; throw error; });
    return state.reading;
  }

  private async read(id: string, state: State): Promise<void> {
    const file = (key: string) => path.join(this.host.directory(), `${key}.json`);
    let durable: State['durable'] = null;
    try {
      const handle = await this.host.fs.open(file(SLOT + id), 'r');
      try {
        const buffer = new Uint8Array(512);
        const { bytesRead } = await handle.read(buffer, 0, buffer.length, 0);
        durable = slotHead(Buffer.from(buffer.subarray(0, bytesRead)).toString('utf8'));
      } finally { await handle.close(); }
    } catch (error) { if (!isMissing(error)) throw error; }
    let entries: JournalEntry[] = [];
    try {
      const parsed: unknown = JSON.parse(await this.host.fs.readFile(file(JOURNAL + id), 'utf8'));
      if (Array.isArray(parsed)) entries = parsed.filter(isRecord) as JournalEntry[];
    } catch (error) { if (!isMissing(error)) throw error; }
    if (state.durable === undefined) state.durable = durable;
    if (state.entries === undefined) state.entries = entries;
    state.seq = Math.max(state.seq, durable?.seq ?? 0, ...entries.map(entry => typeof entry.seq === 'number' ? entry.seq : 0));
  }

  /** A slot write the engine accepted: its bytes as stored (for main's reads). */
  slot(id: number, key: string, serialized: string): Uint8Array {
    const state = this.state(projectOf(key)!);
    const seq = ++state.seq;
    const envelope = serialized.startsWith('{"v":');
    const cp = envelope ? this.host.randomId() : null;
    const data = cp === null ? serialized : stamp(serialized, cp, seq);
    state.slot = { id, seq, cp, data };
    this.schedule(state, projectOf(key)!);
    return encoder.encode(data);
  }

  deleteSlot(id: number, key: string): void {
    const state = this.state(projectOf(key)!);
    state.slot = { id, seq: ++state.seq, deleted: true };
    this.schedule(state, projectOf(key)!);
  }

  /** An entry (one edit's patches): numbered, tagged and appended. Its journal as stored, or a refusal. */
  append(id: number, key: string, entry: unknown, limit: number): Uint8Array {
    const projectId = projectOf(key)!;
    const state = this.state(projectId);
    if (state.closed) throw new Error('the project journal is closed after an entry that was not saved, until the project is saved');
    try { return this.accept(id, projectId, state, entry, limit); }
    catch (error) { state.closed = true; throw error; }
  }

  /** A replay the engine could not apply still numbered its writes: later ones never reuse those numbers. */
  replayedSeq(key: string, bytes: Uint8Array | null): void {
    if (!bytes) return;
    const state = this.state(projectOf(key)!);
    if (isSlotKey(key)) { state.seq = Math.max(state.seq, slotHead(Buffer.from(bytes.subarray(0, 512)).toString('utf8')).seq ?? 0); return; }
    try {
      const parsed: unknown = JSON.parse(Buffer.from(bytes).toString('utf8'));
      if (Array.isArray(parsed)) for (const entry of parsed) if (isRecord(entry) && typeof entry.seq === 'number') state.seq = Math.max(state.seq, entry.seq);
    } catch { /* not a journal: nothing numbered */ }
  }

  /** Main closes a project's journal (an entry it refused or lost), or reopens it (a slot holding every such edit landed). */
  close(key: string): void { this.state(projectOf(key)!).closed = true; }
  open(key: string): void { this.state(projectOf(key)!).closed = false; }
  /** A replacement engine could not read the project for an entry. */
  readFailed(key: string): void { const state = this.state(projectOf(key)!); state.closed = true; }

  private accept(id: number, projectId: string, state: State, serialized: unknown, limit: number): Uint8Array {
    const entry: unknown = typeof serialized === 'string' ? JSON.parse(serialized) : serialized;
    if (!isRecord(entry) || !Array.isArray(entry.patches)) throw new Error('a journal entry needs patches');
    const tagged: JournalEntry = { ...entry, seq: state.seq + 1 };
    if (typeof state.durable?.cp === 'string') tagged.cp = state.durable.cp; else delete tagged.cp;
    const next = [...superseded(state.entries ?? [], entry.patches), tagged];
    const bytes = encoder.encode(JSON.stringify(next));
    if (bytes.byteLength > limit) throw new Error(`the project journal would be ${bytes.byteLength} bytes (maximum ${limit}); it is kept until the project is saved`);
    state.seq++;
    state.entries = next;
    state.journalDirty = true;
    state.journalHeld = false;
    state.acks.push(id);
    this.schedule(state, projectId);
    return bytes;
  }



  /** What main had not seen reach disk before the engine was replaced. */
  restoreSlot(id: number, key: string, bytes: Uint8Array | null): void {
    const state = this.state(projectOf(key)!);
    if (!bytes) { state.slot = { id, seq: ++state.seq, deleted: true }; this.schedule(state, projectOf(key)!); return; }
    const head = slotHead(Buffer.from(bytes.subarray(0, 512)).toString('utf8'));
    if (head.seq === null || head.cp === null) {
      // A bare project (not an envelope) has no number: written again, keeping every entry.
      if (!state.slot) { state.slot = { id, seq: 0, cp: null, data: bytes }; this.schedule(state, projectOf(key)!); }
      return;
    }
    state.seq = Math.max(state.seq, head.seq);
    // Only the newest slot write matters, and an older one must not undo a newer one already on disk.
    if (state.slot && state.slot.seq > head.seq) return;
    if (state.durable && state.durable.seq !== null && state.durable.seq >= head.seq) { this.host.written(id, key); return; }
    state.slot = { id, seq: head.seq, cp: head.cp, data: bytes };
    this.schedule(state, projectOf(key)!);
  }

  restoreJournal(id: number, key: string, bytes: Uint8Array | null): void {
    const state = this.state(projectOf(key)!);
    if (!bytes) return;
    let parsed: unknown;
    try { parsed = JSON.parse(Buffer.from(bytes).toString('utf8')); } catch { return; }
    if (!Array.isArray(parsed)) return;
    // Main's newest journal holds every entry on disk and those that were not yet.
    const restored = (parsed.filter(isRecord) as JournalEntry[]).filter(entry => !this.before(state, entry));
    for (const entry of restored) if (typeof entry.seq === 'number') state.seq = Math.max(state.seq, entry.seq);
    state.entries = restored;
    state.journalDirty = true;
    state.journalHeld = false;
    state.acks.push(id);
    this.schedule(state, projectOf(key)!);
  }

  /** Every pending write started and finished (failures stay reported). */
  async flush(): Promise<void> {
    for (const state of this.states.values()) state.journalHeld = false;
    for (;;) {
      const tasks: Promise<void>[] = [];
      for (const [id, state] of this.states) {
        if (state.timer !== undefined) { this.host.cancel(state.timer); state.timer = undefined; }
        this.start(state, id);
        if (state.slotTask) tasks.push(state.slotTask);
        if (state.journalTask) tasks.push(state.journalTask);
      }
      if (!tasks.length) return;
      await Promise.all(tasks);
    }
  }

  /** Everything that decides what this project's pending work will do (model check). */
  inspect(id: string): unknown {
    const state = this.states.get(id);
    if (!state) return null;
    const slot = state.slot && { id: state.slot.id, seq: state.slot.seq, deleted: 'deleted' in state.slot, cp: 'deleted' in state.slot ? null : state.slot.cp, data: 'deleted' in state.slot ? null : String(typeof state.slot.data === 'string' ? state.slot.data : Buffer.from(state.slot.data).toString('utf8')) };
    return { seq: state.seq, closed: !!state.closed, durable: state.durable, entries: state.entries, slot, journalDirty: state.journalDirty, journalHeld: state.journalHeld, acks: state.acks,
      writingSlot: state.slotTask ? state.writingSlot : null, writingAcks: state.journalTask ? state.writingAcks : null, reading: !!state.reading && (state.durable === undefined || state.entries === undefined) };
  }

  private state(id: string): State {
    let state = this.states.get(id);
    if (!state) {
      state = { seq: 0, durable: this.host.loaded() ? null : undefined, entries: this.host.loaded() ? [] : undefined, journalDirty: false, acks: [], journalHeld: false };
      this.states.set(id, state);
    }
    return state;
  }

  /** Whether an entry is already in the slot on disk. */
  private before(state: State, entry: JournalEntry): boolean {
    const durable = state.durable?.seq;
    return typeof durable === 'number' && (typeof entry.seq !== 'number' || entry.seq <= durable);
  }

  private schedule(state: State, id: string): void {
    if (state.timer !== undefined) this.host.cancel(state.timer);
    state.timer = this.host.later(() => { state.timer = undefined; this.start(state, id); }, this.host.writeDelayMs());
  }

  private start(state: State, id: string): void {
    if (!state.slotTask && state.slot) {
      const slot = state.slot;
      state.slot = undefined;
      state.slotTask = this.writeSlot(state, id, slot).finally(() => {
        state.slotTask = undefined;
        this.start(state, id);
      });
    }
    if (!state.journalTask && state.journalDirty && !state.journalHeld && state.entries !== undefined) {
      state.journalTask = this.writeJournal(state, id).finally(() => {
        state.journalTask = undefined;
        this.start(state, id);
      });
    }
  }

  private async writeSlot(state: State, id: string, slot: Slot): Promise<void> {
    const key = SLOT + id;
    state.writingSlot = { id: slot.id, seq: slot.seq, cp: 'deleted' in slot ? null : slot.cp, deleted: 'deleted' in slot };
    try {
      await this.host.persist(key, 'deleted' in slot ? null : slot.data);
      // The slot is on stable storage before its journal can shrink.
      if (!('deleted' in slot)) await this.host.barrier(key);
    } catch (error) {
      // The slot on disk, and so its journal, stay as they were.
      this.host.failed(key, error);
      return;
    }
    if ('deleted' in slot) {
      state.durable = null;
      if (state.entries !== undefined || this.host.loaded()) { state.entries = []; state.journalDirty = true; state.journalHeld = false; }
    } else if (slot.cp === null) {
      // A bare project: no number on disk, so recovery replays every entry; keep those after it.
      state.durable = { seq: null, cp: null };
      if (state.entries !== undefined) {
        const after = state.entries.filter(entry => typeof entry.seq === 'number' && entry.seq > slot.seq);
        if (after.length !== state.entries.length) { state.entries = after; state.journalDirty = true; state.journalHeld = false; }
      }
    } else {
      state.durable = { seq: slot.seq, cp: slot.cp };
      if (state.entries !== undefined) {
        // The journal against the new slot: the entries after it, tagged with it.
        const after = state.entries.filter(entry => !this.before(state, entry));
        state.entries = after.map(entry => entry.cp === slot.cp ? entry : { ...entry, cp: slot.cp as string });
        state.journalDirty = true; state.journalHeld = false;
      }
    }
    this.host.written(slot.id, key);
  }

  private async writeJournal(state: State, id: string): Promise<void> {
    const key = JOURNAL + id;
    const entries = state.entries!;
    const acks = state.acks.splice(0);
    state.writingAcks = acks;
    state.journalDirty = false;
    try {
      await this.host.persist(key, entries.length ? JSON.stringify(entries) : null);
    } catch (error) {
      state.journalDirty = true;
      state.journalHeld = true;
      state.acks.unshift(...acks);
      this.host.failed(key, error);
      return;
    }
    for (const ack of acks) this.host.written(ack, key);
    if (!acks.length) this.host.written(0, key);
  }
}

function isMissing(error: unknown): boolean {
  return !!error && typeof error === 'object' && (error as { code?: unknown }).code === 'ENOENT';
}
