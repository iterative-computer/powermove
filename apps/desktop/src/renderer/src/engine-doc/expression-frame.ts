import type { PmWasm } from './wasm';

/** One resident expression id. `pos` is its index in the group's members (-1
 * while it is not one); `seen` and `done` are epoch serials; `at` is the
 * group's read clock at its last read. */
export interface ExpressionFrameSlot {
  readonly id: number;
  readonly group: ExpressionFrameGroup;
  pos: number;
  seen: number;
  done: number;
  at: number;
  released: boolean;
}

/** The resident ids of one composition scope. An epoch is one (time,
 * revision): every read in it gets the same answer the scalar call would. */
export interface ExpressionFrameGroup {
  /** Members in the order frames read them. */
  members: ExpressionFrameSlot[];
  batch?: ReturnType<PmWasm['prepareBatch']>;
  wasm?: PmWasm;
  values: Float64Array;
  kinds: Uint8Array;
  serial: number;
  time?: number;
  revision: number;
  /** Distinct slots read this epoch, in read order. */
  reads: ExpressionFrameSlot[];
  misses: number;
  breaks: number;
  /** End of the last evaluated run, and that run's length. */
  runEnd: number;
  run: number;
  epochs: number;
  /** Distinct reads so far (one per slot and epoch). */
  clock: number;
  /** Registered slots (members or not). */
  size: number;
}

/** A run that starts anywhere evaluates up to four ids; each continuation
 * doubles. Runs only extend over recently read members. */
const FIRST_RUN = 4;
/** Members unread for this many epochs leave the group. */
const PRUNE_EPOCHS = 64;

/**
 * Resident expression ids, grouped by the exact scope they were interned in,
 * evaluated in runs.
 *
 * Today's evaluator is lazy: a frame pays only for the channels it reads (a
 * project of sequential layers reads few of them at any time). Evaluating every
 * registered id at each new time would pay for inactive and deleted layers too.
 * Instead, each group orders its members as frames read them, and the first
 * read of a member at a new time evaluates a run starting at that member,
 * whose length doubles while reads continue along the order (4, 8, 16, …),
 * over members read recently. A frame that reads the members in order
 * crosses into wasm about log2(n) times; other patterns evaluate at most
 * about twice the ids they read (plus three per run). Reads of
 * ids that are not members fall back to the scalar call and join the members
 * for the next epoch; members no frame has read for 64 epochs leave.
 *
 * Revision, module, scope and Object.is(time) are exact. Every evaluation is
 * of ids pm-wasm validates itself; re-evaluating an id at the same time and
 * revision is its memo. Nonnumeric results keep the scalar path's decoding.
 */
export class ExpressionFrames {
  private groups = new Map<string, ExpressionFrameGroup>();
  private slots = new Map<number, ExpressionFrameSlot>();
  /** The value of the last `read` that answered (status >= 0). */
  value = NaN;

  register(comp: string, id: number): ExpressionFrameSlot {
    const existing = this.slots.get(id);
    if (existing && !existing.released) {
      if (this.groups.get(comp) === existing.group) return existing;
      this.release(id);
    }
    let group = this.groups.get(comp);
    if (!group) {
      group = { members: [], values: new Float64Array(0), kinds: new Uint8Array(0), serial: 0, revision: -1, reads: [], misses: 0, breaks: 0, runEnd: -1, run: 0, epochs: 0, clock: 0, size: 0 };
      this.groups.set(comp, group);
    }
    const slot: ExpressionFrameSlot = { id, group, pos: -1, seen: -1, done: -1, at: -1, released: false };
    this.slots.set(id, slot);
    group.size += 1;
    // Registered after its scalar read in the current epoch: it is that
    // epoch's miss, so it joins the members at the next one.
    if (group.serial > 0) this.note(group, slot);
    return slot;
  }

  has(id: number): boolean { return this.slots.has(id); }

  slot(id: number): ExpressionFrameSlot | undefined { return this.slots.get(id); }

