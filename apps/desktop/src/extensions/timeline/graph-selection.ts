export type GraphMoveItem<Property = unknown> = {
  id: string;
  property: Property;
  time: number;
  minTime?: number;
  maxTime?: number;
  selected: boolean;
};

export type GraphMovePlan<Property = unknown> = {
  delta: number;
  moves: Array<{ item: GraphMoveItem<Property>; time: number }>;
  removed: Array<GraphMoveItem<Property>>;
};

export type GraphPoint = { id: string; x: number; y: number };
export type GraphSelectionBounds = { x0: number; y0: number; x1: number; y1: number };
export type GraphFocus = { layerId: string; trackKey: string };

type GraphTargetCandidate = {
  key: string;
  L: { id: string };
};

/** The Graph Editor owns a property focus independently from ordinary layer
 * selection. Layer-strip clicks can therefore select layers for other editing
 * without silently replacing the curve currently open in the graph. */
export function resolveGraphTarget<T extends GraphTargetCandidate>(
  candidates: T[],
  focus: GraphFocus | null | undefined,
  selectedLayerIds: string[],
  isSelectedTrack: (candidate: T) => boolean,
): T | null {
  if (focus) {
    const focused = candidates.find(candidate => candidate.L.id === focus.layerId && candidate.key === focus.trackKey);
    if (focused) return focused;
  }
  const selected = new Set(selectedLayerIds);
  return candidates.find(candidate => selected.has(candidate.L.id) && isSelectedTrack(candidate))
    ?? candidates[0]
    ?? null;
}

/** AE-style Shift marquee toggles every enclosed key against the selection
 * captured at pointer-down. A plain marquee replaces that selection. */
export function selectionAfterMarquee(baseIds: string[], pickedIds: string[], toggle = false): string[] {
  if (!toggle) return [...new Set(pickedIds)];
  const next = new Set(baseIds);
  for (const id of pickedIds) next.has(id) ? next.delete(id) : next.add(id);
  return [...next];
}

/** Additive key gestures defer deselection until pointer-up: Shift-click
 * toggles a selected key, while Shift-drag can still move the established
 * selection. Linked-channel ids are handled as one hit group. */
export function selectionAfterKeyGesture(
  baseIds: string[], hitIds: string[], additive: boolean, dragged: boolean,
): string[] {
  const base = new Set(baseIds);
  const hit = [...new Set(hitIds)];
  const hitIsSelected = hit.some(id => base.has(id));
  if (!additive) return hitIsSelected ? [...base] : hit;
  if (!dragged && hitIsSelected) {
    hit.forEach(id => base.delete(id));
    return [...base];
  }
  hit.forEach(id => base.add(id));
  return [...base];
}

export function graphSelectionBounds(points: GraphPoint[], minimumSize = 16, padding = 0): GraphSelectionBounds | null {
  if (points.length < 2) return null;
  let x0 = Math.min(...points.map(point => point.x)) - padding;
  let x1 = Math.max(...points.map(point => point.x)) + padding;
  let y0 = Math.min(...points.map(point => point.y)) - padding;
  let y1 = Math.max(...points.map(point => point.y)) + padding;
  if (x1 - x0 < minimumSize) { const center = (x0 + x1) / 2; x0 = center - minimumSize / 2; x1 = center + minimumSize / 2; }
  if (y1 - y0 < minimumSize) { const center = (y0 + y1) / 2; y0 = center - minimumSize / 2; y1 = center + minimumSize / 2; }
  return { x0, y0, x1, y1 };
}

/* ── AE transform box ─────────────────────────────────────────────────────
   After Effects frames a multi-key selection in a box with eight grips: the
   four corners scale time and value together, the edges scale one axis. A
   drag anchors the opposite edge (or the box centre while Cmd/Ctrl is held),
   exactly as the Graph Editor's "Show Transform Box" does. */

export type GraphTransformHandle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w';

