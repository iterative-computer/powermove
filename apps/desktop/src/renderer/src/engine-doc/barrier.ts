/*
 * The write barrier and the canonical patch stream (docs/rust-engine.md,
 * section 6; contract 5, docs/engine/rfcs/05-patches.md).
 *
 * The renderer stays the authority for the document. Every write records a
 * dirty path; before any pm-wasm query the barrier flushes: for each dirty
 * path it diffs a shadow (the canonical document exactly as last sent) against
 * the live document with `diffOps` (id-keyed structural operations, leaf
 * patches otherwise), applies the operations to the shadow, encodes them as a
 * canonicalV1 body, and applies the body to pm-wasm as the next revision. The
 * same body is offered to subscribers (engine sessions in Phase 2).
 *
 * Writes reach the barrier four ways:
 *
 * 1. `write(paths, fn?)` and `touched(paths)`: a writer names what it wrote
 *    (paths relative to `PM.proj`, as writers see it);
 * 2. history: commits, undo, and redo hand over their canonical patch paths;
 * 3. `touchAll()`: a `touch()` without a path diffs the open composition and
 *    the project's own members;
 * 4. in development and test builds, the write-tracking Proxy
 *    (`proxy.ts`) records every write made through `PM.proj`.
 *
 * Replacing `PM.proj` with another document resyncs the mirror; replacing it
 * with another version of the same project diffs the whole document.
 */
import type { PathPart } from '../../../shared/patch';
import { applyOps, type PatchOp } from '../../../shared/patch-ids';
import { diffJsonOps as diffOps, JsonMemo } from './json-diff';
import { encodePatchOps } from '../../../shared/patch-binary';
import { canonicalHash, canonicalProject, canonicalStringify } from '../../../shared/canonical-json';
import { canonicalPath } from '../legacy/core/compositions';
import { APPLY, type PmWasm } from './wasm';

/** Why a batch was produced (contract 5, `PatchOrigin`). */
export type PatchOrigin = 'edit' | 'undo' | 'redo' | 'metadata' | 'structuralDiff' | 'load';

/**
 * A batch as the patch stream carries it (contract 5, `PatchBatch`). The body
 * is `canonicalV1` (`shared/patch-binary.ts`). A batch with origin `load` is
 * a resync: its body is one leaf set of the whole document at the empty path,
 * applicable whatever the receiver's revision (contract 5, invariant 4).
 */
export interface PatchBatch {
  baseRevision: number;
  revision: number;
  origin: PatchOrigin;
  count: number;
  body: Uint8Array;
  /** Whether the batch changes the project's assets (or its id, or the
   *  whole document): listeners that only follow assets skip the rest. */
  assets: boolean;
  /** Made before the rest of the document was compared (`flushProvisional`):
   *  its frames wait for a `confirmed` item naming its revision, and the
   *  next batch supersedes it. */
  provisional?: boolean;
  /** No operations: the provisional batch that made `revision` was the
   *  whole change (`baseRevision === revision`, an empty body). */
  confirmed?: boolean;
}

/** Layers whose writes a provisional batch carries first: those the latest
 *  unscoped diffs found changed (a drag writes the same layer each time). */
const HOT_LAYERS = 4;

export interface ConsistencyResult {
  ok: boolean;
  reason: string;
  revision: number;
  /** The live document's canonical hash and the mirror's. */
  live: string;
  mirror: string;
  /** Paths where the live document and the shadow differed (missed writes). */
  missed: string[];
  ms: number;
}

export interface BarrierStats {
  flushes: number;
  batches: number;
  ops: number;
  bytes: number;
  resyncs: number;
  structuralDiffs: number;
  /** Batches pm-wasm refused (each followed by a resync). */
  refused: number;
  /** Time spent diffing, encoding, and applying, in ms. */
  flushMs: number;
  maxFlushMs: number;
  checks: number;
  checkMismatches: number;
}

/**
 * The origins an incremental batch can carry. `load` is not one: only
 * `resync()` produces it, and its body is always the whole document at `[]`.
 */
export type IncrementalOrigin = Exclude<PatchOrigin, 'load'>;

