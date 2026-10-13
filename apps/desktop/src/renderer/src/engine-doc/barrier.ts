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
import { diffJsonOps as diffOps } from './json-diff';
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
}

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

const ORIGIN_RANK: Record<PatchOrigin, number> = { metadata: 0, edit: 1, undo: 2, redo: 2, structuralDiff: 3, load: 4 };

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
  /** The canonical document exactly as pm-wasm has it. */
  shadow: any = undefined;
  private dirty = new Map<string, PathPart[]>();
  private structural = false;
  private resyncPending = false;
  private origin: PatchOrigin = 'metadata';
  private subscribers = new Set<(batch: PatchBatch) => void>();
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

  constructor(
    public wasm: PmWasm,
    /** The live document (raw, never the tracking Proxy). */
    private project: () => any,
    options: { onMismatch?: (result: ConsistencyResult) => void; revision?: number; beforeFlush?: () => void } = {},
  ) {
    this.onMismatch = options.onMismatch ?? (() => {});
    this.beforeFlush = options.beforeFlush ?? (() => {});
    // A replacement instance (after a trap) continues the revisions, so the
    // stream's receivers never see them move backward (contract 5, invariant 1).
    this.revision = Math.max(0, Math.floor(options.revision ?? 0));
  }

  /** Whether anything waits to be flushed. */
  get pending(): boolean { return this.resyncPending || this.structural || this.dirty.size > 0; }

  /** Resyncs at the next flush (the live document was replaced by another). */
  markResync(): void { this.resyncPending = true; }

  subscribe(fn: (batch: PatchBatch) => void): () => void {
    this.subscribers.add(fn);
    return () => this.subscribers.delete(fn);
  }

  private bump(origin: PatchOrigin) {
    if (ORIGIN_RANK[origin] > ORIGIN_RANK[this.origin]) this.origin = origin;
  }

  /** Records canonical paths as dirty. */
  touchedCanonical(paths: readonly PathPart[][], origin: PatchOrigin = 'edit'): void {
    for (const path of paths) this.dirty.set(pathKey(path), [...path]);
    if (paths.length) this.bump(origin);
  }

  /** Records paths relative to `PM.proj` (the realized form writers see). */
  touched(paths: readonly PathPart[][], origin: PatchOrigin = 'edit'): void {
    const rootId = this.project()?.compId;
    this.touchedCanonical(paths.map(path => canonicalPath([...path], rootId)), origin);
  }

  /** A `touch()` without a path: diff the open composition and the project's members. */
  touchAll(): void {
    this.structural = true;
    this.bump('structuralDiff');
  }

  /** Replaces the mirror with the live document. */
  resync(): void {
    const json = JSON.stringify(canonicalProject(this.project()));
    this.revision += 1;
    this.shadow = json === undefined ? undefined : JSON.parse(json);
    this.wasm.resync(json ?? '', this.revision);
    if (json && json.length > 4 * 1024 * 1024) this.wasm.releaseInput();
    this.dirty.clear();
    this.structural = false;
    this.resyncPending = false;
    this.origin = 'metadata';
    this.stats.resyncs += 1;
    this.emit({ baseRevision: this.revision - 1, revision: this.revision, origin: 'load', count: 1, body: encodePatchOps([{ path: [], exists: json !== undefined, value: this.shadow }]) });
  }

  /** The paths a structural diff covers: the open composition, compositions added or removed, and every project member but `comps`. */
  private structuralPaths(live: any): PathPart[][] {
    if (!isRecord(live)) return [[]];
    const paths: PathPart[][] = [];
    const keys = new Set([...Object.keys(live), ...(isRecord(this.shadow) ? Object.keys(this.shadow) : [])]);
    for (const key of keys) if (key !== 'comps') paths.push([key]);
    const open = live.compId;
    if (typeof open === 'string' && isRecord(live.comps)) {
      paths.push(['comps', open]);
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
      if (before.exists && after.exists) diffOps(before.value, after.value, path, ops);
      else if (after.exists) ops.push({ path: [...path], exists: true, value: after.value === undefined ? undefined : JSON.parse(JSON.stringify(after.value) ?? 'null') });
      else if (before.exists) ops.push({ path: [...path], exists: false });
    }
    return ops;
  }

  /** Flushes dirty paths into pm-wasm (section 6, item 2: before every query). */
  flush(): void {
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
      const ops = this.diff(live, paths);
      if (!ops.length) return;
      this.send(ops, origin);
    } finally {
      this.flushing = false;
      const ms = performance.now() - started;
      this.stats.flushMs += ms;
      if (ms > this.stats.maxFlushMs) this.stats.maxFlushMs = ms;
    }
  }

  private send(ops: PatchOp[], origin: PatchOrigin): void {
    let body: Uint8Array;
    try {
      body = encodePatchOps(ops);
      this.shadow = applyOps(this.shadow, ops);
    } catch (error) {
      // A value the format cannot carry (a lone surrogate in a key is fine;
      // a cycle is not) or a shadow that disagrees with its own ops: resync.
      console.warn('[engine-doc] patch encoding failed; resyncing', error);
      this.resync();
      return;
    }
    const base = this.revision;
    const result = this.wasm.apply(body, base, base + 1);
    this.revision = base + 1;
    if (result.code !== APPLY.ok) {
      this.stats.refused += 1;
      console.warn(`[engine-doc] pm-wasm refused a batch (${result.code}: ${result.message}); resyncing`);
      this.resync();
      return;
    }
    this.stats.batches += 1;
    this.stats.ops += ops.length;
    this.stats.bytes += body.length;
    this.emit({ baseRevision: base, revision: base + 1, origin, count: ops.length, body });
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
  repair(origin: PatchOrigin = 'structuralDiff'): string[] {
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
