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
import type { DocumentBarrier } from './barrier';
import type { PmWasm } from './wasm';
import type { DualAnswers } from './dual';
import { layer3DRole } from '../core/scene3d/layers';

type Legacy = Record<string, any>;

/** A native answer: the value, a throw to rethrow, or a reason to ask the legacy code. */
type Native<T> = { value: T } | { threw: Error } | { fallback: string };

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
  fallbacks: Record<string, number>;
  fastPath: number;
  interned: number;
}

export class NativeApi {
  readonly stats: NativeStats = { calls: {}, fallbacks: {}, fastPath: 0, interned: 0 };
  /** Live channel object → interned id (validated on every use). */
  private props = new WeakMap<object, { layer: object; path: PathPart[]; comp: string; id: number }>();
  /** Live layer → its interned transform channels. */
  private channels = new WeakMap<object, { comp: string; ids: Map<string, number> }>();
  private refs = new Map<number, number>();
  private finalizer = typeof FinalizationRegistry === 'function'
    ? new FinalizationRegistry<number>(id => this.unref(id)) : null;
  private indexed = new WeakMap<any[], Indexed>();
  /** `expressionErrors` for channels evaluated natively (live objects). */
  readonly errors = new WeakMap<object, string>();
  private definitions = new Map<string, string>();

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
    const first = typeof raw.id === 'string' && this.index(layers).firstById.get(raw.id) === i;
    return { comp: c.comp, open: c.open, layer: i, raw, first, live };
  }

  private query<T>(op: string, request: Record<string, unknown>, map?: (v: any) => T): Native<T> {
    const wasm = this.ready();
    if (!wasm) return { fallback: 'unavailable' };
    let reply: any;
    try { reply = wasm.query({ op, ...request }); } catch (error) { console.warn('[engine-doc] pm-wasm query failed', error); return { fallback: 'trap' }; }
    if ('ok' in reply) return { value: map ? map(reply.ok) : reply.ok };
    if (reply.throw) {
      const error = new (reply.throw.kind === 'TypeError' ? TypeError : reply.throw.kind === 'RangeError' ? RangeError : Error)(reply.throw.message);
      return { threw: error };
    }
    if (reply.host) return { fallback: 'host' };
    if (reply.fonts) return { fallback: 'fonts' };
    return { fallback: `error:${reply.error ?? 'unknown'}` };
  }

  private layerQuery<T>(op: string, L: any, extra: Record<string, unknown>, map?: (v: any, ctx: Context) => T): Native<T> {
    const ctx = this.context(L);
    if (!ctx) return { fallback: 'context' };
    return this.query(op, { comp: ctx.comp, open: ctx.open, layer: ctx.layer, ...extra }, map ? (v) => map(v, ctx) : undefined);
  }

  /** Native answer (or legacy fallback), compared with the legacy answer in dual mode. */
  private answer<T>(op: string, native: () => Native<T>, legacy: () => T, describe: () => Record<string, unknown>): T {
    this.count(op);
    let got: Native<T>;
    try { got = native(); } catch (error) { got = { fallback: 'exception' }; console.warn(`[engine-doc] ${op} failed natively`, error); }
    if ('fallback' in got) {
      this.fell(op, got.fallback);
      return legacy();
    }
    if (this.dual) {
      let expected: { value?: T; threw?: unknown };
      try { expected = { value: legacy() }; } catch (error) { expected = { threw: error }; }
      this.dual.compare(op, 'value' in got ? { value: got.value } : { threw: got.threw }, expected, describe, this.unwrap);
    }
    if ('threw' in got) throw got.threw;
    return got.value;
  }

  private intern(wasm: PmWasm, address: Parameters<PmWasm['intern']>[0], owner: object): number {
    const id = wasm.intern(address);
    if (id >= 0) {
      this.refs.set(id, (this.refs.get(id) ?? 0) + 1);
      this.finalizer?.register(owner, id);
      this.stats.interned += 1;
    }
    return id;
  }

  private unref(id: number) {
    const n = (this.refs.get(id) ?? 1) - 1;
    if (n > 0) { this.refs.set(id, n); return; }
    this.refs.delete(id);
    const wasm = this.wasm();
    if (wasm && !wasm.dead) try { wasm.release(id); } catch { /* the instance is gone */ }
  }

  /** Drops every interned id (after the module is replaced). */
  reset(): void {
    this.props = new WeakMap();
    this.channels = new WeakMap();
    this.refs.clear();
    this.indexed = new WeakMap();
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

  ev(L: any, key: any, T: any): any {
    return this.answer('ev', () => {
      const wasm = this.ready();
      if (!wasm) return { fallback: 'unavailable' };
      const ctx = this.context(L);
      if (!ctx) return { fallback: 'context' };
      if (typeof key !== 'string' || typeof T !== 'number' || !ctx.first || typeof ctx.layer !== 'number') {
        return this.query('ev', { comp: ctx.comp, open: ctx.open, layer: ctx.layer, key: String(key), t: T });
      }
      let entry = this.channels.get(ctx.raw);
      if (!entry || entry.comp !== ctx.comp) { entry = { comp: ctx.comp, ids: new Map() }; this.channels.set(ctx.raw, entry); }
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
    return this.answer('evP', () => {
      const wasm = this.ready();
      if (!wasm) return { fallback: 'unavailable' };
      const ctx = this.context(L);
      if (!ctx) return { fallback: 'context' };
      const rawProp = this.unwrap(prop);
      let entry = this.props.get(rawProp);
      const valid = entry && entry.layer === ctx.raw && entry.comp === ctx.comp && at(ctx.raw, entry.path) === rawProp;
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
        entry = { layer: ctx.raw, path, comp: ctx.comp, id };
        this.props.set(rawProp, entry);
      }
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
      return this.layerQuery('allProps', L, {}, (entries: any[]) => entries.map(e => ({
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
    return this.answer('wouldCycle', () => this.layerQuery('wouldCycle', L, { parent: parentId === undefined ? { $u: 1 } : parentId }),
      () => this.legacy.wouldCycle(L, parentId), () => ({ layer: L?.id, parentId }));
  }

  resolveContent(L: any, T: any = this.PM.time): any {
    return this.answer('resolveContent', () => this.layerQuery('resolveContent', L, { t: T }),
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
    return this.answer('groups.bounds', () => this.layerQuery('groupBounds', group, { t: time }),
      () => this.legacy.groupBounds(group, time), () => ({ layer: group?.id, t: time }));
  }

  groupAncestors(L: any, layers?: any[]): any[] {
    if (layers !== undefined) {
      const c = this.comps();
      if (!c || this.unwrap(layers) !== c.cur.layers) return this.legacy.groupAncestors(L, layers);
    }
    return this.answer('groups.ancestors', () => this.layerQuery('groupAncestors', L, {}, (indexes: number[], ctx) => indexes.map(i => ctx.live[i])),
      () => this.legacy.groupAncestors(L, layers), () => ({ layer: L?.id }));
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
    return this.answer('groups.transformRoots', () => {
      const c = this.comps();
      if (!c) return { fallback: 'context' };
      return this.query('transformRoots', { comp: c.comp, open: c.open, ids: [...ids] }, (indexes: number[]) => indexes.map(i => c.liveCur.layers[i]));
    }, () => this.legacy.transformRoots(ids), () => ({ ids }));
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
    return this.answer('space3d.is3DLayer', () => this.layerQuery('is3DLayer', L, {}), () => this.legacy.is3DLayer(L), () => ({ layer: L?.id }));
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
    return this.answer('gl.bounds', () => this.layerQuery('bounds', L, { t: T }), () => this.legacy.bounds(L, T), () => ({ layer: L?.id, t: T }));
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
    }, () => this.legacy.pick(x, y, T, options), () => ({ x, y, t: T }));
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
      return this.query('pathValues', { comp: ctx.comp, open: ctx.open, layer: ctx.layer, index, t: time, ...(prefix === undefined ? {} : { prefix }) });
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
