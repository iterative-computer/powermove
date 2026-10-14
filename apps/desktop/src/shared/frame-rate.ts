/*
 * The single frame-rate validator (docs/rust-engine.md, section 5, rule 1):
 * the UI, `set_composition`, project load and validation, Home's media
 * matching, and MCP `create_project` all resolve rates here, and the result
 * is stored as `fps` plus `fpsRational`.
 *
 * Resolution is pm-time's (`engine/crates/pm-time/src/snap.rs`,
 * `rate_from_document`), reproduced exactly and checked against pm-wasm:
 *
 * 1. a valid `fpsRational` wins;
 * 2. a float within 0.02% of k × 1000/1001 (k in 24, 30, 48, 60, 120, 240) is
 *    that NTSC rate;
 * 3. otherwise within 0.02% of an integer supported rate, that rate;
 * 4. otherwise the rational with the smallest denominator within 1e-9 of the
 *    float's exact binary value.
 *
 * New rates (typed, set by a command, created, or matched to media) must
 * divide the flick (705,600,000 ticks per second) and lie in 1 to 240 fps;
 * they are stored as `num / den` so the float and the rational agree. Rates
 * already in a document keep their float and keep working (pm-time's ceil and
 * Euclidean frame math handles any rational); load only adds `fpsRational`.
 * Media rates are nominal: a measured average that is not a valid new rate
 * becomes the supported rate whose frame duration is closest (section 13,
 * item 5 and amendment 2).
 */

/** Engine ticks per second (the flick). */
export const TICKS_PER_SECOND = 705_600_000;

/** The 16 supported rates of section 5. */
export const SUPPORTED_FRAME_RATES: ReadonlyArray<readonly [number, number]> = [
  [24000, 1001], [24, 1], [25, 1], [30000, 1001], [30, 1], [48000, 1001], [48, 1], [50, 1],
  [60000, 1001], [60, 1], [90, 1], [100, 1], [120000, 1001], [120, 1], [240000, 1001], [240, 1],
];

const NTSC_MULTIPLES = [24, 30, 48, 60, 120, 240];
const SNAP_INTEGER_RATES = [24, 25, 30, 48, 50, 60, 90, 100, 120, 240];
const SNAP_RELATIVE_TOLERANCE = 0.0002;
const RATIONAL_TOLERANCE = 1e-9;
export const MIN_FRAME_RATE = 1;
export const MAX_FRAME_RATE = 240;

export type RateSource = 'exact' | 'ntsc' | 'integer' | 'approximated';

export interface ResolvedRate {
  num: number;
  den: number;
  source: RateSource;
}

const gcd = (a: bigint, b: bigint): bigint => { while (b) [a, b] = [b, a % b]; return a < 0n ? -a : a; };

/** pm-time's `Rate::new`: reduced with a positive denominator, 0 < num/den ≤ the flick. */
function rate(num: number, den: number): ResolvedRate | null {
  if (!Number.isSafeInteger(num) || !Number.isSafeInteger(den) || den === 0) return null;
  let n = BigInt(num), d = BigInt(den);
  const g = gcd(n, d);
  n /= g; d /= g;
  if (d < 0n) { n = -n; d = -d; }
  if (n <= 0n || n > BigInt(TICKS_PER_SECOND) * d) return null;
  return { num: Number(n), den: Number(d), source: 'exact' };
}

/** `[num, den]` from a document's `fpsRational` when both are safe integers. */
export function fpsRationalOf(value: unknown): [number, number] | null {
  if (!Array.isArray(value) || value.length !== 2) return null;
  const [a, b] = value;
  return Number.isSafeInteger(a) && Number.isSafeInteger(b) ? [a, b] : null;
}

/** The exact value of a positive finite double as mantissa × 2^exponent. */
function decompose(value: number): [bigint, number] {
  const view = new DataView(new ArrayBuffer(8));
  view.setFloat64(0, value);
  const bits = view.getBigUint64(0);
  const exponent = Number((bits >> 52n) & 0x7ffn);
  const fraction = bits & ((1n << 52n) - 1n);
  if (exponent === 0) return [fraction, -1074];
  return [fraction | (1n << 52n), exponent - 1075];
}

