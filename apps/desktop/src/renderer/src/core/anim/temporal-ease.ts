/**
 * After Effects temporal interpolation.
 *
 * AE does not store easing as a normalized unit-square curve the way CSS does.
 * Each side of a keyframe carries a *speed* (value units per second) and an
 * *influence* (percentage of the neighbouring segment's duration). Those two
 * numbers are only interchangeable with unit handles while a segment keeps the
 * same duration and value delta, so the native units are what make moving a
 * key, uneven segment lengths, flat segments and continuous tangents behave
 * the way they do in AE.
 *
 * Segment A -> B is the cubic through
 *   P0 = (tA, vA)
 *   P1 = (tA + fOut*dt, vA + fOut*dt*speedOut)
 *   P2 = (tB - fIn*dt,  vB - fIn*dt*speedIn)
 *   P3 = (tB, vB)
 * with f = influence/100 and dt = tB - tA.
 */

export type InterpolationType = 'linear' | 'bezier' | 'hold';

export interface TemporalEase {
  /** Value units per second at the keyframe. */
  speed: number;
  /** Percentage of the adjoining segment the handle spans, 0.1 – 100. */
  influence: number;
}

export interface EaseKeyframe {
  t: number;
  v: unknown;
  inInterp: InterpolationType;
  outInterp: InterpolationType;
  inEase: TemporalEase;
  outEase: TemporalEase;
  /** AE "Auto Bezier": the tangent is derived from the neighbouring keys. */
  autoBezier: boolean;
  /** AE "Continuous Bezier": both sides share one speed through the key. */
  continuous: boolean;
}

export interface TimeValue {
  t: number;
  v: number;
}

export type UnitHandle = [number, number];
export type UnitCurve = [number, number, number, number];

/** AE refuses to retract a handle completely; 0.1% is its floor. */
export const MIN_INFLUENCE = 0.1;
export const MAX_INFLUENCE = 100;
/** A linear side is the chord's own third — the exact straight line. */
export const LINEAR_INFLUENCE = 100 / 3;
/** AE's influence when a key is converted to Auto/Continuous Bezier. */
export const AUTO_INFLUENCE = 100 / 6;
/** AE's Easy Ease (F9). */
export const EASY_EASE_INFLUENCE = 100 / 3;

const EPSILON = 1e-9;

export const clampInfluence = (value: unknown): number => {
  const number = Number(value);
  if (!Number.isFinite(number)) return LINEAR_INFLUENCE;
  return number < MIN_INFLUENCE ? MIN_INFLUENCE : number > MAX_INFLUENCE ? MAX_INFLUENCE : number;
};

const finiteSpeed = (value: unknown): number => {
  const number = Number(value);
  return Number.isFinite(number) ? number : 0;
};

export const makeEase = (speed: unknown, influence: unknown): TemporalEase =>
  ({ speed: finiteSpeed(speed), influence: clampInfluence(influence) });

export const isInterpolationType = (value: unknown): value is InterpolationType =>
  value === 'linear' || value === 'bezier' || value === 'hold';

export const isTemporalEase = (value: unknown): value is TemporalEase =>
  typeof value === 'object' && value !== null
  && Number.isFinite(Number((value as TemporalEase).speed))
  && Number.isFinite(Number((value as TemporalEase).influence));

/** Numeric keys are the only ones AE interpolates; the rest step at the key. */
const numeric = (value: unknown): value is number =>
  typeof value === 'number' && Number.isFinite(value);

/** Straight-line speed across a segment, in value units per second. */
export function chordSpeed(a: EaseKeyframe, b: EaseKeyframe): number {
  const dt = b.t - a.t;
  if (!(dt > EPSILON) || !numeric(a.v) || !numeric(b.v)) return 0;
  return (b.v - a.v) / dt;
}

/** The out ease actually used on A -> B, with `linear` resolved to the chord. */
export function resolvedOutEase(a: EaseKeyframe, b: EaseKeyframe): TemporalEase {
  if (a.outInterp !== 'bezier') return { speed: chordSpeed(a, b), influence: LINEAR_INFLUENCE };
  return makeEase(a.outEase?.speed, a.outEase?.influence);
}

