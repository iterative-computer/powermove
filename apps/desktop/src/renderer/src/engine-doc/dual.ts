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
 * Live objects (layers, channels, keyframes) compare by identity; object keys
 * compare as sets (pm-model stores members sorted, allowed difference 5 in
 * docs/engine/pm-eval.md). A throw matches a throw. The native answer is the
 * one returned; mismatches are recorded and reported.
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
  mismatched: Record<string, number>;
  /** The first mismatches of each operation (bounded). */
  samples: Mismatch[];
}

type Answer = { value?: unknown; threw?: unknown };

const BOUNDS_OPS = new Set(['gl.bounds', 'groups.bounds']);
const TEXT_OPS = new Set(['textLayout', 'rasterMetrics']);
const MAX_SAMPLES_PER_OP = 20;

function tolerance(op: string): (a: number, b: number) => boolean {
  if (BOUNDS_OPS.has(op)) return (a, b) => Math.abs(a - b) <= 1e-6;
  if (TEXT_OPS.has(op)) return (a, b) => Math.abs(a - b) <= 0.25;
  return (a, b) => Math.abs(a - b) <= 1e-9 + 1e-9 * Math.abs(b);
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
  readonly stats: DualStats = { compared: {}, mismatched: {}, samples: [] };
  private perOp = new Map<string, number>();

  constructor(private report: (mismatch: Mismatch) => void = () => {}) {}

  compare(op: string, native: Answer, legacy: Answer, where: () => Record<string, unknown>, unwrap: <T>(v: T) => T): boolean {
    this.stats.compared[op] = (this.stats.compared[op] ?? 0) + 1;
    let path: string | null;
    if ('threw' in native || 'threw' in legacy) {
      path = ('threw' in native) === ('threw' in legacy) ? null : '(throw)';
    } else {
      // allProps lists object members in pm-model's sorted order (allowed
      // difference 5): compare the listings as sets of keys.
      const a = op === 'allProps' ? byKey(native.value) : native.value;
      const b = op === 'allProps' ? byKey(legacy.value) : legacy.value;
      path = identity(op, a, b, unwrap) ?? differs(a, b, tolerance(op), unwrap);
    }
    if (path === null) return true;
    this.stats.mismatched[op] = (this.stats.mismatched[op] ?? 0) + 1;
    const n = this.perOp.get(op) ?? 0;
    if (n < MAX_SAMPLES_PER_OP) {
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
    return false;
  }

  total(): { compared: number; mismatched: number } {
    const sum = (r: Record<string, number>) => Object.values(r).reduce((a, b) => a + b, 0);
    return { compared: sum(this.stats.compared), mismatched: sum(this.stats.mismatched) };
  }
}

const byKey = (list: unknown) => Array.isArray(list)
  ? [...list].sort((x: any, y: any) => (String(x?.key) < String(y?.key) ? -1 : String(x?.key) > String(y?.key) ? 1 : 0))
  : list;

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
