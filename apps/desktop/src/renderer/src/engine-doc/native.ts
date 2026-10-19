/*
 * The kernel's synchronous queries answered by pm-wasm (docs/rust-engine.md,
 * sections 6, 7, and 20), behind the "native evaluator" flag.
 *
 * Every call flushes the write barrier first, so pm-wasm answers on exactly
 * the document the caller sees. The identity registry maps the live objects
 * callers pass to canonical addresses: a layer to its composition and index
 * (a layer that is not in the composition is evaluated ad hoc from its
 * JSON), a channel object to its path inside the layer (interned once as a
 * numeric id for the fast path), and answers come back as the live objects
 * (`findProp` and `allProps` return the layer's own channel objects, `pick`
 * the live layer, `hasKeyAt` the live keyframe). Expression errors land on the
 * live channel objects.
 *
 * What pm-wasm cannot answer in Phase 1 falls back to the legacy code and is
 * counted: 3D objects, model groups, and extension scenes without a
 * background (three.js until Phase 3c), and text layout before main has
 * served the fonts it needs.
 */
import type { PathPart } from '../../../shared/patch';
import { lookup, type DocumentBarrier } from './barrier';
import type { PmWasm } from './wasm';
import type { DualAnswers } from './dual';
import { geometryRequest, refreshModels, resolveModels } from '../core/scene3d/engine-geometry';
import type { FontLoader } from './fonts';

type Legacy = Record<string, any>;

/** A native answer: the value, a throw to rethrow, or a reason to ask the legacy code. */
type Native<T> = { value: T } | { threw: Error } | { fallback: string };

/** Legacy answers at the time moved by ±1e-12 s (amendment 20's fragility test). */
function timePerturbations<T>(time: unknown, at: (t: number) => T): Array<() => T> {
  if (typeof time !== 'number' || !Number.isFinite(time)) return [];
  return [() => at(time + 1e-12), () => at(time - 1e-12)];
}

/** A fast-path miss (the slow path answers). */
const MISS = Symbol('miss');

/** Finds `target` inside `root` by identity; its path, or null. */
export function findPath(root: any, target: any, limit = 12): PathPart[] | null {
  const path: PathPart[] = [];
  const seen = new Set<any>();
  const visit = (node: any, depth: number): boolean => {
    if (node === target) return true;
    if (!node || typeof node !== 'object' || depth >= limit || seen.has(node)) return false;
    seen.add(node);
    if (Array.isArray(node)) {
      for (let i = 0; i < node.length; i++) {
        path.push(i);
        if (visit(node[i], depth + 1)) return true;
        path.pop();
      }
    } else {
      for (const key of Object.keys(node)) {
        const d = Object.getOwnPropertyDescriptor(node, key);
        if (!d || !('value' in d)) continue;
        path.push(key);
        if (visit(d.value, depth + 1)) return true;
        path.pop();
      }
    }
    return false;
  };
  return visit(root, 0) ? path : null;
}

/**
 * `obj` with its keys in the order JavaScript lists `live`'s (pm-model stores
 * members sorted; callers that iterate answers see the document's order).
 * Keys `live` lacks follow in their given order.
 */
export function keyOrder(obj: any, live: any): any {
  if (!obj || typeof obj !== 'object' || Array.isArray(obj) || !live || typeof live !== 'object') return obj;
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(live)) if (Object.prototype.hasOwnProperty.call(obj, key)) out[key] = obj[key];
  for (const key of Object.keys(obj)) if (!Object.prototype.hasOwnProperty.call(out, key)) out[key] = obj[key];
  return out;
}

const BOUNDS_ORDER = ['x0', 'y0', 'x1', 'y1'];
const isBounds = (v: any) => !!v && typeof v === 'object' && !Array.isArray(v) && Object.keys(v).length === 4 && BOUNDS_ORDER.every(k => k in v);
const isLiveProperty = (v: any) => !!v && typeof v === 'object' && Array.isArray(v.kf) && 'v' in v;

/**
 * `keyOrder` through the whole answer: members follow the live object's order
 * at every depth (an evaluated animated member's order is its value's), and computed
 * bounds (`sizeAnchorBounds`, `fontAnchorBounds`) list x0, y0, x1, y1 as the
 * legacy code builds them. pm-eval lists members sorted (allowed difference 5).
 */
export function deepKeyOrder(obj: any, live: any, depth = 0): any {
  if (!obj || typeof obj !== 'object' || depth > 32) return obj;
  // An evaluated member takes its value's order; a member answered unevaluated
  // (an extension's raw `params`) keeps the property's own order.
  if (isLiveProperty(live) && !isLiveProperty(obj)) live = live.v;
  if (Array.isArray(obj)) {
    if (!Array.isArray(live)) return obj.map(v => deepKeyOrder(v, undefined, depth + 1));
    return obj.map((v, i) => deepKeyOrder(v, live[i], depth + 1));
  }
  if (!live || typeof live !== 'object' || Array.isArray(live)) {
    if (isBounds(obj)) return { x0: obj.x0, y0: obj.y0, x1: obj.x1, y1: obj.y1 };
    let changed = false;
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(obj)) { const v = deepKeyOrder(obj[key], undefined, depth + 1); if (v !== obj[key]) changed = true; out[key] = v; }
    return changed ? out : obj;
  }
  const ordered = keyOrder(obj, live);
  for (const key of Object.keys(ordered)) ordered[key] = deepKeyOrder(ordered[key], Object.prototype.hasOwnProperty.call(live, key) ? live[key] : undefined, depth + 1);
  return ordered;
}

/**
 * `allProps` entries with runs that list members of one object put back in
 * the object's own key order (the legacy listing order; pm-eval's is sorted).
 */
export function liveMemberOrder(L: any, entries: any[]): any[] {
  const out = entries.slice();
  let start = 0;
  const parentKey = (e: any) => (Array.isArray(e?.path) && e.path.length ? JSON.stringify(e.path.slice(0, -1)) : null);
  while (start < out.length) {
    const parent = parentKey(out[start]);
    let end = start + 1;
    while (parent !== null && end < out.length && parentKey(out[end]) === parent) end++;
    if (parent !== null && end - start > 1) {
      const owner = at(L, JSON.parse(parent));
      const order = owner && typeof owner === 'object' ? Object.keys(owner) : [];
      const rank = (e: any) => { const i = order.indexOf(String(e.path[e.path.length - 1])); return i < 0 ? Number.MAX_SAFE_INTEGER : i; };
      const run = out.slice(start, end).map((e, i) => [e, rank(e), i] as const).sort((a, b) => a[1] - b[1] || a[2] - b[2]).map(x => x[0]);
      out.splice(start, end - start, ...run);
    }
    start = end;
  }
  return out;
}