/** The in ease actually used on A -> B, with `linear` resolved to the chord. */
export function resolvedInEase(a: EaseKeyframe, b: EaseKeyframe): TemporalEase {
  if (b.inInterp !== 'bezier') return { speed: chordSpeed(a, b), influence: LINEAR_INFLUENCE };
  return makeEase(b.inEase?.speed, b.inEase?.influence);
}

export interface SegmentCurve {
  p0: TimeValue;
  p1: TimeValue;
  p2: TimeValue;
  p3: TimeValue;
}

/** The four control points of A -> B in real (seconds, value) space. */
export function segmentControlPoints(a: EaseKeyframe, b: EaseKeyframe): SegmentCurve {
  const dt = b.t - a.t;
  const va = numeric(a.v) ? a.v : 0;
  const vb = numeric(b.v) ? b.v : va;
  const out = resolvedOutEase(a, b);
  const incoming = resolvedInEase(a, b);
  const fOut = out.influence / 100;
  const fIn = incoming.influence / 100;
  return {
    p0: { t: a.t, v: va },
    p1: { t: a.t + fOut * dt, v: va + fOut * dt * out.speed },
    p2: { t: b.t - fIn * dt, v: vb - fIn * dt * incoming.speed },
    p3: { t: b.t, v: vb },
  };
}

const cubic = (p0: number, p1: number, p2: number, p3: number, u: number): number => {
  const m = 1 - u;
  return m * m * m * p0 + 3 * m * m * u * p1 + 3 * m * u * u * p2 + u * u * u * p3;
};

/**
 * Parameter u where the curve's time component reaches `x` (0..1 of the
 * segment). Influence never exceeds 100%, so both time control points stay in
 * [0,1] and the time component is always monotonic — Newton converges, and
 * bisection finishes whatever it misses.
 */
export function solveCurveParam(x1: number, x2: number, x: number): number {
  if (!(x > 0)) return 0;
  if (x >= 1) return 1;
  const cx = 3 * x1;
  const bx = 3 * (x2 - x1) - cx;
  const ax = 1 - cx - bx;
  let u = x;
  for (let i = 0; i < 8; i++) {
    const d = ((ax * u + bx) * u + cx) * u - x;
    if (d > -1e-7 && d < 1e-7) return u;
    const slope = (3 * ax * u + 2 * bx) * u + cx;
    if (slope > -1e-7 && slope < 1e-7) break;
    const next = u - d / slope;
    if (!(next >= 0 && next <= 1)) break;
    u = next;
  }
  let lo = 0, hi = 1;
  u = x;
  for (let i = 0; i < 32; i++) {
    const d = ((ax * u + bx) * u + cx) * u - x;
    if (d > -1e-7 && d < 1e-7) break;
    if (d > 0) hi = u; else lo = u;
    u = (lo + hi) / 2;
  }
  return u;
}

/** How far through segment A -> B the curve is at `time` (0..1 curve param). */
export function segmentParamAtTime(a: EaseKeyframe, b: EaseKeyframe, time: number): number {
  const dt = b.t - a.t;
  if (!(dt > EPSILON)) return 1;
  const x = (time - a.t) / dt;
  if (!(x > 0)) return 0;
  if (x >= 1) return 1;
  const curve = segmentControlPoints(a, b);
  return solveCurveParam((curve.p1.t - a.t) / dt, (curve.p2.t - a.t) / dt, x);
}

/** True when the segment freezes: AE lets either side declare the hold. */
export const isHeldSegment = (a: EaseKeyframe, b: EaseKeyframe): boolean =>
  a.outInterp === 'hold' || b.inInterp === 'hold';

