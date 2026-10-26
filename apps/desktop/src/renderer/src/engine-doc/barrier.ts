import { changedPathsAsync } from './changed-paths-async';
import type { DigestInstruction } from '../../../shared/document-digest';
/*
 * The write barrier and the canonical patch stream (docs/rust-engine.md,
 * section 6; contract 5, docs/engine/rfcs/05-patches.md).
 *
 * The renderer stays the authority for the document. Every write records a
 * dirty path; before any pm-wasm query the barrier flushes: for each dirty
 * path it diffs a shadow (the canonical document exactly as last sent) against
 * the live document with `diffOps` (id-keyed structural operations, leaf
 * patches otherwise), encodes a canonicalV1 body, and updates the mirror and
 * shadow as the next revision. Native apply succeeds before the shadow moves. The
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
import { sameJsonAsync } from './json-diff-async';
import { jsonCloneAsync } from '../../../shared/canonical-async';
import type { PathPart } from '../../../shared/patch';
import { applyOps, type PatchOp } from '../../../shared/patch-ids';
import { diffJsonOps as diffOps, JsonMemo, sameJson } from './json-diff';
import { ConsistencyHash } from './consistency-hash';
import { encodePatchOps } from '../../../shared/patch-binary';
import { canonicalProject, canonicalStringify, jsonClone } from '../../../shared/canonical-json';
import { canonicalPath } from '../legacy/core/compositions';
import { APPLY, type PmWasm } from './wasm';
import { RemotePmWasm } from './remote';
import { documentId } from '../../../shared/engine-document-id';

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
  /** The local integrity digests (wire snapshots keep their canonical hash). */
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
  /** Refused batches (WASM resyncs; native evaluators recover asynchronously). */
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
 * first member one side lacks (the parent when that side is not an object).
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
    // A member one side lacks is still its own path (an insertion or a
    // deletion there); widening to its parent would diff the whole parent
    // (a composition's every layer for an optional setting it never had).
    if (!hasOwn(a, part) || !hasOwn(b, part)) return isRecord(a) && isRecord(b) ? path.slice(0, i + 1) : path.slice(0, i);
    a = a[part as any];
    b = b[part as any];
  }
  return path.slice();
}

export class DocumentBarrier {
  revision = 0;
  /** Native apply can stamp a revision before a later operation fails.
   * Async replacement must follow this floor without publishing a batch. */
  recoveryRevision = 0;
  /** Announcements fence asynchronous preparation even before they flush. */
  changeEpoch = 0;
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
  private integrity = new ConsistencyHash();
  private checkpoint: { revision: number; hash: string } | null = null;
  private journal: PatchBatch[] = [];
  private journalBytes = 0;
  private resyncPending = false;
  /** The open document's layers that recent diffs found changed, newest last. */
  private hot: Array<{ comp: string; id: string }> = [];
  /** The revision of a provisional batch not yet confirmed or superseded. */
  provisionalRevision: number | null = null;
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

  /** Whether the next flush covers every path anyway (a resync, or a diff
   *  of the whole document): a write needs no path of its own. */
  get coversDocument(): boolean { return this.resyncPending || this.dirty.has(pathKey([])); }

  /** Optional page-clock attribution for interactive performance diagnostics. */
  trace: ((stage: string, began: number) => void) | null = null;

  /** Resyncs at the next flush (the live document was replaced by another). */
  markResync(): void {
    this.changeEpoch++;
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
    if (paths.length) this.changeEpoch++;
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
    this.changeEpoch++;
    const was = this.pending;
    this.structural = true;
    this.bump('structuralDiff');
    if (!was && options.announce !== false) this.announcePending();
  }

  /** Replaces the mirror with the live document. */
  resync(): void {
    this.provisionalRevision = null;
    this.requireNativeAvailable();
    this.checkpoint = null; this.journal = []; this.journalBytes = 0;
    const canonical = canonicalProject(this.project());
    const json = JSON.stringify(canonical);
    this.revision += 1;
    // The parse of `json`, built directly and sharing the live document's
    // strings (shared/canonical-json.ts `jsonClone`).
    this.shadow = json === undefined ? undefined : jsonClone(canonical);
    this.wasm.resync(json ?? '', this.revision, documentId(canonical?.id));
    this.shapeRevision = this.revision;
    if (json && json.length > 4 * 1024 * 1024) this.wasm.releaseInput();
    this.dirty.clear();
    this.structural = false;
    this.resyncPending = false;
    this.origin = 'metadata';
    this.stats.resyncs += 1;
    // A `load` batch's receivers resync from `snapshot()` rather than apply
    // it, so its body (the whole document again) is encoded only if asked.
    const shadow = this.shadow;
    let body: Uint8Array | null = null;
    this.emit({
      baseRevision: this.revision - 1, revision: this.revision, origin: 'load', count: 1, assets: true,
      get body() { return (body ??= encodePatchOps([{ path: [], exists: json !== undefined, value: shadow }])); }
    });
  }