/** The value at a path of own members (null when missing). */
export function at(root: any, path: readonly PathPart[]): any {
  let node = root;
  for (const part of path) {
    if (!node || typeof node !== 'object' || !Object.prototype.hasOwnProperty.call(node, part)) return undefined;
    node = node[part as any];
  }
  return node;
}

interface Context {
  comp: string;
  open: string;
  /** The layer's index in the composition, or its JSON when detached. */
  layer: number | any;
  /** The raw layer (also when detached). */
  raw: any;
  /** Whether the layer is the first with its id (interning addresses the first). */
  first: boolean;
  /** The live layers (as `PM.curComp().layers` reads, proxies in tracking mode). */
  live: any[];
}

interface Indexed { layers: any[]; revision: number; index: Map<any, number>; firstById: Map<string, number> }

export interface NativeStats {
  calls: Record<string, number>;
  /** Structural answers served again at an unchanged shape (`shaped`). */
  shapeHits: number;
  /** Queries that found the mirror missing a structural write and repaired it first. */
  repairs: number;
  fallbacks: Record<string, number>;
  fastPath: number;
  interned: number;
}

export class NativeApi {
  readonly stats: NativeStats = { calls: {}, shapeHits: 0, repairs: 0, fallbacks: {}, fastPath: 0, interned: 0 };
  /**
   * Structural answers (group ancestry, parenting cycles, the 3D switch) by
   * live layer, valid while the document's shape is unchanged
   * (`DocumentBarrier.shapeRevision`) and the same composition is current.
   * The timeline asks for every row's group ancestry at each redraw and the
   * inspector whether each layer could be a parent at each change; through
   * pm-wasm those were a third of the page's time in a drag
   * (docs/engine/perf.md).
   */
  private shapes = new WeakMap<object, Map<string, { shape: number; cur: unknown; live: unknown; index: number; value: unknown }>>();
  private rootsAnswer: { key: string; shape: number; cur: unknown; live: unknown; value: any[] } | null = null;
  /** Set by `layerQuery` and `answer` for `shaped`: the last answer came from
   *  pm-wasm for a layer of the current composition. */
  private indexedQuery = false;
  /** The layer's index in its composition for that answer. */
  private queriedIndex = -1;
  private nativeAnswer = false;
  /** Nesting of `answer` (a legacy fallback may ask further queries). */
  private depth = 0;
  private answeredNatively = false;
  /** Live channel object → interned id (validated on every use). */
  private props = new WeakMap<object, { layer: object; path: PathPart[]; comp: string; id: number; key: unknown; revision: number; cur: unknown }>();
  /** Live layer → its interned transform channels (valid at `revision` with `cur` as `PM.curComp()`). */
  private channels = new WeakMap<object, { comp: string; ids: Map<string, number>; revision: number; cur: unknown }>();
  private refs = new Map<number, number>();
  /**
   * The module instance the interned ids belong to. `reset()` (a replaced
   * instance) moves it on; a registration from an earlier instance names an
   * id the current one may have handed to another channel, so it is ignored
   * when its owner is collected.
   */
  private generation = 0;
  private finalizer = typeof FinalizationRegistry === 'function'
    ? new FinalizationRegistry<{ id: number; generation: number }>(held => this.unref(held.id, held.generation)) : null;
  private indexed = new WeakMap<any[], Indexed>();
  /** `expressionErrors` for channels evaluated natively (live objects). */
  readonly errors = new WeakMap<object, string>();
  private definitions = new Map<string, string>();
  /** Where text layout gets registry answers and subsets (null: none). */
  fonts: FontLoader | null = null;
  /** Called when a dual answer differed because the mirror missed a write. */
  onStale: ((op: string, paths: string[]) => void) | null = null;

  constructor(
    private PM: Legacy,
    private wasm: () => PmWasm | null,
    private barrier: () => DocumentBarrier | null,
    private unwrap: <T>(v: T) => T,
    /** The legacy implementations, captured before routing. */
    private legacy: Legacy,
    private dual: DualAnswers | null,
  ) {}

  // ── plumbing ──────────────────────────────────────────────────────────

  /** The module, flushed and ready, or null (then the legacy code answers). */
  private ready(): PmWasm | null {
    const wasm = this.wasm();
    const barrier = this.barrier();
    if (!wasm || wasm.dead || !barrier) return null;
    try { barrier.flush(); } catch (error) { console.warn('[engine-doc] flush failed', error); return null; }
    return wasm.dead ? null : wasm;
  }

  private count(op: string) { this.stats.calls[op] = (this.stats.calls[op] ?? 0) + 1; }
  private fell(op: string, why: string) { const k = `${op}:${why}`; this.stats.fallbacks[k] = (this.stats.fallbacks[k] ?? 0) + 1; }

  private index(layers: any[]): Indexed {
    const revision = this.barrier()?.revision ?? -1;
    let cached = this.indexed.get(layers);
    if (!cached || cached.revision !== revision || cached.layers.length !== layers.length) {
      const index = new Map<any, number>();
      const firstById = new Map<string, number>();
      layers.forEach((layer, i) => {
        if (!index.has(layer)) index.set(layer, i);
        if (layer && typeof layer.id === 'string' && !firstById.has(layer.id)) firstById.set(layer.id, i);
      });
      cached = { layers, revision, index, firstById };
      this.indexed.set(layers, cached);
    }
    return cached;
  }

  /** `PM.curComp()`'s id and the open composition's. */
  private comps(): { comp: string; open: string; cur: any; liveCur: any } | null {
    const proj = this.unwrap(this.PM.proj);
    if (!proj || typeof proj.compId !== 'string') return null;
    const liveCur = this.PM.curComp?.() ?? this.PM.proj;
    const cur = this.unwrap(liveCur);
    if (cur === proj) return { comp: proj.compId, open: proj.compId, cur, liveCur };
    const id = typeof cur?.id === 'string' ? cur.id : null;
    if (!id || this.unwrap(proj.comps?.[id]) !== cur) return null;
    return { comp: id, open: proj.compId, cur, liveCur };
  }