/** Value of segment A -> B at absolute `time`, honouring hold keys. */
export function valueAtTime(a: EaseKeyframe, b: EaseKeyframe, time: number): number {
  const va = numeric(a.v) ? a.v : 0;
  if (isHeldSegment(a, b)) return va;
  if (!numeric(b.v)) return va;
  if (a.outInterp === 'linear' && b.inInterp === 'linear') {
    const dt = b.t - a.t;
    if (!(dt > EPSILON)) return b.v;
    const x = (time - a.t) / dt;
    return va + (b.v - va) * (x < 0 ? 0 : x > 1 ? 1 : x);
  }
  const curve = segmentControlPoints(a, b);
  const u = segmentParamAtTime(a, b, time);
  return cubic(curve.p0.v, curve.p1.v, curve.p2.v, curve.p3.v, u);
}

/* ── unit-square conversion (presets, legacy files, curve pickers) ───────── */

/**
 * Turn a normalized handle into AE units. `dt`/`dv` describe the segment the
 * handle belongs to; a flat segment carries no vertical scale, so the speed
 * collapses to zero exactly as a retracted AE handle would.
 */
export function easeFromUnitHandle(
  handle: UnitHandle, dt: number, dv: number, side: 'in' | 'out',
): TemporalEase {
  const x = Number(handle?.[0]);
  const y = Number(handle?.[1]);
  if (!Number.isFinite(x) || !Number.isFinite(y)) return makeEase(0, LINEAR_INFLUENCE);
  const fraction = side === 'out' ? x : 1 - x;
  const rise = side === 'out' ? y : 1 - y;
  const influence = clampInfluence(fraction * 100);
  const span = influence / 100 * dt;
  if (!(Math.abs(span) > EPSILON) || !Number.isFinite(dv)) return makeEase(0, influence);
  return makeEase(rise * dv / span, influence);
}

/** Inverse of {@link easeFromUnitHandle}; null when the segment is flat. */
export function unitHandleFromEase(
  ease: TemporalEase, dt: number, dv: number, side: 'in' | 'out',
): UnitHandle | null {
  if (!(dt > EPSILON) || !(Math.abs(dv) > EPSILON)) return null;
  const fraction = clampInfluence(ease.influence) / 100;
  const rise = fraction * dt * ease.speed / dv;
  return side === 'out' ? [fraction, rise] : [1 - fraction, 1 - rise];
}

/** The segment's shape as a CSS-style `cubic-bezier`; null when it is flat. */
export function segmentUnitCurve(a: EaseKeyframe, b: EaseKeyframe): UnitCurve | null {
  const dt = b.t - a.t;
  if (!(dt > EPSILON) || !numeric(a.v) || !numeric(b.v)) return null;
  const dv = b.v - a.v;
  if (!(Math.abs(dv) > EPSILON)) return null;
  const curve = segmentControlPoints(a, b);
  return [
    (curve.p1.t - a.t) / dt, (curve.p1.v - a.v) / dv,
    (curve.p2.t - a.t) / dt, (curve.p2.v - a.v) / dv,
  ];
}

/* ── auto bezier ─────────────────────────────────────────────────────────── */

/**
 * AE's Auto Bezier tangent: the slope through the neighbouring keys, flattened
 * at a local maximum or minimum so an auto key can never overshoot.
 */
export function autoBezierSpeed(
  previous: EaseKeyframe | undefined, key: EaseKeyframe, next: EaseKeyframe | undefined,
): number {
  if (!previous || !next) return 0;
  if (!numeric(previous.v) || !numeric(key.v) || !numeric(next.v)) return 0;
  const dt = next.t - previous.t;
  if (!(dt > EPSILON)) return 0;
  const before = key.v - previous.v;
  const after = next.v - key.v;
  if (before === 0 || after === 0 || (before > 0) !== (after > 0)) return 0;
  return (next.v - previous.v) / dt;
}

/** Recompute every auto-bezier tangent in a track. Cheap and idempotent. */
export function refreshAutoBezier(keys: EaseKeyframe[]): void {
  for (let index = 0; index < keys.length; index++) {
    const key = keys[index]!;
    if (!key.autoBezier) continue;
    const ease = makeEase(autoBezierSpeed(keys[index - 1], key, keys[index + 1]), AUTO_INFLUENCE);
    if (key.inInterp !== 'hold') { key.inInterp = 'bezier'; key.inEase = { ...ease }; }
    if (key.outInterp !== 'hold') { key.outInterp = 'bezier'; key.outEase = { ...ease }; }
  }
}

