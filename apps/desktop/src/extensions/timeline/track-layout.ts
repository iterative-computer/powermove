/* Track timeline layout (the Premiere-style view).

   Powermove's source of truth stays the After Effects layer stack: index 0
   composites on top, groups composite their members as one unit. The track
   view is a projection of that stack onto horizontal lanes:

   - Clips that never overlap in time can share a lane, because their stack
     order has no visible effect.
   - Clips that do overlap keep their stack order as lane order: the clip that
     composites on top always sits on a higher video track.
   - A group is one block. Its bar (the "nest") takes the top lane of the block
     and, while expanded, its members are packed into the lanes beneath it.
     Members never interleave with outside clips, so a group still composites
     as a unit.
   - Each layer may carry a preferred `track` (relative to its parent block).
     Packing honours it as a minimum, so lanes stay where the user put them.

   Everything here is pure so the drag preview and the committed edit share
   one model. */

export interface TrackLayer {
  id: string;
  type: string;
  from: number;
  dur: number;
  group?: string | null;
  track?: number;
}

export interface TrackLayoutOptions {
  /** Whether a group's members are hidden behind its nest bar. */
  collapsed(groupId: string): boolean;
  /** Composition duration, used as the span of an empty group. */
  duration: number;
}

export type TrackArea = 'video' | 'audio';

export interface TrackItem {
  id: string;
  kind: 'clip' | 'group';
  area: TrackArea;
  /** Absolute lane within its area; 0 is V1 / A1. */
  lane: number;
  from: number;
  dur: number;
  depth: number;
  parent: string | null;
}

export interface TrackBand {
  id: string;
  area: TrackArea;
  /** Member lanes, [laneStart, laneEnd); the group's own bar is laneEnd. */
  laneStart: number;
  laneEnd: number;
  from: number;
  dur: number;
  depth: number;
}

export interface TrackLayout {
  video: number;
  audio: number;
  items: TrackItem[];
  bands: TrackBand[];
  byId: Map<string, TrackItem>;
}

interface Unit {
  layer: TrackLayer;
  from: number;
  end: number;
  height: number;
  base: number;
  /** Member units of an expanded group, positioned relative to the block. */
  children: Unit[];
  /** Height of the member lanes (0 for a collapsed or empty group). */
  inner: number;
}

const EPS = 1e-9;
const overlaps = (a: { from: number; end: number }, b: { from: number; end: number }) =>
  a.from < b.end - EPS && b.from < a.end - EPS;

export const isAudioLayer = (layer: Pick<TrackLayer, 'type'>) => layer.type === 'audio';

function childrenOf(layers: TrackLayer[]) {
  const ids = new Set(layers.map(layer => layer.id));
  const children = new Map<string | null, TrackLayer[]>();
  for (const layer of layers) {
    const parent = layer.group && ids.has(layer.group) && layer.group !== layer.id ? layer.group : null;
    const list = children.get(parent);
    if (list) list.push(layer); else children.set(parent, [layer]);
  }
  return children;
}

/** Bottom-up packing of one sibling list. Siblings arrive in stack order (top
    first). Each unit rests on the highest overlapping unit beneath it in the
    stack, or on its preferred track if that is higher. */
function pack(units: Unit[]): number {
  let height = 0;
  for (let i = units.length - 1; i >= 0; i--) {
    const unit = units[i]!;
    let base = Math.max(0, Math.floor(Number(unit.layer.track) || 0));
    for (let j = units.length - 1; j > i; j--) {
      const below = units[j]!;
      if (overlaps(unit, below)) base = Math.max(base, below.base + below.height);
    }
    unit.base = base;
    height = Math.max(height, base + unit.height);
  }
  return height;
}