  private context(L: any): Context | null {
    const c = this.comps();
    if (!c) return null;
    const raw = this.unwrap(L);
    if (!raw || typeof raw !== 'object') return null;
    const layers: any[] = Array.isArray(c.cur.layers) ? c.cur.layers : [];
    const live: any[] = Array.isArray(c.liveCur?.layers) ? c.liveCur.layers : layers;
    const idx = this.index(layers);
    let i = idx.index.get(raw);
    if (i !== undefined && layers[i] !== raw) { this.indexed.delete(layers); i = this.index(layers).index.get(raw); }
    if (i === undefined) return { comp: c.comp, open: c.open, layer: JSON.parse(JSON.stringify(raw)), raw, first: false, live };
    // The mirror must hold this layer at this index (the shadow is exactly
    // what it holds). If not, a structural write bypassed the barrier: repair
    // before answering rather than answer for another layer.
    const barrier = this.barrier();
    const mirrored = barrier ? lookup(barrier.shadow, ['comps', c.comp, 'layers', i, 'id']) : null;
    if (barrier && (!mirrored?.exists || mirrored.value !== raw.id)) {
      this.stats.repairs += 1;
      const missed = barrier.repair('structuralDiff');
      if (missed.length) this.onStale?.('context', missed);
    }
    const first = typeof raw.id === 'string' && this.index(layers).firstById.get(raw.id) === i;
    return { comp: c.comp, open: c.open, layer: i, raw, first, live };
  }

  private query<T>(op: string, request: Record<string, unknown>, map?: (v: any) => T): Native<T> {
    const wasm = this.ready();
    if (!wasm) return { fallback: 'unavailable' };
    let reply: any;
    try {
      refreshModels(this.PM, wasm);
      reply = wasm.query({ op, ...request });
      // 3D geometry met model assets the mirror lacks: add them and ask again.
      if (reply?.models) reply = resolveModels(this.PM, wasm, () => wasm.query({ op, ...request }), reply);
    } catch (error) { console.warn('[engine-doc] pm-wasm query failed', error); return { fallback: 'trap' }; }
    if (reply?.fonts && this.fonts) {
      // Text layout met fonts the mirror lacks: load them (at once from a
      // synchronous registry, in the background otherwise) and ask again.
      try {
        reply = this.fonts.resolve(wasm, () => wasm.query({ op, ...request }), (r: any) => !!r?.fonts) ?? reply;
      } catch (error) { console.warn('[engine-doc] pm-wasm query failed', error); return { fallback: 'trap' }; }
    }
    if ('ok' in reply) return { value: map ? map(reply.ok) : reply.ok };
    if (reply.throw) {
      const error = new (reply.throw.kind === 'TypeError' ? TypeError : reply.throw.kind === 'RangeError' ? RangeError : Error)(reply.throw.message);
      return { threw: error };
    }
    if (typeof reply.unsupported === 'string') {
      // Content the engine cannot answer for (a 3D model it cannot load):
      // named, counted, and refused by strict hosts; never the missing box.
      console.warn(`[engine-doc] ${op}: ${reply.unsupported}`);
      return { fallback: 'unsupported' };
    }
    if (reply.host) return { fallback: 'host' };
    if (reply.fonts) return { fallback: 'fonts' };
    if (reply.ink) return { fallback: 'ink' };
    return { fallback: `error:${reply.error ?? 'unknown'}` };
  }

  private layerQuery<T>(op: string, L: any, extra: Record<string, unknown>, map?: (v: any, ctx: Context) => T): Native<T> {
    const ctx = this.context(L);
    if (!ctx) return { fallback: 'context' };
    if (this.depth === 1) {
      this.indexedQuery = typeof ctx.layer === 'number';
      this.queriedIndex = typeof ctx.layer === 'number' ? ctx.layer : -1;
    }
    return this.query(op, { comp: ctx.comp, open: ctx.open, layer: ctx.layer, ...extra }, map ? (v) => map(v, ctx) : undefined);
  }

  /**
   * Native answer (or legacy fallback), compared with the legacy answer in
   * dual mode. `perturbed` (bounds and picks) gives the legacy answers under
   * a 1e-12 s or 1e-7 px perturbation: when one of them differs from the
   * legacy answer, the case is fragile (amendment 20). `fragileWhen` names
   * the cases fragile by construction, which no nudge shows: a point on a
   * layer's edge, a group matrix whose condition exceeds 1e5.
   */
  private answer<T>(op: string, native: () => Native<T>, legacy: () => T, describe: () => Record<string, unknown>, perturbed?: Array<() => T>, fragileWhen?: () => boolean): T {
    this.depth += 1;
    try {
      const value = this.answerAt(op, native, legacy, describe, perturbed, fragileWhen);
      if (this.depth === 1) this.nativeAnswer = this.answeredNatively;
      return value;
    } finally {
      this.depth -= 1;
      this.answeredNatively = false;
    }
  }