export const GRAPH_TRANSFORM_HANDLE_HIT = 6;

export function graphTransformHandles(
  bounds: GraphSelectionBounds,
): Array<{ handle: GraphTransformHandle; x: number; y: number }> {
  const mx = (bounds.x0 + bounds.x1) / 2, my = (bounds.y0 + bounds.y1) / 2;
  return [
    { handle: 'nw', x: bounds.x0, y: bounds.y0 }, { handle: 'n', x: mx, y: bounds.y0 },
    { handle: 'ne', x: bounds.x1, y: bounds.y0 }, { handle: 'e', x: bounds.x1, y: my },
    { handle: 'se', x: bounds.x1, y: bounds.y1 }, { handle: 's', x: mx, y: bounds.y1 },
    { handle: 'sw', x: bounds.x0, y: bounds.y1 }, { handle: 'w', x: bounds.x0, y: my },
  ];
}

export function graphTransformHandleAtPoint(
  bounds: GraphSelectionBounds | null | undefined, x: number, y: number,
  radius = GRAPH_TRANSFORM_HANDLE_HIT,
): GraphTransformHandle | null {
  if (!bounds) return null;
  let best: GraphTransformHandle | null = null, bestDistance = radius;
  for (const grip of graphTransformHandles(bounds)) {
    const distance = Math.hypot(x - grip.x, y - grip.y);
    if (distance <= bestDistance) { best = grip.handle; bestDistance = distance; }
  }
  return best;
}

export function graphTransformCursor(handle: GraphTransformHandle): string {
  if (handle === 'n' || handle === 's') return 'ns-resize';
  if (handle === 'e' || handle === 'w') return 'ew-resize';
  return handle === 'nw' || handle === 'se' ? 'nwse-resize' : 'nesw-resize';
}

export type GraphScaleItem<Property = unknown> = GraphMoveItem<Property> & {
  /** Layer-local `time` placed on the composition timeline. */
  compositionTime?: number;
  value?: number;
};

export type GraphScaleRequest = {
  timeScale: number;
  valueScale: number;
  /** Fixed point of the scale, in composition time and property value. */
  anchorTime: number;
  anchorValue: number;
};

export type GraphScalePlan<Property = unknown> = {
  timeScale: number;
  valueScale: number;
  moves: Array<{ item: GraphScaleItem<Property>; time: number; value: number }>;
};

/** Scale a graph selection about a fixed anchor. Like the move planner, this
 * never overruns a key it does not own: the time scale is clamped to the
 * widest factor that keeps every selected key inside its own window, a frame
 * clear of its unselected neighbours and of the next selected key. Value
 * scaling is unconstrained and may pass through zero to flip a curve. */