function buildUnits(
  siblings: TrackLayer[], children: Map<string | null, TrackLayer[]>, options: TrackLayoutOptions, seen: Set<string>,
): Unit[] {
  const units: Unit[] = [];
  for (const layer of siblings) {
    if (seen.has(layer.id)) continue;
    seen.add(layer.id);
    if (layer.type !== 'group') {
      const from = Number(layer.from) || 0;
      units.push({ layer, from, end: from + Math.max(0, Number(layer.dur) || 0), height: 1, base: 0, children: [], inner: 0 });
      continue;
    }
    const members = buildUnits(children.get(layer.id) || [], children, options, seen);
    const span = groupSpan(members, options.duration);
    const open = !options.collapsed(layer.id);
    const inner = open && members.length ? pack(members) : 0;
    units.push({ layer, from: span.from, end: span.end, height: inner + 1, base: 0, children: open ? members : [], inner });
  }
  return units;
}

/** Same rule as the editor's group span: the extent of every non-group
    descendant, or the whole composition for an empty group. */
function groupSpan(members: Unit[], duration: number) {
  if (!members.length) return { from: 0, end: duration };
  return { from: Math.min(...members.map(unit => unit.from)), end: Math.max(...members.map(unit => unit.end)) };
}

function emit(units: Unit[], area: TrackArea, offset: number, depth: number, parent: string | null, layout: TrackLayout) {
  for (const unit of units) {
    const base = offset + unit.base;
    const kind = unit.layer.type === 'group' ? 'group' : 'clip';
    const item: TrackItem = {
      id: unit.layer.id, kind, area, lane: base + unit.height - 1,
      from: unit.from, dur: unit.end - unit.from, depth, parent,
    };
    layout.items.push(item);
    layout.byId.set(item.id, item);
    if (kind === 'group' && unit.inner > 0) {
      layout.bands.push({ id: unit.layer.id, area, laneStart: base, laneEnd: base + unit.inner, from: unit.from, dur: unit.end - unit.from, depth });
      emit(unit.children, area, base, depth + 1, unit.layer.id, layout);
    }
  }
}

/** Top-level audio clips live in their own area below the video tracks.
    Audio inside a group stays in the group's block. */
function rootUnits(layers: TrackLayer[], options: TrackLayoutOptions) {
  const children = childrenOf(layers);
  const units = buildUnits(children.get(null) || [], children, options, new Set());
  return {
    video: units.filter(unit => !isAudioLayer(unit.layer)),
    audio: units.filter(unit => isAudioLayer(unit.layer)),
  };
}

export function layoutTracks(layers: TrackLayer[], options: TrackLayoutOptions): TrackLayout {
  const { video, audio } = rootUnits(layers, options);
  const layout: TrackLayout = { video: pack(video), audio: pack(audio), items: [], bands: [], byId: new Map() };
  emit(video, 'video', 0, 0, null, layout);
  emit(audio, 'audio', 0, 0, null, layout);
  return layout;
}

export interface TrackMove {
  /** Units being dragged. Members of a selected group are implied. */
  ids: string[];
  /** Time offset applied to every moved layer (including group members). */
  dt: number;
  /** Lane offset per area, in lanes. Positive moves to a higher track. */
  lanes: Record<TrackArea, number>;
}

export interface TrackMovePlan {
  /** Every layer, in its new stack order, with its new start and track. */
  layers: Array<TrackLayer & { from: number; track: number }>;
}

/** Plan a clip move in the track view. Moved units change lane inside their
    own parent block only (moving between groups is a membership edit, not a
    track move). Lane order is then written back as stack order per level,
    and every sibling's lane is persisted as its preferred track, so packing
    the result reproduces the layout the user saw while dragging. */