  private answerAt<T>(op: string, native: () => Native<T>, legacy: () => T, describe: () => Record<string, unknown>, perturbed?: Array<() => T>, fragileWhen?: () => boolean): T {
    this.count(op);
    const run = (): Native<T> => {
      try { return native(); } catch (error) { console.warn(`[engine-doc] ${op} failed natively`, error); return { fallback: 'exception' }; }
    };
    let got = run();
    if ('fallback' in got) {
      this.fell(op, got.fallback);
      return legacy();
    }
    if (this.dual) {
      const dual = this.dual;
      let expected: { value?: T; threw?: unknown };
      try { expected = { value: legacy() }; } catch (error) { expected = { threw: error }; }
      const answer = (g: Native<T>) => ('value' in g ? { value: g.value } : { threw: (g as { threw: Error }).threw });
      let verdict = dual.classify(op, answer(got), expected, this.unwrap);
      if (verdict.path !== null) {
        // A write the barrier did not see makes the mirror stale: that is a
        // barrier gap, not an evaluation difference. Repair, then ask again.
        const missed = this.barrier()?.repair('structuralDiff') ?? [];
        if (missed.length) {
          dual.stats.stale[op] = (dual.stats.stale[op] ?? 0) + 1;
          this.onStale?.(op, missed);
          const again = run();
          if ('fallback' in again) { this.fell(op, again.fallback); return legacy(); }
          got = again;
          verdict = dual.classify(op, answer(got), expected, this.unwrap);
        }
      }
      let fragile = false;
      if ((perturbed?.length || fragileWhen) && !('threw' in expected)) {
        let structural = false;
        try { structural = fragileWhen?.() === true; } catch { structural = false; }
        fragile = structural || !!perturbed?.some(other => {
          let value: { value?: T; threw?: unknown };
          try { value = { value: other() }; } catch (error) { value = { threw: error }; }
          return dual.classify(op, value, expected, this.unwrap).path !== null;
        });
        if (fragile) dual.noteFragile(op);
      }
      if (verdict.path !== null && fragile) {
        dual.allow(op, 'fragile');
        dual.record(op, null, answer(got), expected, describe);
      } else {
        if (verdict.allowed) dual.allow(op, verdict.allowed);
        dual.record(op, verdict.path, answer(got), expected, describe);
      }
    }
    if ('threw' in got) throw got.threw;
    this.answeredNatively = true;
    return got.value;
  }

  /**
   * Whether `raw` is a layer of the current composition that the mirror holds
   * at the same index (what `context` checks before every query; a miss lets
   * the query repair a write the barrier did not see).
   */
  private mirrored(raw: any, i: number, comp: string): boolean {
    const layers = (this.unwrap(this.PM.curComp?.() ?? this.PM.proj) as any)?.layers;
    if (!Array.isArray(layers) || layers[i] !== raw) return false;
    const mirrored = lookup(this.barrier()!.shadow, ['comps', comp, 'layers', i, 'id']);
    return mirrored.exists && mirrored.value === raw.id;
  }

  /** The composition structural answers are valid for, flushed first; null when there is none. */
  private shapeStamp(): { shape: number; cur: unknown; live: unknown; comp: string } | null {
    if (this.dual) return null;
    const barrier = this.barrier();
    const wasm = this.wasm();
    if (!barrier || !wasm || wasm.dead) return null;
    try { barrier.flush(); } catch { return null; }
    const c = this.comps();
    if (!c) return null;
    return { shape: barrier.shapeRevision, cur: c.cur, live: c.liveCur, comp: c.comp };
  }

  /**
   * A structural answer for `L` (the answer depends on the composition's
   * shape only, `changesShape`), from the memo while the shape and the
   * current composition are unchanged. Only native answers for layers of
   * the current composition are kept: a layer outside it can change without
   * the barrier seeing it. Dual mode always asks (it compares every answer).
   */
  private shaped<T>(op: string, L: any, key: string, compute: () => T, copy: (v: T) => T = v => v): T {
    const raw = L && typeof L === 'object' ? this.unwrap(L) : null;
    const stamp = raw ? this.shapeStamp() : null;
    if (!stamp) return compute();
    const memo = this.shapes.get(raw);
    const hit = memo?.get(key);
    // At an unchanged shape the layer is where it was: one look at that index.
    if (hit && hit.shape === stamp.shape && hit.cur === stamp.cur && hit.live === stamp.live && this.mirrored(raw, hit.index, stamp.comp)) {
      this.count(op);
      this.stats.shapeHits += 1;
      return copy(hit.value as T);
    }
    this.indexedQuery = false;
    this.queriedIndex = -1;
    this.nativeAnswer = false;
    const value = compute();
    const shape = this.barrier()?.shapeRevision;
    if (this.indexedQuery && this.nativeAnswer && shape !== undefined && this.queriedIndex >= 0) {
      const entries = memo ?? new Map();
      if (!memo) this.shapes.set(raw, entries);
      entries.set(key, { shape, cur: stamp.cur, live: stamp.live, index: this.queriedIndex, value: copy(value) });
    }
    return value;
  }

  private intern(wasm: PmWasm, address: Parameters<PmWasm['intern']>[0], owner: object): number {
    const id = wasm.intern(address);
    if (id >= 0) {
      this.refs.set(id, (this.refs.get(id) ?? 0) + 1);
      this.finalizer?.register(owner, { id, generation: this.generation });
      this.stats.interned += 1;
    }
    return id;
  }

  private unref(id: number, generation: number) {
    // An id of a replaced instance, or one this instance never counted.
    if (generation !== this.generation || !this.refs.has(id)) return;
    const n = this.refs.get(id)! - 1;
    if (n > 0) { this.refs.set(id, n); return; }
    this.refs.delete(id);
    const wasm = this.wasm();
    if (wasm && !wasm.dead) try { wasm.release(id); } catch { /* the instance is gone */ }
  }

  /** Drops every interned id (after the module is replaced). */
  reset(): void {
    this.generation += 1;
    this.props = new WeakMap();
    this.channels = new WeakMap();
    this.refs.clear();
    this.indexed = new WeakMap();
    this.shapes = new WeakMap();
    this.rootsAnswer = null;
  }

  private noteError(prop: any, wasm: PmWasm, id: number) {
    if (!prop || typeof prop !== 'object') return;
    const raw = this.unwrap(prop);
    if (!raw.expr) { this.errors.delete(raw); return; }
    const message = wasm.expressionError(id);
    if (message == null) this.errors.delete(raw);
    else this.errors.set(raw, message);
  }

  private evById(wasm: PmWasm, id: number, t: number): Native<any> {
    const r = wasm.ev(id, t);
    if (r.threw !== undefined) return { threw: new TypeError(r.threw) };
    this.stats.fastPath += 1;
    return { value: r.value };
  }

  // ── anim ──────────────────────────────────────────────────────────────

  /** The fast path's validity stamp: the mirror revision and `PM.curComp()`. */
  private stamp(): { revision: number; cur: unknown } | null {
    const barrier = this.barrier();
    if (!barrier) return null;
    // flush() returns at once when nothing waits (it also notices writes a
    // plain transaction has not announced yet).
    barrier.flush();
    return { revision: barrier.revision, cur: this.PM.scope?.length ? this.unwrap(this.PM.curComp()) : null };
  }