const ORIGIN_RANK: Record<IncrementalOrigin, number> = { metadata: 0, edit: 1, undo: 2, redo: 2, structuralDiff: 3 };

const isRecord = (v: any): v is Record<string, any> => !!v && typeof v === 'object' && !Array.isArray(v);
/** Whether `k` is a JSON member of `o`: an array index in range, or an enumerable own member with a JSON value. */
const hasOwn = (o: any, k: PathPart) => {
  if (o == null || typeof o !== 'object') return false;
  if (Array.isArray(o)) return typeof k === 'number' ? k < o.length : Object.prototype.hasOwnProperty.call(o, k);
  if (!Object.prototype.propertyIsEnumerable.call(o, k)) return false;
  const v = o[k as any];
  return v !== undefined && typeof v !== 'function' && typeof v !== 'symbol';
};

/** The value at a path, and whether it exists (JSON members only). */
export function lookup(root: any, path: readonly PathPart[]): { exists: boolean; value?: any } {
  let node = root;
  for (const part of path) {
    if (!hasOwn(node, part)) return { exists: false };
    node = node[part as any];
  }
  return { exists: root !== undefined, value: node };
}

const pathKey = (path: readonly PathPart[]) => JSON.stringify(path);

/** Drops paths an ancestor in the list already covers. */
export function coverPaths(paths: readonly PathPart[][]): PathPart[][] {
  const sorted = [...paths].sort((a, b) => a.length - b.length);
  const kept: PathPart[][] = [];
  const seen = new Set<string>();
  outer: for (const path of sorted) {
    for (let n = 0; n <= path.length; n++) if (seen.has(pathKey(path.slice(0, n)))) continue outer;
    seen.add(pathKey(path));
    kept.push(path);
  }
  return kept;
}

const idOf = (v: any) => (isRecord(v) && typeof v.id === 'string' ? v.id : undefined);

/** The layer members a composition's shape is made of: identity, kind,
 *  parenting, group membership, the 3D switch, and locks. */
const SHAPE_MEMBERS = new Set(['id', 'type', 'parent', 'group', 'threeD', 'lock']);

/**
 * Whether a canonical patch path can change the shape of a composition: its
 * set or order of layers, or a layer's `id`, `type`, `parent`, `group`,
 * `threeD`, or `lock`; also which composition is open, or the compositions
 * themselves. Channel values, content, timing, composition settings, and
 * project members such as `edits` and `revision` cannot. Structural answers
 * (group ancestry, parenting cycles, the 3D switch, transform roots) depend
 * on the shape only, so they hold until it changes (`shapeRevision`).
 */
export function changesShape(path: readonly PathPart[]): boolean {
  if (path.length === 0) return true;
  const [head] = path;
  // A document without an open composition id keeps its layers at the root.
  if (head === 'layers') return path.length < 3 || SHAPE_MEMBERS.has(String(path[2]));
  if (head !== 'comps') return head === 'compId';
  if (path.length < 3) return true;
  if (path[2] !== 'layers') return false;
  return path.length < 5 || SHAPE_MEMBERS.has(String(path[4]));
}

/**
 * Where to diff for a dirty path: the path itself, cut at the first array
 * whose length or `id` order differs between the shadow and the live
 * document (so structural changes become id-keyed operations), or at the
 * first missing member.
 */
export function anchorPath(shadow: any, live: any, path: readonly PathPart[]): PathPart[] {
  let a = shadow, b = live;
  for (let i = 0; i < path.length; i++) {
    if (Array.isArray(a) || Array.isArray(b)) {
      if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return path.slice(0, i);
      const index = path[i];
      if (typeof index === 'number' && idOf(a[index]) !== idOf(b[index])) return path.slice(0, i);
    }
    const part = path[i]!;
    if (!hasOwn(a, part) || !hasOwn(b, part)) return path.slice(0, i);
    a = a[part as any];
    b = b[part as any];
  }
  return path.slice();
}