/** Copy the edited side's speed across a continuous key, keeping influences. */
export function applyContinuity(key: EaseKeyframe, edited: 'in' | 'out'): void {
  if (!key.continuous) return;
  const speed = edited === 'in' ? key.inEase.speed : key.outEase.speed;
  if (edited === 'in') {
    key.outEase = makeEase(speed, key.outEase?.influence ?? AUTO_INFLUENCE);
    if (key.outInterp === 'linear') key.outInterp = 'bezier';
  } else {
    key.inEase = makeEase(speed, key.inEase?.influence ?? AUTO_INFLUENCE);
    if (key.inInterp === 'linear') key.inInterp = 'bezier';
  }
}

/* ── graph-editor handles ────────────────────────────────────────────────── */

/** Where a key's direction handle sits, in (seconds, value); null when hidden. */
export function handleControlPoint(
  keys: EaseKeyframe[], index: number, side: 'in' | 'out',
): TimeValue | null {
  const key = keys[index];
  if (!key) return null;
  if (side === 'out') {
    const next = keys[index + 1];
    if (!next || isHeldSegment(key, next)) return null;
    return segmentControlPoints(key, next).p1;
  }
  const previous = keys[index - 1];
  if (!previous || isHeldSegment(previous, key)) return null;
  return segmentControlPoints(previous, key).p2;
}

export interface HandleDragOptions {
  /** AE's Cmd/Ctrl-drag: break a continuous tangent into independent handles. */
  breakContinuity?: boolean;
}

/**
 * Drop a direction handle at `point`. Horizontal distance becomes influence,
 * vertical slope becomes speed — the two axes of AE's Keyframe Velocity dialog.
 */
export function setHandleFromPoint(
  keys: EaseKeyframe[], index: number, side: 'in' | 'out', point: TimeValue,
  { breakContinuity = false }: HandleDragOptions = {},
): boolean {
  const key = keys[index];
  const neighbour = side === 'out' ? keys[index + 1] : keys[index - 1];
  if (!key || !neighbour) return false;
  const dt = Math.abs(neighbour.t - key.t);
  if (!(dt > EPSILON)) return false;
  const reach = side === 'out' ? point.t - key.t : key.t - point.t;
  const influence = clampInfluence(Math.abs(reach) / dt * 100);
  const span = influence / 100 * dt;
  const keyValue = numeric(key.v) ? key.v : 0;
  const rise = side === 'out' ? point.v - keyValue : keyValue - point.v;
  const ease = makeEase(rise / span, influence);
  /* AE promotes an auto key to continuous bezier the moment you touch it. */
  if (key.autoBezier) { key.autoBezier = false; key.continuous = true; }
  if (breakContinuity) key.continuous = false;
  if (side === 'out') {
    key.outEase = ease;
    if (key.outInterp === 'linear') key.outInterp = 'bezier';
  } else {
    key.inEase = ease;
    if (key.inInterp === 'linear') key.inInterp = 'bezier';
  }
  applyContinuity(key, side);
  return true;
}

/* ── interpolation commands ──────────────────────────────────────────────── */

export type EaseSide = 'in' | 'out' | 'both';

const sides = (side: EaseSide): Array<'in' | 'out'> =>
  side === 'both' ? ['in', 'out'] : [side];

/** AE's Easy Ease: bezier at 33.33% influence with the tangent flattened. */
export function easyEase(keys: EaseKeyframe[], index: number, side: EaseSide = 'both'): void {
  const key = keys[index];
  if (!key) return;
  key.autoBezier = false;
  key.continuous = false;
  for (const which of sides(side)) {
    if (which === 'in') {
      key.inInterp = 'bezier';
      key.inEase = makeEase(0, EASY_EASE_INFLUENCE);
    } else {
      key.outInterp = 'bezier';
      key.outEase = makeEase(0, EASY_EASE_INFLUENCE);
    }
  }
}

