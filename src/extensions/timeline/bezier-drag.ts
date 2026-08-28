export type Point = [number, number];

/** Both handles are normalized from the segment's earlier key to its later key.
 * In particular, the incoming handle is NOT measured backwards from its key. */
export function moveBezierHandle(start: Point, delta: Point, segmentStart: Point, segmentEnd: Point): Point {
  const width = segmentEnd[0] - segmentStart[0];
  const height = segmentEnd[1] - segmentStart[1];
  const x = Math.abs(width) < 1e-6 ? start[0] : start[0] + delta[0] / width;
  // A constant-value segment has no vertical amplitude to normalize against.
  const y = Math.abs(height) < 1e-6 ? start[1] : start[1] + delta[1] / height;
  return [Math.max(0, Math.min(1, x)), y];
}

/** Linear endpoints overlap the key dots. Display equivalent, collinear handles
 * at thirds, so a linear segment has usable handles without changing its curve. */
export function visibleBezierHandle(key: any, neighbor: any, which: 'eo' | 'ei'): Point {
  const outgoing = which === 'eo' ? key.eo : neighbor.eo;
  const incoming = which === 'ei' ? key.ei : neighbor.ei;
  const linear = outgoing[0] === outgoing[1] && incoming[0] === incoming[1];
  return linear ? (which === 'eo' ? [1 / 3, 1 / 3] : [2 / 3, 2 / 3]) : [...key[which]] as Point;
}