export function planTrackMove(layers: TrackLayer[], options: TrackLayoutOptions, move: TrackMove): TrackMovePlan {
  const byId = new Map(layers.map(layer => [layer.id, layer]));
  const children = childrenOf(layers);
  const roots = new Set(move.ids.filter(id => byId.has(id)));
  // A unit under a moved group already moves with it.
  for (const id of [...roots]) {
    for (let parent = byId.get(id)?.group; parent; parent = byId.get(parent)?.group) {
      if (roots.has(parent)) { roots.delete(id); break; }
    }
  }
  const shifted = new Set<string>();
  const shift = (id: string) => {
    if (shifted.has(id)) return;
    shifted.add(id);
    for (const child of children.get(id) || []) shift(child.id);
  };
  roots.forEach(shift);
  const timeOf = (layer: TrackLayer) => (Number(layer.from) || 0) + (shifted.has(layer.id) ? move.dt : 0);

  // Members hidden behind a collapsed bar have no lane; they keep their
  // stored track untouched.
  const current = layoutTracks(layers, options);
  const parentKey = (layer: TrackLayer) => (layer.group && byId.has(layer.group) ? layer.group : null);
  const relativeLane = (layer: TrackLayer): number | null => {
    const item = current.byId.get(layer.id);
    if (!item) return null;
    const top = item.lane - (layer.type === 'group' ? blockInner(current, layer.id) : 0);
    const parent = parentKey(layer);
    if (!parent) return top;
    const band = current.bands.find(entry => entry.id === parent);
    return band ? top - band.laneStart : null;
  };
  const areaOf = (layer: TrackLayer): TrackArea => {
    let root = layer;
    for (let parent = parentKey(root); parent; parent = parentKey(root)) root = byId.get(parent)!;
    return isAudioLayer(root) ? 'audio' : 'video';
  };

  const next = new Map<string, TrackLayer & { from: number; track: number }>();
  const ordered: TrackLayer[] = [];
  const visit = (parent: string | null) => {
    const siblings = children.get(parent) || [];
    const blockHeight = parent ? blockInner(current, parent) : null;
    const lanes = new Map<string, number>();
    for (const layer of siblings) {
      const lane = relativeLane(layer) ?? Math.max(0, Math.floor(Number(layer.track) || 0));
      if (!roots.has(layer.id)) { lanes.set(layer.id, lane); continue; }
      const area = areaOf(layer);
      const limit = parent ? (blockHeight ?? 0) : area === 'audio' ? current.audio : current.video;
      lanes.set(layer.id, Math.max(0, Math.min(limit, lane + (move.lanes[area] || 0))));
    }
    // Slot-preserving sort: video and audio units each keep the stack slots
    // they occupied, so moving tracks never reorders audio against video.
    const sortSlots = (audio: boolean) => {
      const slots = siblings.map((layer, index) => ({ layer, index })).filter(entry => isAudioLayer(entry.layer) === audio);
      const sorted = [...slots].sort((a, b) => (lanes.get(b.layer.id)! - lanes.get(a.layer.id)!)
        // On a shared lane the moved unit sits on top in the stack, so if it
        // collides with a resting clip it lands on the next free track.
        || Number(roots.has(b.layer.id)) - Number(roots.has(a.layer.id))
        || a.index - b.index);
      return slots.map((slot, i) => ({ index: slot.index, layer: sorted[i]!.layer }));
    };
    const placed = new Array<TrackLayer>(siblings.length);
    for (const entry of [...sortSlots(false), ...sortSlots(true)]) placed[entry.index] = entry.layer;
    for (const layer of placed) {
      next.set(layer.id, { ...layer, from: timeOf(layer), track: lanes.get(layer.id)! });
      ordered.push(layer);
      if (layer.type === 'group') visit(layer.id);
    }
  };
  visit(null);
  // Orphans (unknown group ids) were treated as roots by childrenOf; any layer
  // still missing keeps its place at the end.
  for (const layer of layers) if (!next.has(layer.id)) {
    next.set(layer.id, { ...layer, from: timeOf(layer), track: Math.max(0, Math.floor(Number(layer.track) || 0)) });
    ordered.push(layer);
  }
  return { layers: ordered.map(layer => next.get(layer.id)!) };
}

/** Member-lane height of a group block in a computed layout. */
function blockInner(layout: TrackLayout, groupId: string): number {
  const band = layout.bands.find(entry => entry.id === groupId);
  return band ? band.laneEnd - band.laneStart : 0;
}
