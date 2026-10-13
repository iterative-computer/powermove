/*
 * Dual-answer mode (docs/rust-engine.md, section 6, item 6; Phase 1 tests):
 * every synchronous query is answered by pm-wasm and by the legacy code, and
 * the answers are compared with section 25's tolerances:
 *
 * - evaluation: |a − b| ≤ 1e-9 + 1e-9·|b|, exact strings and booleans;
 * - bounds (`gl.bounds`, `groups.bounds`): within 1e-6;
 * - text layout and raster placement: within 0.25 px (section 12's advance
 *   tolerance), identical line breaks and text.
 *
 * Live objects (layers, channels, keyframes) compare by identity, and object
 * members and listings in order: the native API restores the document's member
 * order (pm-eval lists them sorted, allowed difference 5 in
 * docs/engine/pm-eval.md), so an order change is a mismatch. A throw matches a
 * throw. The native answer is the
 * one returned; mismatches are recorded and reported.
 *
 * Allowed differences (docs/rust-engine.md, section 26, amendments 16 and 20;
 * docs/engine/pm-eval-layers.md), counted under `allowed`, never as
 * mismatches:
 *
 * - `atan2`: a number that is 180 on one side and −180 on the other (a
 *   rotation exactly 360° apart at the `atan2` branch cut);
 * - `fragile`: a bounds or pick case whose legacy answer itself changes under
 *   a 1e-12 s time or 1e-7 px point perturbation. Every bounds and pick
 *   comparison is classified, so `fragile` counts the population and
 *   `allowed['<op>:fragile']` the fragile cases that differ; the gate allows
 *   at most 5% of fragile cases to differ (`fragileBudget()`);
 * - `nested-retime` (amendment 16): variable-speed source times of nested
 *   compositions. No routed query integrates speed in Phase 1 (`sourceTime`
 *   is not a kernel query), so this class is declared and stays at zero.
 */

export interface Mismatch {
  op: string;
  where: Record<string, unknown>;
  path: string;
  native: string;
  legacy: string;
}

export interface DualStats {
  compared: Record<string, number>;
  /** Evaluation mismatches: the answers differ on the same document. */
  mismatched: Record<string, number>;
  /** Answers that differed because the mirror missed a write (a barrier
   *  gap, repaired and re-answered; counted, not evaluation mismatches). */
  stale: Record<string, number>;
  /** Differences of an allowed class, by `<op>:<class>` (never mismatches). */
  allowed: Record<string, number>;
  /** Bounds and pick comparisons whose legacy answer is fragile, by op. */
  fragile: Record<string, number>;
  /** The first mismatches of each operation (bounded). */
  samples: Mismatch[];
}

/** Classes of allowed differences (amendments 16 and 20). */
export type AllowedClass = 'atan2' | 'fragile' | 'nested-retime';

/** The share of fragile bounds and pick cases allowed to differ (amendment 20). */
export const FRAGILE_BUDGET = 0.05;

type Answer = { value?: unknown; threw?: unknown };

const BOUNDS_OPS = new Set(['gl.bounds', 'groups.bounds']);
const TEXT_OPS = new Set(['textLayout', 'rasterMetrics']);
const MAX_SAMPLES_PER_OP = 20;

export const FRAGILE_OPS = new Set(['gl.bounds', 'groups.bounds', 'gl.pick']);

function tolerance(op: string): (a: number, b: number) => boolean {
  if (BOUNDS_OPS.has(op)) return (a, b) => Math.abs(a - b) <= 1e-6;
  if (TEXT_OPS.has(op)) return (a, b) => Math.abs(a - b) <= 0.25;
  return (a, b) => Math.abs(a - b) <= 1e-9 + 1e-9 * Math.abs(b);
}

/** 180 against −180 (either way), within the evaluation tolerance. */
export function atan2BranchCut(a: number, b: number): boolean {
  const near = (x: number, y: number) => Math.abs(x - y) <= 1e-9 + 1e-9 * Math.abs(y);
  return (near(a, 180) && near(b, -180)) || (near(a, -180) && near(b, 180));
}