export class DocumentBarrier {
  revision = 0;
  /**
   * The revision of the last batch that could change a composition's shape
   * (`changesShape`; every resync). Answers that depend on the shape only
   * stay valid while it holds.
   */
  shapeRevision = 0;
  /** The canonical document exactly as pm-wasm has it. */
  shadow: any = undefined;
  /** What unchanged shadow elements were last compared equal to. */
  private memo = new JsonMemo();
  private dirty = new Map<string, PathPart[]>();
  private structural = false;
  private resyncPending = false;
  /** The open document's layers that recent diffs found changed, newest last. */
  private hot: Array<{ comp: string; id: string }> = [];
  /** The revision of a provisional batch not yet confirmed or superseded. */
  provisionalRevision: number | null = null;
  /** Inside `provisionalQueries`: queries read the provisional mirror. */
  private provisionalReads = 0;
  private origin: IncrementalOrigin = 'metadata';
  private subscribers = new Set<(batch: PatchBatch) => void>();
  /** Told when the barrier goes from nothing pending to something pending. */
  private pendingListeners = new Set<() => void>();
  private onMismatch: (result: ConsistencyResult) => void;
  readonly stats: BarrierStats = { flushes: 0, batches: 0, ops: 0, bytes: 0, resyncs: 0, structuralDiffs: 0, refused: 0, flushMs: 0, maxFlushMs: 0, checks: 0, checkMismatches: 0 };
  /** Set while a flush runs, so reads of the document during it are not writes. */
  flushing = false;
  /**
   * Called as a flush starts, before it looks at what is dirty: the owner
   * marks writes that may be waiting unannounced (an open history
   * transaction's, which its commit hands over only at the end).
   */
  private beforeFlush: () => void;
  /** The nested composition the editor has entered (`PM.curComp()`), if any. */
  private scopedComp: () => string | null;

  constructor(
    public wasm: PmWasm,
    /** The live document (raw, never the tracking Proxy). */
    private project: () => any,
    options: { onMismatch?: (result: ConsistencyResult) => void; revision?: number; beforeFlush?: () => void; scoped?: () => string | null } = {},
  ) {
    this.scopedComp = options.scoped ?? (() => null);
    this.onMismatch = options.onMismatch ?? (() => {});
    this.beforeFlush = options.beforeFlush ?? (() => {});
    // A replacement instance (after a trap) continues the revisions, so the
    // stream's receivers never see them move backward (contract 5, invariant 1).
    this.revision = Math.max(0, Math.floor(options.revision ?? 0));
  }

  /** Whether anything waits to be flushed. */
  get pending(): boolean { return this.resyncPending || this.structural || this.dirty.size > 0; }

  /** Optional page-clock attribution for interactive performance diagnostics. */
  trace: ((stage: string, began: number) => void) | null = null;

  /** Resyncs at the next flush (the live document was replaced by another). */
  markResync(): void {
    const was = this.pending;
    this.resyncPending = true;
    if (!was) this.announcePending();
  }

  /**
   * `fn` runs when a write leaves something to flush where nothing was
   * (once until the next flush). The engine connection flushes soon after,
   * so a write reaches the engine's patch stream without waiting for the
   * next query (section 6, "The patch stream").
   */
  onPending(fn: () => void): () => void {
    this.pendingListeners.add(fn);
    return () => this.pendingListeners.delete(fn);
  }

  private announcePending(): void {
    for (const fn of this.pendingListeners) {
      try { fn(); } catch (error) { console.error('[engine-doc] pending listener failed', error); }
    }
  }

  subscribe(fn: (batch: PatchBatch) => void): () => void {
    this.subscribers.add(fn);
    return () => this.subscribers.delete(fn);
  }

  private bump(origin: IncrementalOrigin) {
    if (ORIGIN_RANK[origin] > ORIGIN_RANK[this.origin]) this.origin = origin;
  }

  /** Records canonical paths as dirty. */
  touchedCanonical(paths: readonly PathPart[][], origin: IncrementalOrigin = 'edit', options: { announce?: boolean } = {}): void {
    const was = this.pending;
    for (const path of paths) this.dirty.set(pathKey(path), [...path]);
    if (paths.length) this.bump(origin);
    if (!was && this.pending && options.announce !== false) this.announcePending();
  }

  /** Records paths relative to `PM.proj` (the realized form writers see). */
  touched(paths: readonly PathPart[][], origin: IncrementalOrigin = 'edit', options: { announce?: boolean } = {}): void {
    const rootId = this.project()?.compId;
    this.touchedCanonical(paths.map(path => canonicalPath([...path], rootId)), origin, options);
  }