  /** `pm_ev` for an id validated at this revision; MISS when it must take the slow path. */
  private fastValue(id: number, T: number, prop: any): unknown {
    const wasm = this.wasm();
    if (!wasm || wasm.dead) return MISS;
    let n: number;
    try { n = wasm.x.pm_ev(id, T); } catch { wasm.dead = true; return MISS; }
    if (n !== n && wasm.x.pm_kind() !== 0) return MISS; // not a number, or a throw: ask the slow path
    this.stats.fastPath += 1;
    const raw = prop && typeof prop === 'object' ? this.unwrap(prop) : null;
    if (raw) {
      if (raw.expr) { const m = wasm.expressionError(id); if (m == null) this.errors.delete(raw); else this.errors.set(raw, m); }
      else this.errors.delete(raw);
    }
    return n;
  }

  ev(L: any, key: any, T: any): any {
    if (!this.dual && typeof key === 'string' && typeof T === 'number' && L && typeof L === 'object') {
      const raw = this.unwrap(L);
      const entry = this.channels.get(raw);
      const stamp = entry && this.stamp();
      const id = stamp && entry.revision === stamp.revision && entry.cur === stamp.cur ? entry.ids.get(key) : undefined;
      if (id !== undefined) {
        const value = this.fastValue(id, T, raw.p?.[key]);
        if (value !== MISS) { this.stats.calls.ev = (this.stats.calls.ev ?? 0) + 1; return value; }
      }
    }
    return this.answer('ev', () => {
      const wasm = this.ready();
      if (!wasm) return { fallback: 'unavailable' };
      const ctx = this.context(L);
      if (!ctx) return { fallback: 'context' };
      if (typeof key !== 'string' || typeof T !== 'number' || !ctx.first || typeof ctx.layer !== 'number') {
        return this.query('ev', { comp: ctx.comp, open: ctx.open, layer: ctx.layer, key: String(key), t: T });
      }
      const stamp = this.stamp()!;
      let entry = this.channels.get(ctx.raw);
      if (!entry || entry.comp !== ctx.comp) { entry = { comp: ctx.comp, ids: new Map(), revision: stamp.revision, cur: stamp.cur }; this.channels.set(ctx.raw, entry); }
      entry.revision = stamp.revision;
      entry.cur = stamp.cur;
      let id = entry.ids.get(key);
      if (id === undefined) {
        id = this.intern(wasm, { comp: ctx.comp, layer: ctx.raw.id, key, via: 'channel' }, ctx.raw);
        if (id < 0) return this.query('ev', { comp: ctx.comp, open: ctx.open, layer: ctx.layer, key, t: T });
        entry.ids.set(key, id);
      }
      const got = this.evById(wasm, id, T);
      this.noteError(ctx.raw.p?.[key], wasm, id);
      return got;
    }, () => this.legacy.ev(L, key, T), () => ({ layer: L?.id, key, t: T }));
  }

  evP(L: any, prop: any, T: any, key: any): any {
    if (!prop || typeof prop !== 'object') return this.legacy.evP(L, prop, T, key);
    if (!this.dual && typeof T === 'number') {
      const rawProp = this.unwrap(prop);
      const entry = this.props.get(rawProp);
      const stamp = entry && this.stamp();
      if (stamp && entry.revision === stamp.revision && entry.cur === stamp.cur && entry.key === key && entry.layer === this.unwrap(L)) {
        const value = this.fastValue(entry.id, T, rawProp);
        if (value !== MISS) { this.stats.calls.evP = (this.stats.calls.evP ?? 0) + 1; return value; }
      }
    }
    return this.answer('evP', () => {
      const wasm = this.ready();
      if (!wasm) return { fallback: 'unavailable' };
      const ctx = this.context(L);
      if (!ctx) return { fallback: 'context' };
      const rawProp = this.unwrap(prop);
      const stamp = this.stamp()!;
      let entry = this.props.get(rawProp);
      const valid = entry && entry.layer === ctx.raw && entry.comp === ctx.comp && entry.key === key && at(ctx.raw, entry.path) === rawProp;
      if (!valid) {
        const path = findPath(ctx.raw, rawProp);
        if (!path || !ctx.first || typeof ctx.layer !== 'number' || typeof key !== 'string' || typeof T !== 'number') {
          const request = path
            ? { comp: ctx.comp, open: ctx.open, layer: ctx.layer, path, key: String(key), t: T }
            : { comp: ctx.comp, open: ctx.open, layer: ctx.layer, prop: JSON.parse(JSON.stringify(rawProp)), key: String(key), t: T };
          return this.query('evP', request);
        }
        const id = this.intern(wasm, { comp: ctx.comp, layer: ctx.raw.id, key, via: path }, rawProp);
        if (id < 0) return this.query('evP', { comp: ctx.comp, open: ctx.open, layer: ctx.layer, path, key, t: T });
        entry = { layer: ctx.raw, path, comp: ctx.comp, id, key, revision: stamp.revision, cur: stamp.cur };
        this.props.set(rawProp, entry);
      }
      entry!.revision = stamp.revision;
      entry!.cur = stamp.cur;
      const got = this.evById(wasm, entry!.id, T);
      this.noteError(rawProp, wasm, entry!.id);
      return got;
    }, () => this.legacy.evP(L, prop, T, key), () => ({ layer: L?.id, key, t: T }));
  }

  active(L: any, T: any): boolean {
    return this.answer('active', () => this.layerQuery('active', L, { t: T }), () => this.legacy.active(L, T), () => ({ layer: L?.id, t: T }));
  }

  findProp(L: any, key: any): any {
    return this.answer('findProp', () => this.layerQuery('findProp', L, { key: String(key) }, (path) => path == null ? null : at(L, path) ?? null),
      () => this.legacy.findProp(L, key), () => ({ layer: L?.id, key }));
  }

  allProps(L: any): any[] {
    return this.answer('allProps', () => {
      this.defineFor(L);
      return this.layerQuery('allProps', L, {}, (entries: any[]) => liveMemberOrder(L, entries).map(e => ({
        key: e.key,
        prop: Array.isArray(e.path) ? at(L, e.path) : e.path && typeof e.path === 'object' ? e.path.value : undefined,
        label: e.label,
        group: e.group,
      })));
    }, () => this.legacy.allProps(L), () => ({ layer: L?.id }));
  }