const show = (v: unknown): string => {
  if (v === undefined) return 'undefined';
  if (typeof v === 'number') return Object.is(v, -0) ? '-0' : String(v);
  if (v instanceof Error) return `${v.name}: ${v.message}`;
  try {
    const text = JSON.stringify(v, (_k, x) => (typeof x === 'number' && !Number.isFinite(x) ? String(x) : x));
    return text === undefined ? String(v) : text.length > 400 ? text.slice(0, 400) + '…' : text;
  } catch { return String(v); }
};

/** Where `a` and `b` first differ, or null when they agree. */
export function differs(a: unknown, b: unknown, close: (a: number, b: number) => boolean, unwrap: <T>(v: T) => T = v => v, path = '', seen = new Set<unknown>()): string | null {
  a = unwrap(a); b = unwrap(b);
  if (Object.is(a, b)) return null;
  if (typeof a === 'number' && typeof b === 'number') {
    if (Number.isNaN(a) && Number.isNaN(b)) return null;
    // -0 and 0 are equal under the tolerance, as they are to every consumer.
    return close(a, b) ? null : path || '.';
  }
  if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) return path || '.';
  if (seen.has(a)) return null;
  seen.add(a);
  if (Array.isArray(a) !== Array.isArray(b)) return path || '.';
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return `${path}.length`;
    for (let i = 0; i < a.length; i++) {
      const d = differs(a[i], b[i], close, unwrap, `${path}[${i}]`, seen);
      if (d) return d;
    }
    return null;
  }
  const ka = Object.keys(a as object).filter(k => (a as any)[k] !== undefined);
  const kb = Object.keys(b as object).filter(k => (b as any)[k] !== undefined);
  if (ka.length !== kb.length || ka.some(k => !kb.includes(k))) {
    const missing = ka.filter(k => !kb.includes(k)).concat(kb.filter(k => !ka.includes(k)));
    return `${path}{${missing.join(',')}}`;
  }
  for (const k of ka) {
    const d = differs((a as any)[k], (b as any)[k], close, unwrap, `${path}.${k}`, seen);
    if (d) return d;
  }
  return null;
}

export class DualAnswers {
  readonly stats: DualStats = { compared: {}, mismatched: {}, stale: {}, allowed: {}, fragile: {}, samples: [] };
  private perOp = new Map<string, number>();

  constructor(private report: (mismatch: Mismatch) => void = () => {}) {}

  /** Where two answers differ beyond the tolerances, or null. */
  check(op: string, native: Answer, legacy: Answer, unwrap: <T>(v: T) => T): string | null {
    return this.classify(op, native, legacy, unwrap).path;
  }

  /**
   * Where two answers differ beyond the tolerances (or null), and the allowed
   * class that let a difference through (counted by `record`).
   */
  classify(op: string, native: Answer, legacy: Answer, unwrap: <T>(v: T) => T): { path: string | null; allowed: AllowedClass | null } {
    if ('threw' in native || 'threw' in legacy) return { path: ('threw' in native) === ('threw' in legacy) ? null : '(throw)', allowed: null };
    // Order matters: the native API restores the document's member order
    // (pm-eval's own listing is sorted, allowed difference 5).
    const within = tolerance(op);
    let allowed: AllowedClass | null = null;
    const close = (a: number, b: number) => {
      if (within(a, b)) return true;
      if (!BOUNDS_OPS.has(op) && !TEXT_OPS.has(op) && atan2BranchCut(a, b)) { allowed = 'atan2'; return true; }
      return false;
    };
    const path = identity(op, native.value, legacy.value, unwrap) ?? differs(native.value, legacy.value, close, unwrap) ?? keyOrderDiffers(native.value, legacy.value);
    return { path, allowed: path === null ? allowed : null };
  }

  /** Counts a difference of an allowed class. */
  allow(op: string, kind: AllowedClass): void {
    const k = `${op}:${kind}`;
    this.stats.allowed[k] = (this.stats.allowed[k] ?? 0) + 1;
  }

  /** Counts a fragile bounds or pick case (the population of the 5% budget). */
  noteFragile(op: string): void {
    this.stats.fragile[op] = (this.stats.fragile[op] ?? 0) + 1;
  }