  /** A `touch()` without a path: diff the open composition and the project's members.
   *  `announce: false` for a speculative diff the owner marks just before a
   *  flush (an open transaction's possible writes): nobody needs telling,
   *  the flush that asked for it follows. */
  touchAll(options: { announce?: boolean } = {}): void {
    const was = this.pending;
    this.structural = true;
    this.bump('structuralDiff');
    if (!was && options.announce !== false) this.announcePending();
  }

  /** Replaces the mirror with the live document. */
  resync(): void {
    this.provisionalRevision = null;
    const json = JSON.stringify(canonicalProject(this.project()));
    this.revision += 1;
    this.shadow = json === undefined ? undefined : JSON.parse(json);
    this.wasm.resync(json ?? '', this.revision);
    this.shapeRevision = this.revision;
    if (json && json.length > 4 * 1024 * 1024) this.wasm.releaseInput();
    this.dirty.clear();
    this.structural = false;
    this.resyncPending = false;
    this.origin = 'metadata';
    this.stats.resyncs += 1;
    this.emit({ baseRevision: this.revision - 1, revision: this.revision, origin: 'load', count: 1, body: encodePatchOps([{ path: [], exists: json !== undefined, value: this.shadow }]), assets: true });
  }

  /** The paths a structural diff covers: the open composition, the entered one, compositions added or removed, and every project member but `comps`. */
  private structuralPaths(live: any): PathPart[][] {
    if (!isRecord(live)) return [[]];
    const paths: PathPart[][] = [];
    const keys = new Set([...Object.keys(live), ...(isRecord(this.shadow) ? Object.keys(this.shadow) : [])]);
    for (const key of keys) if (key !== 'comps') paths.push([key]);
    const open = live.compId;
    if (typeof open === 'string' && isRecord(live.comps)) {
      paths.push(['comps', open]);
      // The composition the editor has entered, whose layers are being edited.
      const scoped = this.scopedComp();
      if (scoped && scoped !== open) paths.push(['comps', scoped]);
      // Compositions added or removed (a precompose, an import that nests).
      const before = isRecord(this.shadow) && isRecord(this.shadow.comps) ? this.shadow.comps : {};
      for (const id of Object.keys(live.comps)) if (!Object.prototype.hasOwnProperty.call(before, id)) paths.push(['comps', id]);
      for (const id of Object.keys(before)) if (!Object.prototype.hasOwnProperty.call(live.comps, id)) paths.push(['comps', id]);
    } else paths.push(['comps']);
    return paths;
  }

  /** Operations turning the shadow into the live document at the given paths. */
  private diff(live: any, paths: readonly PathPart[][]): PatchOp[] {
    const ops: PatchOp[] = [];
    if (this.shadow === undefined || live === undefined || !isRecord(this.shadow) || !isRecord(live)) {
      diffOps(this.shadow, live, [], ops);
      return ops;
    }
    const anchored = coverPaths(paths.map(path => anchorPath(this.shadow, live, path)));
    for (const path of anchored) {
      const before = lookup(this.shadow, path), after = lookup(live, path);
      if (before.exists && after.exists) diffOps(before.value, after.value, path, ops, this.memo);
      else if (after.exists) ops.push({ path: [...path], exists: true, value: after.value === undefined ? undefined : JSON.parse(JSON.stringify(after.value) ?? 'null') });
      else if (before.exists) ops.push({ path: [...path], exists: false });
    }
    return ops;
  }