  /** Hands pm-wasm the labels of an extension layer's definition (`PM.layerDefinition`). */
  private defineFor(L: any) {
    const raw = this.unwrap(L);
    if (raw?.type !== 'extension' || typeof raw.d?.definition !== 'string') return;
    const definition = this.PM.layerDefinition?.(raw.d.definition);
    const label = typeof definition?.label === 'string' ? definition.label : undefined;
    const params: Record<string, string> = {};
    for (const item of definition?.params || []) if (typeof item?.k === 'string' && typeof item.label === 'string') params[item.k] = item.label;
    const signature = JSON.stringify([label, params]);
    if (this.definitions.get(raw.d.definition) === signature) return;
    this.wasm()?.define({ id: raw.d.definition, label, params });
    this.definitions.set(raw.d.definition, signature);
  }

  hasKeyAt(L: any, prop: any, T: any): any {
    return this.answer('hasKeyAt', () => {
      const ctx = this.context(L);
      if (!ctx) return { fallback: 'context' };
      const path = findPath(ctx.raw, this.unwrap(prop));
      if (!path) return { fallback: 'detached-prop' };
      return this.query('hasKeyAt', { comp: ctx.comp, open: ctx.open, layer: ctx.layer, path, t: T }, (i) => i == null ? null : prop.kf[i] ?? null);
    }, () => this.legacy.hasKeyAt(L, prop, T), () => ({ layer: L?.id, t: T }));
  }

  wouldCycle(L: any, parentId: any): boolean {
    const ask = () => this.answer('wouldCycle', () => this.layerQuery('wouldCycle', L, { parent: parentId === undefined ? { $u: 1 } : parentId }),
      () => this.legacy.wouldCycle(L, parentId), () => ({ layer: L?.id, parentId }));
    if (parentId != null && typeof parentId !== 'string') return ask();
    return this.shaped('wouldCycle', L, `wouldCycle:${parentId === undefined ? 'u' : parentId === null ? 'n' : `s${parentId}`}`, ask);
  }

  resolveContent(L: any, T: any = this.PM.time): any {
    return this.answer('resolveContent', () => this.layerQuery('resolveContent', L, { t: T }, (d: any) => deepKeyOrder(d, this.unwrap(L)?.d)),
      () => this.legacy.resolveContent(L, T), () => ({ layer: L?.id, t: T }));
  }

  localMatrix(L: any, T: any): number[] {
    return this.answer('localMatrix', () => this.layerQuery('localMatrix', L, { t: T }), () => this.legacy.localMatrix(L, T), () => ({ layer: L?.id, t: T }));
  }

  worldMatrix(L: any, T: any): number[] {
    return this.answer('worldMatrix', () => this.layerQuery('worldMatrix', L, { t: T }), () => this.legacy.worldMatrix(L, T), () => ({ layer: L?.id, t: T }));
  }

  transformParentMatrix(L: any, T: any, ...rest: any[]): number[] {
    return this.answer('transformParentMatrix', () => {
      const extra: Record<string, unknown> = { t: T };
      if (rest.length) {
        const parent = this.unwrap(rest[0]);
        if (parent == null) extra.parent = null;
        else {
          const ctx = this.context(parent);
          if (!ctx || typeof ctx.layer !== 'number') return { fallback: 'detached-parent' };
          extra.parent = ctx.layer;
        }
      }
      return this.layerQuery('transformParentMatrix', L, extra);
    }, () => this.legacy.transformParentMatrix(L, T, ...rest), () => ({ layer: L?.id, t: T }));
  }

  mul(a: readonly number[], b: readonly number[]): number[] {
    return this.answer('mul', () => this.query('mul', { a: [...a], b: [...b] }), () => this.legacy.mul(a, b), () => ({}));
  }

  // ── groups ────────────────────────────────────────────────────────────

  groupBounds(group: any, time: number): any {
    return this.answer('groups.bounds', () => this.layerQuery('groupBounds', group, { t: time, ...geometryRequest(this.PM) }),
      () => this.legacy.groupBounds(group, time), () => ({ layer: group?.id, t: time }),
      this.dual ? timePerturbations(time, t => this.legacy.groupBounds(group, t)) : undefined,
      this.dual ? () => this.illConditioned(group, time) : undefined);
  }

  groupAncestors(L: any, layers?: any[]): any[] {
    if (layers !== undefined) {
      const c = this.comps();
      if (!c || this.unwrap(layers) !== c.cur.layers) return this.legacy.groupAncestors(L, layers);
    }
    return this.shaped('groups.ancestors', L, 'groupAncestors',
      () => this.answer('groups.ancestors', () => this.layerQuery('groupAncestors', L, {}, (indexes: number[], ctx) => indexes.map(i => ctx.live[i])),
        () => this.legacy.groupAncestors(L, layers), () => ({ layer: L?.id })),
      list => Array.isArray(list) ? list.slice() : list);
  }

  groupSpan(group: any): any {
    return this.answer('groups.span', () => this.layerQuery('groupSpan', group, {}), () => this.legacy.groupSpan(group), () => ({ layer: group?.id }));
  }

  expandGroups(ids: any[]): any[] {
    return this.answer('groups.expand', () => {
      const c = this.comps();
      if (!c) return { fallback: 'context' };
      return this.query('expandGroups', { comp: c.open, open: c.open, ids: [...ids] });
    }, () => this.legacy.expandGroups(ids), () => ({ ids }));
  }

  transformRoots(ids: any[]): any[] {
    const ask = () => this.answer('groups.transformRoots', () => {
      const c = this.comps();
      if (!c) return { fallback: 'context' };
      if (this.depth === 1) this.indexedQuery = true;
      return this.query('transformRoots', { comp: c.comp, open: c.open, ids: [...ids] }, (indexes: number[]) => indexes.map(i => c.liveCur.layers[i]));
    }, () => this.legacy.transformRoots(ids), () => ({ ids }));
    // The same selection's roots at an unchanged shape (each move of a drag asks).
    const stamp = Array.isArray(ids) && ids.every(id => typeof id === 'string') ? this.shapeStamp() : null;
    if (!stamp) return ask();
    const key = JSON.stringify(ids);
    const hit = this.rootsAnswer;
    if (hit && hit.key === key && hit.shape === stamp.shape && hit.cur === stamp.cur && hit.live === stamp.live) {
      this.count('groups.transformRoots');
      this.stats.shapeHits += 1;
      return hit.value.slice();
    }
    this.indexedQuery = false;
    this.nativeAnswer = false;
    const value = ask();
    const shape = this.barrier()?.shapeRevision;
    if (this.indexedQuery && this.nativeAnswer && shape !== undefined && Array.isArray(value)) {
      this.rootsAnswer = { key, shape, cur: stamp.cur, live: stamp.live, value: value.slice() };
    }
    return value;
  }