/** Simplest rational in [a/b, c/d] (Stern–Brocot), as pm-time computes it. */
function simplestInClosed(a: bigint, b: bigint, c: bigint, d: bigint): [bigint, bigint] | null {
  const floorDiv = (x: bigint, y: bigint) => { const q = x / y; return (x % y !== 0n && (x < 0n) !== (y < 0n)) ? q - 1n : q; };
  const fl = floorDiv(a, b);
  if (fl * b === a) return [fl, 1n];
  if ((fl + 1n) * d <= c) return [fl + 1n, 1n];
  const b2 = c - fl * d;
  if (b2 === 0n) return null;
  const inner = simplestInClosed(d, b2, b, a - fl * b);
  if (!inner) return null;
  const [p, q] = inner;
  return [fl * p + q, p];
}

function simplestRationalNear(value: number, tolerance: number): ResolvedRate | null {
  if (!(Number.isFinite(value) && value > tolerance)) return null;
  const [vm, ve] = decompose(value);
  const [tm, te] = decompose(tolerance);
  const shift = Math.max(0, -Math.min(ve, te));
  if (shift > 120) return null;
  const v = vm << BigInt(ve + shift);
  const t = tm << BigInt(te + shift);
  const den = 1n << BigInt(shift);
  const found = simplestInClosed(v - t, den, v + t, den);
  if (!found) return null;
  const [p, q] = found;
  if (p > BigInt(Number.MAX_SAFE_INTEGER) || q > BigInt(Number.MAX_SAFE_INTEGER)) return null;
  const r = rate(Number(p), Number(q));
  return r && { ...r, source: 'approximated' };
}

/** pm-time's `snap_float_rate`. */
export function snapFloatRate(fps: number): ResolvedRate | null {
  if (!Number.isFinite(fps) || fps <= 0 || fps > TICKS_PER_SECOND) return null;
  for (const k of NTSC_MULTIPLES) {
    const target = (k * 1000) / 1001;
    if (Math.abs(fps - target) <= SNAP_RELATIVE_TOLERANCE * target) return { ...rate(k * 1000, 1001)!, source: 'ntsc' };
  }
  for (const k of SNAP_INTEGER_RATES) {
    if (Math.abs(fps - k) <= SNAP_RELATIVE_TOLERANCE * k) return { num: k, den: 1, source: 'integer' };
  }
  return simplestRationalNear(fps, RATIONAL_TOLERANCE);
}

/** pm-time's `rate_from_document`: `fpsRational` when valid, else the float. */
export function resolveFrameRate(fps: unknown, fpsRational?: unknown): ResolvedRate | null {
  const pair = fpsRationalOf(fpsRational);
  if (pair) {
    const exact = rate(pair[0], pair[1]);
    if (exact) return exact;
  }
  return snapFloatRate(typeof fps === 'number' ? fps : NaN);
}

/** Whether a frame lasts a whole number of ticks. */
export const dividesFlick = (r: { num: number; den: number }) => (BigInt(TICKS_PER_SECOND) * BigInt(r.den)) % BigInt(r.num) === 0n;

/** Whether the rate is one of the 16 supported rates. */
export const isSupportedRate = (r: { num: number; den: number }) => SUPPORTED_FRAME_RATES.some(([n, d]) => n === r.num && d === r.den);

export type FrameRateCheck =
  | { ok: true; fps: number; fpsRational: [number, number]; source: RateSource }
  | { ok: false; message: string };

export type FrameRateUse =
  /** Typed in the UI, set by `set_composition`, or `create_project`. */
  | 'new'
  /** Read from a document (load, validation): unusual rates keep working. */
  | 'document'
  /** A media file's measured rate (Home's media matching). */
  | 'media';