  /**
   * The first half of a `touch()` without paths: diffs only the paths
   * written by name and the layers recent diffs found changed, and sends
   * them as a provisional batch, so the engine starts on the edit while the
   * rest of the document is compared. The structural diff stays pending:
   * the next `flush` (soon after, or any query's) compares everything, then
   * confirms the provisional revision or supersedes it with the rest.
   * Returns whether it sent one. Exact by construction: frames of a
   * provisional revision are only presented once confirmed.
   */
  flushProvisional(): boolean {
    if (this.flushing || this.provisionalRevision !== null || this.resyncPending || !this.structural || !this.hot.length || !isRecord(this.shadow)) return false;
    this.beforeFlush();
    const live = canonicalProject(this.project());
    const paths = [...this.dirty.values()];
    for (const { comp, id } of this.hot) {
      const before = this.shadow.comps?.[comp]?.layers, after = live?.comps?.[comp]?.layers;
      if (!Array.isArray(before) || !Array.isArray(after) || before.length !== after.length) return false;
      const index = after.findIndex((layer: any) => layer?.id === id);
      if (index < 0 || before[index]?.id !== id) return false;
      paths.push(['comps', comp, 'layers', index]);
    }
    this.flushing = true;
    try {
      const diffStarted = this.trace ? performance.now() : 0;
      const ops = this.diff(live, paths);
      this.trace?.('provisional diff', diffStarted);
      this.dirty.clear();
      if (!ops.length) return false;
      this.send(ops, this.origin, true);
      return this.provisionalRevision !== null;
    } finally {
      this.flushing = false;
    }
  }

  /**
   * Runs `fn` with queries answered by the provisional mirror: while a
   * provisional revision waits, `flush` leaves the rest of the diff for
   * later. Only for work tagged with that revision (a paused edit's own
   * raster answers): it counts only if the revision is confirmed, when the
   * provisional mirror was the whole document.
   */
  provisionalQueries<T>(fn: () => T): T {
    this.provisionalReads++;
    try { return fn(); } finally { this.provisionalReads--; }
  }

  /** Flushes dirty paths into pm-wasm (section 6, item 2: before every query). */
  flush(): void {
    if (this.flushing || (this.provisionalReads > 0 && this.provisionalRevision !== null)) return;
    if (this.shadow !== undefined) this.beforeFlush();
    if (!this.pending) return;
    this.flushing = true;
    const started = performance.now();
    try {
      this.stats.flushes += 1;
      if (this.resyncPending) {
        this.resync();
        return;
      }
      const live = canonicalProject(this.project());
      let paths = [...this.dirty.values()];
      if (this.structural) {
        this.stats.structuralDiffs += 1;
        paths = paths.concat(this.structuralPaths(live));
      }
      const origin = this.origin;
      this.dirty.clear();
      this.structural = false;
      this.origin = 'metadata';
      const diffStarted = this.trace ? performance.now() : 0;
      const ops = this.diff(live, paths);
      this.trace?.('document diff', diffStarted);
      if (ops.length) this.send(ops, origin);
      else if (this.provisionalRevision !== null) this.confirmProvisional();
    } finally {
      this.flushing = false;
      const ms = performance.now() - started;
      this.stats.flushMs += ms;
      if (ms > this.stats.maxFlushMs) this.stats.maxFlushMs = ms;
    }
  }

  /** The provisional revision was the whole change: its frames may show. */
  private confirmProvisional(): void {
    const revision = this.provisionalRevision!;
    this.provisionalRevision = null;
    this.emit({ baseRevision: revision, revision, origin: 'metadata', count: 0, body: new Uint8Array(0), assets: false, confirmed: true });
  }

  /** Remembers the open document's layers that `ops` changed in place. */
  private noteHot(ops: readonly PatchOp[]): void {
    for (const op of ops) {
      const [root, comp, layers, index] = op.path;
      if (root !== 'comps' || layers !== 'layers' || typeof comp !== 'string' || typeof index !== 'number') continue;
      const id = this.shadow?.comps?.[comp]?.layers?.[index]?.id;
      if (typeof id !== 'string') continue;
      this.hot = this.hot.filter(h => h.comp !== comp || h.id !== id);
      this.hot.push({ comp, id });
      if (this.hot.length > HOT_LAYERS) this.hot.shift();
    }
  }