  // ── space3d ───────────────────────────────────────────────────────────

  local3D(L: any, T: number): number[] {
    return this.answer('space3d.local3D', () => this.layerQuery('local3D', L, { t: T }), () => this.legacy.local3D(L, T), () => ({ layer: L?.id, t: T }));
  }

  parent3D(L: any, T: number, ...rest: any[]): number[] {
    return this.answer('space3d.parent3D', () => {
      const extra: Record<string, unknown> = { t: T };
      if (rest.length) {
        const parent = this.unwrap(rest[0]);
        if (parent == null) extra.parent = null;
        else {
          const ctx = this.context(parent);
          if (!ctx || typeof ctx.layer !== 'number') return { fallback: 'detached-parent' };
          extra.parent = ctx.layer;
        }
      }
      return this.layerQuery('parent3D', L, extra);
    }, () => this.legacy.parent3D(L, T, ...rest), () => ({ layer: L?.id, t: T }));
  }

  world3D(L: any, T: number, positioning = false): number[] {
    return this.answer('space3d.world3D', () => this.layerQuery('world3D', L, { t: T, positioning }), () => this.legacy.world3D(L, T, positioning), () => ({ layer: L?.id, t: T }));
  }

  is3DLayer(L: any): boolean {
    return this.shaped('space3d.is3DLayer', L, 'is3DLayer',
      () => this.answer('space3d.is3DLayer', () => this.layerQuery('is3DLayer', L, {}), () => this.legacy.is3DLayer(L), () => ({ layer: L?.id })));
  }

  perspectiveAmount(L: any, T: number): number {
    return this.answer('space3d.perspectiveAmount', () => this.layerQuery('perspectiveAmount', L, { t: T }), () => this.legacy.perspectiveAmount(L, T), () => ({ layer: L?.id, t: T }));
  }

  planeMatrix(L: any, T: number, positioning = false): number[] {
    return this.answer('space3d.planeMatrix', () => this.layerQuery('planeMatrix', L, { t: T, positioning }), () => this.legacy.planeMatrix(L, T, positioning), () => ({ layer: L?.id, t: T }));
  }

  projectPoint(m: readonly number[], p: { x: number; y: number }): { x: number; y: number } {
    return this.answer('space3d.projectPoint', () => this.query('projectPoint', { m: [...m], x: p.x, y: p.y }), () => this.legacy.projectPoint(m, p), () => ({}));
  }

  inversePlane(m: readonly number[]): number[] | null {
    return this.answer('space3d.inversePlane', () => this.query('inversePlane', { m: [...m] }), () => this.legacy.inversePlane(m), () => ({}));
  }

  planeContains(L: any, T: number, x: number, y: number, b: any): boolean {
    return this.answer('space3d.planeContains', () => this.layerQuery('planeContains', L, { t: T, x, y, bounds: { x0: b?.x0, y0: b?.y0, x1: b?.x1, y1: b?.y1 } }),
      () => this.legacy.planeContains(L, T, x, y, b), () => ({ layer: L?.id, t: T, x, y }));
  }

  // ── render ────────────────────────────────────────────────────────────

  bounds(L: any, T: any): any {
    return this.answer('gl.bounds', () => this.layerQuery('bounds', L, { t: T, ...geometryRequest(this.PM) }), () => this.legacy.bounds(L, T), () => ({ layer: L?.id, t: T }),
      this.dual ? timePerturbations(T, t => this.legacy.bounds(L, t)) : undefined,
      this.dual ? () => this.illConditioned(L, T) : undefined);
  }

  /**
   * Amendment 20's ill-conditioned class: a group whose world matrix has a
   * condition above 1e5 maps its members through a near-singular inverse,
   * which amplifies last-bit differences (as the layer vectors class it).
   */
  private illConditioned(L: any, T: unknown): boolean {
    if (this.unwrap(L)?.type !== 'group' || typeof T !== 'number') return false;
    const [a, b, c, d] = this.legacy.worldMatrix(L, T) as number[];
    return (a! * a! + b! * b! + c! * c! + d! * d!) / Math.abs(a! * d! - b! * c!) > 1e5;
  }

  /**
   * Amendment 20's edge class: the point lies on some layer's edge in that
   * layer's plane (within 1e-6 relative), where a zero-width layer is hit
   * only exactly on its line and no nudge of the point shows it.
   */
  private onLayerEdge(x: number, y: number, T: number): boolean {
    const layers: any[] = this.PM.proj?.layers ?? [];
    const e = (v: number) => 1e-6 * (1 + Math.abs(v));
    const inside = (v: number, lo: number, hi: number) => v >= lo - e(v) && v <= hi + e(v);
    const near = (v: number, lo: number, hi: number) => inside(v, lo, hi) && (Math.abs(v - lo) <= e(v) || Math.abs(v - hi) <= e(v));
    return layers.some(layer => {
      try {
        const b = this.legacy.bounds(layer, T);
        const inv = b && this.legacy.inversePlane(this.legacy.planeMatrix(layer, T));
        if (!inv) return false;
        const q = this.legacy.projectPoint(inv, { x, y });
        return (near(q.x, b.x0, b.x1) && inside(q.y, b.y0, b.y1)) || (near(q.y, b.y0, b.y1) && inside(q.x, b.x0, b.x1));
      } catch { return false; }
    });
  }