export function planGraphKeyframeScale<Property>(
  items: Array<GraphScaleItem<Property>>, request: GraphScaleRequest, fps: number,
): GraphScalePlan<Property> {
  const rate = Number.isFinite(fps) && fps > 0 ? fps : 30;
  const frame = 1 / rate;
  const selected = items.filter(item => item.selected && Number.isFinite(item.time));
  if (!selected.length) return { timeScale: 1, valueScale: 1, moves: [] };

  const anchorFor = (item: GraphScaleItem<Property>) =>
    request.anchorTime - ((Number.isFinite(item.compositionTime) ? item.compositionTime! : item.time) - item.time);

  // Never mirror keys through the anchor: a negative factor would reorder them.
  let low = 0, high = Infinity;
  for (const item of selected) {
    const anchor = anchorFor(item);
    const span = item.time - anchor;
    let min = Number.isFinite(item.minTime) ? Math.min(item.time, item.minTime!) : 0;
    let max = Number.isFinite(item.maxTime) ? Math.max(item.time, item.maxTime!) : Infinity;
    for (const fixed of items) {
      if (fixed.selected || fixed.property !== item.property || !Number.isFinite(fixed.time)) continue;
      if (fixed.time < item.time) min = Math.max(min, fixed.time + frame);
      else if (fixed.time > item.time) max = Math.min(max, fixed.time - frame);
    }
    if (Math.abs(span) < 1e-9) continue;
    const a = (min - anchor) / span, b = (max - anchor) / span;
    low = Math.max(low, Math.min(a, b));
    high = Math.min(high, Math.max(a, b));
  }
  // Keep adjacent selected keys at least one frame apart as the box collapses.
  for (const item of selected) for (const other of selected) {
    if (other === item || other.property !== item.property) continue;
    const gap = Math.abs(other.time - item.time);
    if (gap > 1e-9) low = Math.max(low, frame / gap);
  }
  /* Never force a change on data that already breaks these rules: a project
     carrying sub-frame gaps stays as it is until the drag actually asks. */
  low = Math.min(low, 1); high = Math.max(high, 1);

  const requested = Number.isFinite(request.timeScale) ? request.timeScale : 1;
  const timeScale = Math.max(low, Math.min(high, requested));
  const valueScale = Number.isFinite(request.valueScale) ? request.valueScale : 1;

  return {
    timeScale, valueScale,
    moves: selected.map(item => {
      const anchor = anchorFor(item);
      // A value-only scale must leave timing intact, including sub-frame keys.
      const time = timeScale === 1 ? item.time
        : Math.round((anchor + (item.time - anchor) * timeScale) * rate) / rate;
      const value = typeof item.value === 'number'
        ? request.anchorValue + (item.value - request.anchorValue) * valueScale
        : NaN;
      return { item, time: Object.is(time, -0) ? 0 : time, value };
    }),
  };
}

export function pointInGraphSelection(bounds: GraphSelectionBounds | null | undefined, x: number, y: number): boolean {
  return Boolean(bounds && x >= bounds.x0 && x <= bounds.x1 && y >= bounds.y0 && y <= bounds.y1);
}

/** Move a graph selection as one rigid group. Unlike the layer-bar move,
 * Graph Editor manipulation never replaces an unselected key: it stops one
 * frame before the nearest key in the direction of travel. */
export function planGraphKeyframeMove<Property>(
  items: Array<GraphMoveItem<Property>>, requestedDelta: number, fps: number,
): GraphMovePlan<Property> {
  const rate = Number.isFinite(fps) && fps > 0 ? fps : 30;
  const frame = 1 / rate;
  const selected = items.filter(item => item.selected && Number.isFinite(item.time));
  if (!selected.length) return { delta: 0, moves: [], removed: [] };
  let delta = Math.round((Number.isFinite(requestedDelta) ? requestedDelta : 0) * rate) / rate;
  const minDelta = Math.max(...selected.map(item => {
    const min = Number.isFinite(item.minTime) ? Math.min(item.time, item.minTime!) : 0;
    return min - item.time;
  }));
  const maxDelta = Math.min(...selected.map(item => {
    const max = Number.isFinite(item.maxTime) ? Math.max(item.time, item.maxTime!) : Infinity;
    return max - item.time;
  }));
  delta = Math.max(minDelta, Math.min(maxDelta, delta));

  if (delta > 0) {
    for (const moving of selected) for (const fixed of items) {
      if (fixed.selected || fixed.property !== moving.property || fixed.time <= moving.time) continue;
      delta = Math.min(delta, Math.max(0, fixed.time - moving.time - frame));
    }
  } else if (delta < 0) {
    for (const moving of selected) for (const fixed of items) {
      if (fixed.selected || fixed.property !== moving.property || fixed.time >= moving.time) continue;
      delta = Math.max(delta, Math.min(0, fixed.time - moving.time + frame));
    }
  }
  delta = Math.round(delta * rate) / rate;
  if (Object.is(delta, -0)) delta = 0;
  return {
    delta,
    moves: selected.map(item => ({ item, time: item.time + delta })),
    removed: [],
  };
}