  /** Adopt an already checked native tree without serializing or parsing on the UI thread. */
  adopt(shadow: any, revision: number, hash: string, integrity: ConsistencyHash): void {
    this.shadow = shadow;
    this.revision = this.shapeRevision = revision;
    this.integrity = integrity;
    this.checkpoint = { revision, hash }; this.journal = []; this.journalBytes = 0;
    this.dirty.clear(); this.structural = this.resyncPending = false; this.origin = 'metadata';
    this.stats.resyncs++;
    let body: Uint8Array | null = null;
    this.emit({ baseRevision: revision - 1, revision, origin: 'load', count: 1, assets: true,
      get body() { return body ??= encodePatchOps([{ path: [], exists: shadow !== undefined, value: shadow }]); } });
  }

  /** Exact alias comparison remains on the object-owning thread in bounded
   * slices; native SHA receives only streamed canonical chunks. Recheck if an
   * announced edit crosses an await, so a save never checks another revision. */
  async checkAsync(reason: string, hash: (value: any, prefix?: string) => Promise<string>, repair?: () => Promise<void>, batch?: (instructions: DigestInstruction[]) => Promise<string[]>): Promise<ConsistencyResult> {
    const started = performance.now();
    this.stats.checks++;
    for (let attempt = 0; attempt < 2; attempt++) {
      this.flush();
      const revision = this.revision, shadow = this.shadow, integrity = this.integrity;
      const live = canonicalProject(this.project());
      const equal = await sameJsonAsync(shadow, live);
      const normalized = equal ? shadow : await jsonCloneAsync(live);
      const liveHash = equal ? await integrity.digestAsync(shadow, hash, batch) : await new ConsistencyHash().digestAsync(normalized, hash, batch);
      if (revision !== this.revision || shadow !== this.shadow || this.pending) continue;
      const mirror = this.wasm.consistencyHash();
      const ok = equal && liveHash === mirror;
      const missed = ok ? [] : await changedPathsAsync(shadow, normalized);
      if (revision !== this.revision || shadow !== this.shadow || this.pending) continue;
      const result = { ok, reason, revision, live: liveHash, mirror, missed, ms: performance.now() - started };
      if (!ok) {
        this.stats.checkMismatches++;
        if (repair) await repair(); else this.resync();
        this.onMismatch(result);
      }
      return result;
    }
    throw new Error('The document kept changing during its consistency check');
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

  /** Flushes dirty paths into pm-wasm (section 6, item 2: before every query). */
  flush(): void {
    this.requireNativeAvailable();
    if (this.flushing) return;
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
    const base = this.revision;
    const native = this.wasm instanceof RemotePmWasm;
    let body: Uint8Array;
    let result: ReturnType<PmWasm['apply']> | undefined;
    const encodeStarted = this.trace ? performance.now() : 0;
    try {
      body = encodePatchOps(ops);
      // Native refusals must leave the JS shadow at its previous revision.
      // A successful native apply followed by a shadow exception is a broken
      // mirror too: the owner rebuilds both from the authoritative raw tree.
      if (native) {
        result = this.wasm.apply(body, base, base + 1);
        if (result.code !== APPLY.ok) {
          this.stats.refused++;
          throw new Error(`The native document refused a batch (${result.code}: ${result.message})`);
        }
      }
      for (const op of ops) {
        this.integrity.invalidate(this.shadow, [op]);
        this.memo.forgetAlong(this.shadow, op.path);
        this.shadow = applyOps(this.shadow, [op]);
      }
      this.trace?.('document encode and shadow', encodeStarted);
    } catch (error) {
      if (native) this.nativeApplyFailed(ops, origin, error);
      // A value the format cannot carry (a lone surrogate in a key is fine;
      // a cycle is not) or a shadow that disagrees with its own ops: resync.
      console.warn('[engine-doc] patch encoding failed; resyncing', error);
      this.resync();
      return;
    }
    const applyStarted = this.trace ? performance.now() : 0;
    result ??= this.wasm.apply(body, base, base + 1);
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

  private requireNativeAvailable(): void {
    if (this.wasm instanceof RemotePmWasm && this.wasm.dead) {
      throw Object.assign(new Error('The native document mirror is unavailable; asynchronous recovery is required'), { code: 'unavailable' });
    }
  }

  private nativeApplyFailed(ops: PatchOp[], origin: IncrementalOrigin, cause: unknown): never {
    // This marks an actual mirror inconsistency, rather than changing the
    // transport's treatment of ordinary refused queries. The installer's
    // existing dead-instance path serializes reconnect/rebuild from PM.proj.
    if (this.wasm instanceof RemotePmWasm) this.wasm.invalidateMirror();
    else this.wasm.dead = true;
    this.recoveryRevision = Math.max(this.recoveryRevision, this.revision + 1);
    // A revision-gap refusal can reveal a tree farther ahead than our one
    // attempted batch. Read only its scalar revision; transport/owner loss
    // can refuse this too, in which case the attempted revision is the floor.
    if (this.wasm instanceof RemotePmWasm) try {
      const reply = JSON.parse(this.wasm.bridge.call('{"op":"revision"}'));
      if (reply.ok && Number.isSafeInteger(reply.value) && reply.value >= 0) {
        this.recoveryRevision = Math.max(this.recoveryRevision, reply.value);
      }
    } catch { /* the recovery will replace the unavailable owner */ }
    this.checkpoint = null; this.journal = []; this.journalBytes = 0;
    // Recovery resyncs: a provisional revision is superseded by it.
    this.provisionalRevision = null;
    this.touchedCanonical(ops.map(op => op.path), origin, { announce: false });
    this.resyncPending = true;
    throw Object.assign(new Error('The native document apply failed; asynchronous recovery is required', { cause }), { code: 'unavailable' });
  }

  private emit(batch: PatchBatch): void {
    if (batch.origin !== 'load' && this.checkpoint) {
      this.journalBytes += batch.body.length;
      if (this.journalBytes > 8 * 1024 * 1024 || this.journal.length >= 128) {
        this.checkpoint = null; this.journal = []; this.journalBytes = 0;
      } else this.journal.push(batch);
    }
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
   * local integrity digests of the live document and pm-wasm at this revision,
   * and on a mismatch resyncs and reports. Also lists writes the barrier
   * missed (paths where the live document differs from what was sent).
   */
  check(reason: string): ConsistencyResult {
    const started = performance.now();
    this.stats.checks += 1;
    this.flush();
    const live = canonicalProject(this.project());
    // Equality is checked independently against mutable live aliases. Hash
    // only changed shadow subtrees; pm-wasm independently hashes its tree
    // using node revisions and weak shared-container identities.
    const equal = sameJson(this.shadow, live);
    const liveHash = equal ? this.integrity.digest(this.shadow) : (live === undefined ? '' : new ConsistencyHash().digest(jsonClone(live)));
    const mirror = this.wasm.consistencyHash();
    const ok = equal && liveHash === mirror;
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
  snapshot(options: { checkpoint?: boolean } = {}): { revision: number; encoding: 'canonicalJsonV1'; bytes: Uint8Array; readonly json: string; hash: string } {
    this.flush();
    // Never a document state that did not exist: a snapshot asked from
    // inside the flush that sent a provisional batch (its listeners) waits.
    if (this.provisionalRevision !== null) throw new Error('The document is being compared; snapshot after the comparison');
    // pm-wasm holds this revision (the shadow equals it): its canonical JSON
    // comes out as bytes, outside the JavaScript heap, for a large document
    // the difference between opening and running out of memory.
    const bytes = this.wasm.toJsonBytes();
    const hash = this.wasm.hash();
    // Export/agent snapshots are independent consumers. Only a full viewer
    // resync may advance its reconnect base and discard the earlier suffix.
    if (options.checkpoint || !this.checkpoint) {
      this.checkpoint = { revision: this.revision, hash }; this.journal = []; this.journalBytes = 0;
    }
    return { revision: this.revision, encoding: 'canonicalJsonV1', bytes, get json() { return new TextDecoder().decode(bytes); }, hash };
  }

  /** A checked base and the bounded patch suffix for a short reconnect.
   * No JSON serialization or staging is needed when the engine kept it. */
  resume(): { revision: number; hash: string; batches: PatchBatch[] } {
    this.flush();
    if (this.provisionalRevision !== null) throw new Error('The document is being compared; resume after the comparison');
    if (!this.checkpoint) {
      this.checkpoint = { revision: this.revision, hash: this.wasm.hash() };
      this.journal = []; this.journalBytes = 0;
    }
    return { ...this.checkpoint, batches: [...this.journal] };
  }

  /** Native sessions can recover an expired suffix without materializing the
   * whole JSON document or hashing it on the renderer's main thread. */
  async refreshResume(force = false): Promise<ReturnType<DocumentBarrier['resume']> | null> {
    const wasm = this.wasm;
    if (!(wasm instanceof RemotePmWasm)) return null;
    for (let attempt = 0; attempt < 2; attempt++) {
      this.flush();
      if (this.checkpoint && !force) return this.resume();
      const revision = this.revision;
      let receipt;
      try { receipt = await wasm.refresh(revision); }
      catch (error) { if (this.revision !== revision && attempt === 0) continue; throw error; }
      this.flush();
      if (this.revision !== revision) continue;
      wasm.checkpoint(receipt);
      this.checkpoint = { revision, hash: receipt.hash }; this.journal = []; this.journalBytes = 0;
      return this.resume();
    }
    throw new Error('The document kept changing during checkpoint refresh');
  }

  /** Whether the shadow equals the live document (the auditor's comparison). */
  shadowMatches(): { ok: boolean; shadow: string; live: string } {
    const live = canonicalStringify(canonicalProject(this.project())) ?? '';
    const shadow = canonicalStringify(this.shadow) ?? '';
    return { ok: live === shadow, shadow, live };
  }
}