  release(id: number): void {
    const slot = this.slots.get(id);
    if (!slot) return;
    this.slots.delete(id);
    slot.released = true;
    const group = slot.group;
    if (slot.pos >= 0) this.setMembers(group, group.members.filter(s => s !== slot));
    group.size -= 1;
    if (!group.size) for (const [comp, g] of this.groups) if (g === group) this.groups.delete(comp);
  }

  /**
   * The status of `slot` at (time, revision) from a batch run, with its number
   * in `this.value`; -1 when the caller must use the scalar call (the slot is
   * not a member yet, or was released).
   */
  read(wasm: PmWasm, slot: ExpressionFrameSlot, time: number, revision: number): number {
    if (slot.released) return -1;
    const group = slot.group;
    if (group.revision !== revision || !Object.is(group.time, time)) this.advance(group, time, revision);
    if (slot.seen !== group.serial) this.note(group, slot);
    const pos = slot.pos;
    if (pos < 0) return -1;
    if (slot.done !== group.serial) {
      if (pos === group.runEnd) group.run *= 2;
      else { group.run = FIRST_RUN; group.breaks += 1; }
      // A run extends only over members read recently (about the last two
      // frames' worth of reads): those frames are about to read again. A
      // layer that went inactive or was deleted stops being evaluated.
      const members = group.members, limit = Math.min(members.length, pos + group.run);
      const recent = group.clock - (2 * members.length + 64);
      let end = pos + 1;
      while (end < limit && members[end]!.at >= recent && members[end]!.at >= 0) end += 1;
      if (!group.batch || group.wasm !== wasm) {
        group.batch = wasm.prepareBatch(group.members.map(s => s.id));
        group.wasm = wasm;
      }
      const result = group.batch(time, pos, end - pos);
      // Another scope may use the ABI buffers before this one is read again.
      const values = group.values, kinds = group.kinds, serial = group.serial;
      for (let i = pos; i < end; i++) {
        values[i] = result.values[i]!;
        kinds[i] = result.kinds[i]!;
        members[i]!.done = serial;
      }
      group.runEnd = end;
    }
    this.value = group.values[pos]!;
    return group.kinds[pos]!;
  }

  private note(group: ExpressionFrameGroup, slot: ExpressionFrameSlot) {
    slot.seen = group.serial;
    slot.at = group.clock++;
    group.reads.push(slot);
    if (slot.pos < 0) group.misses += 1;
  }

  /** Starts an epoch, first adapting the members to the reads of the last. */
  private advance(group: ExpressionFrameGroup, time: number, revision: number) {
    if (group.serial > 0) {
      const reads = group.reads;
      const live = (s: ExpressionFrameSlot) => !s.released;
      if (group.misses > 0) {
        // New channels join after the members they followed least: their
        // reads break a run once, and a later reorder places them exactly.
        this.setMembers(group, [...group.members, ...reads.filter(s => s.pos < 0 && live(s))]);
      } else if (group.breaks > 2 && reads.length >= Math.max(8, group.members.length / 2)) {
        // Full frames read in another order now (layers moved): adopt it,
        // keeping the members this epoch did not read after them. Small
        // epochs (a motion-blur sample of one layer) never reorder.
        const read = new Set(reads);
        this.setMembers(group, [...reads.filter(live), ...group.members.filter(s => !read.has(s))]);
      }
      group.epochs += 1;
      if (group.epochs % PRUNE_EPOCHS === 0) {
        const kept = group.members.filter(s => group.serial - s.seen < PRUNE_EPOCHS);
        if (kept.length !== group.members.length) this.setMembers(group, kept);
      }
    }
    group.serial += 1;
    group.time = time;
    group.revision = revision;
    group.reads = [];
    group.misses = 0;
    group.breaks = 0;
    group.runEnd = -1;
    group.run = 0;
  }

  private setMembers(group: ExpressionFrameGroup, members: ExpressionFrameSlot[]) {
    for (const s of group.members) s.pos = -1;
    members.forEach((s, i) => { s.pos = i; s.done = -1; });
    group.members = members;
    group.batch = undefined;
    group.values = new Float64Array(members.length);
    group.kinds = new Uint8Array(members.length);
    group.runEnd = -1;
  }
}
