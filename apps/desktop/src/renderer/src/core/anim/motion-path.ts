/**
 * Arc-length timing for the original numeric position channels.
 *
 * The table approximates distance-to-source-time only. Positions are ALWAYS
 * evaluated from the original curves, never interpolated between sampled
 * vertices. Retiming therefore cannot turn a curved route into a polyline.
 */
export interface PathTiming {
  readonly length: number;
  readonly samples: number;
  sourceTime(progress: number): number;
  position(progress: number): number[];
}
export function createPathTiming(
  sample: (sourceTime: number) => number[], knots: readonly number[], start: number, end: number,
): PathTiming {
  if (!Number.isFinite(start) || !Number.isFinite(end) || !(end > start)) throw new Error('Motion path needs a finite time range');
  const read = (time: number) => {
    const value = sample(time);
    if (!value.length || value.length > 3 || !value.every(Number.isFinite)) throw new Error('Motion path requires finite numeric position channels');
    return value;
  };
  const distance = (a: number[], b: number[]) => Math.hypot(...a.map((v, i) => v - b[i]!));
  const table = [{ time: start, distance: 0 }];
  let total = 0;
  function subdivide(a: number, av: number[], b: number, bv: number[], depth: number): void {
    if (table.length > 65536) throw new Error('Motion path timing is too complex');
    const times = [.25, .5, .75].map(q => a + (b - a) * q);
    const values = times.map(read), chain = [av, ...values, bv];
    let length = 0;
    for (let i = 1; i < chain.length; i++) length += distance(chain[i - 1]!, chain[i]!);
    const chord = distance(av, bv);
    if (!Number.isFinite(length) || !Number.isFinite(chord)) throw new Error('Motion path distance must be finite');
    // The second test also refines straight but unevenly timed curves. A
    // flatness-only table would leave their old easing in place.
    const timingError = Math.max(...values.map((v, i) => distance(v, av.map((n, axis) => n + (bv[axis]! - n) * (i + 1) / 4))));
    const tolerance = 1e-5 + Math.max(...chain.flatMap(p => p.map(Math.abs))) * 1e-8 + length * 1e-7;
    if (depth < 18 && (depth < 2 || length - chord > tolerance || timingError > tolerance)) {
      subdivide(a, av, times[1]!, values[1]!, depth + 1);
      subdivide(times[1]!, values[1]!, b, bv, depth + 1);
    } else {
      // Preserve all four subintervals in the timing map, not a spatial mesh.
      for (let i = 1; i < chain.length; i++) {
        total += distance(chain[i - 1]!, chain[i]!);
        table.push({ time: i === 4 ? b : times[i - 1]!, distance: total });
      }
    }
  }
  const times = [...new Set([start, ...knots.filter(t => Number.isFinite(t) && t > start && t < end), end])].sort((a, b) => a - b);
  for (let i = 1; i < times.length; i++) subdivide(times[i - 1]!, read(times[i - 1]!), times[i]!, read(times[i]!), 0);
  if (!(total > 1e-9)) throw new Error('The selected position path has no movement');
  function sourceTime(progress: number): number {
    if (!Number.isFinite(progress)) throw new Error('Motion path progress must be finite');
    if (progress <= 0) return start;
    if (progress >= 1) return end;
    const target = progress * total;
    let lo = 0, hi = table.length - 1;
    while (hi - lo > 1) { const mid = (lo + hi) >>> 1; if (table[mid]!.distance < target) lo = mid; else hi = mid; }
    const a = table[lo]!, b = table[hi]!, length = b.distance - a.distance;
    return length > 0 ? a.time + (b.time - a.time) * (target - a.distance) / length : b.time;
  }
  return { length: total, samples: table.length, sourceTime, position: progress => read(sourceTime(progress)) };
}
