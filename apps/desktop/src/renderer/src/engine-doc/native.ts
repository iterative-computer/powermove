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
import { DetachedScopes, copyOf } from './detached';
import type { DualAnswers } from './dual';
import { layer3DRole } from '../core/scene3d/layers';
import { COMP_FIELDS, canonicalPath } from '../legacy/core/compositions';
import type { FontLoader } from './fonts';
import { ExpressionFrames, type ExpressionFrameSlot } from './expression-frame';

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

/**
 * The engine address prefix of `rootComp(PM.proj)` (pm-eval's `ROOT_VIEW`):
 * the open composition's layers and fields seen through the wrapper
 * `PM.Comps.get(openId)` returns. Expressions get the wrapper as `comp` and
 * `param()` reads its `params`, so the wrapper has its own address; the open
 * composition's address would hand them the project.
 */
const ROOT_VIEW = '@root:';
/** The fast path's stamp for a scope addressed as {@link ROOT_VIEW}. */
const ROOT_STAMP = Object.freeze({ rootView: true });

const hasOwn = (o: object, key: string): boolean => Object.prototype.hasOwnProperty.call(o, key);

/**
 * Whether `cur` is, member for member, what `rootComp(proj)` builds now
 * (legacy/core/compositions.ts): every member an expression can read holds
 * the project's value (`id: compId`, `name: compName`, each composition
 * field), and it has no other member. Anything else an expression could
 * read differently (an added `params`, a stale `fps`) makes it a detached
 * scope instead. Runs on every expression evaluation under the wrapper
 * view, so the fields are named reads (inline caches) and the members one
 * for-in walk (V8's enum cache): about 30 ns. `fields` is how many
 * composition fields the project has (see `projectFields`). Members for-in
 * does not list (non-enumerable or symbol keys) are not compared; rootComp
 * never makes them (docs/engine/document-host.md).
 */
function isRootWrapper(cur: any, proj: any, fields: number, unwrap: <T>(v: T) => T): boolean {
  const same = (a: unknown, b: unknown): boolean => Object.is(a, b) || (!!a && typeof a === 'object' && unwrap(a) === b);
  if (Object.getPrototypeOf(cur) !== Object.prototype) return false;
  if (!same(cur.id, proj.compId) || !same(cur.name, proj.compName) || !same(cur.layers, proj.layers)
    || !same(cur.w, proj.w) || !same(cur.h, proj.h) || !same(cur.fps, proj.fps) || !same(cur.fpsRational, proj.fpsRational)
    || !same(cur.dur, proj.dur) || !same(cur.bg, proj.bg) || !same(cur.backgroundFill, proj.backgroundFill)
    || !same(cur.markers, proj.markers) || !same(cur.work, proj.work) || !same(cur.shutter, proj.shutter)
    || !same(cur.audioGain, proj.audioGain) || !same(cur.render3d, proj.render3d)) return false;
  let members = 0;
  for (const key in cur) {
    switch (key) {
      // COMP_FIELDS (a test checks this list against it), id and name.
      case 'id': case 'name': case 'w': case 'h': case 'fps': case 'fpsRational': case 'dur': case 'bg':
      case 'backgroundFill': case 'layers': case 'markers': case 'work': case 'shutter': case 'audioGain': case 'render3d':
        members += 1;
        break;
      default:
        return false;
    }
  }
  return members === fields + 2;
}
/** The members isRootWrapper compares, besides `id` and `name` (must equal COMP_FIELDS). */
export const ROOT_WRAPPER_FIELDS = ['w', 'h', 'fps', 'fpsRational', 'dur', 'bg', 'backgroundFill', 'layers', 'markers', 'work', 'shutter', 'audioGain', 'render3d'] as const;

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
  // Resident transform/content properties use two own members. Unroll that
  // revision check without weakening the missing/inherited-member guards.
  if (path.length === 2) {
    if (!root || typeof root !== 'object' || !Object.hasOwn(root, path[0]!)) return undefined;
    const owner = root[path[0]!];
    return owner && typeof owner === 'object' && Object.hasOwn(owner, path[1]!) ? owner[path[1]!] : undefined;
  }
  let node = root;
  for (const part of path) {
    if (!node || typeof node !== 'object' || !Object.prototype.hasOwnProperty.call(node, part)) return undefined;
    node = node[part as any];
  }
  return node;
}

/** A detached input as the JSON the mirror would hold (evaluated ad hoc). */
const asJson = (value: any): any => JSON.parse(JSON.stringify(value));

/** Whether canonicalProject moves this root into `comps[compId]` (shared/canonical-json.ts). */
const canonicalRoot = (proj: any): boolean => typeof proj?.compId === 'string' && !!proj.compId
  && !(Object.prototype.hasOwnProperty.call(proj, 'comps') && !(proj.comps && typeof proj.comps === 'object' && !Array.isArray(proj.comps)));
/**
 * A caller's own layer list as the structural questions read it
 * (`groupAncestors`: `id`, `group`, `type`; `validMatteSource`: `id`, `type`,
 * `matteSource`; pm-eval's `group_ancestors` and `valid_matte_source` read
 * nothing else): each layer's JSON reduced to those members, so a list of
 * heavy layers is not serialized whole on every call. Other items keep their JSON.
 */
