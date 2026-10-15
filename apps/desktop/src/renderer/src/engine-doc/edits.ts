/*
 * The edit kernels (`pm-edit`, docs/engine/pm-edit.md) behind the native
 * evaluator flag (docs/rust-engine.md, section 6, item 4; section 26,
 * Phase 1: "geometry-dependent edits switch to pm-wasm behind a flag").
 *
 * The renderer stays the authority for the document and Undo. A kernel reads
 * pm-wasm's mirror and answers with canonical patch operations (contract 5's
 * JSON form); they are applied here to the live document, inside the caller's
 * history transaction, so Undo records them like any other write. Kernels
 * never touch the mirror: the applied operations reach it through the barrier
 * (`PM.doc.write`).
 *
 * Every entry point takes the legacy implementation as a callback:
 *
 * - module not ready, or a question pm-wasm cannot answer in Phase 1
 *   (three.js geometry, fonts not served, ink without outlines): the legacy
 *   code runs (counted under `fallbacks`);
 * - `on`: the kernel's operations are applied and the legacy code does not
 *   run; a legacy throw is the kernel's throw (same message);
 * - dual modes (`audit`, `audit-proxy`, `strict`): the legacy code runs and
 *   its result is what the document keeps; the kernel's operations, applied
 *   to a copy of the document as it was before, must give the same
 *   compositions within section 25's evaluation tolerance (created ids are
 *   compared by order of appearance, numbers inside expression strings with
 *   the same tolerance). Differences are recorded as dual mismatches under
 *   `edit.<name>`.
 */
import type { PathPart } from '../../../shared/patch';
import { applyOps, type PatchOp } from '../../../shared/patch-ids';
import { canonicalProject } from '../../../shared/canonical-json';
import { adoptTemporalEase } from '../core/anim/temporal-ease';
import { resolveFrameRate } from '../../../shared/frame-rate';
import type { DocumentBarrier } from './barrier';
import type { FontLoader } from './fonts';
import { atan2BranchCut, differs, type DualAnswers } from './dual';
import type { PmWasm } from './wasm';

type Legacy = Record<string, any>;

/** Pre-compose › Leave all attributes: the new composition's size and anchor shift. */
export type LeaveBounds = { w: number; h: number; ax: number; ay: number };

/** What a kernel answered. */
export type KernelReply = { ops: PatchOp[]; next: number; [extra: string]: unknown };

type Outcome = { reply: KernelReply } | { threw: Error } | { fallback: string };

export interface NativeEditStats {
  calls: Record<string, number>;
  /** Kernels whose operations were applied (mode `on`). */
  applied: Record<string, number>;
  /** Operations applied to the live document. */
  ops: number;
  fallbacks: Record<string, number>;
}

/** Section 25's evaluation tolerance, with the `atan2` branch cut allowed. */
const close = (a: number, b: number) => Math.abs(a - b) <= 1e-9 + 1e-9 * Math.abs(b) || atan2BranchCut(a, b);