  pick(x: number, y: number, T: number, options: { includeLocked?: boolean } = {}): any {
    return this.answer('gl.pick', () => {
      const c = this.comps();
      if (!c) return { fallback: 'context' };
      // 3D layers are picked on the engine's geometry with the editor's view
      // camera and Local View (core/scene3d/engine-geometry.ts).
      const live = this.PM.proj.layers;
      return this.query('pick', { comp: c.open, open: c.open, x, y, t: T, includeLocked: !!options?.includeLocked, localView: null, ...geometryRequest(this.PM) },
        (hit: number[] | null) => hit == null ? null : live[hit[0]!] ?? null);
    }, () => this.legacy.pick(x, y, T, options), () => ({ x, y, t: T }),
      this.dual ? [
        ...timePerturbations(T, t => this.legacy.pick(x, y, t, options)),
        ...[[1e-7, 0], [-1e-7, 0], [0, 1e-7], [0, -1e-7]].map(([dx, dy]) => () => this.legacy.pick(x + dx!, y + dy!, T, options)),
      ] : undefined,
      this.dual ? () => this.onLayerEdge(x, y, T) : undefined);
  }

  /**
   * `Box3.expandByObject` of each 3D object layer in the composition's
   * runtime at `T` (`viewportBounds`), from pm-wasm's geometry: `[minX,
   * minY, minZ, maxX, maxY, maxZ]` per id, null for an id without one.
   * `legacy` is today's three.js answer: it runs only where every other
   * query's legacy fallback runs (counted in `stats.fallbacks`, refused by
   * strict hosts), and in dual mode as the reference.
   */
  objectBoxes3d(ids: readonly string[], T: number, legacy: () => Record<string, number[] | null>): Record<string, number[] | null> {
    return this.answer('objectBoxes3d', () => {
      const c = this.comps();
      if (!c) return { fallback: 'context' };
      return this.query<Record<string, number[] | null>>('objectBoxes3d', { comp: c.comp, open: c.open, ids: [...ids], t: T, ...geometryRequest(this.PM) });
    }, legacy, () => ({ ids: [...ids], t: T }));
  }

  textLayout(L: any, T: number): any {
    return this.answer('textLayout', () => this.layerQuery('textLayout', L, { t: T }), () => this.legacy.textLayout(L, T), () => ({ layer: L?.id, t: T }));
  }

  /**
   * `render.raster()` for a text layer at scale 1: the renderer's pixels with
   * pm-wasm's placement (raster box, anchor, selection; the font anchor
   * applied, as the legacy surface has it).
   */
  rasterSurface(surface: any, L: any, T: number): any {
    const raw = this.unwrap(L);
    if (raw?.type !== 'text' || !surface || typeof surface !== 'object') return surface;
    const pick = (s: any) => ({ anchorX: s.anchorX, anchorY: s.anchorY, selection: s.selection ? { x0: s.selection.x0, y0: s.selection.y0, x1: s.selection.x1, y1: s.selection.y1, w: s.selection.w, h: s.selection.h } : undefined });
    const placement = this.answer('rasterMetrics', () => this.layerQuery('textRaster', L, { t: T }, (r: any) => pick(r)), () => pick(surface), () => ({ layer: raw.id, t: T }));
    return { ...surface, anchorX: placement.anchorX, anchorY: placement.anchorY, selection: placement.selection ? { ...surface.selection, ...placement.selection } : surface.selection };
  }

  // ── editor helpers ────────────────────────────────────────────────────

  pathValues(L: any, path: any, time: number, prefix?: string): any {
    return this.answer('pathValues', () => {
      const ctx = this.context(L);
      if (!ctx) return { fallback: 'context' };
      const index = (Array.isArray(ctx.raw.d?.paths) ? ctx.raw.d.paths : []).indexOf(this.unwrap(path));
      if (index < 0) return { fallback: 'detached-path' };
      const live = this.unwrap(path);
      return this.query('pathValues', { comp: ctx.comp, open: ctx.open, layer: ctx.layer, index, t: time, ...(prefix === undefined ? {} : { prefix }) },
        (v: any) => {
          const out = keyOrder(v, live?.p);
          // `vertices` last, each in its own `p` order, as pathValues builds them.
          const vertices = Array.isArray(v?.vertices) ? v.vertices.map((x: any, i: number) => keyOrder(x, live?.vertices?.[i]?.p)) : v?.vertices;
          delete out.vertices;
          out.vertices = vertices;
          return out;
        });
    }, () => this.legacy.pathValues(L, path, time, prefix), () => ({ layer: L?.id, t: time }));
  }

  groupMatrix(L: any, path: any, time: number): number[] {
    return this.answer('groupMatrix', () => {
      const ctx = this.context(L);
      if (!ctx) return { fallback: 'context' };
      const index = (Array.isArray(ctx.raw.d?.paths) ? ctx.raw.d.paths : []).indexOf(this.unwrap(path));
      if (index < 0) return { fallback: 'detached-path' };
      return this.query('groupMatrix', { comp: ctx.comp, open: ctx.open, layer: ctx.layer, index, t: time });
    }, () => this.legacy.groupMatrix(L, path, time), () => ({ layer: L?.id, t: time }));
  }

  evaluatedValue(L: any, value: any, time: number, path: string): any {
    // `isProperty(value) ? PM.evP(...) : value` (content-properties.ts).
    if (!(!!value && typeof value === 'object' && Array.isArray(value.kf) && 'v' in value)) return value;
    return this.evP(L, value, time, path);
  }

  validMatteSource(layers: any[], layer: any, id: any): boolean {
    const c = this.comps();
    if (!c || this.unwrap(layers) !== c.cur.layers) return this.legacy.validMatteSource(layers, layer, id);
    return this.answer('validMatteSource', () => this.layerQuery('validMatteSource', layer, { id: id === undefined ? { $u: 1 } : id }),
      () => this.legacy.validMatteSource(layers, layer, id), () => ({ layer: layer?.id, id }));
  }

  countTextUnits(lines: Array<{ text: string }>, unit: string): number {
    return this.answer('countTextUnits', () => this.query('countTextUnits', { lines: lines.map(l => String(l?.text ?? '')), unit }),
      () => this.legacy.countTextUnits(lines, unit), () => ({ unit }));
  }

  expressionDiagnostic(source: string): string | null {
    return this.answer('expressionDiagnostic', () => this.query('expressionDiagnostic', { source: String(source) }),
      () => this.legacy.expressionDiagnostic(source), () => ({ source }));
  }
}
