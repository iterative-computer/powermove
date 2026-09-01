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

export function graphSelectionBounds(points: GraphPoint[], minimumSize = 16): GraphSelectionBounds | null {
  if (points.length < 2) return null;
  let x0 = Math.min(...points.map(point => point.x));
  let x1 = Math.max(...points.map(point => point.x));
  let y0 = Math.min(...points.map(point => point.y));
  let y1 = Math.max(...points.map(point => point.y));
  if (x1 - x0 < minimumSize) { const center = (x0 + x1) / 2; x0 = center - minimumSize / 2; x1 = center + minimumSize / 2; }
  if (y1 - y0 < minimumSize) { const center = (y0 + y1) / 2; y0 = center - minimumSize / 2; y1 = center + minimumSize / 2; }
  return { x0, y0, x1, y1 };
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