/** Numbers inside a string (an expression a kernel extended). */
const NUMBER = /-?(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?|NaN|-?Infinity/gi;

/**
 * `a` and `b` with every id the `before` document lacks (members `i` and
 * `id`) renamed by order of first appearance, and strings with numbers split
 * into text and numbers, so created ids and re-printed numbers compare.
 */
export function normalizeEdit(value: any, known: Set<string>): any {
  const names = new Map<string, string>();
  const walk = (v: any, key: string | null): any => {
    if (Array.isArray(v)) return v.map(x => walk(x, null));
    if (v && typeof v === 'object') {
      const out: Record<string, any> = {};
      for (const k of Object.keys(v).sort()) if (v[k] !== undefined) out[k] = walk(v[k], k);
      return out;
    }
    if (typeof v === 'string' && (key === 'i' || key === 'id') && !known.has(v)) {
      if (!names.has(v)) names.set(v, `#created${names.size}`);
      return names.get(v);
    }
    if (typeof v === 'string' && key === 'expr') {
      const parts: any[] = [];
      let last = 0;
      for (const m of v.matchAll(NUMBER)) {
        parts.push(v.slice(last, m.index), Number(m[0]));
        last = m.index! + m[0].length;
      }
      parts.push(v.slice(last));
      return { '\u0000expr': parts };
    }
    return v;
  };
  return walk(value, null);
}

/** Every `i` and `id` string in a value. */
export function knownIds(value: any, out = new Set<string>()): Set<string> {
  if (Array.isArray(value)) { for (const v of value) knownIds(v, out); return out; }
  if (value && typeof value === 'object') {
    for (const [k, v] of Object.entries(value)) {
      if ((k === 'i' || k === 'id') && typeof v === 'string') out.add(v);
      else knownIds(v, out);
    }
  }
  return out;
}

/**
 * How far the compared compositions' float rates are from the exact rates
 * the native engine keeps their times on (`drift`: 29.97 against 30000/1001
 * is about 1e-6; 0 for exact rates), and the largest layer start (`span`).
 */
export interface TimeGrid { drift: number; span: number }

export function timeGrid(comps: Record<string, any> | null | undefined): TimeGrid {
  let drift = 0, span = 0;
  for (const comp of Object.values(comps ?? {})) {
    if (!comp || typeof comp !== 'object') continue;
    const fps = Number(comp.fps);
    const rate = resolveFrameRate(comp.fps, comp.fpsRational);
    if (rate && Number.isFinite(fps)) drift = Math.max(drift, Math.abs(fps / (rate.num / rate.den) - 1));
    for (const layer of Array.isArray(comp.layers) ? comp.layers : []) {
      const from = Number(layer?.from);
      if (Number.isFinite(from)) span = Math.max(span, Math.abs(from));
    }
  }
  return { drift, span };
}

const KEY_TIME = /\.kf\[\d+\]\.t$/;

/**
 * Where an edit's expected and actual results differ beyond the tolerances,
 * or null. With a `grid`, key times a kernel created or moved may also
 * differ by the float rate's drift: pm-edit computes them on the exact grid
 * (section 5, rule 4 with amendment 1) where the TypeScript edit adds float
 * frame times (`n / 29.97 + from - to`).
 */
export function editDiffers(expected: any, actual: any, before: any, grid?: TimeGrid): string | null {
  const known = before instanceof Set ? before as Set<string> : knownIds(before);
  const near = (a: number, b: number, path: string) => close(a, b) || (!!grid && grid.drift > 0 && KEY_TIME.test(path)
    && Math.abs(a - b) <= 2 * grid.drift * (Math.abs(a) + Math.abs(b) + grid.span + 1) + 1e-9);
  return differs(normalizeEdit(expected, known), normalizeEdit(actual, known), near);
}

const clone = <T,>(v: T): T => (v === undefined ? v : JSON.parse(JSON.stringify(v)));

/**
 * Every keyframe track in `value` (a copy) brought to the native ease model.
 * Legacy evaluation adopts stored `eo`/`ei`/`hold` handles in the live
 * document as a side effect of reading it (temporal-bridge.ts), so the legacy
 * edit leaves adopted keys wherever it evaluated; adoption is idempotent, so
 * both sides of a comparison are adopted first.
 */
export function adoptKeys<T>(value: T): T {
  const walk = (v: any) => {
    if (Array.isArray(v)) { for (const x of v) walk(x); return; }
    if (!v || typeof v !== 'object') return;
    const kf = v.kf;
    if (Array.isArray(kf) && kf.length && kf.every((k: any) => k && typeof k === 'object' && typeof k.t === 'number')) adoptTemporalEase(kf);
    for (const k of Object.keys(v)) walk(v[k]);
  };
  walk(value);
  return value;
}

/** A canonical path on the stored project whose open composition is `open`. */
export function realizePath(path: readonly PathPart[], open: string): PathPart[] {
  if (path[0] === 'comps' && path[1] === open && path.length >= 3) {
    const key = String(path[2]);
    const root = key === 'id' ? 'compId' : key === 'name' ? 'compName' : key;
    return [root, ...path.slice(3)];
  }
  return [...path];
}

export function realizeOps(ops: readonly PatchOp[], open: string): PatchOp[] {
  return ops.map(op => ({ ...op, path: realizePath(op.path, open) }) as PatchOp);
}

export class NativeEdits {
  readonly stats: NativeEditStats = { calls: {}, applied: {}, ops: 0, fallbacks: {} };

  constructor(
    private PM: Legacy,
    private wasm: () => PmWasm | null,
    private barrier: () => DocumentBarrier | null,
    private unwrap: <T>(v: T) => T,
    private dual: DualAnswers | null,
    /** The text layout's font loader (engine-doc/fonts.ts), as for queries. */
    private fonts: () => FontLoader | null = () => null,
  ) {}

  private count(table: Record<string, number>, key: string) { table[key] = (table[key] ?? 0) + 1; }

  /** The open composition, the composition being edited, and its live layers. */
  private scope(): { open: string; comp: string; layers: any[] } | null {
    const proj = this.unwrap(this.PM.proj);
    if (!proj || typeof proj.compId !== 'string') return null;
    // Edits work on the open composition (`PM.proj`); a nested scope is a
    // render in progress, never an edit.
    if (this.PM.scope?.length) return null;
    return { open: proj.compId, comp: proj.compId, layers: Array.isArray(proj.layers) ? proj.layers : [] };
  }

  /** The index of a live layer in the open composition, or -1. */
  index(layer: any): number {
    const s = this.scope();
    if (!s || !layer) return -1;
    return s.layers.indexOf(this.unwrap(layer));
  }

  /**
   * Asks kernel `name`. The open composition's layers (and the other
   * compositions when `wide`) are marked written first, so the mirror holds
   * every write of the transaction so far, whoever made it.
   */
  private ask(name: string, request: Record<string, unknown>, wide = false): Outcome {
    const wasm = this.wasm();
    const barrier = this.barrier();
    const s = this.scope();
    if (!wasm || wasm.dead || !barrier) return { fallback: 'unavailable' };
    if (!s) return { fallback: 'context' };
    try {
      this.PM.doc?.touched?.(wide ? [['layers'], ['comps']] : [['layers']]);
      barrier.flush();
    } catch (error) {
      console.warn('[engine-doc] flush failed', error);
      return { fallback: 'flush' };
    }
    if (wasm.dead) return { fallback: 'unavailable' };
    // Created ids are `prefix + base + n`: the separator keeps them apart
    // from every id `PM.uid` makes (`prefix + random`, or a counter in tests).
    const uid = { base: `${String(this.PM.uid?.('') ?? Math.random().toString(36).slice(2, 9))}_`, start: 1 };
    let reply: any;
    const run = () => wasm.query({ op: `edit.${name}`, comp: s.comp, open: s.open, uid, ...request });
    try {
      reply = run();
      // Text layout met fonts the mirror lacks: load them (at once from a
      // synchronous registry; in the background otherwise, and this edit
      // runs the legacy code) and ask again.
      const fonts = this.fonts();
      if (reply?.fonts && fonts) reply = fonts.resolve(wasm, run, (r: any) => !!r?.fonts) ?? reply;
    } catch (error) {
      console.warn(`[engine-doc] edit.${name} failed`, error);
      return { fallback: 'trap' };
    }
    if (reply && 'ok' in reply) return { reply: reply.ok as KernelReply };
    if (reply?.throw) {
      const Kind = reply.throw.kind === 'TypeError' ? TypeError : reply.throw.kind === 'RangeError' ? RangeError : Error;
      return { threw: new Kind(reply.throw.message) };
    }
    if (reply?.host) return { fallback: 'host' };
    if (reply?.fonts) return { fallback: 'fonts' };
    if (reply?.ink) return { fallback: 'ink' };
    return { fallback: `error:${reply?.error ?? 'unknown'}` };
  }

  /** Applies a kernel's operations to the live document as a known writer. */
  apply(ops: readonly PatchOp[]): void {
    const s = this.scope();
    if (!s || !ops.length) return;
    const live = realizeOps(ops, s.open);
    const paths = live.map(op => [...op.path]);
    const write = () => { this.PM.proj = applyOps(this.PM.proj, live); };
    if (this.PM.doc?.write) this.PM.doc.write(paths, write);
    else write();
    this.stats.ops += live.length;
  }

  /** Comps a kernel's operations touch, plus the edited one. */
  private touchedComps(ops: readonly PatchOp[], comp: string): string[] {
    const ids = new Set<string>([comp]);
    for (const op of ops) if (op.path[0] === 'comps' && typeof op.path[1] === 'string') ids.add(op.path[1]);
    return [...ids];
  }

  /**
   * A document edit: the kernel's operations (mode `on`), or the legacy edit
   * compared with them (dual modes), or the legacy edit alone (fallback).
   * `done` receives the reply after the operations are applied and gives the
   * caller's result.
   */
  edit<T>(name: string, request: Record<string, unknown>, legacy: () => T, done: (reply: KernelReply) => T, options: {
    wide?: boolean;
    where?: () => Record<string, unknown>;
    /** Dual modes: compare the reply with the legacy result instead of the documents (null: same). */
    compareWith?: (reply: KernelReply, legacyResult: T, before: Set<string>) => string | null;
  } = {}): T {
    this.count(this.stats.calls, name);
    const got = this.ask(name, request, options.wide);
    if ('fallback' in got) { this.count(this.stats.fallbacks, `${name}:${got.fallback}`); return legacy(); }
    if (this.dual) return this.compare(name, got, legacy, options.where ?? (() => request), options.compareWith);
    if ('threw' in got) throw got.threw;
    this.apply(got.reply.ops);
    this.count(this.stats.applied, name);
    return done(got.reply);
  }

  /**
   * A kernel whose answer is a value (anchor bounds, alignment moves,
   * composition bounds): the native answer (mode `on`), or the legacy one
   * compared with it (dual modes, where the legacy answer is what the caller
   * goes on with, as for `edit`). `ops` (if any) are applied by the caller
   * through `apply` where the legacy code writes.
   */
  answer<T>(name: string, request: Record<string, unknown>, native: (reply: KernelReply) => T, legacy: () => T, compare: (native: T, legacy: T) => string | null = (a, b) => differs(a, b, close), options: { wide?: boolean } = {}): T {
    this.count(this.stats.calls, name);
    const got = this.ask(name, request, options.wide);
    if ('fallback' in got) { this.count(this.stats.fallbacks, `${name}:${got.fallback}`); return legacy(); }
    const value = 'threw' in got ? undefined : native(got.reply);
    if (this.dual) {
      let expected: { value?: T; threw?: unknown };
      try { expected = { value: legacy() }; } catch (error) { expected = { threw: error }; }
      const nativeAnswer = 'threw' in got ? { threw: got.threw } : { value };
      let path: string | null;
      if ('threw' in got || 'threw' in expected) {
        path = ('threw' in got) && ('threw' in expected) && (got as any).threw.message === (expected.threw as Error)?.message ? null : '(throw)';
      } else path = compare(value as T, expected.value as T);
      this.dual.record(`edit.${name}`, path, nativeAnswer, expected, () => request);
      if ('threw' in expected) throw expected.threw;
      return expected.value as T;
    }
    if ('threw' in got) throw got.threw;
    this.count(this.stats.applied, name);
    return value as T;
  }

  /**
   * The alignment strip's `set_property` commands for aligning `layers`
   * (`mode`: left, hcenter, right, top, vcenter, bottom) to the composition
   * frame at `time`: pm-edit's `align` kernel when every layer is in the open
   * composition, `legacy` otherwise (and compared with it in dual modes).
   */
  align<T>(layers: any[], mode: string, time: number, legacy: () => T): T {
    const indexes = layers.map(layer => this.index(layer));
    if (!indexes.length || indexes.some(i => i < 0)) return legacy();
    const live = this.scope()?.layers ?? [];
    const commands = (reply: KernelReply) => (reply.moves as Array<{ layer: number; channel: string; value: number }>).map(move => ({
      type: 'set_property', target: live[move.layer]?.id, path: move.channel, value: move.value, time, mode: 'auto', preserveHandEdits: false,
    })) as unknown as T;
    return this.answer('align', { layers: indexes, mode, t: time }, commands, legacy);
  }

  /**
   * Pre-compose › Leave all attributes: the new composition's size and the
   * anchor shift (`comp_bounds`), with the shift as operations the caller
   * applies inside its transaction (`ops`, null when the legacy code runs).
   */
  leaveAttributes(layer: any, time: number, legacy: () => LeaveBounds): LeaveBounds & { ops: PatchOp[] | null } {
    const index = this.index(layer);
    type Answer = LeaveBounds & { ops: PatchOp[] | null };
    const plain = (): Answer => ({ ...legacy(), ops: null });
    if (index < 0) return plain();
    const box = (b: LeaveBounds) => ({ w: b.w, h: b.h, ax: b.ax, ay: b.ay });
    return this.answer<Answer>('leaveAttributes', { layer: index, t: time },
      reply => ({ w: reply.w as number, h: reply.h as number, ax: reply.ax as number, ay: reply.ay as number, ops: reply.ops }), plain,
      (native, old) => differs(box(native), box(old), close));
  }

  /**
   * The bounds an anchor edit rescales recorded font or size bounds against
   * (`anchor_reference`): the raster selection with the font anchor applied,
   * or the native size bounds shifted by the size anchor; null when nothing
   * is recorded. Font bounds compare within section 12's 0.25 px raster
   * placement.
   */
  anchorReference(layer: any, kind: 'font' | 'size', time: number, legacy: () => any): any {
    const index = this.index(layer);
    if (index < 0) return legacy();
    const box = (b: any) => (b ? { x0: b.x0, y0: b.y0, x1: b.x1, y1: b.y1 } : null);
    const compare = kind === 'font'
      ? (a: any, b: any) => differs(box(a), box(b), (x, y) => Math.abs(x - y) <= 0.25)
      : (a: any, b: any) => differs(box(a), box(b), close);
    return this.answer('anchorReference', { layer: index, kind, t: time }, reply => reply.bounds, legacy, compare);
  }

  /**
   * Split text (`split_text`): the kernel lays the pieces out and builds the
   * new layers; `apply` inserts them (one list per source), hides the
   * sources, and groups them, as the legacy split does. Returns null when a
   * source is outside the open composition (the caller runs `legacy`).
   */
  splitText<T>(sources: any[], mode: string, time: number, legacy: () => T, apply: (children: any[][]) => T): T | null {
    const indexes = sources.map(source => this.index(source));
    if (indexes.some(i => i < 0)) return null;
    return this.edit('splitText', { sources: indexes, mode, t: time }, legacy,
      reply => apply((reply.plans as Array<{ children: any[] }>).map(plan => plan.children)),
      { compareWith: (reply, created, known) => compareSplit(reply.plans as Array<{ children: any[] }>, created, known) });
  }

  /** Dual modes: run the legacy edit and compare the document it leaves with the kernel's. */
  private compare<T>(name: string, got: { reply: KernelReply } | { threw: Error }, legacy: () => T, where: () => Record<string, unknown>, compareWith?: (reply: KernelReply, legacyResult: T, before: Set<string>) => string | null): T {
    const dual = this.dual!;
    const s = this.scope()!;
    const shadow = this.barrier()?.shadow;
    const comps = 'reply' in got ? this.touchedComps(got.reply.ops, s.comp) : [s.comp];
    const before: Record<string, any> = {};
    for (const id of comps) before[id] = clone(shadow?.comps?.[id]);
    let result: T | undefined, threw: unknown, legacyThrew = false;
    try { result = legacy(); } catch (error) { threw = error; legacyThrew = true; }
    const op = `edit.${name}`;
    if ('threw' in got || legacyThrew) {
      const same = 'threw' in got && legacyThrew && got.threw.message === (threw as Error)?.message;
      dual.record(op, same ? null : '(throw)', 'threw' in got ? { threw: got.threw } : { value: '(applied)' }, legacyThrew ? { threw } : { value: '(applied)' }, where);
    } else if (compareWith) {
      const path = compareWith(got.reply, result as T, knownIds(before));
      dual.record(op, path, { value: path ? '(kernel)' : '(same)' }, { value: path ? '(legacy)' : '(same)' }, where);
    } else {
      const expected = applyOps({ comps: clone(before) }, got.reply.ops.filter(o => o.path[0] === 'comps')).comps;
      const live = canonicalProject(this.unwrap(this.PM.proj));
      const actual: Record<string, any> = {};
      for (const id of comps) actual[id] = clone(live?.comps?.[id]);
      const path = editDiffers(adoptKeys(expected), adoptKeys(actual), before, timeGrid(before));
      dual.record(op, path, { value: path ? pick(expected, path) : '(same)' }, { value: path ? pick(actual, path) : '(same)' }, where);
    }
    if (legacyThrew) throw threw;
    return result as T;
  }
}

/** The kernel's split layers against the ones the legacy split created
 *  (group membership aside: the new groups have fresh ids on both sides). */
export function compareSplit(plans: Array<{ children: any[] }>, created: unknown, known: Set<string>): string | null {
  const strip = (layers: any[]) => layers.map(layer => { const copy = clone(layer); delete copy.group; return copy; });
  const native = strip(plans.flatMap(plan => plan.children));
  // The legacy split answers `false` (nothing to split) or `PM.Edit.mutate`'s
  // result, whose `data.result` is the created layers.
  const result = (created as any)?.data?.result;
  const layers = Array.isArray(created) ? created : Array.isArray(result) ? result : null;
  if (!layers) return native.length ? '(nothing split)' : null;
  return editDiffers(native, strip(layers), known);
}

/** A short excerpt of a value around a `differs` path, for mismatch reports. */
function pick(value: any, path: string): unknown {
  const parts = path.split(/[.[\]{}]+/).filter(Boolean);
  let v = value;
  for (const p of parts.slice(0, 8)) { if (v == null || typeof v !== 'object' || !(p in v)) break; v = v[p]; }
  return v;
}