/** The supported rate whose frame duration is closest to `fps`'s. */
export function nominalRate(fps: number): [number, number] | null {
  if (!Number.isFinite(fps) || fps <= 0) return null;
  let best: readonly [number, number] | null = null, bestDelta = Infinity;
  for (const r of SUPPORTED_FRAME_RATES) {
    const delta = Math.abs(r[1] / r[0] - 1 / fps);
    if (delta < bestDelta) { best = r; bestDelta = delta; }
  }
  return best ? [best[0], best[1]] : null;
}

const toNumber = (value: unknown): number => {
  if (typeof value === 'number') return value;
  if (typeof value === 'string' && value.trim() !== '') return Number(value);
  return NaN;
};

/**
 * Validates a frame rate for `use`. A new or media rate is stored as
 * `num / den`; a document rate keeps its float.
 */
export function validateFrameRate(value: unknown, use: FrameRateUse = 'new', fpsRational?: unknown): FrameRateCheck {
  const fps = toNumber(value);
  let resolved = resolveFrameRate(fps, use === 'document' ? fpsRational : undefined);
  if (use === 'media' && (!resolved || !dividesFlick(resolved) || resolved.num / resolved.den > MAX_FRAME_RATE || resolved.num / resolved.den < MIN_FRAME_RATE)) {
    const nominal = nominalRate(fps);
    resolved = nominal ? { num: nominal[0], den: nominal[1], source: nominal[1] === 1 ? 'integer' : 'ntsc' } : null;
  }
  if (!resolved) return { ok: false, message: 'Frame rate must be a positive number of frames per second.' };
  const exact = resolved.num / resolved.den;
  if (use !== 'document') {
    if (exact < MIN_FRAME_RATE || exact > MAX_FRAME_RATE) return { ok: false, message: `Frame rate must be between ${MIN_FRAME_RATE} and ${MAX_FRAME_RATE} fps.` };
    if (!dividesFlick(resolved)) return { ok: false, message: `${formatFrameRate(fps)} fps is not a supported frame rate. Use a rate such as 23.976, 24, 25, 29.97, 30, 50, 59.94, or 60 fps.` };
  }
  const stored = use === 'document' && Number.isFinite(fps) && fps > 0 ? fps : exact;
  return { ok: true, fps: stored, fpsRational: [resolved.num, resolved.den], source: resolved.source };
}

/** A rate for display: integers as they are, others to three decimals. */
export function formatFrameRate(fps: number): string {
  if (!Number.isFinite(fps)) return String(fps);
  return Number.isInteger(fps) ? String(fps) : String(Number(fps.toFixed(3)));
}

/**
 * Applies a validated rate to a project or composition object: `fps` and
 * `fpsRational` together. Returns the check (unchanged object on failure).
 */
export function applyFrameRate(target: { fps?: unknown; fpsRational?: unknown }, value: unknown, use: FrameRateUse = 'new'): FrameRateCheck {
  const check = validateFrameRate(value, use, use === 'document' ? target.fpsRational : undefined);
  if (check.ok) {
    target.fps = check.fps;
    target.fpsRational = check.fpsRational;
  }
  return check;
}

/**
 * A typed rate for the UI: blank keeps `fallback`; otherwise clamped to 1 to
 * 240, then validated as a new rate; a rate that does not divide the flick
 * becomes the nearest whole rate that does, else the nominal supported rate.
 */
export function coerceFrameRate(value: unknown, fallback: number): number {
  if (typeof value === 'string' && !value.trim()) return fallback;
  const number = toNumber(value);
  if (!Number.isFinite(number)) return fallback;
  const clamped = Math.min(MAX_FRAME_RATE, Math.max(MIN_FRAME_RATE, number));
  for (const candidate of [clamped, Math.round(clamped)]) {
    const check = validateFrameRate(candidate, 'new');
    if (check.ok) return check.fps;
  }
  const check = validateFrameRate(clamped, 'media');
  return check.ok ? check.fps : fallback;
}