/** Set a key's interpolation the way AE's Keyframe Interpolation dialog does. */
export function setInterpolation(
  keys: EaseKeyframe[], index: number, side: EaseSide,
  type: InterpolationType | 'continuous' | 'auto',
): void {
  const key = keys[index];
  if (!key) return;
  const which = sides(side);
  if (type === 'auto' || type === 'continuous') {
    key.autoBezier = type === 'auto';
    key.continuous = true;
    const speed = type === 'auto'
      ? autoBezierSpeed(keys[index - 1], key, keys[index + 1])
      : (key.outInterp === 'bezier' ? key.outEase.speed
        : key.inInterp === 'bezier' ? key.inEase.speed
        : neighbourChordSpeed(keys, index));
    for (const item of which) {
      if (item === 'in') key.inEase = makeEase(speed, influenceFor(key.inEase, type));
      else key.outEase = makeEase(speed, influenceFor(key.outEase, type));
    }
    if (which.includes('in')) key.inInterp = 'bezier';
    if (which.includes('out')) key.outInterp = 'bezier';
    return;
  }
  key.autoBezier = false;
  if (type !== 'bezier') key.continuous = false;
  for (const item of which) {
    if (item === 'in') {
      key.inInterp = type;
      if (type === 'bezier') key.inEase = makeEase(key.inEase?.speed ?? 0, AUTO_INFLUENCE);
    } else {
      key.outInterp = type;
      if (type === 'bezier') key.outEase = makeEase(key.outEase?.speed ?? 0, AUTO_INFLUENCE);
    }
  }
}

const influenceFor = (ease: TemporalEase | undefined, type: 'auto' | 'continuous'): number =>
  type === 'auto' || !ease || !Number.isFinite(ease.influence) ? AUTO_INFLUENCE : ease.influence;

function neighbourChordSpeed(keys: EaseKeyframe[], index: number): number {
  const key = keys[index]!;
  const next = keys[index + 1];
  const previous = keys[index - 1];
  if (next) return chordSpeed(key, next);
  if (previous) return chordSpeed(previous, key);
  return 0;
}

/** AE's Toggle Hold Keyframe: hold runs from this key to the next one. */
export function setHold(keys: EaseKeyframe[], index: number, hold: boolean): void {
  const key = keys[index];
  if (!key) return;
  if (hold) {
    key.outInterp = 'hold';
    key.autoBezier = false;
    key.continuous = false;
  } else if (key.outInterp === 'hold') {
    key.outInterp = 'linear';
  }
}

/**
 * Apply a unit-square preset (`[x1,y1,x2,y2]`) to one key: `x1,y1` shape the
 * segment leaving it and `x2,y2` the segment arriving at it, which is how a
 * one-curve preset spreads across a whole track.
 */
export function applyUnitCurve(keys: EaseKeyframe[], index: number, curve: UnitCurve): void {
  const key = keys[index];
  if (!key) return;
  const next = keys[index + 1];
  const previous = keys[index - 1];
  const isLinear = curve[0] === curve[1] && curve[2] === curve[3];
  key.autoBezier = false;
  key.continuous = false;
  if (key.outInterp !== 'hold') key.outInterp = isLinear ? 'linear' : 'bezier';
  if (key.inInterp !== 'hold') key.inInterp = isLinear ? 'linear' : 'bezier';
  if (isLinear) return;
  if (next && numeric(key.v) && numeric(next.v)) {
    key.outEase = easeFromUnitHandle([curve[0], curve[1]], next.t - key.t, next.v - key.v, 'out');
  } else {
    key.outEase = makeEase(0, clampInfluence(curve[0] * 100));
  }
  if (previous && numeric(key.v) && numeric(previous.v)) {
    key.inEase = easeFromUnitHandle([curve[2], curve[3]], key.t - previous.t, key.v - previous.v, 'in');
  } else {
    key.inEase = makeEase(0, clampInfluence((1 - curve[2]) * 100));
  }
}

/* ── construction & migration ────────────────────────────────────────────── */

export function linearKeyframeEase(): Pick<EaseKeyframe,
  'inInterp' | 'outInterp' | 'inEase' | 'outEase' | 'autoBezier' | 'continuous'> {
  return {
    inInterp: 'linear',
    outInterp: 'linear',
    inEase: makeEase(0, LINEAR_INFLUENCE),
    outEase: makeEase(0, LINEAR_INFLUENCE),
    autoBezier: false,
    continuous: false,
  };
}

