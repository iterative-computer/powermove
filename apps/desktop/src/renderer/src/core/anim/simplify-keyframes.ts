import { adoptTemporalEase, isHeldSegment, type EaseKeyframe } from './temporal-ease';

export interface SimplifyOptions {
  /** The actual editor evaluator. It receives private copies, never source keys. */
  evaluate: (keys: any[], time: number) => unknown;
  fps: number;
  from: number;
  duration: number;
  compositionDuration: number;
  shutter?: number;
  /** False for nested/time-mapped compositions, parent rigs, and unknown consumers. */
  trimOutside?: boolean;
  expression?: unknown;
}

const knownFields = new Set(['i', 't', 'v', 'inInterp', 'outInterp', 'inEase', 'outEase',
  'autoBezier', 'continuous', 'eo', 'ei', 'hold', 'bezierMode']);

function flat(a: EaseKeyframe, b: EaseKeyframe): boolean {
  if (a.v !== b.v) return false;
  if (isHeldSegment(a, b) || a.outInterp === 'linear' && b.inInterp === 'linear') return true;
  // The real numeric evaluator expands Beziers into polynomial coefficients.
  // Even equal controls can acquire a non-zero coefficient through floating
  // point cancellation. Retain those tracks instead of relying on a tolerance.
  const value = a.v as number;
  return (a.outInterp !== 'bezier' || a.outEase.speed === 0)
    && (b.inInterp !== 'bezier' || b.inEase.speed === 0)
    && value - 2 * value + value === 0
    && value - 3 * value + 3 * value - value === 0;
}

/**
 * Conservative, linear-time candidate construction plus real-evaluator verification.
 * Sampling alone is not a proof: only analytically constant segments are merged.
 * All other segments retain their original endpoints and temporal parameters.
 * Unknown/spatial/structured values and tangent-dependent tracks are left intact.
 */
export function simplifyKeyframes<T extends Record<string, any>>(source: T[], options: SimplifyOptions): T[] {
  if (options.expression || source.length < 3 || !Number.isFinite(options.fps) || options.fps <= 0
    || ![options.from, options.duration, options.compositionDuration].every(Number.isFinite)
    || options.duration <= 0 || options.compositionDuration <= 0) return source;
  for (let i = 0; i < source.length; i++) {
    const key = source[i]!;
    if (typeof key.v !== 'number' || !Number.isFinite(key.v) || !Number.isFinite(key.t)
      || i > 0 && key.t <= source[i - 1]!.t || key.autoBezier || key.continuous
      || Object.keys(key).some(field => !knownFields.has(field))) return source;
  }
  // Evaluation migrates legacy ease and refreshes tangents in place. Isolate both
  // tracks, including ease objects, so rejected candidates cannot change source.
  const copy = (keys: T[]) => keys.map(key => ({ ...key,
    ...(key.inEase ? { inEase: { ...key.inEase } } : {}),
    ...(key.outEase ? { outEase: { ...key.outEase } } : {}),
  }));
  const before = copy(source);
  adoptTemporalEase(before);
  if (before.some(key => key.autoBezier || key.continuous)) return source;
  const radius = Math.max(.5, Number(options.shutter) || .5) / options.fps;
  let first = 0, last = before.length - 1;
  if (options.trimOutside) {
    // Keep the nearest outside key on each edge, including the shutter margin.
    const start = -radius, end = options.duration + radius;
    while (first + 1 < before.length && before[first + 1]!.t < start) first++;
    while (last - 1 > first && before[last - 1]!.t > end) last--;
  }
  const candidate = [before[first]!];
  for (let i = first + 1; i < last; i++) {
    const a = candidate[candidate.length - 1]!, b = before[i]!, c = before[i + 1]!;
    if (!(flat(a as unknown as EaseKeyframe, b as unknown as EaseKeyframe)
      && flat(b as unknown as EaseKeyframe, c as unknown as EaseKeyframe)
      && flat(a as unknown as EaseKeyframe, c as unknown as EaseKeyframe))) candidate.push(b);
  }
  if (last > first) candidate.push(before[last]!);
  if (candidate.length === source.length) return source;
  const after = copy(candidate);
  const identical = (time: number) => {
    const a = options.evaluate(before, time), b = options.evaluate(after, time);
    return a === b;
  };
  // Verify every composition frame, including parent evaluation when a layer is
  // inactive. When trimming, restrict to its visible domain plus shutter margin.
  const start = options.trimOutside ? -radius : -options.from - radius;
  const end = options.trimOutside ? options.duration + radius
    : options.compositionDuration - options.from + radius;
  const shutter = (Number(options.shutter) || .5) / options.fps;
  const offsets = [0, -.5 / options.fps, -.25 / options.fps, .25 / options.fps, .5 / options.fps,
    ...Array.from({ length: 10 }, (_, sample) => ((sample + .5) / 10 - .5) * shutter)];
  const frameStart = Math.floor((start + options.from) * options.fps);
  const frameEnd = Math.ceil((end + options.from) * options.fps);
  // Refuse unbounded/malformed timelines rather than hanging an edit.
  if (frameEnd - frameStart > 1_000_000) return source;
  for (let frame = frameStart; frame <= frameEnd; frame++) {
    const time = frame / options.fps - options.from;
    for (const offset of offsets) {
      const t = time + offset;
      if (t >= start && t <= end && !identical(t)) return source;
    }
  }
  // Catch very short segments/holds between frames too. The analytical rule
  // protects arbitrary future sub-frame sample counts, beyond this lattice.
  for (let i = 0; i < before.length; i++) {
    const time = before[i]!.t;
    if (time >= start && time <= end && !identical(time)) return source;
    const next = before[i + 1];
    if (next) for (const f of [.25, .5, .75]) {
      const t = time + (next.t - time) * f;
      if (t >= start && t <= end && !identical(t)) return source;
    }
  }
  return after as T[];
}