const STRUCTURE = ['id', 'group', 'type', 'matteSource'] as const;
const structural = (item: any): any => {
  if (!item || typeof item !== 'object' || Array.isArray(item)) return item;
  const out: Record<string, unknown> = {};
  for (const key of STRUCTURE) if (Object.prototype.propertyIsEnumerable.call(item, key)) out[key] = item[key];
  return out;
};
const structuralList = (list: any[]): any => asJson(Array.from(list, structural));
/** A composition's own scalar members with `layers` (no `comps`, no layer contents). */
const structuralComp = (cur: any, layers: any[]): any => {
  const out: Record<string, unknown> = {};
  if (cur && typeof cur === 'object') for (const [key, value] of Object.entries(cur)) if (value === null || typeof value !== 'object') out[key] = value;
  out.layers = structuralList(layers);
  return out;
};

/** V8's TypeError for `value[key]` on a nullish `value`. */
const readThrows = (value: unknown, key: string): TypeError => new TypeError(`Cannot read properties of ${value === null ? 'null' : 'undefined'} (reading '${key}')`);
/** V8's TypeError for `Object.entries(undefined)`. */
const entriesThrows = (): TypeError => new TypeError('Cannot convert undefined or null to object');
const isObject = (value: unknown): boolean => !!value && typeof value === 'object';

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
  private props = new WeakMap<object, { layer: object; layerId: unknown; path: PathPart[]; comp: string; id: number; key: unknown; revision: number; shape: number; project: unknown; cur: unknown; wasm: PmWasm; time?: number; number?: number; frame?: ExpressionFrameSlot }>();
  /** A detached property's validated address; its inputs are rechecked by DetachedScopes. */
  private detachedProps = new WeakMap<object, { layer: object; cur: unknown; key: unknown; path: PathPart[]; request: Record<string, unknown> }>();
  /** Live layer → interned transform channels, valid at the current revision and scope. */
  private channels = new WeakMap<object, { layer: object; layerId: unknown; comp: string; ids: Map<string, number>; revision: number; shape: number; project: unknown; cur: unknown }>();
  private refs = new Map<number, number>();
  private expressionFrames = new ExpressionFrames();
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
  /**
   * Operations with no legacy answer to compare with: the headless document
   * host has no compositor, so `gl.bounds` and `gl.pick` come from pm-wasm
   * alone there (the editor's dual answers check them against the GPU path).
   */
  readonly noReference = new Set<string>();
  /** Strict hosts report and throw instead of executing any legacy fallback. */
  onFallback: ((op: string, reason: string) => never) | null = null;
  /** Re-instantiates a trapped module synchronously (install.ts); false when it cannot. */
  revive: (() => boolean) | null = null;
  /** A named failure ("Use New Engine" on; install.ts decides whether to report it). */
  onFailure: ((code: 'pm-wasm-trap' | 'pm-wasm-error' | 'font-service-unavailable', message: string) => void) | null = null;

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
    let wasm = this.wasm();
    if (wasm?.dead && this.revive?.()) wasm = this.wasm();
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

  /**
   * `PM.curComp()`'s address and the open composition's. `wrapperFree`: the
   * caller's answer cannot read the scope object itself (a channel without
   * an expression: `evalKfs` reads only the channel and the layer), so a
   * scope over the open composition's own layers is addressed as the
   * wrapper view even when it is not a pristine `rootComp`.
   */
  private comps(wrapperFree = false): { comp: string; open: string; cur: any; liveCur: any } | null {
    const proj = this.unwrap(this.PM.proj);
    if (!proj || typeof proj !== 'object') return null;
    const open = typeof proj.compId === 'string' ? proj.compId : typeof proj.id === 'string' ? proj.id : '';
    const liveCur = this.PM.curComp?.() ?? this.PM.proj;
    const cur = this.unwrap(liveCur);
    if (cur === proj) return { comp: open, open, cur: proj, liveCur: proj };
    // Comps.get(rootId) constructs a new rootComp object on every call. Its
    // layers are the open composition's live array, so address the mirror
    // (no serialization) through the wrapper's own view: expressions and
    // param() see the wrapper's members, not the project's.
    if (this.overOpen(cur, proj) && (wrapperFree || isRootWrapper(cur, proj, this.projectFields(proj), this.unwrap))) {
      return { comp: ROOT_VIEW + open, open, cur: proj, liveCur: proj };
    }
    const comp = this.address(proj, cur);
    if (comp === null) return cur && typeof cur === 'object' ? { comp: this.detached.address(cur), open, cur, liveCur } : null;
    return { comp, open, cur, liveCur };
  }

  /** Where each composition object last was: checked by walking its path, so a stale entry is never used. */
  private detached = new DetachedScopes();
  private addresses = new WeakMap<object, PathPart[] | null>();
  private detachedAddresses = new WeakMap<object, { proj: unknown; revision: number }>();
  /** The object last found outside the document, and where (see `address`). */
  private lastDetached: { ref: WeakRef<object>; proj: unknown; revision: number } | null = null;

  /**
   * The engine address of composition object `cur` inside `proj`: its id when
   * it is a top-level composition (O(1)), else `@path:` with its canonical path.
   * Nested compositions are found once by identity and then answered from the
   * cache, verified in O(depth) per call; null when it is not in the document.
   */
  private address(proj: any, cur: any): string | null {
    if (!cur || typeof cur !== 'object') return null;
    if (typeof cur.id === 'string' && proj.comps && typeof proj.comps === 'object' && proj.comps[cur.id] === cur) return cur.id;
    const revision = this.barrier()?.revision ?? -1;
    const detached = this.detachedAddresses.get(cur);
    if (detached && detached.proj === proj && detached.revision === revision) return null;
    const cached = this.addresses.get(cur);
    let path = cached && at(proj, cached) === cur ? cached : undefined;
    // A fresh copy of the composition object last found outside the document,
    // at the same revision (a caller building its scope object for every
    // query): no walk of the whole document per query. Addressing it as
    // detached is exact even if it were inside (the detached path evaluates
    // the live object itself), only slower.
    const seen = this.lastDetached;
    const last = !path && seen && seen.proj === proj && seen.revision === revision ? seen.ref.deref() : undefined;
    if (!path && !(last && copyOf(cur, last))) {
      path = findPath(proj, cur, 64) ?? undefined;
      if (path) this.addresses.set(cur, path);
    }
    if (!path) {
      this.detachedAddresses.set(cur, { proj, revision });
      this.lastDetached = { ref: new WeakRef(cur), proj, revision };
      return null;
    }
    return path.length === 2 && path[0] === 'comps' ? String(path[1]) : '@path:' + JSON.stringify(canonicalPath(path, proj.compId));
  }

  private fields: { proj: unknown; revision: number; count: number } = { proj: null, revision: -1, count: 0 };
  /** How many composition fields `proj` has (cached per barrier revision: they change only through it). */
  private projectFields(proj: any): number {
    const revision = this.barrier()?.revision ?? -1;
    const f = this.fields;
    if (f.proj !== proj || f.revision !== revision || revision < 0) {
      let count = 0;
      for (const key of COMP_FIELDS) if (hasOwn(proj, key)) count += 1;
      f.proj = proj; f.revision = revision; f.count = count;
    }
    return f.count;
  }

  /** Whether scope object `cur` is a view of the open composition: its own `layers` and `id: compId`. */
  private overOpen(cur: any, proj: any): boolean {
    return !!cur && typeof cur === 'object' && this.unwrap(cur.layers) === proj.layers && cur.id === proj.compId;
  }

  /** Where the mirror holds composition `cur` (at engine address `comp`). */
  private mirrorBase(comp: string, cur: any): PathPart[] {
    if (comp.startsWith(ROOT_VIEW)) comp = comp.slice(ROOT_VIEW.length);
    if (comp.startsWith('@path:')) return JSON.parse(comp.slice(6));
    // A root canonicalProject leaves as it is (no compId): the mirror holds its layers at the top.
    if (cur === this.unwrap(this.PM.proj) && !canonicalRoot(cur)) return [];
    return ['comps', comp];
  }

  /**
   * The composition whose `layers` array `list` is, anywhere in the
   * document (the open one, `comps`, or nested `comps`): its engine address
   * and the composition, or null for a list of the caller's own.
   */
  private ownerOf(list: any): { comp: string; cur: any } | null {
    const proj = this.unwrap(this.PM.proj);
    if (!proj || typeof proj !== 'object' || !Array.isArray(list)) return null;
    if (proj.layers === list) return { comp: typeof proj.compId === 'string' ? proj.compId : typeof proj.id === 'string' ? proj.id : '', cur: proj };
    const seen = new Set<any>();
    const visit = (comps: any, depth: number): { comp: string; cur: any } | null => {
      if (!comps || typeof comps !== 'object' || depth > 16 || seen.has(comps)) return null;
      seen.add(comps);
      for (const comp of Object.values(comps)) {
        if (!comp || typeof comp !== 'object') continue;
        if ((comp as any).layers === list) {
          const address = this.address(proj, comp);
          return address === null ? null : { comp: address, cur: comp };
        }
        const nested = visit((comp as any).comps, depth + 1);
        if (nested) return nested;
      }
      return null;
    };
    return visit(proj.comps, 0);
  }

  /**
   * A layer of an explicit list that is some composition's own `layers`:
   * its index there (checked against the mirror, repaired if a write
   * bypassed the barrier), or its JSON when it is not in the list.
   */
  private listLayer(owner: { comp: string; cur: any }, L: any): number | any {
    const layers: any[] = owner.cur.layers;
    const raw = this.unwrap(L);
    const i = this.index(layers).index.get(raw);
    // Not in the list: its JSON (the questions read its structure only); not an object: no members.
    if (i === undefined || layers[i] !== raw) return raw && typeof raw === 'object' ? asJson(structural(raw)) : {};
    const barrier = this.barrier();
    const mirrored = barrier ? lookup(barrier.shadow, [...this.mirrorBase(owner.comp, owner.cur), 'layers', i, 'id']) : null;
    if (barrier && (!mirrored?.exists || mirrored.value !== raw.id)) {
      this.stats.repairs += 1;
      const missed = barrier.repair('structuralDiff');
      if (missed.length) this.onStale?.('context', missed);
    }
    return i;
  }

  private context(L: any, wrapperFree = false): Context | null {
    const c = this.comps(wrapperFree);
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
    const mirrored = barrier ? lookup(barrier.shadow, [...this.mirrorBase(c.comp, c.cur), 'layers', i, 'id']) : null;
    if (barrier && !c.comp.startsWith('@scope:') && (!mirrored?.exists || mirrored.value !== raw.id)) {
      this.stats.repairs += 1;
      const missed = barrier.repair('structuralDiff');
      if (missed.length) this.onStale?.('context', missed);
    }
    const first = !c.comp.startsWith('@scope:') && typeof raw.id === 'string' && this.index(layers).firstById.get(raw.id) === i;
    return { comp: c.comp, open: c.open, layer: i, raw, first, live };
  }

  private query<T>(op: string, request: Record<string, unknown>, map?: (v: any) => T): Native<T> {
    let wasm = this.ready();
    if (!wasm) return { fallback: 'unavailable' };
    const ask = (instance: PmWasm): any => {
      const payload: Record<string, unknown> = { op, ...request };
      if (typeof request.comp === 'string' && request.comp.startsWith('@json:')) {
        payload.compJsonText = request.comp.slice(6);
        payload.comp = '@detached';
      }
      const run = () => typeof request.comp === 'string' && request.comp.startsWith('@scope:')
        && !('compJson' in payload) && !('compJsonText' in payload)
        ? this.detached.query(request.comp, payload, instance, this.barrier()?.revision ?? -1)
        : instance.query(payload);
      let reply = run();
      if (reply?.fonts && this.fonts) {
        // Text layout met fonts the mirror lacks: load them (at once from a
        // synchronous registry, in the background otherwise) and ask again.
        reply = this.fonts.resolve(instance, run, (r: any) => !!r?.fonts) ?? reply;
      }
      return reply;
    };
    let reply: any;
    try { reply = ask(wasm); } catch (error) {
      // A trap: a fresh instance, resynced, answers the same question (the
      // answer stays native). A second trap on it is a pm-wasm defect.
      console.warn('[engine-doc] pm-wasm query failed', error);
      if (!this.revive?.() || !(wasm = this.ready())) return { fallback: 'trap' };
      try { reply = ask(wasm); } catch (again) {
        console.warn('[engine-doc] pm-wasm query failed again on a fresh instance', again);
        this.onFailure?.('pm-wasm-trap', `pm-wasm trapped twice answering ${op}: ${String((again as Error)?.message ?? again)}`);
        this.revive?.();
        return { fallback: 'trap' };
      }
    }
    if (reply?.fonts && this.fonts?.disabled) this.onFailure?.('font-service-unavailable', 'The native font service stopped answering, so text layout cannot run in the new engine.');
    if ('ok' in reply) return { value: map ? map(reply.ok) : reply.ok };
    if (reply.throw) {
      const error = new (reply.throw.kind === 'TypeError' ? TypeError : reply.throw.kind === 'RangeError' ? RangeError : Error)(reply.throw.message);
      return { threw: error };
    }
    if (reply.host) return { fallback: 'host' };
    if (reply.fonts) return { fallback: 'fonts' };
    if (reply.ink) return { fallback: 'ink' };
    this.onFailure?.('pm-wasm-error', `pm-wasm could not answer ${op}: ${reply.error ?? 'unknown'}`);
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
      if (this.onFallback) return this.onFallback(op, got.fallback);
      return legacy();
    }
    if (this.dual && !this.noReference.has(op)) {
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
          if ('fallback' in again) { this.fell(op, again.fallback); if (this.onFallback) return this.onFallback(op, again.fallback); return legacy(); }
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
    if (!c || c.comp.startsWith('@scope:')) return null;
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
    this.expressionFrames.release(id);
    const wasm = this.wasm();
    if (wasm && !wasm.dead) try { wasm.release(id); } catch { /* the instance is gone */ }
  }

  /** Drops every interned id (after the module is replaced). */
  reset(): void {
    this.generation += 1;
    this.props = new WeakMap();
    this.detached = new DetachedScopes();
    this.detachedProps = new WeakMap();
    this.channels = new WeakMap();
    this.refs.clear();
    this.expressionFrames = new ExpressionFrames();
    this.indexed = new WeakMap();
    this.shapes = new WeakMap();
    this.rootsAnswer = null;
  }

  private noteError(prop: any, wasm: PmWasm, id: number, comp?: string) {
    if (!prop || typeof prop !== 'object') return;
    const raw = this.unwrap(prop);
    if (!raw.expr) {
      this.errors.delete(raw);
      this.expressionFrames.release(id);
      return;
    }
    if (comp && !this.dual) {
      const slot = this.expressionFrames.register(comp, id);
      const entry = this.props.get(raw);
      if (entry?.id === id) entry.frame = slot;
    }
    const message = (wasm.evaluationStatus() & 0x80) ? wasm.expressionError(id) : null;
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
  private stamp(): { revision: number; cur: unknown; scope: any } | null {
    const barrier = this.barrier();
    if (!barrier) return null;
    // flush() returns at once when nothing waits (it also notices writes a
    // plain transaction has not announced yet).
    barrier.flush();
    const proj = this.unwrap(this.PM.proj);
    const cur = this.PM.scope?.length ? this.unwrap(this.PM.curComp()) : null;
    if (!cur || cur === proj) return { revision: barrier.revision, cur: null, scope: proj };
    // Ids interned under the wrapper view serve any scope over the open
    // composition's layers; an expression channel also needs the scope to
    // be a pristine rootComp (see `current`).
    return { revision: barrier.revision, cur: proj && this.overOpen(cur, proj) ? ROOT_STAMP : cur, scope: cur };
  }

  /**
   * Whether an interned id (with the revision and scope stamp it was last
   * validated at) still answers channel `raw`. Without an expression the
   * answer reads only the channel and its layer (`evalKfs(prop.kf, T -
   * L.from)` or `prop.v`), never the scope, so the mirror revision decides
   * alone and `PM.curComp()` is not called. With one, the scope must be the
   * one it was interned under, and for the wrapper view a pristine
   * rootComp: checked on every call, since the scope object can change
   * without the barrier seeing it.
   */
  private current(entry: { revision: number; shape: number; project: unknown; cur: unknown; comp: string; layer: object; layerId: unknown; path?: PathPart[]; time?: number; number?: number }, raw: any): boolean {
    const barrier = this.barrier();
    if (!barrier) return false;
    barrier.flush();
    const revision = barrier.revision;
    // An unchanged layer shape keeps the interned address in the same owner.
    // A leaf edit may have changed the expression's reads; pm-wasm validates
    // and rebinds that id against the new revision on its next evaluation.
    const revised = entry.revision !== revision;
    if (revised) {
      const project = this.unwrap(this.PM.proj);
      if (entry.project !== project || entry.layerId !== (entry.layer as any)?.id
        || entry.path && at(entry.layer, entry.path) !== raw) return false;
      if (entry.shape !== barrier.shapeRevision) {
        // An added, removed, or reordered layer need not discard every ID.
        // Check the open composition's membership and first-ID rule once per
        // layer shape (index() shares its map across all channels in a frame).
        const open = project?.compId ?? project?.id ?? '';
        if ((entry.comp !== open && entry.comp !== ROOT_VIEW + open)
          || !Array.isArray(project?.layers)) return false;
        const indexed = this.index(project.layers);
        const atIndex = indexed.index.get(entry.layer);
        if (atIndex === undefined || indexed.firstById.get(String(entry.layerId)) !== atIndex) return false;
      }
    }
    let scopeValid: boolean;
    if (this.wrapperFreeChannel(raw)) scopeValid = true;
    // The ordinary open-project scope has no wrapper to validate. Avoid
    // constructing a scope stamp for every resident channel of the frame.
    else if (!this.PM.scope?.length) scopeValid = entry.cur === null;
    else {
      const stamp = this.stamp();
      scopeValid = !!stamp && stamp.revision === revision && entry.cur === stamp.cur
        && (stamp.cur !== ROOT_STAMP || isRootWrapper(stamp.scope, this.unwrap(this.PM.proj), this.projectFields(this.unwrap(this.PM.proj)), this.unwrap));
    }
    if (!scopeValid) return false;
    if (revised) {
      entry.revision = revision;
      entry.shape = barrier.shapeRevision;
      entry.time = undefined;
    }
    return true;
  }

  private wrapperFreeChannel(raw: any): boolean {
    // `PM.ev`/`PM.evP` without an expression: `evalKfs(prop.kf, T - L.from)` or `prop.v`.
    return !!raw && typeof raw === 'object' && !raw.expr;
  }

  /** `pm_ev` for an id validated at this revision; MISS when it must take the slow path. */
  private fastValue(id: number, T: number, prop: any): unknown {
    const wasm = this.wasm();
    if (!wasm || wasm.dead) return MISS;
    const raw = prop && typeof prop === 'object' ? this.unwrap(prop) : null;
    // The ID still names this channel after a source removal, but it must no
    // longer occupy a resident expression slot in subsequent frame batches.
    if (!raw?.expr && this.expressionFrames.has(id)) this.expressionFrames.release(id);
    let n: number, status: number;
    try {
      if (raw?.expr) {
        const slot = this.expressionFrames.slot(id);
        status = slot ? this.expressionFrames.read(wasm, slot, T, this.barrier()!.revision) : -1;
        if (status >= 0) n = this.expressionFrames.value;
        else { n = wasm.x.pm_ev(id, T); status = wasm.evaluationStatus(); }
      } else {
        n = wasm.x.pm_ev(id, T);
        status = n === n ? 0 : wasm.x.pm_kind();
      }
    } catch { wasm.dead = true; return MISS; }
    if ((status & 0x7f) !== 0) return MISS; // not a number, or a throw: ask the slow path
    this.stats.fastPath += 1;
    if (raw) {
      if (raw.expr) { const m = (status & 0x80) ? wasm.expressionError(id) : null; if (m == null) this.errors.delete(raw); else this.errors.set(raw, m); }
      else this.errors.delete(raw);
    }
    return n;
  }

  ev(L: any, key: any, T: any): any {
    if (!this.dual && typeof key === 'string' && typeof T === 'number' && L && typeof L === 'object') {
      const raw = this.unwrap(L);
      const entry = this.channels.get(raw);
      const id = entry?.ids.get(key);
      if (id !== undefined && Object.prototype.hasOwnProperty.call(raw.p ?? {}, key) && this.current(entry!, raw.p?.[key])) {
        const value = this.fastValue(id, T, raw.p?.[key]);
        if (value !== MISS && raw.p?.[key]?.expr && !this.expressionFrames.has(id)) this.noteError(raw.p[key], this.wasm()!, id, entry!.comp);
        if (value !== MISS) { this.stats.calls.ev = (this.stats.calls.ev ?? 0) + 1; return value; }
      }
    }
    return this.answer('ev', () => {
      const wasm = this.ready();
      if (!wasm) return { fallback: 'unavailable' };
      const rawL = this.unwrap(L);
      const ctx = this.context(L, !!rawL && typeof rawL === 'object' && !!rawL.p && typeof rawL.p === 'object' && Object.prototype.hasOwnProperty.call(rawL.p, key) && this.wrapperFreeChannel(rawL.p[key]));
      if (!ctx) return { fallback: 'context' };
      if (typeof key !== 'string' || typeof T !== 'number' || !ctx.first || typeof ctx.layer !== 'number') {
        return this.query('ev', { comp: ctx.comp, open: ctx.open, layer: ctx.layer, key: String(key), t: T });
      }
      const stamp = this.stamp()!;
      let entry = this.channels.get(ctx.raw);
      if (!entry || entry.comp !== ctx.comp || entry.project !== this.unwrap(this.PM.proj)
        || entry.layerId !== ctx.raw.id || entry.cur !== stamp.cur) { entry = { layer: ctx.raw, layerId: ctx.raw.id, comp: ctx.comp, ids: new Map(), revision: stamp.revision, shape: this.barrier()!.shapeRevision, project: this.unwrap(this.PM.proj), cur: stamp.cur }; this.channels.set(ctx.raw, entry); }
      entry.revision = stamp.revision;
      entry.shape = this.barrier()!.shapeRevision;
      entry.cur = stamp.cur;
      let id = entry.ids.get(key);
      if (id === undefined) {
        id = this.intern(wasm, { comp: ctx.comp, layer: ctx.raw.id, key, via: 'channel' }, ctx.raw);
        if (id < 0) return this.query('ev', { comp: ctx.comp, open: ctx.open, layer: ctx.layer, key, t: T });
        entry.ids.set(key, id);
      }
      const got = this.evById(wasm, id, T);
      this.noteError(ctx.raw.p?.[key], wasm, id, ctx.comp);
      return got;
    }, () => this.legacy.ev(L, key, T), () => ({ layer: L?.id, key, t: T }));
  }

  evP(L: any, prop: any, T: any, key: any): any {
    if (!prop || typeof prop !== 'object') return null;
    if (!this.dual && typeof T === 'number') {
      const rawProp = this.unwrap(prop);
      const entry = this.props.get(rawProp);
      if (entry && entry.key === key && entry.layer === this.unwrap(L) && this.current(entry, rawProp)) {
        if (!rawProp.expr && entry.frame) {
          this.expressionFrames.release(entry.id);
          entry.frame = undefined;
          this.errors.delete(rawProp);
        }
        // The value was already answered by pm-wasm at this exact time and
        // revision. Like its own last-answer memo, keep numeric answers only.
        if (entry.time !== undefined && Object.is(entry.time, T)) {
          // reset() drops these entries when an instance is replaced.
          if (!entry.wasm.dead) {
            this.stats.fastPath += 1;
            this.stats.calls.evP = this.stats.calls.evP! + 1;
            return entry.number;
          }
        }
        // A resident expression reads its frame run (see ExpressionFrames).
        // Anything but a number takes fastValue, which decodes or falls back.
        const frame = entry.frame;
        if (rawProp.expr && frame && !frame.released && !entry.wasm.dead) {
          let status = -1;
          try { status = this.expressionFrames.read(entry.wasm, frame, T, entry.revision); } catch { entry.wasm.dead = true; }
          if (status >= 0 && (status & 0x7f) === 0) {
            const value = this.expressionFrames.value;
            const message = (status & 0x80) ? entry.wasm.expressionError(entry.id) : null;
            if (message == null) this.errors.delete(rawProp); else this.errors.set(rawProp, message);
            entry.time = T; entry.number = value;
            this.stats.fastPath += 1;
            this.stats.calls.evP = (this.stats.calls.evP ?? 0) + 1;
            return value;
          }
        }
        const value = this.fastValue(entry.id, T, rawProp);
        if (value !== MISS && rawProp.expr && (!entry.frame || entry.frame.released)) {
          const slot = this.expressionFrames.slot(entry.id);
          if (slot) entry.frame = slot;
          else this.noteError(rawProp, entry.wasm, entry.id, entry.comp);
        }
        if (typeof value === 'number') { entry.time = T; entry.number = value; }
        if (value !== MISS) { this.stats.calls.evP = (this.stats.calls.evP ?? 0) + 1; return value; }
      }
    }
    if (!this.dual && typeof T === 'number' && typeof key === 'string') {
      const rawProp = this.unwrap(prop), entry = this.detachedProps.get(rawProp);
      const stamp = entry && this.stamp();
      // A fresh copy of the scope object (`copyOf`) shares its detached
      // scope, which validates the answer's reads on the new object.
      if (stamp && entry.cur !== stamp.cur && stamp.cur && typeof stamp.cur === 'object' && stamp.cur !== ROOT_STAMP
        && copyOf(stamp.cur, entry.cur) && this.detached.address(stamp.cur) === entry.request.comp) entry.cur = stamp.cur;
      if (stamp && entry.cur === stamp.cur && entry.request.open === (this.PM.proj?.compId ?? this.PM.proj?.id ?? '')
        && (stamp.cur as any)?.layers?.[entry.request.layer as number] === entry.layer
        && entry.layer === this.unwrap(L) && entry.key === key && at(entry.layer, entry.path) === rawProp) {
        entry.request.t = T;
        const reply = this.query<any>('evP', entry.request);
        if ('value' in reply && (reply.value === null || typeof reply.value !== 'object')) {
          this.count('evP');
          this.stats.fastPath += 1;
          return reply.value;
        }
      }
    }
    return this.answer('evP', () => {
      const wasm = this.ready();
      if (!wasm) return { fallback: 'unavailable' };
      const rawProp = this.unwrap(prop);
      const ctx = this.context(L, this.wrapperFreeChannel(rawProp));
      if (!ctx) return { fallback: 'context' };
      const stamp = this.stamp()!;
      let entry = this.props.get(rawProp);
      const valid = entry && entry.layer === ctx.raw && entry.comp === ctx.comp && entry.key === key
        && ctx.first && entry.layerId === ctx.raw.id && entry.project === this.unwrap(this.PM.proj)
        && (!rawProp.expr || entry.cur === stamp.cur)
        && at(ctx.raw, entry.path) === rawProp;
      if (!valid) {
        const path = findPath(ctx.raw, rawProp);
        if (!path || !ctx.first || typeof ctx.layer !== 'number' || typeof key !== 'string' || typeof T !== 'number') {
          const request = path
            ? { comp: ctx.comp, open: ctx.open, layer: ctx.layer, path, key: String(key), t: T }
            : { comp: ctx.comp, open: ctx.open, layer: ctx.layer, prop: JSON.parse(JSON.stringify(rawProp)), key: String(key), t: T };
          if (path && stamp.cur && stamp.cur !== ROOT_STAMP && typeof key === 'string' && typeof ctx.layer === 'number' && ctx.comp.startsWith('@scope:')) this.detachedProps.set(rawProp, { layer: ctx.raw, cur: stamp.cur, key, path, request: { op: 'evP', ...request } });
          return this.query('evP', request);
        }
        const id = this.intern(wasm, { comp: ctx.comp, layer: ctx.raw.id, key, via: path }, rawProp);
        if (id < 0) return this.query('evP', { comp: ctx.comp, open: ctx.open, layer: ctx.layer, path, key, t: T });
        entry = { layer: ctx.raw, layerId: ctx.raw.id, path, comp: ctx.comp, id, key, revision: stamp.revision, shape: this.barrier()!.shapeRevision, project: this.unwrap(this.PM.proj), cur: stamp.cur, wasm };
        this.props.set(rawProp, entry);
      }
      entry!.revision = stamp.revision;
      entry!.shape = this.barrier()!.shapeRevision;
      entry!.cur = stamp.cur;
      const got = this.evById(wasm, entry!.id, T);
      entry!.time = 'value' in got && typeof got.value === 'number' ? T : undefined;
      if ('value' in got && typeof got.value === 'number') entry!.number = got.value;
      this.noteError(rawProp, wasm, entry!.id, ctx.comp);
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
      // Not a channel at all: `p.kf.find(...)` throws before it reads the layer.
      if (!isObject(prop)) return { threw: prop == null ? readThrows(prop, 'kf') : readThrows(undefined, 'find') };
      const ctx = this.context(L);
      if (!ctx) return { fallback: 'context' };
      const path = findPath(ctx.raw, this.unwrap(prop));
      // A channel outside the layer: pm-wasm reads its JSON (its keys are
      // found by index, so the answer is still the live key).
      const detached = path ? {} : { prop: prop && typeof prop === 'object' ? asJson(this.unwrap(prop)) : prop };
      return this.query('hasKeyAt', { comp: ctx.comp, open: ctx.open, layer: ctx.layer, path: path ?? [], ...detached, t: T }, (i) => i == null ? null : prop.kf[i] ?? null);
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
          if (!ctx) return { fallback: 'context' };
          // A parent outside the composition is evaluated from its JSON.
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
    return this.answer('groups.bounds', () => this.layerQuery('groupBounds', group, { t: time }),
      () => this.legacy.groupBounds(group, time), () => ({ layer: group?.id, t: time }),
      this.dual ? timePerturbations(time, t => this.legacy.groupBounds(group, t)) : undefined,
      this.dual ? () => this.illConditioned(group, time) : undefined);
  }

  groupAncestors(L: any, layers?: any[]): any[] {
    // `if (!layer?.group) return []` comes before anything reads the list
    // (an explicit list, or a layer that is not one; the shaped path below
    // caches the rest per revision).
    const raw = this.unwrap(L);
    if (!raw?.group && (layers !== undefined || !raw || typeof raw !== 'object')) {
      return this.answer('groups.ancestors', () => ({ value: [] }), () => this.legacy.groupAncestors(L, layers), () => ({ layer: L?.id }));
    }
    if (layers !== undefined) {
      const c = this.comps();
      if (!c || this.unwrap(layers) !== c.cur.layers) {
        // Another composition's own layers (audio, the mixer, a render of
        // another composition): asked by its address, not as JSON.
        const owner = c ? this.ownerOf(this.unwrap(layers)) : null;
        if (owner) return this.answer('groups.ancestors',
          () => this.query('groupAncestors', { comp: owner.comp, open: c!.open, layer: this.listLayer(owner, L) }, (indexes: number[]) => indexes.map(i => layers[i])),
          () => this.legacy.groupAncestors(L, layers), () => ({ layer: L?.id }));
        return this.answer('groups.ancestors',
          () => this.query('groupAncestors', { comp: c?.comp ?? '', open: c?.open ?? '', compJson: structuralComp(c?.cur, layers), layer: asJson(structural(this.unwrap(L))) }, (indexes: number[]) => indexes.map(i => layers[i])),
          () => this.legacy.groupAncestors(L, layers), () => ({ layer: L?.id }));
      }
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
          if (!ctx) return { fallback: 'context' };
          // A parent outside the composition is evaluated from its JSON.
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
    return this.answer('gl.bounds', () => this.layerQuery('bounds', L, { t: T }), () => this.legacy.bounds(L, T), () => ({ layer: L?.id, t: T }),
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
      const proj = this.unwrap(this.PM.proj);
      // With 3D layers the order follows the editor's view camera, which is
      // not in the document: three.js answers until Phase 3c.
      if ((proj.layers || []).some((layer: any) => layer3DRole(this.unwrap(layer)))) return { fallback: '3d' };
      const live = this.PM.proj.layers;
      return this.query('pick', { comp: c.open, open: c.open, x, y, t: T, includeLocked: !!options?.includeLocked, localView: null },
        (hit: number[] | null) => hit == null ? null : live[hit[0]!] ?? null);
    }, () => this.legacy.pick(x, y, T, options), () => ({ x, y, t: T }),
      this.dual ? [
        ...timePerturbations(T, t => this.legacy.pick(x, y, t, options)),
        ...[[1e-7, 0], [-1e-7, 0], [0, 1e-7], [0, -1e-7]].map(([dx, dy]) => () => this.legacy.pick(x + dx!, y + dy!, T, options)),
      ] : undefined,
      this.dual ? () => this.onLayerEdge(x, y, T) : undefined);
  }

  /**
   * Loads the faces the open composition's text layers need before anything
   * asks, so with an asynchronous provider (the editor, through main) the
   * first question about them is answered natively too. Asks pm-wasm
   * directly: nothing here is counted or answered.
   */
  async warmFonts(): Promise<void> {
    const loader = this.fonts;
    if (!loader || loader.disabled) return;
    const proj = this.unwrap(this.PM.proj);
    if (!proj) return;
    const open = proj.compId ?? proj.id ?? '';
    const visit = async (comp: any, address: string): Promise<void> => {
      const layers = Array.isArray(comp.layers) ? comp.layers : [];
      for (let index = 0; index < layers.length; index++) {
        const L = layers[index];
        if (L?.type !== 'text') continue;
        // Visit every keyed time, including content/range font changes. The
        // on-demand synchronous provider covers interpolated and expression faces.
        const times = new Set<number>([Number(L.from) || 0]);
        const scan = (value: any) => {
          if (!value || typeof value !== 'object') return;
          if (Array.isArray(value.kf)) for (const key of value.kf) if (Number.isFinite(key.t)) times.add((Number(L.from) || 0) + key.t);
          for (const child of Object.values(value)) scan(child);
        };
        scan(L);
        for (const t of times) for (let round = 0; round < 6; round++) {
          const wasm = this.ready();
          if (!wasm) return;
          const ask = () => wasm.query({ op: 'textLayout', comp: address, open, layer: index, t });
          const reply: any = ask();
          if (!reply?.fonts) break;
          loader.resolve(wasm, ask, (r: any) => !!r?.fonts);
          await loader.whenIdle();
          if (loader.disabled) return;
        }
      }
      for (const child of Object.values(comp.comps ?? {})) {
        const address = this.address(proj, child);
        if (address !== null) await visit(child, address);
      }
    };
    await visit(proj, open);
  }

  /** `PM.textLayout` (the glyph layout: characters, words, lines) for a host without a raster. */
  glyphLayout(L: any, T: number): any {
    return this.answer('glyphLayout', () => this.layerQuery('glyphLayout', L, { t: T }), () => this.legacy.glyphLayout(L, T), () => ({ layer: L?.id, t: T }));
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
      // Not a path: `path.id` (the default prefix) or `Object.entries(path.p)` throws before the layer is read.
      if (!isObject(path)) return { threw: path == null ? readThrows(path, prefix === undefined ? 'id' : 'p') : entriesThrows() };
      const ctx = this.context(L);
      if (!ctx) return { fallback: 'context' };
      const index = (Array.isArray(ctx.raw.d?.paths) ? ctx.raw.d.paths : []).indexOf(this.unwrap(path));
      const live = this.unwrap(path);
      // A path outside `d.paths` is evaluated from its JSON, with the layer as context.
      const detached = index < 0 ? { pathJson: asJson(live) } : {};
      return this.query('pathValues', { comp: ctx.comp, open: ctx.open, layer: ctx.layer, index, ...detached, t: time, ...(prefix === undefined ? {} : { prefix }) },
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
      // Not a path: `seen.has(path.id)` or pathValues' `Object.entries(path.p)` throws before the layer is read.
      if (!isObject(path)) return { threw: path == null ? readThrows(path, 'id') : entriesThrows() };
      const ctx = this.context(L);
      if (!ctx) return { fallback: 'context' };
      const index = (Array.isArray(ctx.raw.d?.paths) ? ctx.raw.d.paths : []).indexOf(this.unwrap(path));
      const live = this.unwrap(path);
      const detached = index < 0 ? { pathJson: asJson(live) } : {};
      return this.query('groupMatrix', { comp: ctx.comp, open: ctx.open, layer: ctx.layer, index, ...detached, t: time });
    }, () => this.legacy.groupMatrix(L, path, time), () => ({ layer: L?.id, t: time }));
  }

  evaluatedValue(L: any, value: any, time: number, path: string): any {
    // `isProperty(value) ? PM.evP(...) : value` (content-properties.ts).
    if (!(!!value && typeof value === 'object' && Array.isArray(value.kf) && 'v' in value)) return value;
    return this.evP(L, value, time, path);
  }

  validMatteSource(layers: any[], layer: any, id: any): boolean {
    // `new Set([layer.id])` throws first for a missing layer.
    if (layer == null) return this.answer('validMatteSource', () => ({ threw: readThrows(layer, 'id') }), () => this.legacy.validMatteSource(layers, layer, id), () => ({ id }));
    const c = this.comps();
    const owner = c && this.unwrap(layers) !== c.cur.layers ? this.ownerOf(this.unwrap(layers)) : null;
    if (owner) return this.answer('validMatteSource',
      () => this.query('validMatteSource', { comp: owner.comp, open: c!.open, layer: this.listLayer(owner, layer), id: id === undefined ? { $u: 1 } : id }),
      () => this.legacy.validMatteSource(layers, layer, id), () => ({ layer: layer?.id, id }));
    if (!c || this.unwrap(layers) !== c.cur.layers) return this.answer('validMatteSource',
      () => this.query('validMatteSource', { comp: c?.comp ?? '', open: c?.open ?? '', compJson: structuralComp(c?.cur, layers), layer: asJson(structural(this.unwrap(layer))), id: id === undefined ? { $u: 1 } : id }),
      () => this.legacy.validMatteSource(layers, layer, id), () => ({ id }));
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
