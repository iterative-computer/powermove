export type Point = [number, number];

export type BezierKey = {
  eo: Point;
  ei: Point;
  bezierMode?: 'continuous' | 'split';
};

type SelectableBezierKey = {
  i?: string;
  t: number;
  members?: Array<{ key?: { i?: string } }>;
};

const EPSILON = 1e-6;

function selectableKeyIds(key: SelectableBezierKey): string[] {
  return [key.i, ...(key.members || []).map(member => member.key?.i)]
    .filter((id): id is string => typeof id === 'string');
}

/** Dragging a handle on any selected key adjusts the matching handle on the
 * whole selection. An unselected key remains an isolated edit, except for a
 * grouped X/Y key at the same time which intentionally behaves as one point. */
export function keysForBezierHandleDrag<T extends SelectableBezierKey>(
  keys: T[], clicked: T, selectedIds: string[],
): T[] {
  const selected = new Set(selectedIds);
  const clickedIsSelected = selectableKeyIds(clicked).some(id => selected.has(id));
  return clickedIsSelected
    ? keys.filter(key => selectableKeyIds(key).some(id => selected.has(id)))
    : keys.filter(key => key.t === clicked.t);
}

function finitePoint(value: unknown, fallback: Point): Point {
  return Array.isArray(value) && Number.isFinite(value[0]) && Number.isFinite(value[1])
    ? [value[0], value[1]] : [...fallback];
}

export function pointForBezierHandle(handle: Point, segmentStart: Point, segmentEnd: Point): Point {
  return [
    segmentStart[0] + (segmentEnd[0] - segmentStart[0]) * handle[0],
    segmentStart[1] + (segmentEnd[1] - segmentStart[1]) * handle[1],
  ];
}

export function bezierHandleAtPoint(
  point: Point, segmentStart: Point, segmentEnd: Point, fallback: Point,
): Point {
  const width = segmentEnd[0] - segmentStart[0];
  const height = segmentEnd[1] - segmentStart[1];
  return [
    Math.abs(width) < EPSILON ? fallback[0] : (point[0] - segmentStart[0]) / width,
    Math.abs(height) < EPSILON ? fallback[1] : (point[1] - segmentStart[1]) / height,
  ];
}

/** After Effects keeps unsplit handles on one tangent through the keyframe.
 * Preserve the opposite handle's influence (screen-space length) while the
 * dragged handle controls the tangent direction. */
export function mirroredBezierHandlePoint(keyPoint: Point, draggedPoint: Point, oppositePoint: Point): Point {
  const dx = draggedPoint[0] - keyPoint[0];
  const dy = draggedPoint[1] - keyPoint[1];
  const dragLength = Math.hypot(dx, dy);
  const oppositeLength = Math.hypot(oppositePoint[0] - keyPoint[0], oppositePoint[1] - keyPoint[1]);
  if (dragLength < EPSILON || oppositeLength < EPSILON) return [...oppositePoint];
  return [
    keyPoint[0] - dx / dragLength * oppositeLength,
    keyPoint[1] - dy / dragLength * oppositeLength,
  ];
}

export function isLinearBezierSegment(previous: BezierKey, next: BezierKey): boolean {
  const outgoing = finitePoint(previous?.eo, [0, 0]);
  const incoming = finitePoint(next?.ei, [1, 1]);
  return Math.abs(outgoing[0] - outgoing[1]) < EPSILON
    && Math.abs(incoming[0] - incoming[1]) < EPSILON;
}

/** Materialize the temporary one-third handles used to display a linear
 * segment. This is curve-preserving (both points remain on y=x) and prevents
 * the untouched handle from collapsing into its keyframe when its neighbor is
 * edited. */
export function materializeLinearBezierSegment(previous: BezierKey, next: BezierKey): boolean {
  if (!isLinearBezierSegment(previous, next)) return false;
  previous.eo = [1 / 3, 1 / 3];
  next.ei = [2 / 3, 2 / 3];
  return true;
}

/** Both handles are normalized from the segment's earlier key to its later key.
 * In particular, the incoming handle is NOT measured backwards from its key. */
export function moveBezierHandle(start: Point, delta: Point, segmentStart: Point, segmentEnd: Point): Point {
  const width = segmentEnd[0] - segmentStart[0];
  const height = segmentEnd[1] - segmentStart[1];
  const x = Math.abs(width) < EPSILON ? start[0] : start[0] + delta[0] / width;
  // A constant-value segment has no vertical amplitude to normalize against.
  const y = Math.abs(height) < EPSILON ? start[1] : start[1] + delta[1] / height;
  return [Math.max(0, Math.min(1, x)), y];
}

/** Linear endpoints overlap the key dots. Display equivalent, collinear handles
 * at thirds, so a linear segment has usable handles without changing its curve. */
export function visibleBezierHandle(key: any, neighbor: any, which: 'eo' | 'ei'): Point {
  const previous = which === 'eo' ? key : neighbor;
  const next = which === 'ei' ? key : neighbor;
  return isLinearBezierSegment(previous, next)
    ? (which === 'eo' ? [1 / 3, 1 / 3] : [2 / 3, 2 / 3])
    : finitePoint(key?.[which], which === 'eo' ? [0, 0] : [1, 1]);
}