export type TemporalEaseFields = Pick<EaseKeyframe,
  'inInterp' | 'outInterp' | 'inEase' | 'outEase' | 'autoBezier' | 'continuous'>;

const hasNativeEase = (key: Record<string, unknown>): boolean =>
  isInterpolationType(key.inInterp) && isInterpolationType(key.outInterp)
  && isTemporalEase(key.inEase) && isTemporalEase(key.outEase);

/** Normalize one key's ease fields in place, without touching anything else. */
export function normalizeKeyEase(key: Record<string, unknown>): void {
  key.inInterp = isInterpolationType(key.inInterp) ? key.inInterp : 'linear';
  key.outInterp = isInterpolationType(key.outInterp) ? key.outInterp : 'linear';
  const inEase = key.inEase as TemporalEase | undefined;
  const outEase = key.outEase as TemporalEase | undefined;
  key.inEase = makeEase(inEase?.speed, isTemporalEase(inEase) ? inEase.influence : LINEAR_INFLUENCE);
  key.outEase = makeEase(outEase?.speed, isTemporalEase(outEase) ? outEase.influence : LINEAR_INFLUENCE);
  key.autoBezier = key.autoBezier === true;
  key.continuous = key.continuous === true;
}

const legacyHandle = (value: unknown, fallback: UnitHandle): UnitHandle =>
  Array.isArray(value) && Number.isFinite(Number(value[0])) && Number.isFinite(Number(value[1]))
    ? [Number(value[0]), Number(value[1])] : fallback;

/**
 * Bring a sorted track up to the AE model. Keys saved before the rewrite carry
 * `eo`/`ei` unit handles plus a `hold` flag; those are converted against the
 * segments they were measured in, so an old project opens looking identical.
 */
export function adoptTemporalEase<T extends Record<string, unknown>>(
  input: T[],
): Array<T & TemporalEaseFields> {
  const keys = input as Array<Record<string, unknown>>;
  for (let index = 0; index < keys.length; index++) {
    const key = keys[index]!;
    if (hasNativeEase(key)) { normalizeKeyEase(key); continue; }
    const previous = keys[index - 1];
    const next = keys[index + 1];
    const outHandle = legacyHandle(key.eo, [0, 0]);
    const inHandle = legacyHandle(key.ei, [1, 1]);
    const value = Number(key.v);
    const hold = key.hold === true;

    if (hold) {
      key.outInterp = 'hold';
      key.outEase = makeEase(0, LINEAR_INFLUENCE);
    } else if (!next || (outHandle[0] === 0 && outHandle[1] === 0)) {
      key.outInterp = 'linear';
      key.outEase = makeEase(0, LINEAR_INFLUENCE);
    } else {
      const nextValue = Number(next.v);
      key.outInterp = 'bezier';
      key.outEase = easeFromUnitHandle(
        outHandle, Number(next.t) - Number(key.t),
        Number.isFinite(nextValue) && Number.isFinite(value) ? nextValue - value : 0, 'out');
    }

    if (!previous || (inHandle[0] === 1 && inHandle[1] === 1)) {
      key.inInterp = 'linear';
      key.inEase = makeEase(0, LINEAR_INFLUENCE);
    } else {
      const previousValue = Number(previous.v);
      key.inInterp = 'bezier';
      key.inEase = easeFromUnitHandle(
        inHandle, Number(key.t) - Number(previous.t),
        Number.isFinite(previousValue) && Number.isFinite(value) ? value - previousValue : 0, 'in');
    }

    key.autoBezier = false;
    /* The old editor only persisted `bezierMode` when a handle was broken; AE
       treats independent handles as the default, so everything lands there. */
    key.continuous = key.bezierMode === 'continuous';
    delete key.eo;
    delete key.ei;
    delete key.hold;
    delete key.bezierMode;
  }
  refreshAutoBezier(keys as unknown as EaseKeyframe[]);
  return input as Array<T & TemporalEaseFields>;
}