  private send(ops: PatchOp[], origin: IncrementalOrigin, provisional = false): void {
    let body: Uint8Array;
    const encodeStarted = this.trace ? performance.now() : 0;
    try {
      body = encodePatchOps(ops);
      for (const op of ops) {
        this.memo.forgetAlong(this.shadow, op.path);
        this.shadow = applyOps(this.shadow, [op]);
      }
      this.trace?.('document encode and shadow', encodeStarted);
    } catch (error) {
      // A value the format cannot carry (a lone surrogate in a key is fine;
      // a cycle is not) or a shadow that disagrees with its own ops: resync.
      console.warn('[engine-doc] patch encoding failed; resyncing', error);
      this.resync();
      return;
    }
    const base = this.revision;
    const applyStarted = this.trace ? performance.now() : 0;
    const result = this.wasm.apply(body, base, base + 1);
    this.trace?.('document wasm apply', applyStarted);
    this.revision = base + 1;
    if (ops.some(op => changesShape(op.path))) this.shapeRevision = this.revision;
    if (result.code !== APPLY.ok) {
      this.stats.refused += 1;
      console.warn(`[engine-doc] pm-wasm refused a batch (${result.code}: ${result.message}); resyncing`);
      this.resync();
      return;
    }
    this.stats.batches += 1;
    this.stats.ops += ops.length;
    this.stats.bytes += body.length;
    this.noteHot(ops);
    // A later batch supersedes a provisional one.
    this.provisionalRevision = provisional ? this.revision : null;
    const assets = ops.some(op => op.path.length === 0 || op.path[0] === 'assets' || op.path[0] === 'id');
    const postStarted = this.trace ? performance.now() : 0;
    this.emit({ baseRevision: base, revision: base + 1, origin, count: ops.length, body, assets, ...(provisional ? { provisional: true } : {}) });
    this.trace?.('document patch dispatch', postStarted);
  }

  private emit(batch: PatchBatch): void {
    for (const fn of this.subscribers) {
      try { fn(batch); } catch (error) { console.error('[engine-doc] patch subscriber failed', error); }
    }
  }

  /**
   * Writes the barrier did not see: the operations from the shadow to the
   * live document over the whole document. Sends them (repairing the mirror)
   * and returns their paths.
   */
  repair(origin: IncrementalOrigin = 'structuralDiff'): string[] {
    this.flush();
    const live = canonicalProject(this.project());
    const ops = diffOps(this.shadow, live);
    if (!ops.length) return [];
    this.send(ops, origin);
    return ops.map(op => op.path.map(String).join('.'));
  }

  /**
   * The production consistency check (section 6, item 7): flushes, compares
   * the canonical hash of the live document with pm-wasm's at this revision,
   * and on a mismatch resyncs and reports. Also lists writes the barrier
   * missed (paths where the live document differs from what was sent).
   */
  check(reason: string): ConsistencyResult {
    const started = performance.now();
    this.stats.checks += 1;
    this.flush();
    const live = canonicalProject(this.project());
    const liveHash = live === undefined ? '' : canonicalHash(live);
    const mirror = this.wasm.hash();
    const ok = liveHash === mirror;
    // Only on a mismatch: where the live document left the shadow (values JSON
    // cannot carry, such as NaN, differ from their stored form every time).
    const missed = ok ? [] : diffOps(this.shadow, live).map(op => op.path.map(String).join('.'));
    const result: ConsistencyResult = { ok, reason, revision: this.revision, live: liveHash, mirror, missed, ms: 0 };
    if (!ok) {
      this.stats.checkMismatches += 1;
      this.resync();
    }
    result.ms = performance.now() - started;
    if (!ok) this.onMismatch(result);
    return result;
  }

  /**
   * The document a new receiver of the stream starts from (contract 5,
   * `Resync` with `canonicalJsonV1`): flushed, the canonical JSON with sorted
   * keys, its revision, and pm-wasm's canonical hash at that revision.
   * Batches with `baseRevision === revision` follow.
   */
  snapshot(): { revision: number; encoding: 'canonicalJsonV1'; json: string; hash: string } {
    this.flush();
    return { revision: this.revision, encoding: 'canonicalJsonV1', json: canonicalStringify(this.shadow) ?? '', hash: this.wasm.hash() };
  }

  /** Whether the shadow equals the live document (the auditor's comparison). */
  shadowMatches(): { ok: boolean; shadow: string; live: string } {
    const live = canonicalStringify(canonicalProject(this.project())) ?? '';
    const shadow = canonicalStringify(this.shadow) ?? '';
    return { ok: live === shadow, shadow, live };
  }
}
