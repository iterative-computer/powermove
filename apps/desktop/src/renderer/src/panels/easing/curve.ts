import {
  applyUnitCurve, linearKeyframeEase, segmentControlPoints, setHandleFromPoint,
  solveCurveParam, valueAtTime, type EaseKeyframe, type TemporalEaseFields, type UnitCurve,
} from '../../core/anim/temporal-ease';

export interface CurvePoint extends TemporalEaseFields { t: number; v: number }
export interface Curve { points: CurvePoint[] }
export const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value));
export const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));
export const point = (t: number, v: number): CurvePoint => ({ t, v, ...linearKeyframeEase() });

export function cubic(handles: UnitCurve = [.6, 0, .4, 1]): Curve {
  const points = [point(0, 0), point(1, 1)];
  points.forEach((_, index) => applyUnitCurve(points, index, handles));
  return { points };
}

export function evaluate(curve: Curve, time: number): number {
  const keys = curve.points;
  if (time <= 0) return 0;
  if (time >= 1) return 1;
  const i = keys.findIndex((key, index) => index < keys.length - 1 && keys[index + 1]!.t >= time);
  return valueAtTime(keys[Math.max(0, i)]!, keys[Math.max(0, i) + 1]!, time);
}

export function normalize(keys: EaseKeyframe[]): Curve {
  if (keys.length < 2) throw new Error('Select two or more consecutive numeric keyframes');
  const first = keys[0]!, last = keys.at(-1)!, dt = last.t - first.t, dv = Number(last.v) - Number(first.v);
  if (!(dt > 0) || Math.abs(dv) < 1e-9) throw new Error('The first and last values must be different');
  if (keys.some(k => typeof k.v !== 'number' || !Number.isFinite(k.v))) throw new Error('Choose a numeric property');
  return { points: keys.map(key => ({ ...clone(key), t: (key.t - first.t) / dt, v: (Number(key.v) - Number(first.v)) / dv,
    inEase: { ...key.inEase, speed: key.inEase.speed * dt / dv }, outEase: { ...key.outEase, speed: key.outEase.speed * dt / dv } })) };
}

export function mirror(curve: Curve): Curve {
  return { points: [...curve.points].reverse().map(k => ({ ...clone(k), t: 1 - k.t, v: 1 - k.v,
    inInterp: k.outInterp, outInterp: k.inInterp, inEase: clone(k.outEase), outEase: clone(k.inEase) })) };
}

/** Split a Bezier exactly, rather than approximating it with a new bend. */
export function insertPoint(curve: Curve, time: number): Curve {
  const next = clone(curve), keys = next.points;
  const index = keys.findIndex((p, i) => i < keys.length - 1 && p.t < time && keys[i + 1]!.t > time);
  if (index < 0 || keys.some(p => Math.abs(p.t - time) < .005)) return next;
  const a = keys[index]!, b = keys[index + 1]!, c = segmentControlPoints(a, b), span = b.t - a.t;
  const u = solveCurveParam((c.p1.t - a.t) / span, (c.p2.t - a.t) / span, (time - a.t) / span);
  const mix = (a: { t: number; v: number }, b: { t: number; v: number }) => ({ t: a.t + (b.t - a.t) * u, v: a.v + (b.v - a.v) * u });
  const q0 = mix(c.p0, c.p1), q1 = mix(c.p1, c.p2), q2 = mix(c.p2, c.p3);
  const r0 = mix(q0, q1), r1 = mix(q1, q2), p = mix(r0, r1);
  keys.splice(index + 1, 0, point(p.t, p.v));
  setHandleFromPoint(keys, index, 'out', q0);
  setHandleFromPoint(keys, index + 1, 'in', r0);
  setHandleFromPoint(keys, index + 1, 'out', r1);
  setHandleFromPoint(keys, index + 2, 'in', q2);
  return next;
}

export function path(curve: Curve, width = 296, height = 178, pad = 9, low = 0, high = 1): string {
  const x = (t: number) => pad + t * (width - 2 * pad);
  const y = (v: number) => height - pad - (v - low) / (high - low) * (height - 2 * pad);
  let d = `M${x(0)} ${y(0)}`;
  for (let i = 0; i < curve.points.length - 1; i++) {
    const a = curve.points[i]!, b = curve.points[i + 1]!;
    if (a.outInterp === 'hold' || b.inInterp === 'hold') d += `H${x(b.t)}V${y(b.v)}`;
    else { const c = segmentControlPoints(a, b); d += `C${x(c.p1.t)} ${y(c.p1.v)} ${x(c.p2.t)} ${y(c.p2.v)} ${x(b.t)} ${y(b.v)}`; }
  }
  return d;
}

export function bounds(curve: Curve): [number, number] {
  const values = [0, 1];
  for (let i = 0; i <= 80; i++) values.push(evaluate(curve, i / 80));
  const lo = Math.min(...values), hi = Math.max(...values);
  return [lo < -.001 ? lo - .1 : 0, hi > 1.001 ? hi + .1 : 1];
}

export function validateCurve(value: unknown): Curve {
  const keys = (value as Curve)?.points;
  if (!Array.isArray(keys) || keys.length < 2 || keys.length > 256) throw new Error('A curve needs 2–256 points');
  const points = keys.map((k, i) => {
    if (!Number.isFinite(k.t) || !Number.isFinite(k.v) || k.t < 0 || k.t > 1 || Math.abs(k.v) > 100
      || (i > 0 && k.t <= keys[i - 1]!.t)) throw new Error('Invalid curve coordinates');
    for (const side of ['in', 'out'] as const) {
      const ease = k[`${side}Ease`];
      if (!['linear', 'bezier', 'hold'].includes(k[`${side}Interp`]) || !ease
        || !Number.isFinite(ease.speed) || Math.abs(ease.speed) > 1e8 || !Number.isFinite(ease.influence)
        || ease.influence < .1 || ease.influence > 100) throw new Error('Invalid curve handles');
    }
    return { t: k.t, v: k.v, inInterp: k.inInterp, outInterp: k.outInterp,
      inEase: clone(k.inEase), outEase: clone(k.outEase), autoBezier: false, continuous: k.continuous === true };
  });
  if (points[0]!.t !== 0 || points[0]!.v !== 0 || points.at(-1)!.t !== 1 || points.at(-1)!.v !== 1) throw new Error('Curve endpoints must be 0,0 and 1,1');
  return { points };
}