  /** Fragile cases and how many of them differed, with the 5% verdict. */
  fragileBudget(): { fragile: number; differed: number; ok: boolean } {
    const fragile = Object.values(this.stats.fragile).reduce((a, b) => a + b, 0);
    const differed = Object.entries(this.stats.allowed).filter(([k]) => k.endsWith(':fragile')).reduce((a, [, n]) => a + n, 0);
    return { fragile, differed, ok: differed <= FRAGILE_BUDGET * fragile };
  }

  /** Counts a comparison and records an evaluation mismatch when `path` is set. */
  record(op: string, path: string | null, native: Answer, legacy: Answer, where: () => Record<string, unknown>): void {
    this.stats.compared[op] = (this.stats.compared[op] ?? 0) + 1;
    if (path === null) return;
    this.stats.mismatched[op] = (this.stats.mismatched[op] ?? 0) + 1;
    const n = this.perOp.get(op) ?? 0;
    if (n >= MAX_SAMPLES_PER_OP) return;
    this.perOp.set(op, n + 1);
    const mismatch: Mismatch = {
      op,
      where: safe(where),
      path,
      native: 'threw' in native ? `throw ${show(native.threw)}` : show(native.value),
      legacy: 'threw' in legacy ? `throw ${show(legacy.threw)}` : show(legacy.value),
    };
    this.stats.samples.push(mismatch);
    this.report(mismatch);
  }

  /** `classify` then `record`; true when the answers agree. */
  compare(op: string, native: Answer, legacy: Answer, where: () => Record<string, unknown>, unwrap: <T>(v: T) => T): boolean {
    const { path, allowed } = this.classify(op, native, legacy, unwrap);
    if (allowed) this.allow(op, allowed);
    this.record(op, path, native, legacy, where);
    return path === null;
  }

  total(): { compared: number; mismatched: number; stale: number; allowed: number } {
    const sum = (r: Record<string, number>) => Object.values(r).reduce((a, b) => a + b, 0);
    return { compared: sum(this.stats.compared), mismatched: sum(this.stats.mismatched), stale: sum(this.stats.stale), allowed: sum(this.stats.allowed) };
  }
}

/** Where two equal answers list object members in a different order, or null. */
function keyOrderDiffers(a: unknown, b: unknown, path = ''): string | null {
  if (!a || !b || typeof a !== 'object' || typeof b !== 'object') return null;
  if (Array.isArray(a) && Array.isArray(b)) {
    for (let i = 0; i < Math.min(a.length, b.length); i++) { const d = keyOrderDiffers(a[i], b[i], `${path}[${i}]`); if (d) return d; }
    return null;
  }
  const ka = Object.keys(a).filter(k => (a as any)[k] !== undefined), kb = Object.keys(b).filter(k => (b as any)[k] !== undefined);
  if (ka.join('\u0000') !== kb.join('\u0000')) return `${path}(member order)`;
  for (const k of ka) {
    const x = (a as any)[k], y = (b as any)[k];
    // Live document objects are the same objects; only answers are compared.
    if (x === y) continue;
    const d = keyOrderDiffers(x, y, `${path}.${k}`);
    if (d) return d;
  }
  return null;
}

/** Live objects in answers (layers, channels, keyframes) must be the very same objects. */
function identity(op: string, a: any, b: any, unwrap: <T>(v: T) => T): string | null {
  const same = (x: unknown, y: unknown) => unwrap(x) === unwrap(y);
  switch (op) {
    case 'findProp': case 'hasKeyAt': case 'gl.pick':
      return same(a, b) ? null : '(identity)';
    case 'groups.ancestors': case 'groups.transformRoots':
      if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return '.length';
      return a.every((x, i) => same(x, b[i])) ? null : '(identity)';
    case 'allProps':
      if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return null; // differs() reports it
      for (let i = 0; i < a.length; i++) if (a[i] && b[i] && typeof a[i].prop === 'object' && a[i].prop && !same(a[i].prop, b[i].prop)) return `[${i}].prop(identity)`;
      return null;
    default:
      return null;
  }
}

function safe(where: () => Record<string, unknown>): Record<string, unknown> {
  try { return where(); } catch { return {}; }
}
