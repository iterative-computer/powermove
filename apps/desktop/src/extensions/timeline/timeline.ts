import { revealedProperties } from './property-reveal';
import { layerDrop } from './layer-drop';
import { graphSample, velocityDialog, scaleGraphDialog } from './graph-controls';
import { temporalKeys, type KernelEvents, type PowermoveAPI, type Space3DAPI } from 'powermove';
import { createGraphSampleCache } from './graph-sample-cache';
import { adjacentKeyframe } from './keyframe-navigation';
import { draggedPropertyValue, propertyMetadata } from './property-values';
import { element, iconNode, normalizeTimelineChrome, selectedLayers } from './api-helpers';
/* Ported from js/ui/timeline.js — behavior-preserving. */
import { expandScaleKeyIds, keyMembers, timelineProperties, trackChannels, trackSelected } from './property-tracks';
import {
  bezierHandleAtPoint,
  keysForBezierHandleDrag,
  materializeLinearBezierSegment,
  mirroredBezierHandlePoint,
  moveBezierHandle,
  pointForBezierHandle,
  visibleBezierHandle,
} from './bezier-drag';
import {
  graphSelectionBounds,
  planGraphKeyframeMove,
  pointInGraphSelection,
  resolveGraphTarget,
  selectionAfterKeyGesture,
  selectionAfterMarquee,
} from './graph-selection';

export interface KeyframeMoveSnapshotItem<Property = unknown> {
  id: string;
  property: Property;
  time: number;
  compositionTime?: number;
  minTime?: number;
  maxTime?: number;
  selected: boolean;
  order?: number;
  offsetIndex?: number;
  offsetCount?: number;
}

export interface KeyframeMovePlan<Property = unknown> {
  delta: number;
  moves: Array<{ item: KeyframeMoveSnapshotItem<Property>; time: number }>;
  removed: Array<KeyframeMoveSnapshotItem<Property>>;
}

export interface TimelineSnapResolution {
  time: number;
  target: number | null;
}

export interface KeyframeGroupSnapLock {
  anchor: number;
  target: number;
}

export interface KeyframeGroupSnapResolution {
  delta: number;
  lock: KeyframeGroupSnapLock | null;
}

export const timelineWorkArea = {
  resize(work: unknown, idx: number, time: number, duration: number, frame: number) {
    const span = Array.isArray(work) ? work : [0, duration];
    const start = Math.max(0, Number(span[0]) || 0);
    const end = Math.max(start + frame, Number(span[1]) || duration);
    const nextTime = Math.max(0, Number(time) || 0);
    if (idx === 0) return { duration, workArea: [Math.max(0, Math.min(end - frame, nextTime)), end] as [number, number] };
    const nextEnd = Math.max(start + frame, nextTime);
    return { duration: Math.max(duration, nextEnd), workArea: [start, nextEnd] as [number, number] };
  },
  move(work: number[], delta: number, duration: number, frame: number) {
    const start = Math.max(0, Number(work[0]) || 0);
    const end = Math.max(start + frame, Number(work[1]) || duration);
    const span = Math.min(duration, end - start);
    const nextStart = Math.max(0, Math.min(Math.max(0, duration - span), start + delta));
    return { workArea: [nextStart, nextStart + span] as [number, number] };
  },
};
export interface QuickOffsetTimingItem {
  time: number;
  offsetIndex: number;
  offsetCount: number;
  minTime?: number;
  maxTime?: number;
}

export interface QuickOffsetTimingPlan {
  total: number;
  perGroup: number;
  times: number[];
}

export interface PropertyValueColumn {
  left: number;
  right: number;
  width: number;
}

/** Keep property values aligned and visibly separated from one another and
    from trailing row actions such as the Scale link control. */
export function propertyValueColumns(
  valueX: number, gutter: number, count: number, actionWidth = 0, gap = 8,
): PropertyValueColumn[] {
  const columns = Math.max(1, Math.floor(count) || 1);
  const rightEdge = Math.max(valueX, gutter - 8 - Math.max(0, actionWidth));
  const available = Math.max(0, rightEdge - valueX - gap * (columns - 1));
  const width = available / columns;
  return Array.from({ length: columns }, (_, index) => {
    const left = valueX + index * (width + gap);
    return { left, right: left + width, width };
  });
}

export function toggleTimelineDisclosure(api: PowermoveAPI, layer: any): boolean {
  if (layer.type === 'group') {
    const collapsed = !api.uiState.getGroupCollapsed(layer);
    api.uiState.setGroupCollapsed(layer, collapsed);
    return collapsed;
  }
  const collapsed = !api.uiState.getLayerCollapsed(layer);
  layer.collapsed = collapsed;
  api.uiState.setLayerCollapsed(layer, collapsed);
  return collapsed;
}

export function toggleTimelineScaleLink(api: PowermoveAPI, layer: any): void {
  api.edit.apply(
    { type: 'set_layer', target: layer.id, patch: { scaleLinked: !layer.scaleLinked } },
    { label: 'Link scale axes', origin: 'timeline' },
  );
}

/** Distribute one pointer delta across an ordered set of groups. The first
    selected group is the anchor, the last follows the pointer, and everything
    between receives an equal share. Items may repeat an index when one layer
    group owns several clips or selected keyframes. */
export function planQuickOffsetTiming(
  items: QuickOffsetTimingItem[], requestedTotal: number,
): QuickOffsetTimingPlan {
  const totalGroups = Math.max(0, ...items.map(item => Number(item.offsetCount) || 0));
  if (totalGroups < 2 || !items.length) {
    return { total: 0, perGroup: 0, times: items.map(item => item.time) };
  }
  let total = Number.isFinite(requestedTotal) ? requestedTotal : 0;
  for (const item of items) {
    const factor = item.offsetIndex / (totalGroups - 1);
    if (!(factor > 0) || !Number.isFinite(item.time)) continue;
    const minTime = Number.isFinite(item.minTime) ? item.minTime! : 0;
    total = Math.max(total, (minTime - item.time) / factor);
    if (Number.isFinite(item.maxTime)) total = Math.min(total, (item.maxTime! - item.time) / factor);
  }
  if (Object.is(total, -0)) total = 0;
  return {
    total,
    perGroup: total / (totalGroups - 1),
    times: items.map(item => item.time + item.offsetIndex / (totalGroups - 1) * total),
  };
}

/** Quick Offset keeps each layer's selected keys rigid while staggering the
    layers themselves. Unlike an ordinary key drag, its advertised precision
    includes subframes, so the total is intentionally not frame-rounded. */
export function planQuickOffsetKeyframes<Property>(
  items: Array<KeyframeMoveSnapshotItem<Property>>, requestedTotal: number, _fps: number,
): KeyframeMovePlan<Property> {
  const selected = items.filter(item => item.selected && item.offsetIndex != null && item.offsetCount != null);
  const timing = planQuickOffsetTiming(selected.map(item => ({
    time: item.time,
    offsetIndex: item.offsetIndex!,
    offsetCount: item.offsetCount!,
    minTime: item.minTime,
    maxTime: item.maxTime,
  })), requestedTotal);
  const moves = selected.map((item, index) => ({ item, time: timing.times[index]! }));
  const removed: Array<KeyframeMoveSnapshotItem<Property>> = [];
  for (const fixed of items) {
    if (fixed.selected) continue;
    if (moves.some(move => move.item.property === fixed.property && Math.abs(move.time - fixed.time) < 1e-9)) removed.push(fixed);
  }
  return { delta: timing.total, moves, removed };
}

/** Resolve a temporary Shift-snap without letting two nearby targets make the
    playhead flicker. A previously acquired target gets a slightly wider
    release radius; otherwise the closest target inside the visual tolerance
    wins. */
export function resolveTimelineSnap(
  time: number,
  targets: number[],
  tolerance: number,
  lockedTarget: number | null = null,
  releaseTolerance = tolerance * 1.5,
): TimelineSnapResolution {
  const raw = Number.isFinite(time) ? time : 0;
  const acquire = Math.max(0, Number.isFinite(tolerance) ? tolerance : 0);
  const release = Math.max(acquire, Number.isFinite(releaseTolerance) ? releaseTolerance : acquire);
  if (lockedTarget != null && Number.isFinite(lockedTarget) && Math.abs(raw - lockedTarget) <= release) {
    return { time: lockedTarget, target: lockedTarget };
  }
  let target: number | null = null;
  let distance = acquire + Number.EPSILON;
  for (const candidate of targets) {
    if (!Number.isFinite(candidate)) continue;
    const next = Math.abs(raw - candidate);
    if (next < distance) { target = candidate; distance = next; }
  }
  return target == null ? { time: raw, target: null } : { time: target, target };
}

/** Snap one rigid keyframe selection by testing every selected key against
    every visible timeline target. Keeping the anchor with the lock prevents
    a dense set of markers or keys from making the whole selection chatter. */
export function resolveKeyframeGroupSnap(
  anchorTimes: number[], requestedDelta: number, targets: number[], tolerance: number,
  locked: KeyframeGroupSnapLock | null = null, releaseTolerance = tolerance * 1.5,
): KeyframeGroupSnapResolution {
  const anchors = anchorTimes.filter(Number.isFinite);
  const rawDelta = Number.isFinite(requestedDelta) ? requestedDelta : 0;
  const acquire = Math.max(0, Number.isFinite(tolerance) ? tolerance : 0);
  const release = Math.max(acquire, Number.isFinite(releaseTolerance) ? releaseTolerance : acquire);
  if (!anchors.length) return { delta: rawDelta, lock: null };

  if (locked && anchors.includes(locked.anchor)
      && Math.abs(locked.anchor + rawDelta - locked.target) <= release) {
    return { delta: locked.target - locked.anchor, lock: locked };
  }

  let lock: KeyframeGroupSnapLock | null = null;
  let distance = acquire + Number.EPSILON;
  for (const anchor of anchors) for (const target of targets) {
    if (!Number.isFinite(target)) continue;
    const next = Math.abs(anchor + rawDelta - target);
    if (next < distance) { distance = next; lock = { anchor, target }; }
  }
  return lock ? { delta: lock.target - lock.anchor, lock } : { delta: rawDelta, lock: null };
}

export type KeyframeContextEntry = { key: { i?: string }; prop: unknown };

/** A context-click on any selected key edits the complete selection. Clicking
    an unselected key remains deliberately local, matching AE's least
    surprising multi-keyframe menu behavior. */
export function keyframeContextEntries<T extends KeyframeContextEntry>(
  clickedEntries: T[], selectedIds: string[], selectedEntries: T[],
): T[] {
  const selected = new Set(selectedIds);
  const clickedIsSelected = clickedEntries.some(entry => entry.key.i && selected.has(entry.key.i));
  const source = clickedIsSelected && selectedEntries.length ? selectedEntries : clickedEntries;
  const seen = new Set<object>();
  return source.filter(entry => {
    if (!entry?.key || seen.has(entry.key)) return false;
    seen.add(entry.key);
    return true;
  });
}

/** Plan one frame-snapped keyframe move from an immutable gesture snapshot.
    All selected keys receive the same delta. Destination collisions are
    scoped to one property and the selected/moving key wins by id. */
export function planKeyframeMove<Property>(
  items: Array<KeyframeMoveSnapshotItem<Property>>, requestedDelta: number, fps: number,
): KeyframeMovePlan<Property> {
  const rate = Number.isFinite(fps) && fps > 0 ? fps : 30;
  const selected = items.filter((item) => item.selected && Number.isFinite(item.time));
  if (!selected.length) return { delta: 0, moves: [], removed: [] };
  const rawDelta = Number.isFinite(requestedDelta) ? requestedDelta : 0;
  let delta = Math.round(rawDelta * rate) / rate;
  const minDelta = Math.max(...selected.map((item) => {
    const minTime = Number.isFinite(item.minTime) ? Math.min(item.time, item.minTime!) : 0;
    return minTime - item.time;
  }));
  delta = Math.max(delta, minDelta);
  const maxDelta = Math.min(...selected.map((item) => {
    const maxTime = Number.isFinite(item.maxTime) ? Math.max(item.time, item.maxTime!) : Infinity;
    return maxTime - item.time;
  }));
  delta = Math.min(delta, maxDelta);
  if (Object.is(delta, -0)) delta = 0;
  const moves = selected.map((item) => ({ item, time: item.time + delta }));

  /* A value-only graph edit must not clean up unrelated legacy duplicates.
     Resolve occupied frames only when the gesture actually moves in time. */
  const removed: Array<KeyframeMoveSnapshotItem<Property>> = [];
  if (Math.abs(delta) > 1e-12) {
    const destinations = new Map<Property, Set<number>>();
    for (const move of moves) {
      let frames = destinations.get(move.item.property);
      if (!frames) destinations.set(move.item.property, frames = new Set());
      frames.add(Math.round(move.time * rate));
    }
    for (const item of items) {
      if (item.selected) continue;
      if (destinations.get(item.property)?.has(Math.round(item.time * rate))) removed.push(item);
    }
  }
  return { delta, moves, removed };
}

/** Materialize a plan without mutating its source. Useful for tests and for
    consumers that keep their own keyframe storage. */
export function applyKeyframeMovePlan<Property>(
  items: Array<KeyframeMoveSnapshotItem<Property>>, plan: KeyframeMovePlan<Property>,
): Array<KeyframeMoveSnapshotItem<Property>> {
  const removed = new Set(plan.removed);
  const moved = new Map(plan.moves.map(({ item, time }) => [item, time]));
  return items.filter((item) => !removed.has(item)).map((item) => ({
    ...item,
    time: moved.has(item) ? moved.get(item)! : item.time,
  }));
}

/** A new ESM instance gets a new token. It can recognize and dispose a
    runtime left behind by the previous hot-reloaded module instance. */
const TIMELINE_RUNTIME_TOKEN = Symbol('powermove.timeline.runtime');
const runtimes = new WeakMap<PowermoveAPI, any>();
/** The WeakMap misses whenever the kernel hands out a fresh API facade for the
    same window. Without a second handle on the mounted runtime that miss builds
    a *duplicate* timeline: the original keeps its listeners and keeps painting
    #tl-canvas, while the newcomer restores pps/scrollT from the session written
    at the last dispose. The two then fight over one canvas, so the track area
    flashes an older zoom/scroll for a frame or two before the live runtime
    repaints — the gutter stays put because it is derived, not restored. */
let mountedRuntime: any = null;
const liveRuntime = (candidate: any) =>
  candidate?.__timelineRuntimeToken === TIMELINE_RUNTIME_TOKEN && !candidate.__timelineRuntimeDisposed
    ? candidate : null;
/* Only reclaim the leftover when it still owns the canvas this activation is
   about to attach to. An unrelated API (another harness, another window) must
   still get a runtime of its own. */
const runtimeOwningLiveCanvas = () => {
  const wrap = mountedRuntime?.__timelineWrap;
  return wrap && wrap.isConnected !== false && wrap === document.querySelector('#tl-canvas-wrap')
    ? mountedRuntime : null;
};

/** Resolve overlapping keyframe hit targets without stealing an established
    selection. With no selected hit, array order remains the visual/top order. */
export function pickKeyframeHit<T extends { i?: string }>(
  keys: T[], selectedIds: string[], distance: (key: T) => number, threshold: number,
): T | null {
  const hits = keys.filter((key) => distance(key) < threshold);
  if (!hits.length) return null;
  const selected = new Set(selectedIds);
  return hits.find((key) => keyMembers(key).some(member => selected.has(member.key.i))) || hits[0]!;
}

export const timelinePanelOptions = {
  title: 'Timeline', flush: true, noscroll: true, headless: true, size: 340, moveSlot: '#tl-head',
  library: { width: 800, height: 440 },
} as const;

/** Audio clips are read by their waveform; a name label only crowds it. */
export function shouldDrawClipLabel(layerType: unknown): boolean {
return layerType !== 'audio';
}

/** Compatibility name for tests and downstream forks of the legacy runtime. */
export function createTimeline(api: PowermoveAPI): any {
return createTimelineRuntime(api, api.space3d);
}

export function createTimelineRuntime(api: PowermoveAPI, space3d: Space3DAPI = api.space3d): any {
const leftover = runtimeOwningLiveCanvas();
const previous = liveRuntime(runtimes.get(api)) ?? liveRuntime(leftover) ?? runtimes.get(api) ?? leftover;
if (liveRuntime(previous)) { runtimes.set(api, previous); return previous; }
const previousHead = previous?.__timelineHead || null;
const previousWrap = previous?.__timelineWrap || null;
if (previous?.disposeRuntime) previous.disposeRuntime();
else if (previous?.dispose) previous.dispose();
const h = element, clamp = api.util.clamp;

const T: any = previous || {
  gut: 224, row: 40, ruler: 28, pps: 90, scrollT: 0, scrollY: 0,
  graph: false, rows: [], cv: null, ctx: null, w: 0, hgt: 0, dpr: 1,
  hover: null, marquee: null, dropRow: null as number | null,
  reorder: null as ReturnType<typeof layerDrop>,
  quickOffset: null as null | { total: number; perGroup: number; x: number; y: number },
  drop: null as null | { at: number; rowIdx: number; index: number; name: string; kind: string; dur?: number },
  style: { clipRadius: 5, keyframeSize: 8, showLayerNumbers: true, showTypeBadges: true, toolbarDensity: 'compact' },
};
/* Built-in extensions activate after project hydration. Restore the current
   project session here, matching the former app bootstrap path that ran after
   the legacy timeline installer. */
const sessionTimeline = previous ?? api.storage.get<{ pps?: number; scrollT?: number; scrollY?: number; graph?: boolean }>(`session:${api.project.get().id}`);
if (sessionTimeline) {
  T.pps = Number.isFinite(sessionTimeline.pps) ? sessionTimeline.pps : T.pps;
  T.scrollT = Number.isFinite(sessionTimeline.scrollT) ? sessionTimeline.scrollT : T.scrollT;
  T.scrollY = Number.isFinite(sessionTimeline.scrollY) ? sessionTimeline.scrollY : T.scrollY;
  T.graph = !!sessionTimeline.graph;
}
runtimes.set(api, T);
mountedRuntime = T;
T.keySelectionActive = !!(T.keySelectionActive || api.selection.keys().length);
T.graphFocus = T.graphFocus && typeof T.graphFocus.layerId === 'string' && typeof T.graphFocus.trackKey === 'string'
  ? T.graphFocus : null;
T.focusGraph = (L: any, channel: string) => {
  const target = typeof api.anim.allProps === 'function'
    ? timelineProperties(api, L, space3d).find((row: any) => trackSelected(row, channel))
    : null;
  T.graphFocus = { layerId: L.id, trackKey: target?.key || channel };
  invalidate('timeline');
};
T.__timelineRuntimeToken = TIMELINE_RUNTIME_TOKEN;
T.__timelineRuntimeDisposed = false;

const runtimeCleanups: Array<() => void> = [];
let headCleanups: Array<() => void> = [];
let canvasCleanups: Array<() => void> = [];
let syncGraphControls = () => {};
const frameIds = new Set<number>();
const timerIds = new Set<number>();
const activeDrags = new Set<any>();
let resizeObserver: ResizeObserver | null = null;
let disposed = false;
let shiftHeld = false;
let refreshActiveScrub: (() => void) | null = null;

function listen(target: any, event: string, handler: any, options?: any, bucket = runtimeCleanups) {
  target?.addEventListener?.(event, handler, options);
  const cleanup = () => target?.removeEventListener?.(event, handler, options);
  bucket.push(cleanup);
  return cleanup;
}
function onEvent<K extends keyof KernelEvents>(event: K, handler: (payload: KernelEvents[K]) => void, bucket = runtimeCleanups) {
  const subscription = api.events.on(event, handler);
  bucket.push(() => subscription.dispose());
}
function scheduleFrame(fn: () => void) {
  let id = 0;
  id = window.requestAnimationFrame(() => { frameIds.delete(id); if (!disposed) fn(); });
  frameIds.add(id);
  return id;
}
function scheduleTimer(fn: () => void, delay: number) {
  let id = 0;
  id = window.setTimeout(() => { timerIds.delete(id); if (!disposed) fn(); }, delay);
  timerIds.add(id);
  return id;
}
let drawPending = false;
function invalidate(what?: string) {
  api.transport.invalidate(what);
  if ((what == null || what === 'timeline') && T.cv && !drawPending) {
    drawPending = true;
    scheduleFrame(() => { drawPending = false; draw(); });
  }
}
function evaluatedValue(layer: any, value: any, time: number, path: string): any {
  return value && typeof value === 'object' && Array.isArray(value.kf) && 'v' in value
    ? api.anim.evP(layer, value, time, path)
    : value;
}
function keyHandles(key: any): any {
  return api.uiState.getKeyHandles(key);
}
function layerSupportsTransform(type: unknown): boolean {
  const metadata = api.model.TYPE_META as Record<string, { transform?: boolean }>;
  return metadata[String(type)]?.transform !== false;
}
function runCleanups(bucket: Array<() => void>) {
  for (const cleanup of bucket.splice(0).reverse()) {
    try { cleanup(); } catch { }
  }
}
function updateShiftHeld(next: boolean) {
  if (shiftHeld === next) return;
  shiftHeld = next;
  refreshActiveScrub?.();
}
listen(window, 'keydown', (event: KeyboardEvent) => {
  if (event.key === 'Shift') updateShiftHeld(true);
}, true);
listen(window, 'keyup', (event: KeyboardEvent) => {
  if (event.key === 'Shift') updateShiftHeld(false);
}, true);
listen(window, 'blur', () => updateShiftHeld(false), undefined);
function shiftSnapping(event?: any) {
  return shiftHeld || !!event?.shiftKey || !!event?.getModifierState?.('Shift');
}
function beginDrag(event: any, options: any) {
  let control: any;
  const finish = (kind: 'up' | 'cancel') => (...args: any[]) => {
    activeDrags.delete(control);
    return options[kind]?.(...args);
  };
  control = api.ui.drag(event, { ...options, up: finish('up'), cancel: finish('cancel') });
  activeDrags.add(control);
  return control;
}
function disposeRuntime() {
  if (disposed) return;
  disposed = true;
  [...activeDrags].forEach((control) => control?.cancel?.()); activeDrags.clear();
  runCleanups(canvasCleanups);
  runCleanups(headCleanups);
  runCleanups(runtimeCleanups);
  resizeObserver?.disconnect(); resizeObserver = null;
  frameIds.forEach((id) => window.cancelAnimationFrame?.(id)); frameIds.clear();
  timerIds.forEach((id) => window.clearTimeout?.(id)); timerIds.clear();
  api.storage.set(`session:${api.project.get().id}`, { pps: T.pps, scrollT: T.scrollT, scrollY: T.scrollY, graph: T.graph });
  if (T.__timelineRuntimeToken === TIMELINE_RUNTIME_TOKEN) {
    T.__timelineRuntimeDisposed = true;
    T.__timelineRuntimeToken = null;
  }
  if (mountedRuntime === T) mountedRuntime = null;
}
T.disposeRuntime = disposeRuntime;
T.dispose = disposeRuntime;

/* After Effects keeps the work area inside the composition. Powermove keeps
   those same editing rules, with one useful extension: pulling the out marker
   past the current composition end grows the composition instead of making the
   handle appear stuck. These helpers stay pure so the gestures are testable. */
/* The work bar owns only a thin strip at the very top of the ruler; the rest
   of the ruler is a scrub surface (click anywhere to move the playhead). */
const WORK_BAR = { top: 0, height: 5, hit: 7 };
const WorkArea = timelineWorkArea;

let attachedHead: HTMLElement | null = null;
let attachedWrap: HTMLElement | null = null;

T.attachHead = (head: HTMLElement) => {
    if (attachedHead === head) return;
    /* Re-bindable: buildHead constructs fresh controls on the new host, so a
       replacement head (HMR) simply rebinds. */
    runCleanups(headCleanups);
    headCleanups = [];
    attachedHead = head;
    T.__timelineHead = head;
    buildHead(head);
};

T.attachCanvas = (wrap: HTMLElement) => {
    if (attachedWrap === wrap) return;
    const cv = wrap.querySelector<HTMLCanvasElement>(':scope > #tl-canvas');
    if (!cv) throw new Error('Timeline host is missing the legacy canvas skeleton');
    /* Re-bindable: the 2D canvas has no cross-host state; rebind on a new host. */
    runCleanups(canvasCleanups);
    canvasCleanups = [];
    resizeObserver?.disconnect(); resizeObserver = null;
    attachedWrap = wrap;
    T.__timelineWrap = wrap;
    /* Keep the backing store transparent while a host resize is in flight.
       An opaque 2D canvas is cleared to black as soon as its bitmap changes,
       which made the whole timeline flash/stick black while a section was
       being adjusted. The timeline still paints its own solid background. */
    T.cv = cv; T.ctx = cv.getContext('2d');
    refreshTimelineManifest();
    bind(cv, wrap);
    resizeObserver = new window.ResizeObserver(() => resize(wrap));
    resizeObserver.observe(wrap);
    scheduleFrame(() => resize(wrap));
};

function buildHead(head: any) {
  /* The layout owns the move handle injected into this slot. Preserve it when
     a hot-reloaded runtime rebuilds only the controls around it. */
  const moveHandles = [...head.children].filter((node: any) => node.classList?.contains('panel-move-handle'));
  head.replaceChildren(...moveHandles);
  const btn = (icon: any, fn: any, title: any) => {
    const button = h('button.iconbtn', { type: 'button', title }, iconNode(api, icon));
    listen(button, 'click', fn, undefined, headCleanups);
    return button;
  };
  const playBtn = btn('play', () => api.transport.toggle(), 'Play / Pause (Space)');
  const time = h('div#tl-time');
  const graph = h('button.iconbtn' + (T.graph ? '.on' : ''), { title: 'Graph editor (Shift+F3)' }, iconNode(api, 'bezier'));
  listen(graph, 'click', () => { T.graph = !T.graph; syncGraphControls(); invalidate('timeline'); }, undefined, headCleanups);
  const graphOptions = btn('more', () => api.ui.menu(graphOptions, [
    { label: 'Value graph', on: T.graphType !== 'speed', run: () => { T.graphType = 'value'; invalidate('timeline'); } },
    { label: 'Speed graph', on: T.graphType === 'speed', run: () => { T.graphType = 'speed'; invalidate('timeline'); } },
    { label: 'Fit selected curves', disabled: !T._graph, run: () => {
      const points = T._graph?.points || [];
      if (!points.length) return;
      const times = points.map((point: any) => point.axis.L.from + point.key.t);
      const first = Math.min(...times), last = Math.max(...times);
      T.pps = clamp((T.w - T.gut - 48) / Math.max(1 / api.project.get().fps, last - first), 4, 4000);
      T.scrollT = Math.max(-.4, first - 24 / T.pps);
      T.graphViewBounds = null; invalidate('timeline');
    } },
  ]), 'Graph options');
  let graphShown: boolean | undefined;
  syncGraphControls = () => {
    const shown = Boolean(T.graph);
    if (graphShown === shown) return;
    graphShown = shown;
    graph.classList.toggle('on', shown);
    graph.setAttribute('aria-pressed', String(shown));
    graphOptions.hidden = !shown;
    graphOptions.style.display = shown ? '' : 'none';
  };
  const graphSlot = h('div.tl-group.tl-graph-slot', graphOptions, graph);
  const transport = h('div.tl-group.tl-transport',
    playBtn,
    time,
  );
  head.append(transport, graphSlot);
  /* The old toolbar node owned the panel move handle, so replacing that node
     during an extension hot update could strand a live panel without a drag
     listener. Make the unoccupied gutter itself a stable drag surface; native
     controls and the draggable timecode remain excluded. */
  listen(head, 'pointerdown', (event: PointerEvent) => {
    const target = event.target as Element | null;
    if (event.button !== 0 || target?.closest('button,input,select,#tl-time')) return;
    const panel = head.closest('.panel[data-panel="timeline"]') as HTMLElement | null;
    const panelHeader = panel?.querySelector(':scope > header');
    if (!panelHeader) return;
    event.stopPropagation();
    /* Reuse the layout's existing, live drag listener without importing its
       private module into the hot-reloadable extension bundle. Subsequent real
       pointer moves are still captured by api.ui.drag on window. */
    panelHeader.dispatchEvent(new PointerEvent('pointerdown', {
      bubbles: true,
      cancelable: true,
      button: event.button,
      buttons: event.buttons,
      clientX: event.clientX,
      clientY: event.clientY,
      pointerId: event.pointerId,
      pointerType: event.pointerType,
      isPrimary: event.isPrimary,
    }));
  }, undefined, headCleanups);
  const syncTime = () => {
    const text = api.util.tc(api.transport.time(), api.project.get().fps);
    if (time.textContent !== text) time.textContent = text;
  };
  const syncTransport = () => {
    /* Keep the icon node stable while playback frames update the timecode.
       Replacing it between pointerdown and pointerup cancels a human-speed
       click in Chromium, which made the visible Pause control unresponsive. */
    const icon = api.transport.playing() ? 'pause' : 'play';
    if (playBtn.querySelector(`[data-icon="${icon}"]`)) return;
    playBtn.replaceChildren(iconNode(api, icon));
  };
  onEvent('time', syncTime, headCleanups); onEvent('transport', syncTransport, headCleanups);
  onEvent('project:changed', () => {
    syncTime();
    syncTransport();
    syncGraphControls();
  }, headCleanups);
  syncTime(); syncTransport(); syncGraphControls(); syncHeadGeometry(head);
  listen(time, 'pointerdown', (e: any) => {
    const startTime = api.transport.time();
    beginDrag(e, { cursor: 'ew-resize', move: (dx: any) => api.transport.setTime(startTime + dx / 12 / api.project.get().fps) });
  }, undefined, headCleanups);
}

function refreshTimelineManifest() {
  syncGraphControls();
  const raw = api.workspace.current()?.chrome?.timeline;
  const config = normalizeTimelineChrome(api, raw);
  T.row = config.rowHeight; T.gut = config.gutterWidth; T.ruler = config.rulerHeight;
  T.style = config;
  const head = document.querySelector<HTMLElement>('#tl-head');
  if (head) {
    if (head.dataset.density !== config.toolbarDensity) head.dataset.density = config.toolbarDensity;
    // The draw resolves the minimum width of property columns before syncing
    // the header. Publishing the narrower configured gutter here and widening
    // it again below forced layout twice on every playback/scrub frame.
  }
  return config;
}

function syncHeadGeometry(head = document.querySelector<HTMLElement>('#tl-head')) {
  if (!head) return;
  const gutter = `${Math.round(T.gut)}px`;
  const ruler = `${Math.round(T.ruler)}px`;
  if (head.style.getPropertyValue('--tl-gutter') !== gutter) head.style.setProperty('--tl-gutter', gutter);
  if (head.style.getPropertyValue('--tl-ruler') !== ruler) head.style.setProperty('--tl-ruler', ruler);
}

function resize(wrap?: any) {
  wrap = wrap || (T.cv && T.cv.parentElement);
  if (!wrap || !T.cv) return;
  const r = wrap.getBoundingClientRect();
  if (r.width < 8 || r.height < 8) return; /* detached / mid-remount */
  T.dpr = Math.min(window.devicePixelRatio || 1, 2);
  T.w = r.width; T.hgt = r.height;
  const width = Math.max(2, Math.round(r.width * T.dpr));
  const height = Math.max(2, Math.round(r.height * T.dpr));
  const changed = T.cv.width !== width || T.cv.height !== height;
  if (T.cv.width !== width) T.cv.width = width;
  if (T.cv.height !== height) T.cv.height = height;
  /* Paint synchronously after the bitmap is cleared. ResizeObserver can run
     after an already-requested animation frame, so relying only on the shared
     invalidation queue can expose the canvas's cleared backing store. */
  if (changed) draw();
  invalidate('timeline');
}
listen(window, 'resize', () => resize());
onEvent('layout', () => {
  refreshTimelineManifest();
  resize();
  /* Second pass after flex settles: the first layout:applied can fire while the
     dock is still animating to its final size (workspace switch, boot with a
     stale saved size), leaving the canvas showing a stale placeholder frame
     until the user nudges a splitter. */
  scheduleFrame(() => resize());
  scheduleTimer(() => resize(), 120);
});

/* ── row model ─────────────────────────────────────────── */
let rowsDirty = true;
let rowsAnimationVersion: number | undefined;
let propertyLabelWidth: number | null = null;
// Selection reveals its ancestor groups without filtering out sibling rows.
// Only the disclosure control should hide children in an expanded group.
function revealSelectedAncestors() {
  for (const layer of selectedLayers(api) || []) {
    for (const group of api.groups.ancestors(layer) || []) {
      api.uiState.setGroupCollapsed(group, false);
    }
  }
  rowsDirty = true;
}
function buildRows() {
  const animationVersion = api.anim.version();
  if (!rowsDirty && rowsAnimationVersion === animationVersion && Array.isArray(T.rows)) return T.rows;
  const rows = [];
  const source = api.project.get().layers;
  const layers: any[] = [];
  const seen = new Set<string>();
  const visit = (parent: string | null) => {
    for (const layer of source) {
      if ((layer.group || null) !== parent || seen.has(layer.id)) continue;
      seen.add(layer.id); layers.push(layer);
      if (layer.type === 'group') visit(layer.id);
    }
  };
  visit(null);
  for (const layer of source) if (!seen.has(layer.id)) layers.push(layer);
  for (let i = 0; i < layers.length; i++) {
    const L = layers[i];
    const ancestors = api.groups.ancestors(L) || [];
    if (!T.search && ancestors.some((group: any) => api.uiState.getGroupCollapsed(group))) continue;
    if (L.shy && !T.showShy) continue;
    const query = String(T.search || '').trim().toLowerCase();
    const matching = query ? visibleProps(L).filter((p: any) => String(p.label).toLowerCase().includes(query)) : [];
    if (query && !L.name.toLowerCase().includes(query) && !matching.length) continue;
    rows.push({ kind: 'layer', L, i: source.indexOf(L), depth: ancestors.length });
    if (!api.uiState.getLayerCollapsed(L) || query) {
      const props = query && matching.length ? matching : visibleProps(L);
      props.forEach((p: any) => rows.push({ kind: 'prop', L, ...p }));
    }
  }
  T.rows = rows;
  rowsDirty = false;
  rowsAnimationVersion = animationVersion;
  propertyLabelWidth = null;
  return rows;
}
function visibleProps(L: any) {
  return revealedProperties(api, L, timelineProperties(api, L, space3d));
}

const x2t = (x: any) => (x - T.gut) / T.pps + T.scrollT;
const t2x = (t: any) => T.gut + (t - T.scrollT) * T.pps;
/* Unshifted row geometry. While a media drag is over the canvas, rows at or
   below the insertion slot move down one row so the ghost lands in a real
   gap: a new layer strip, never on top of an existing one. */
const rawRowY = (idx: any) => Math.round(T.ruler + idx * T.row - T.scrollY);
const rowShift = (idx: any) => T.drop && !T.graph && idx >= T.drop.rowIdx ? 1 : 0;
const rowY = (idx: any) => rawRowY(idx + rowShift(idx));

/* ── draw ──────────────────────────────────────────────── */
onEvent('project:changed', () => { rowsDirty = true; invalidate('timeline'); });
// Host-side invalidations (reveal, collapse, history and selection restore)
// arrive as a coalesced 'timeline' repaint request; rows depend on UI state,
// so they are rebuilt, not just repainted.
onEvent('invalidate', (what) => { if (what === 'timeline') { rowsDirty = true; draw(); } });
onEvent('time', () => invalidate('timeline'));
onEvent('selection', () => {
  revealSelectedAncestors();
  buildRows();
  /* Selection can originate in the composition canvas, marquee, inspector,
     or another panel. Never move the user's timeline viewport to chase it. */
  invalidate('timeline');
});
revealSelectedAncestors();

function css(v: any) { return window.getComputedStyle(document.documentElement).getPropertyValue(v).trim(); }
let theme: any = null;
const refreshTheme = () => {
  const reversed = document.documentElement.dataset.timelineSurfaces === 'reversed';
  theme = {
    accent: css('--accent') || '#F0580A', tx: css('--tx') || '#1C1C1F',
    tx2: css('--tx-2') || '#5D5D65', tx3: css('--tx-3') || '#8B8B93',
    tx4: css('--tx-4') || '#A9A9AE',
    /* Both timeline surfaces stay in the panel family: chrome (gutter/ruler)
       on --bg-panel, tracks one step down on --bg-panel-2. Never --bg-sunken:
       in dark themes it sits below the window color, so the gutter would
       dissolve into the app background instead of reading as a panel. */
    panel: css(reversed ? '--bg-panel-2' : '--bg-panel') || '#FCFCFD',
    float: css('--bg-float') || '#FFFFFF',
    sunken: css(reversed ? '--bg-panel' : '--bg-panel-2') || '#F0F0F3',
    line: css('--line') || 'rgba(15,15,20,.09)',
  };
};
onEvent('theme:changed', () => { refreshTheme(); refreshInk(); invalidate('timeline'); });

/* Ink-on-paper colors for canvas chrome. Light theme uses black alpha;
   dark theme uses white alpha — resolved on every theme refresh. */
const INK = { over:'', over2:'', grid:'', tick:'', sub:'', hi:'', lo:'', thumb:'', key:'', handle:'', inv:'' };
function refreshInk() {
  const dark = document.documentElement.dataset.theme === 'dark';
  const a = (x: any) => (dark ? 'rgba(255,255,255,' : 'rgba(15,15,20,') + x + ')';
  INK.over = a('.045'); INK.over2 = a('.06'); INK.grid = a('.05'); INK.tick = a('.13');
  INK.sub = a('.32'); INK.hi = a('.72'); INK.lo = a('.16'); INK.thumb = a('.14');
  INK.key = dark ? '#d7d7db' : '#54545c';
  INK.handle = dark ? '#8f8f96' : '#8B8B93';
  INK.inv = dark ? '#fff' : '#1C1C1F';
}
refreshInk();

function draw() {
  try { drawInner(); } catch (e: any) { console.error('[timeline draw]', e, e.stack); }
}
type TimelinePreviewTarget = { canvas: HTMLCanvasElement; width: number; height: number };

function drawInner(preview?: TimelinePreviewTarget) {
  if (!preview) refreshTimelineManifest();
  /* Re-resolve the live canvas every frame: workspace rebuilds can replace the
     panel element, and drawing into a detached canvas is the root cause of
     gutter/clip misalignment after layout changes. */
  if (preview) {
    T.cv = preview.canvas;
    T.dpr = Math.min(window.devicePixelRatio || 1, 2);
    T.w = preview.width;
    T.hgt = preview.height;
    T.cv.width = Math.max(2, Math.round(preview.width * T.dpr));
    T.cv.height = Math.max(2, Math.round(preview.height * T.dpr));
    T.ctx = T.cv.getContext('2d');
  } else {
    const liveCv = document.querySelector<HTMLCanvasElement>('#tl-canvas');
    if (liveCv && liveCv !== T.cv) { T.cv = liveCv; T.ctx = liveCv.getContext('2d'); }
    /* Unconditional size sync: measure the wrap every draw so the bitmap always
       matches the laid-out size, regardless of missed observer/RAF frames. */
    const host = T.cv && T.cv.parentElement;
    if (host && (!api.transport.playing() || !T.w || !T.hgt)) {
      const r = host.getBoundingClientRect();
      if (r.width >= 8 && r.height >= 8) {
        const bw = Math.max(2, Math.round(r.width * T.dpr));
        const bh = Math.max(2, Math.round(r.height * T.dpr));
        if (T.cv.width !== bw || T.cv.height !== bh || T.w !== r.width || T.hgt !== r.height) {
          T.w = r.width; T.hgt = r.height; T.cv.width = bw; T.cv.height = bh;
        }
      }
    }
  }
  const c = T.ctx; if (!c) return null;
  if (!theme) refreshTheme();
  const p = api.project.get();
  const W = T.w, H = T.hgt;
  c.setTransform(T.dpr, 0, 0, T.dpr, 0, 0);
  c.fillStyle = theme.panel; c.fillRect(0, 0, W, H);
  buildRows();
  // Column widths are based on full labels, never on the fluctuating values.
  // Keep enough room for both Scale dimensions even at narrow saved gutters.
  c.font = '400 11px ' + fui();
  if (propertyLabelWidth == null) propertyLabelWidth = Math.max(64, ...T.rows.filter((row: any) => row.kind === 'prop')
    .map((row: any) => Math.ceil(c.measureText(row.label).width)));
  const labelWidth = propertyLabelWidth;
  T.propertyValueX = 100 + labelWidth + 12;
  T.gut = Math.max(T.gut, T.propertyValueX + 90);
  if (preview) {
    T.scrollT = 0;
    T.scrollY = 0;
    T.pps = Math.max(.01, (W - T.gut - 16) / Math.max(.5, Number(api.project.get().dur) || .5));
  } else syncHeadGeometry();

  const maxScroll = Math.max(0, T.rows.length * T.row - (H - T.ruler));
  T.scrollY = clamp(T.scrollY, 0, maxScroll);

  /* keep playhead in view while playing (AE follow) */
  if (api.transport.playing()) {
    const px = t2x(api.transport.time());
    if (px > W - 50) T.scrollT = api.transport.time() - (W - T.gut - 50) / T.pps;
    else if (px < T.gut) T.scrollT = api.transport.time() - 60 / T.pps;
    T.scrollT = Math.max(-.4, T.scrollT);
  }

  drawTracksBg(c, W, H);
  if (T.graph) drawGraph(c, W, H);
  else { drawClips(c, W, H); drawDropGhost(c, W, H); }
  drawGutter(c, W, H);
  if (T.reorder) {
    const d = T.reorder, y = rowY(d.row), left = 74 + Math.min(48, d.depth * 12);
    c.save(); c.beginPath(); c.rect(0, T.ruler, W, H - T.ruler); c.clip();
    c.strokeStyle = theme.accent; c.fillStyle = theme.accent; c.lineWidth = 2;
    if (d.mode === 'inside') {
      c.fillStyle = rgba(theme.accent, .16); c.fillRect(left - 12, y, W - left + 12, T.row);
      c.strokeRect(left - 12, y + 1, W - left + 11, T.row - 2);
    } else {
      c.beginPath(); c.moveTo(left, y); c.lineTo(W, y); c.stroke();
      c.beginPath(); c.arc(left, y, 3, 0, Math.PI * 2); c.fill();
    }
    const label = `${d.mode === 'inside' ? 'Move into' : d.mode === 'before' ? 'Move before' : 'Move after'} ${d.name}`;
    c.font = '11px system-ui';
    const width = c.measureText(label).width + 16, top = Math.max(T.ruler + 2, Math.min(H - 24, y - 24));
    c.fillStyle = theme.accent; c.fillRect(T.gut + 10, top, width, 20);
    c.fillStyle = '#fff'; c.fillText(label, T.gut + 18, top + 14); c.restore();
  }
  drawRuler(c, W, H);
  drawPlayhead(c, W, H);
  drawQuickOffset(c, W, H);
  drawScrollThumb(c, W, H, maxScroll);
  if ((window as any).__tlDebug) {
    const px = t2x(api.transport.time());
    const msg = '[tl] t=' + api.transport.time().toFixed(3) + ' px=' + (isFinite(px) ? px.toFixed(1) : String(px)) + ' gut=' + T.gut + ' W=' + W + ' sT=' + T.scrollT.toFixed(3) + ' pps=' + T.pps + ' rows=' + T.rows.length + ' graph=' + T.graph;
    try { (window as any).webkit.messageHandlers.pmLog.postMessage(msg); } catch (e) { }
    console.log(msg);
  }
  if (T.marquee) {
    c.save(); c.strokeStyle = theme.accent; c.fillStyle = rgba(theme.accent, .14); c.lineWidth = 1;
    const m = T.marquee;
    c.fillRect(m.x0, m.y0, m.x1 - m.x0, m.y1 - m.y0);
    c.strokeRect(m.x0 + .5, m.y0 + .5, m.x1 - m.x0 - 1, m.y1 - m.y0 - 1); c.restore();
  }
  if (preview) {
    preview.canvas.dataset.timelinePreviewMode = 'full-duration';
    preview.canvas.dataset.timelinePreviewStart = '0';
    preview.canvas.dataset.timelinePreviewEnd = String(api.project.get().dur);
    preview.canvas.dataset.timelinePreviewRight = String(T.gut + Math.max(0, Number(api.project.get().dur) || 0) * T.pps);
    return { gutter: T.gut, ruler: T.ruler, pps: T.pps };
  }
  return null;
}

T.renderPreview = (canvas: HTMLCanvasElement, width: number, height: number) => {
  const saved = {
    cv: T.cv, ctx: T.ctx, w: T.w, hgt: T.hgt, dpr: T.dpr,
    gut: T.gut, row: T.row, ruler: T.ruler, pps: T.pps,
    scrollT: T.scrollT, scrollY: T.scrollY, graph: T.graph,
    style: T.style, rows: T.rows, propertyValueX: T.propertyValueX,
    hover: T.hover, marquee: T.marquee, dropRow: T.dropRow, drop: T.drop, reorder: T.reorder,
    quickOffset: T.quickOffset,
  };
  try {
    T.gut = 224;
    T.row = 30;
    T.ruler = 28;
    T.graph = false;
    T.hover = null;
    T.marquee = null;
    T.dropRow = null;
    T.drop = null;
    T.reorder = null;
    T.quickOffset = null;
    T.style = {
      clipRadius: 5, keyframeSize: 8, showLayerNumbers: true,
      showTypeBadges: true, toolbarDensity: 'compact'
    };
    return drawInner({ canvas, width, height }) || { gutter: T.gut, ruler: T.ruler, pps: T.pps };
  } finally {
    Object.assign(T, saved);
  }
};

/* Thin scrollbar on the right edge of the track area when rows overflow. */
function drawScrollThumb(c: any, W: any, H: any, maxScroll: any) {
  if (maxScroll <= 0) return;
  const trackH = H - T.ruler;
  const total = T.rows.length * T.row;
  const th = Math.max(18, trackH * trackH / total);
  const ty = T.ruler + (trackH - th) * (T.scrollY / maxScroll);
  c.fillStyle = INK.thumb;
  roundRect(c, W - 5, ty, 3, th, 1.5); c.fill();
}

/* After expanding/collapsing a layer, scroll just enough to keep the toggled
   row and its newly revealed children inside the viewport (AE behavior). */
function keepRowsVisible(rowIdx: any, childCount: any) {
  const viewH = T.hgt - T.ruler;
  if (viewH <= 0) return;
  const top = rowIdx * T.row;
  const bottom = top + (1 + childCount) * T.row;
  if (bottom - T.scrollY > viewH) T.scrollY = bottom - viewH;
  if (top < T.scrollY) T.scrollY = top;
  T.scrollY = Math.max(0, T.scrollY);
}

function drawTracksBg(c: any, W: any, H: any) {
  c.save(); c.beginPath(); c.rect(T.gut, T.ruler, W - T.gut, H - T.ruler); c.clip();
  c.fillStyle = theme.sunken; c.fillRect(T.gut, T.ruler, W - T.gut, H - T.ruler);
  /* Keep the active work area clear and shade only the range outside it. */
  const p = api.project.get();
  const wa = p.work || [0, p.dur];
  const x0 = t2x(wa[0]), x1 = t2x(wa[1]);
  c.fillStyle = INK.over;
  c.fillRect(T.gut, T.ruler, Math.max(0, x0 - T.gut), H - T.ruler);
  c.fillRect(x1, T.ruler, Math.max(0, W - x1), H - T.ruler);
  /* second gridlines */
  const step = niceStep(T.pps);
  c.strokeStyle = INK.grid; c.lineWidth = 1;
  c.beginPath();
  for (let t = Math.floor(T.scrollT / step) * step; t2x(t) < W; t += step) {
    const x = Math.round(t2x(t)) + .5;
    if (x < T.gut) continue;
    c.moveTo(x, T.ruler); c.lineTo(x, H);
  }
  c.stroke();
  /* row stripes + lane hairlines */
  for (let i = 0; i < T.rows.length; i++) {
    const y = rowY(i);
    if (y + T.row < T.ruler || y > H) continue;
    const r = T.rows[i];
    /* Row states are alpha washes: selected > hovered. No hairlines between
       rows; the clip bodies give the rows their structure. */
    if (r.kind === 'layer' && api.selection.layers().includes(r.L.id)) {
      c.fillStyle = INK.over; c.fillRect(T.gut, y, W - T.gut, T.row);
    } else if (T.hoverRow === i) {
      c.fillStyle = INK.over; c.globalAlpha = .7; c.fillRect(T.gut, y, W - T.gut, T.row); c.globalAlpha = 1;
    }
    if (T.dropRow === i) {
      c.fillStyle = INK.over2; c.fillRect(T.gut, y, W - T.gut, T.row);
      c.strokeStyle = theme.accent; c.lineWidth = 1;
      c.strokeRect(T.gut + .5, y + .5, W - T.gut - 1, T.row - 1);
    }
  }
  c.restore();
}

function niceStep(pps: any) {
  const targets = [1 / 30, 1 / 10, .2, .5, 1, 2, 5, 10, 15, 30, 60, 120, 300];
  for (const s of targets) if (s * pps > 62) return s;
  return 600;
}

function drawRuler(c: any, W: any, H: any) {
  const p = api.project.get();
  c.fillStyle = theme.panel; c.fillRect(0, 0, W, T.ruler);
  c.strokeStyle = theme.line; c.beginPath(); c.moveTo(0, T.ruler - .5); c.lineTo(W, T.ruler - .5); c.stroke();
  c.save(); c.beginPath(); c.rect(T.gut, 0, W - T.gut, T.ruler); c.clip();
  const step = niceStep(T.pps);
  /* Bottom-anchored ticks — 8px major, 3px minor — with light mono labels
     centered over the major tick, so numbers read as a scale, not a list. */
  c.font = '300 10px ' + fmono();
  c.fillStyle = theme.tx3; c.textBaseline = 'middle'; c.textAlign = 'center';
  c.strokeStyle = INK.tick;
  c.beginPath();
  for (let t = Math.floor(T.scrollT / step) * step; t2x(t) < W; t += step) {
    const x = Math.round(t2x(t)) + .5;
    if (x >= T.gut - 1) {
      c.moveTo(x, T.ruler - 8); c.lineTo(x, T.ruler);
      c.fillText(fmtRuler(t, step, p.fps), x, WORK_BAR.height + 8);
    }
    /* minor ticks between labeled steps */
    for (let m = 1; m < 4; m++) {
      const mx = Math.round(t2x(t + step * m / 4)) + .5;
      if (mx > T.gut && mx < W) { c.moveTo(mx, T.ruler - 3); c.lineTo(mx, T.ruler); }
    }
  }
  c.stroke();
  c.textAlign = 'left';
  /* Work-area brackets stay visible without drawing a line across the ruler. */
  const wa = p.work || [0, p.dur];
  const x0 = t2x(wa[0]), x1 = t2x(wa[1]);
  drawWorkBracket(c, x0, 0); drawWorkBracket(c, x1, 1);
  c.restore();
}
function drawWorkBracket(c: any, x: any, idx: any) {
  const y = WORK_BAR.top, h = WORK_BAR.height, wing = 6;
  c.fillStyle = INK.sub;
  c.beginPath();
  if (idx === 0) {
    c.moveTo(x, y); c.lineTo(x + wing, y); c.lineTo(x + wing, y + 3);
    c.lineTo(x + 2, y + 3); c.lineTo(x + 2, y + h); c.lineTo(x, y + h);
  } else {
    c.moveTo(x, y); c.lineTo(x - wing, y); c.lineTo(x - wing, y + 3);
    c.lineTo(x - 2, y + 3); c.lineTo(x - 2, y + h); c.lineTo(x, y + h);
  }
  c.closePath(); c.fill();
}
function fmtRuler(t: any, step: any, fps: any) {
  if (step < 1) return api.util.tc(t, fps).slice(-5);
  const m = Math.floor(t / 60), s = Math.round(t % 60);
  return (m ? m + ':' : '0:') + String(s).padStart(2, '0');
}
const fui = () => '"SF Pro Text",-apple-system,BlinkMacSystemFont,sans-serif';
const fmono = () => '"JetBrains Mono","SF Mono",ui-monospace,Menlo,monospace';

function drawClips(c: any, W: any, H: any) {
  c.save(); c.beginPath(); c.rect(T.gut, T.ruler, W - T.gut, H - T.ruler); c.clip();
  for (let i = 0; i < T.rows.length; i++) {
    const y = rowY(i);
    if (y + T.row < T.ruler || y > H) continue;
    const r = T.rows[i];
    if (r.kind === 'layer') drawClip(c, r.L.type === 'group' ? { ...r.L, ...api.groups.span(r.L) } : r.L, y);
    else drawPropKeys(c, r, y);
  }
  c.restore();
}

/* Drop ghost: a translucent clip in the media type's palette, dashed ring,
   sitting exactly where the layer will land, plus a hairline at the drop
   frame so the time reads against the ruler. Rows below the stack get an
   insertion slot so "append at bottom" is still visible. */
function drawDropGhost(c: any, W: any, H: any) {
  const d = T.drop; if (!d || T.graph) return;
  const p = api.project.get();
  const type = d.kind === 'audio' ? 'audio' : d.kind === 'video' ? 'video' : d.kind === 'image' ? 'image' : null;
  const pal = clipPalette({ type });
  const dur = d.dur || Math.max(1 / p.fps, Math.min(4, p.dur - d.at) || 4);
  const x0 = t2x(d.at), w = Math.max(6, dur * T.pps);
  /* the slot is the gap opened by rowShift, so it uses unshifted geometry */
  const y = rawRowY(d.rowIdx);
  const yy = y + 1, hh = T.row - 2;
  const r = Math.min(4, T.style.clipRadius);
  c.save();
  c.beginPath(); c.rect(T.gut, T.ruler, W - T.gut, H - T.ruler); c.clip();
  /* landing slot behind the ghost */
  c.fillStyle = INK.over2; c.fillRect(T.gut, y, W - T.gut, T.row);
  /* body */
  c.globalAlpha = .55;
  roundRect(c, x0, yy, w, hh, r);
  c.fillStyle = pal.body; c.fill();
  c.globalAlpha = 1;
  c.setLineDash([4, 3]);
  c.strokeStyle = pal.primary; c.lineWidth = 1.25;
  roundRect(c, x0 + .5, yy + .5, w - 1, hh - 1, r); c.stroke();
  c.setLineDash([]);
  /* label */
  c.save();
  roundRect(c, x0, yy, w, hh, r); c.clip();
  c.font = '500 11px ' + fui();
  c.textBaseline = 'middle';
  c.fillStyle = pal.foreground;
  const badge = type ? BADGE[type] : 'new';
  c.globalAlpha = .7; c.fillText(badge, x0 + 7, yy + hh / 2);
  c.globalAlpha = 1;
  const bw = c.measureText(badge).width + 12;
  const label = d.name;
  if (w > bw + 24) c.fillText(label, x0 + 7 + bw, yy + hh / 2, w - bw - 14);
  c.restore();
  /* drop-frame hairline + ruler tag */
  const hx = Math.round(x0) + .5;
  c.strokeStyle = theme.accent; c.lineWidth = 1;
  c.setLineDash([2, 3]);
  c.beginPath(); c.moveTo(hx, T.ruler); c.lineTo(hx, yy); c.stroke();
  c.setLineDash([]);
  c.restore();
  const tag = api.util.tc(d.at, p.fps);
  c.save();
  c.font = '500 10px ' + fui(); c.textBaseline = 'middle';
  const tw = c.measureText(tag).width + 10;
  const tx = Math.min(W - tw - 2, Math.max(T.gut, x0));
  roundRect(c, tx, 2, tw, T.ruler - 4, 3);
  c.fillStyle = theme.accent; c.fill();
  c.fillStyle = css('--on-accent') || '#fff';
  c.fillText(tag, tx + 5, T.ruler / 2);
  c.restore();
}

function rgba(hex: any, a: any) { const [r = 0, g = 0, b = 0] = api.util.hex2rgb(hex); return `rgba(${r * 255 | 0},${g * 255 | 0},${b * 255 | 0},${a})`; }

const BADGE: any = { text: 'T', shape: 'S', solid: 'S', shader: 'fx', extension: 'ext', null: 'N', image: 'img', video: 'vid', audio: 'aud' };

function drawAudioClipWaveform(c: any, L: any, { x, y, width, height, color }: any) {
  const clipLeft = Math.max(x, T.gut);
  c.save();
  /* Waveform in the clip's bright tint. With no name label to stay clear of,
     the bars stand on the clip floor and use the full body height. */
  const top = 2;
  try {
    if (api.media.audio.drawWaveform(c, L, {
      x, y: y + top, width, height: height - top - 2, clipLeft, color, placeholderColor: color,
      step: 1, anchor: 1,
    })) {
      c.restore();
      return;
    }
  } catch (error) { /* Keep the timeline usable while waveform data is unavailable. */ }
  const right = Math.min(x + width, T.w);
  if (right > clipLeft) {
    c.strokeStyle = color;
    c.lineWidth = 1;
    c.beginPath();
    c.moveTo(clipLeft, Math.round(y + height / 2) + .5);
    c.lineTo(right, Math.round(y + height / 2) + .5);
    c.stroke();
  }
  c.restore();
}

/* Clip material is keyed by layer TYPE, not by the layer's swatch color —
   a fixed, hand-tuned palette so every text clip is the same slate, every
   audio clip the same deep green with a mint waveform. (Editor does this;
   deriving bodies from arbitrary layer hues produces mud.) The layer color
   stays a tag in the gutter. */
const CLIP_TYPES: Record<string, { body: string; primary: string; foreground: string }> = {
  /* Muted, low-chroma bodies: enough hue to tell types apart, never loud. */
  text:   { body: '#3A4756', primary: '#9FB3C8', foreground: '#EAF0F6' },
  audio:  { body: '#25473C', primary: '#5FC29C', foreground: '#DDF5EA' },
  video:  { body: '#2E4266', primary: '#8FB0E8', foreground: '#E2EAF8' },
  image:  { body: '#5E4A2E', primary: '#D9B57C', foreground: '#F6EFE3' },
  shape:  { body: '#5E3D36', primary: '#D99A8A', foreground: '#F7E9E5' },
  solid:  { body: '#573A4E', primary: '#CF93B8', foreground: '#F6E6F0' },
  shader: { body: '#413B60', primary: '#A99AE0', foreground: '#ECE8F8' },
  extension: { body: '#40395D', primary: '#B2A6EF', foreground: '#F0EDFF' },
  null:   { body: '#3E434B', primary: '#A2A9B3', foreground: '#EEF0F3' },
};
const CLIP_FALLBACK = { body: '#34505A', primary: '#8DB8C6', foreground: '#E6F1F4' };
function mixHex(color: string, toward: [number, number, number], amount: number) {
  const m1 = color.match(/^rgb\((\d+),(\d+),(\d+)\)$/);
  const [r = 0, g = 0, b = 0] = m1 ? [+m1[1]! / 255, +m1[2]! / 255, +m1[3]! / 255] : api.util.hex2rgb(color);
  const m = (v: number, t: number) => Math.round((v * 255) * (1 - amount) + t * amount);
  return `rgb(${m(r, toward[0])},${m(g, toward[1])},${m(b, toward[2])})`;
}
function clipPalette(L: any) {
  const base = CLIP_TYPES[L?.type] || CLIP_FALLBACK;
  const dark = document.documentElement.dataset.theme === 'dark';
  if (dark) return { ...base, ring: 'rgba(0,0,0,.55)' };
  /* Light theme: the same hues lifted toward paper, ink from the body. */
  return {
    body: mixHex(base.body, [255, 255, 255], .72),
    primary: mixHex(base.body, [255, 255, 255], .18),
    foreground: mixHex(base.body, [0, 0, 0], .35),
    ring: mixHex(base.body, [255, 255, 255], .45),
  };
}

function drawClip(c: any, L: any, y: any) {
  const x0 = t2x(L.from), x1 = t2x(L.from + L.dur);
  if (x1 < T.gut || x0 > T.w) return;
  /* Clips fill their row with a 1px breath between neighbours. */
  const hh = T.row - 2;
  const yy = y + 1;
  const sel = api.selection.layers().includes(L.id);
  const r = Math.min(4, T.style.clipRadius);
  const w = Math.max(4, x1 - x0);
  const pal = clipPalette(L);
  const off = !evaluatedValue(L, L.on, api.transport.time(), 'l.on');
  const dark = document.documentElement.dataset.theme === 'dark';
  c.save();
  if (off) c.globalAlpha = .5;
  /* Tactile chip material: soft drop shadow (y3 blur4), vertical gradient
     body, inner top highlight, hairline ring one shade under the body. */
  c.shadowColor = dark ? 'rgba(0,0,0,.35)' : 'rgba(15,15,20,.12)';
  c.shadowBlur = 4; c.shadowOffsetY = 2;
  roundRect(c, x0, yy, w, hh, r);
  const g = c.createLinearGradient(0, yy, 0, yy + hh);
  g.addColorStop(0, mixHex(pal.body, [255, 255, 255], dark ? .06 : .04));
  g.addColorStop(1, mixHex(pal.body, [0, 0, 0], dark ? .10 : .05));
  c.fillStyle = g; c.fill();
  c.shadowColor = 'transparent'; c.shadowBlur = 0; c.shadowOffsetY = 0;
  c.save();
  c.clip();
  /* inner top highlight — white 33% on light, a hint on dark */
  c.strokeStyle = dark ? 'rgba(255,255,255,.10)' : 'rgba(255,255,255,.45)';
  c.lineWidth = 1;
  c.beginPath(); c.moveTo(x0 + r * .7, yy + 1.5); c.lineTo(x0 + w - r * .7, yy + 1.5); c.stroke();
  /* Transition windows read as wedges inside the clip instead of timeline
     handles: they communicate timing without adding another hit target. */
  c.save();
  c.globalAlpha *= .55;
  c.fillStyle = pal.primary;
  const inWidth = Math.min(w, Math.max(0, Number(L.transitionIn?.dur) || 0) * T.pps);
  if (L.transitionIn && inWidth > 0) {
    c.beginPath();
    c.moveTo(x0, yy);
    c.lineTo(x0 + inWidth, yy);
    c.lineTo(x0, yy + hh);
    c.closePath();
    c.fill();
  }
  const outWidth = Math.min(w, Math.max(0, Number(L.transitionOut?.dur) || 0) * T.pps);
  if (L.transitionOut && outWidth > 0) {
    c.beginPath();
    c.moveTo(x0 + w, yy);
    c.lineTo(x0 + w - outWidth, yy);
    c.lineTo(x0 + w, yy + hh);
    c.closePath();
    c.fill();
  }
  c.restore();
  if (L.type === 'audio') drawAudioClipWaveform(c, L, {
    x: x0, y: yy, width: w, height: hh, color: pal.primary,
  });
  if (shouldDrawClipLabel(L.type)) {
    /* Name only, top-left, sticking to the viewport edge as the clip scrolls
       off. Short rows center it. */
    c.textBaseline = 'middle';
    const lx = Math.max(x0, T.gut) + 6;
    const maxX = x0 + w - 6;
    c.font = '400 11px ' + fui();
    c.fillStyle = pal.foreground;
    if (maxX - lx > 12) clipText(c, L.name, lx, hh >= 30 ? yy + 10 : yy + hh / 2 + .5, maxX - lx);
  }
  c.restore();
  /* Inset ring: 1px dark seam at rest, 2px accent when selected. Clipped to
     the body so the stroke never grows the clip. */
  c.save();
  roundRect(c, x0, yy, w, hh, r); c.clip();
  c.strokeStyle = sel ? theme.accent : mixHex(pal.body, [0, 0, 0], dark ? .4 : .18);
  c.lineWidth = sel ? 4 : 2; c.stroke();
  c.restore();
  c.restore();
}

function drawPropKeys(c: any, r: any, y: any) {
  const L = r.L, cy = y + T.row / 2;
  c.strokeStyle = INK.grid;
  c.beginPath(); c.moveTo(T.gut, cy); c.lineTo(T.w, cy); c.stroke();
  const kf = r.prop.kf;
  const keyRadius = T.style.keyframeSize / 2;
  /* First/last keys are half-filled toward the animated span; interior keys
     are solid. Tells you where a curve starts and ends at a glance. */
  const first = kf[0]?.t, last = kf.at(-1)?.t;
  const localStart = T.scrollT - L.from - 8 / Math.max(.01, T.pps);
  const localEnd = x2t(T.w + 8) - L.from;
  let lo = 0, hi = kf.length;
  while (lo < hi) { const mid = (lo + hi) >> 1; if (kf[mid].t < localStart) lo = mid + 1; else hi = mid; }
  c.lineWidth = 1;
  for (let index = lo; index < kf.length && kf[index].t <= localEnd; index++) {
    const k = kf[index];
    const x = t2x(L.from + k.t);
    if (x < T.gut - 6 || x > T.w + 6) continue;
    const sel = keySelected(k);
    const color = sel ? theme.accent : INK.key;
    c.fillStyle = color; c.strokeStyle = color;
    if (k.hold) { c.fillRect(x - keyRadius, cy - keyRadius, keyRadius * 2, keyRadius * 2); continue; }
    c.beginPath(); c.moveTo(x, cy - keyRadius); c.lineTo(x + keyRadius, cy); c.lineTo(x, cy + keyRadius); c.lineTo(x - keyRadius, cy); c.closePath();
    c.stroke();
    const isFirst = k.t === first, isLast = k.t === last;
    if (isFirst === isLast) { c.fill(); continue; }
    c.beginPath(); c.moveTo(x, cy - keyRadius);
    c.lineTo(x + (isFirst ? keyRadius : -keyRadius), cy);
    c.lineTo(x, cy + keyRadius); c.closePath(); c.fill();
  }
}

function drawGutter(c: any, W: any, H: any) {
  c.fillStyle = theme.panel;
  c.fillRect(0, T.ruler, T.gut, H - T.ruler);
  c.strokeStyle = theme.line; c.beginPath();
  c.moveTo(T.gut - .5, 0); c.lineTo(T.gut - .5, H); c.stroke();
  c.save(); c.beginPath(); c.rect(0, T.ruler, T.gut, H - T.ruler); c.clip();
  for (let i = 0; i < T.rows.length; i++) {
    const y = rowY(i);
    if (y + T.row < T.ruler || y > H) continue;
    const r = T.rows[i];
    if (r.kind === 'layer') {
      const L = r.L, sel = api.selection.layers().includes(L.id);
      if (sel) { c.fillStyle = INK.over2; c.fillRect(0, y, T.gut, T.row); }
      else if (T.hoverRow === i) { c.fillStyle = INK.over; c.fillRect(0, y, T.gut, T.row); }
      /* Quiet card: controls surface on hover/selection or when they carry
         state (hidden or locked). Idle rows show number, tag, name. */
      const active = sel || T.hoverRow === i;
      const indent = Math.min(48, (r.depth || 0) * 12);
      const collapsed = L.type === 'group'
        ? api.uiState.getGroupCollapsed(L)
        : api.uiState.getLayerCollapsed(L);
      c.font = '400 10px ' + fmono();
      c.fillStyle = INK.lo; c.textBaseline = 'middle';
      if (T.style.showLayerNumbers) c.fillText(String(r.i + 1).padStart(2, '0'), 8, y + T.row / 2);
      /* eye / lock */
      if (active || !evaluatedValue(L, L.on, api.transport.time(), 'l.on')) icoEye(c, 30, y + T.row / 2, evaluatedValue(L, L.on, api.transport.time(), 'l.on'));
      if (active || L.lock) icoLock(c, 48, y + T.row / 2, L.lock);
      /* twirl */
      if (active || !collapsed || L.type === 'group') {
        c.save();
        c.translate(64 + indent, y + T.row / 2); c.rotate(collapsed ? 0 : Math.PI / 2);
        c.strokeStyle = active ? theme.tx2 : theme.tx3; c.lineWidth = 1.4; c.beginPath();
        c.moveTo(-1.6, -3.4); c.lineTo(2, 0); c.lineTo(-1.6, 3.4); c.stroke();
        c.restore();
      }
      /* type icon: a swatch of the clip body with the type letter in its
         bright tint, so the card and the clip share one material */
      const pal = clipPalette(L);
      const iy = y + T.row / 2;
      c.fillStyle = pal.body; roundRect(c, 74 + indent, iy - 8, 16, 16, 3); c.fill();
      c.strokeStyle = pal.ring; c.lineWidth = 1; c.stroke();
      c.font = '600 8px ' + fui(); c.fillStyle = pal.primary; c.textAlign = 'center';
      c.fillText((L.type === 'group' ? 'G' : BADGE[L.type] || '·').toUpperCase().slice(0, 1), 82 + indent, iy + .5);
      c.textAlign = 'left';
      c.font = (sel ? '500 ' : '400 ') + '12px ' + fui();
      c.fillStyle = sel ? theme.tx : theme.tx2;
      const hasParentControl = layerSupportsTransform(L.type);
      const showParentControl = hasParentControl && (active || !!L.parent);
      const statusWidth = (L.solo ? 12 : 0) + (evaluatedValue(L, L.mblur, api.transport.time(), 'l.mblur') ? 12 : 0);
      const nameRight = T.gut - (hasParentControl ? 50 : 12) - statusWidth;
      clipText(c, L.name, 96 + indent, iy, Math.max(0, nameRight - 96 - indent));
      if (showParentControl) {
        c.strokeStyle = L.parent ? theme.accent : theme.tx3; c.lineWidth = 1.2;
        c.beginPath();
        for (let step = 0; step <= 32; step++) {
          const angle = step / 32 * Math.PI * 3, radius = 1 + step / 32 * 4;
          const x = T.gut - 30 + Math.cos(angle) * radius, py = iy + Math.sin(angle) * radius;
          if (!step) c.moveTo(x, py); else c.lineTo(x, py);
        }
        c.stroke();
        c.beginPath(); c.moveTo(T.gut - 15, iy - 1.5); c.lineTo(T.gut - 12, iy + 1.5); c.lineTo(T.gut - 9, iy - 1.5); c.stroke();
      }
      let statusX = nameRight + 8;
      if (L.solo) { c.fillStyle = theme.accent; c.fillText('●', statusX, iy); statusX += 12; }
      if (evaluatedValue(L, L.mblur, api.transport.time(), 'l.mblur')) {
        c.fillStyle = theme.accent; c.beginPath(); c.arc(statusX, iy, 2, 0, 7); c.fill();
      }
    } else {
      const L = r.L;
      const selected = api.selection.layers().includes(L.id) && trackSelected(r, api.selection.chan() ?? '');
      if (selected) { c.fillStyle = INK.over2; c.fillRect(0, y, T.gut, T.row); }
      else if (T.hoverRow === i) { c.fillStyle = INK.over; c.fillRect(0, y, T.gut, T.row); }
      c.font = '400 11px ' + fui();
      c.fillStyle = selected ? theme.accent : theme.tx3;
      const labelX = 100, valueX = T.propertyValueX;
      if (r.prop.kf.length) {
        drawKeyArrow(c, 28, y + T.row / 2, -1, adjacentKeyframe(api, -1, r, space3d) != null);
        drawKeyArrow(c, 52, y + T.row / 2, 1, adjacentKeyframe(api, 1, r, space3d) != null);
      }
      icoAnimationDiamond(c, 85, y + T.row / 2, trackChannels(r).some(axis => axis.prop.kf.length > 0), trackChannels(r).some(axis => !!api.anim.hasKeyAt(r.L, axis.prop, api.transport.time())));
      clipText(c, r.label, labelX, y + T.row / 2, valueX - labelX - 12);
      /* value at playhead */
      const values = trackChannels(r).map(axis => api.anim.evP(L, axis.prop, api.transport.time(), axis.key));
      c.font = '400 10px ' + fmono();
      c.fillStyle = theme.accent;
      c.textAlign = 'right';
      const actionWidth = isScaleTrack(r) ? 24 : 0;
      const columns = propertyValueColumns(valueX, T.gut, values.length, actionWidth);
      values.forEach((value, index) => {
        const column = columns[index]!;
        const text = fittedPropertyValue(c, value, r.channels ? '%' : '', column.width);
        c.fillText(text, column.right, y + T.row / 2);
      });
      if (isScaleTrack(r)) icoScaleLink(c, T.gut - 12, y + T.row / 2, !!L.scaleLinked);
      c.textAlign = 'left';
      if (r.prop.expr) { c.fillStyle = theme.accent; c.fillText('ƒ', 70, y + T.row / 2); }
    }
  }
  /* The media-drop slot reads as an incoming layer card: wash, type swatch
     and the asset name, in the gap the rows below have opened up. */
  const d = T.drop;
  if (d && !T.graph) {
    const y = rawRowY(d.rowIdx);
    if (y + T.row >= T.ruler && y <= H) {
      const type = d.kind === 'audio' ? 'audio' : d.kind === 'video' ? 'video' : d.kind === 'image' ? 'image' : null;
      const pal = clipPalette({ type });
      const iy = y + T.row / 2;
      c.fillStyle = INK.over2; c.fillRect(0, y, T.gut, T.row);
      c.globalAlpha = .7;
      c.fillStyle = pal.body; roundRect(c, 74, iy - 8, 16, 16, 3); c.fill();
      c.setLineDash([3, 2]); c.strokeStyle = pal.primary; c.lineWidth = 1; c.stroke(); c.setLineDash([]);
      c.font = '600 8px ' + fui(); c.fillStyle = pal.primary; c.textAlign = 'center'; c.textBaseline = 'middle';
      c.fillText((type ? BADGE[type] : 'new').toUpperCase().slice(0, 1), 82, iy + .5);
      c.textAlign = 'left';
      c.font = '400 12px ' + fui(); c.fillStyle = theme.tx2;
      clipText(c, d.name, 96, iy, Math.max(0, T.gut - 12 - 96));
      c.globalAlpha = 1;
    }
  }
  c.restore();
}
const clippedLabels = new Map<string, string>();
listen(document.fonts, 'loadingdone', () => { clippedLabels.clear(); invalidate('timeline'); });
function clipText(c: any, s: any, x: any, y: any, max: any) {
  if (max <= 0) return;
  const key = JSON.stringify([c.font, c.letterSpacing, String(s), max]);
  const cached = clippedLabels.get(key);
  if (cached !== undefined) { c.fillText(cached, x, y); return; }
  let t = String(s);
  if (c.measureText(t).width > max) {
    if (c.measureText('…').width > max) return;
    while (t.length && c.measureText(t + '…').width > max) t = t.slice(0, -1);
    t += '…';
  }
  if (clippedLabels.size >= 1024) clippedLabels.clear();
  clippedLabels.set(key, t);
  c.fillText(t, x, y);
}
function fittedPropertyValue(c: any, value: any, unit: string, maxWidth: number) {
  if (typeof value === 'number' && Number.isFinite(value)) {
    const candidates = [
      `${api.util.round(value, 1)}${unit}`,
      `${api.util.round(value, 0)}${unit}`,
      ...(Math.abs(value) >= 1000 ? [`${api.util.round(value / 1000, 1)}k${unit}`] : []),
    ];
    return candidates.find(candidate => c.measureText(candidate).width <= maxWidth) ?? candidates.at(-1)!;
  }
  let text = String(value).slice(0, 20);
  if (c.measureText(text).width <= maxWidth) return text;
  while (text.length && c.measureText(text + '…').width > maxWidth) text = text.slice(0, -1);
  return text ? text + '…' : '…';
}
function icoAnimationDiamond(c: any, x: number, y: number, animated: boolean, current: boolean) {
  c.save(); c.strokeStyle = animated ? theme.accent : theme.tx3; c.fillStyle = theme.accent; c.lineWidth = 1.2;
  c.beginPath(); c.moveTo(x, y - 5); c.lineTo(x + 4, y); c.lineTo(x, y + 5); c.lineTo(x - 4, y); c.closePath();
  if (current) c.fill(); else c.stroke(); c.restore();
}
function icoScaleLink(c: any, x: number, y: number, linked: boolean) {
  c.save(); c.translate(x, y); c.rotate(-Math.PI / 4);
  c.strokeStyle = linked ? theme.accent : theme.tx3; c.lineWidth = 1.3;
  roundRect(c, -7, -4, 9, 5, 2.5); c.stroke();
  roundRect(c, -2, -1, 9, 5, 2.5); c.stroke();
  c.restore();
}
function drawKeyArrow(c: any, x: number, y: number, direction: number, enabled: boolean) {
  c.save(); c.strokeStyle = enabled ? theme.tx2 : theme.tx3; c.globalAlpha = enabled ? 1 : .3; c.lineWidth = 1.4;
  c.beginPath(); c.moveTo(x - direction * 2, y - 4); c.lineTo(x + direction * 2, y); c.lineTo(x - direction * 2, y + 4); c.stroke(); c.restore();
}
function navigateKeyframe(direction: -1 | 1, row?: any) {
  const time = adjacentKeyframe(api, direction, row, space3d);
  if (time != null) api.transport.setTime(time);
}
function clearTimelineSelection() {
  T.keySelectionActive = false;
  api.selection.set({ chan: null, keys: [], layers: [] });
  invalidate('timeline');
}
function icoEye(c: any, x: any, y: any, on: any) {
  c.strokeStyle = on ? INK.hi : INK.lo;
  c.lineWidth = 1.1; c.beginPath();
  c.ellipse(x, y, 5, 3.2, 0, 0, 7); c.stroke();
  if (on) { c.fillStyle = INK.hi; c.beginPath(); c.arc(x, y, 1.5, 0, 7); c.fill(); }
}
function icoLock(c: any, x: any, y: any, on: any) {
  c.strokeStyle = on ? theme.accent : INK.lo;
  c.lineWidth = 1.1;
  c.strokeRect(x - 3.4, y - 1, 6.8, 5);
  c.beginPath(); c.arc(x, y - 1, 2.4, Math.PI, 0); c.stroke();
}
function roundRect(c: any, x: any, y: any, w: any, hh: any, r: any) {
  r = Math.min(r, Math.abs(w) / 2, Math.abs(hh) / 2);
  c.beginPath();
  c.moveTo(x + r, y); c.arcTo(x + w, y, x + w, y + hh, r);
  c.arcTo(x + w, y + hh, x, y + hh, r); c.arcTo(x, y + hh, x, y, r);
  c.arcTo(x, y, x + w, y, r); c.closePath();
}

/* Playhead motion glow. A critically damped spring follows the measured
   playhead velocity (px/s) so the trail works for playback and scrubbing
   alike, and settles smoothly once the head stops. Closed-form update keeps
   it stable across uneven frame lengths. */
const TRAIL = { pxPerVelocity: .3, response: .15, maxWidth: 150, alpha: .22 };
const trail = { time: NaN, stamp: 0, width: 0, velocity: 0 };
function stepTrail(): number {
  const now = performance.now() / 1000;
  const dt = trail.stamp ? Math.min(now - trail.stamp, .25) : 0;
  const moved = Number.isFinite(trail.time) ? (api.transport.time() - trail.time) * T.pps : 0;
  trail.time = api.transport.time(); trail.stamp = now;
  const target = dt > 0 ? clamp(TRAIL.pxPerVelocity * moved / dt, -TRAIL.maxWidth, TRAIL.maxWidth) : 0;
  if (dt > 0) {
    const omega = 2 / TRAIL.response, decay = Math.exp(-omega * dt);
    const error = trail.width - target, b = trail.velocity + omega * error;
    trail.width = target + (error + b * dt) * decay;
    trail.velocity = (trail.velocity - omega * b * dt) * decay;
  }
  if (Math.abs(trail.width) < .5 && Math.abs(target) < .5) { trail.width = 0; trail.velocity = 0; }
  return trail.width;
}

function drawPlayhead(c: any, W: any, H: any) {
  const x = Math.round(t2x(api.transport.time())) + .5;
  const width = stepTrail();
  /* Keep animating until the trail has fully settled. */
  if (width !== 0) scheduleFrame(() => invalidate('timeline'));
  if (x < T.gut) return;
  c.save(); c.beginPath(); c.rect(T.gut, 0, W - T.gut, H); c.clip();
  /* glow trail behind the direction of travel */
  const tw = Math.abs(width);
  if (tw > .5) {
    const reverse = width < 0;
    const g = c.createLinearGradient(reverse ? x + tw : x - tw, 0, x, 0);
    g.addColorStop(0, rgba(theme.accent, 0)); g.addColorStop(1, rgba(theme.accent, TRAIL.alpha));
    c.fillStyle = g;
    c.fillRect(reverse ? x : x - tw, T.ruler, tw, H - T.ruler);
  }
  /* stem: a 3px panel-colored halo under the 1px accent core, so the line
     stays legible over clips of any color */
  const stemTop = WORK_BAR.height + 2;
  c.beginPath(); c.moveTo(x, stemTop); c.lineTo(x, H);
  c.strokeStyle = theme.panel; c.lineWidth = 3; c.stroke();
  c.strokeStyle = theme.accent; c.lineWidth = 1; c.stroke();
  /* knob: rounded teardrop, outlined in the panel color to lift off the ruler */
  const gw = 10, gh = 13, r = 2, gy = stemTop, gx = x - gw / 2;
  c.beginPath();
  c.moveTo(gx + r, gy);
  c.lineTo(gx + gw - r, gy); c.quadraticCurveTo(gx + gw, gy, gx + gw, gy + r);
  c.lineTo(gx + gw, gy + gh - 5.5);
  c.quadraticCurveTo(gx + gw, gy + gh - 4, gx + gw - 1.2, gy + gh - 3);
  c.lineTo(x + 1.4, gy + gh - .4); c.quadraticCurveTo(x, gy + gh + .6, x - 1.4, gy + gh - .4);
  c.lineTo(gx + 1.2, gy + gh - 3);
  c.quadraticCurveTo(gx, gy + gh - 4, gx, gy + gh - 5.5);
  c.lineTo(gx, gy + r); c.quadraticCurveTo(gx, gy, gx + r, gy);
  c.closePath();
  c.strokeStyle = theme.panel; c.lineWidth = 2; c.lineJoin = 'round'; c.stroke();
  c.fillStyle = theme.accent; c.fill();
  c.restore();
}

function formatOffsetTime(seconds: number): string {
  const sign = seconds < 0 ? '−' : '+';
  return sign + api.util.tc(Math.abs(seconds), api.project.get().fps);
}

function drawQuickOffset(c: any, W: number, H: number) {
  const offset = T.quickOffset;
  if (!offset) return;
  const width = 214, height = 45;
  const left = clamp(offset.x + 14, T.gut + 8, Math.max(T.gut + 8, W - width - 8));
  const top = clamp(offset.y + 14, T.ruler + 8, Math.max(T.ruler + 8, H - height - 8));
  c.save();
  c.fillStyle = theme.float;
  c.strokeStyle = theme.line;
  c.lineWidth = 1;
  roundRect(c, left, top, width, height, 7); c.fill(); c.stroke();
  c.textBaseline = 'middle';
  c.font = '400 10px ' + fui(); c.fillStyle = theme.tx3;
  c.fillText('TOTAL OFFSET', left + 10, top + 14);
  c.fillText('PER LAYER', left + 112, top + 14);
  c.font = '500 11px ' + fmono(); c.fillStyle = theme.tx;
  c.fillText(formatOffsetTime(offset.total), left + 10, top + 31);
  c.fillText(formatOffsetTime(offset.perGroup), left + 112, top + 31);
  c.restore();
}

/* ── graph editor ──────────────────────────────────────── */
const graphSamples = createGraphSampleCache((axis, time, speed) => graphSample(api, axis, time, speed));
function drawGraph(c: any, W: any, H: any) {
  graphSamples.begin(api.project.get(), [api.anim.version(), api.project.get().fps, T.pps, T.scrollT, W, T.graphType].join(':'));
  T._graph = null;
  /* Build from every project property, not only expanded timeline rows. A
     collapsed strip must not make the focused curve disappear. */
  const rows = api.project.get().layers.flatMap((L: any) => timelineProperties(api, L, space3d)
    .map((row: any) => ({ kind: 'prop', L, ...row })))
    .filter((r: any) => trackSelected(r, api.selection.chan() ?? '') || r.prop.kf.length);
  const target = resolveGraphTarget(rows, T.graphFocus, api.selection.layers(), (r: any) => trackSelected(r, api.selection.chan() ?? ''));
  if (target) T.graphFocus = { layerId: target.L.id, trackKey: target.key };
  c.save(); c.beginPath(); c.rect(T.gut, T.ruler, W - T.gut, H - T.ruler); c.clip();
  if (!target) {
    c.fillStyle = theme.tx3; c.font = '400 11.5px ' + fui(); c.textAlign = 'center';
    c.fillText('Select an animated property to edit its curve', (W + T.gut) / 2, (H + T.ruler) / 2);
    c.textAlign = 'left'; c.restore(); return;
  }
  const selectedKeyIds = new Set(T.graphMarqueeIds ?? api.selection.keys());
  const series = rows
    .flatMap((r:any)=>trackChannels(r).map((axis:any)=>({...axis,L:r.L,trackKey:r.key})))
    .filter((axis:any)=>typeof axis.prop.v==='number' && axis.prop.kf.some((key:any)=>selectedKeyIds.has(key.i)));
  if (!series.length) {
    c.fillStyle = theme.tx3; c.font = '400 11.5px ' + fui(); c.textAlign = 'center';
    c.fillText('Select keyframes in the timeline to edit their curves', (W + T.gut) / 2, (H + T.ruler) / 2);
    c.textAlign = 'left'; c.restore(); return;
  }
  const L = target.L, speedMode=T.graphType==='speed';
  const viewKey = api.project.get().id + ':' + T.graphType + ':' + series.map((axis: any) => axis.L.id + ':' + axis.key).join('|');
  if (T.graphViewKey !== viewKey) { T.graphViewKey = viewKey; T.graphViewBounds = null; }
  series.forEach((axis:any)=>temporalKeys(axis.prop.kf));
  let vmin = Infinity, vmax = -Infinity;
  series.forEach((axis:any)=> {
    const keys=axis.prop.kf;
    for(let i=0;i<keys.length;i++) for(let sample=0;sample<=48;sample++) {
      const t=axis.L.from+keys[i].t+((keys[i+1]?.t ?? keys[i].t)-keys[i].t)*sample/48;
      const v=graphSamples.sample(axis,t,speedMode); if(Number.isFinite(v)){vmin=Math.min(vmin,v);vmax=Math.max(vmax,v);}
    }
  });
  if (!isFinite(vmin)) { vmin = 0; vmax = 1; }
  if (vmax - vmin < 1e-6) { vmax = vmin + 1; }
  const padv = (vmax - vmin) * .22;
  vmin -= padv; vmax += padv;
  if (T.graphViewBounds) [vmin, vmax] = T.graphViewBounds;
  if (T.graphDragBounds) [vmin, vmax] = T.graphDragBounds;
  const top = T.ruler + 42, bot = Math.max(top + 1, H - 16);
  const v2y = (v: any) => bot - (v - vmin) / (vmax - vmin) * (bot - top);
  T._graph = { target, series, vmin, vmax, v2y, y2v: (y: any) => vmin + (bot - y) / (bot - top) * (vmax - vmin), points: [], selectionBounds: null };

  /* value gridlines */
  c.strokeStyle = INK.grid; c.font = '400 9.5px ' + fui(); c.fillStyle = theme.tx3;
  for (let i = 0; i <= 4; i++) {
    const v = vmin + (vmax - vmin) * i / 4, y = Math.round(v2y(v)) + .5;
    c.beginPath(); c.moveTo(T.gut, y); c.lineTo(W, y); c.stroke();
    c.fillText(api.util.round(v, 1), T.gut + 5, y - 4);
  }
  /* curve */
  series.forEach((axis: any, axisIndex: number) => {
  const L = axis.L;
  const kf = axis.prop.kf;
  const curveColor = [theme.accent,'#5495dc','#a276d4','#309886','#ba8541'][axisIndex%5];
  c.strokeStyle = curveColor; c.lineWidth = 1.8;
  c.beginPath();
  const x0 = Math.max(T.gut, t2x(L.from + Math.min(0, ...kf.map((key: any) => key.t))));
  const x1 = Math.min(W, t2x(L.from + Math.max(L.dur, ...kf.map((key: any) => key.t))));
  for (let x = x0; x <= x1; x += 1.5) {
    const tl = x2t(x) - L.from;
    const v = graphSamples.sample(axis,tl+L.from,speedMode);
    const y = v2y(v == null ? 0 : v);
    x === x0 ? c.moveTo(x, y) : c.lineTo(x, y);
  }
  c.stroke();
  /* handles + keys */
  kf.forEach((k: any, i: any) => {
    api.uiState.setKeyHandles(k, { ho: null, hi: null, pt: null });
    if (!selectedKeyIds.has(k.i)) return;
    const x = t2x(L.from + k.t), y = v2y(speedMode ? graphSamples.sample(axis,L.from+k.t,true) : k.v);
    const nx = kf[i + 1], pv = kf[i - 1];
    c.strokeStyle = INK.sub; c.lineWidth = 1;
    if (nx && !k.hold && !speedMode) {
      const handle = visibleBezierHandle(k, nx, 'eo');
      const [hx, hy] = pointForBezierHandle(handle, [x, y], [t2x(L.from + nx.t), v2y(nx.v)]);
      c.beginPath(); c.moveTo(x, y); c.lineTo(hx, hy); c.stroke();
      c.fillStyle = INK.handle; c.beginPath(); c.arc(hx, hy, 3, 0, 7); c.fill();
      api.uiState.setKeyHandles(k, { ho: [hx, hy] });
    }
    if (pv && !pv.hold && !speedMode) {
      const handle = visibleBezierHandle(k, pv, 'ei');
      const px = t2x(L.from + pv.t), py = v2y(pv.v);
      const [hx, hy] = pointForBezierHandle(handle, [px, py], [x, y]);
      c.beginPath(); c.moveTo(x, y); c.lineTo(hx, hy); c.stroke();
      c.fillStyle = INK.handle; c.beginPath(); c.arc(hx, hy, 3, 0, 7); c.fill();
      api.uiState.setKeyHandles(k, { hi: [hx, hy] });
    }
    const sel = api.selection.keys().includes(k.i);
    c.fillStyle = sel ? curveColor : INK.inv;
    c.beginPath(); c.arc(x, y, 4.2, 0, 7); c.fill();
    api.uiState.setKeyHandles(k, { pt: [x, y] });
  });
  });
  const graphPoints: any[] = series.flatMap((axis: any) => axis.prop.kf.map((key: any) => {
    const point = keyHandles(key)?.pt;
    return point ? { id: key.i, key, axis, x: point[0], y: point[1] } : null;
  })).filter(Boolean);
  const selectedIds = new Set(api.selection.keys());
  T._graph.points = graphPoints;
  T._graph.selectionBounds = graphSelectionBounds(graphPoints.filter(point => selectedIds.has(point.id)));
  if (T._graph.selectionBounds) {
    const bounds = T._graph.selectionBounds;
    c.save(); c.strokeStyle = rgba(theme.accent, .4); c.lineWidth = 1;
    c.setLineDash([3, 3]);
    c.strokeRect(bounds.x0 + .5, bounds.y0 + .5, bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
    c.restore();
  }
  // Keep the legend in its own strip, clear of curves and value ticks.
  c.fillStyle = theme.panel; c.fillRect(T.gut, T.ruler, W - T.gut, 28);
  c.font = '500 10px ' + fui();
  let legendX = T.gut + 12;
  for (const [index, axis] of series.entries()) {
    const label = axis.label === 'fontAxis.wght' ? 'Weight' : axis.label;
    const unit = api.model.CH[axis.key]?.unit;
    const text = label + (unit ? ` (${unit}${speedMode ? '/s' : ''})` : speedMode ? ' /s' : '');
    const width = c.measureText(text).width + 25;
    if (legendX + width > W - 8) { c.fillStyle = theme.tx3; c.fillText('…', legendX, T.ruler + 17); break; }
    c.fillStyle = [theme.accent,'#5495dc','#a276d4','#309886','#ba8541'][index % 5];
    c.fillRect(legendX, T.ruler + 11, 10, 2);
    c.fillStyle = theme.tx2; c.fillText(text, legendX + 15, T.ruler + 17);
    legendX += width;
  }
  c.restore();
}

/* ── interaction ───────────────────────────────────────── */
function hitRow(y: any) {
  if (y < T.ruler) return null;
  const i = Math.floor((y - T.ruler + T.scrollY) / T.row);
  return T.rows[i] ? { row: T.rows[i], i } : null;
}

function bind(cv: any, wrap: any) {
  listen(cv, 'pointerdown', onDown, undefined, canvasCleanups);
  listen(cv, 'pointermove', onMove, undefined, canvasCleanups);
  listen(cv, 'pointerleave', () => setHoverRow(null), undefined, canvasCleanups);
  listen(cv, 'dblclick', onDbl, undefined, canvasCleanups);
  listen(cv, 'contextmenu', onCtx, undefined, canvasCleanups);
  /* FX browser drops. Non-fx drags (OS files) fall through to the window
     import handler untouched. */
  const setDropRow = (index: number | null) => {
    if (T.dropRow === index) return;
    T.dropRow = index;
    invalidate('timeline');
  };
  const dropLayerAt = (y: number) => {
    const hr = hitRow(y);
    return hr?.row?.L ? { layer: hr.row.L, index: hr.i } : null;
  };
  /* Media drops (asset cards, OS files): a new layer is always its own strip,
     so the pointer resolves to a boundary between layer rows, never onto one.
     The upper half of a layer row inserts above it, the lower half (and any
     of its property rows) inserts below its block. Rows below the last layer
     append to the bottom of the stack. `rowIdx` is the slot in T.rows where
     the gap opens; `index` is the matching position in project.layers. */
  const mediaDropAt = (e: any) => {
    const p = api.project.get();
    const at = Math.max(0, api.util.snapF(x2t(Math.max(e.offsetX, T.gut)), p.fps));
    const rows = T.rows;
    const raw = (Math.max(e.offsetY, T.ruler) - T.ruler + T.scrollY) / T.row;
    let slot = Math.floor(raw);
    const open = T.drop ? T.drop.rowIdx : null;
    /* the gap already open shifts rows below it; keep the slot while the
       pointer stays inside it, otherwise map back to the unshifted row */
    if (open != null && slot === open) return { at, rowIdx: open, index: T.drop.index };
    if (open != null && slot > open) slot--;
    let rowIdx = rows.length;
    if (slot < rows.length) {
      const fraction = raw - Math.floor(raw);
      let i = slot;
      while (i > 0 && rows[i].kind !== 'layer') i--;
      const before = rows[slot].kind === 'layer' && fraction < .5;
      if (before) rowIdx = i;
      else {
        /* below the whole block: property rows and, for groups, children */
        const depth = rows[i].depth || 0;
        rowIdx = i + 1;
        while (rowIdx < rows.length && (rows[rowIdx].kind !== 'layer' || (rows[rowIdx].depth || 0) > depth)) rowIdx++;
      }
    }
    const next = rows[rowIdx]?.L;
    const index = next ? p.layers.indexOf(next) : p.layers.length;
    return { at, rowIdx, index };
  };
  const setMediaDrop = (next: any) => {
    const prev = T.drop;
    if (!next && !prev) return;
    if (next && prev && next.at === prev.at && next.rowIdx === prev.rowIdx && next.name === prev.name) return;
    T.drop = next;
    invalidate('timeline');
  };
  listen(cv, 'dragover', (e: any) => {
    const dt = e.dataTransfer;
    if (api.dnd.hasFxDrag(dt)) {
      e.preventDefault(); dt.dropEffect = 'copy';
      const hit = e.offsetX > T.gut ? dropLayerAt(e.offsetY) : null;
      setDropRow(hit ? hit.index : null);
      return;
    }
    if (!api.dnd.hasMediaDrag(dt)) return;
    e.preventDefault(); e.stopPropagation(); dt.dropEffect = 'copy';
    /* Payload data is unreadable during dragover; the drag source parks a
       description on api so the ghost can carry its name, kind and length. */
    const src = api.dnd.mediaDrag || { name: 'Media', kind: 'file', dur: undefined };
    setMediaDrop({ ...mediaDropAt(e), name: src.name, kind: src.kind, dur: src.dur });
  }, undefined, canvasCleanups);
  listen(cv, 'dragleave', () => { setDropRow(null); setMediaDrop(null); }, undefined, canvasCleanups);
  listen(cv, 'drop', (e: any) => {
    const dt = e.dataTransfer;
    const fx = api.dnd.readFxDrag(dt);
    /* resolve while the slot is still open so the drop lands where the ghost showed */
    const placement = mediaDropAt(e);
    setDropRow(null); setMediaDrop(null);
    if (fx) {
      e.preventDefault(); e.stopPropagation();
      const hit = dropLayerAt(e.offsetY);
      const L = hit?.layer || api.selection.first();
      let edge: 'in' | 'out' | undefined;
      if (L && fx.kind === 'transition') {
        const mid = t2x(L.from + L.dur / 2);
        edge = e.offsetX < mid ? 'in' : 'out';
      }
      api.dnd.applyFxDrop(fx, L?.id, edge);
      return;
    }
    const asset = api.dnd.readAssetDrag(dt);
    const files = api.dnd.hasFileDrag(dt) ? Array.from(dt.files || []) as File[] : [];
    if (!asset && !files.length) return;
    e.preventDefault(); e.stopPropagation();
    const { at, index } = placement;
    if (asset) {
      const command = api.media.commandForAsset(asset.id, at);
      if (!command || command.type !== 'add_layer') return;
      command.index = index;
      const result = api.edit.apply(command, { label: 'Add ' + asset.name, origin: 'command' });
      if (result?.ok === false) api.ui.toast(result.message || 'Could not add ' + asset.name);
    } else {
      api.media.importFiles(files, { placement: { at, index } });
    }
  }, undefined, canvasCleanups);
  listen(cv, 'wheel', (e: any) => {
    e.preventDefault();
    if (e.ctrlKey || e.metaKey) {
      const tAt = x2t(e.offsetX);
      T.pps = clamp(T.pps * (1 - e.deltaY * .004), 4, 4000);
      T.scrollT = tAt - (e.offsetX - T.gut) / T.pps;
    } else if (e.shiftKey) {
      T.scrollT += e.deltaY / T.pps;
    } else {
      if (T.graph && e.offsetX >= T.gut && T._graph && !T.graphDragBounds) {
        const g = T._graph;
        const pixels = e.deltaY * (e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? T.hgt : 1);
        const delta = g.y2v(pixels) - g.y2v(0);
        T.graphViewBounds = [g.vmin + delta, g.vmax + delta];
      } else if (!T.graph || e.offsetX < T.gut) T.scrollY += e.deltaY;
      T.scrollT += e.deltaX / T.pps;
    }
    T.scrollT = Math.max(-.4, T.scrollT);
    invalidate('timeline');
  }, { passive: false }, canvasCleanups);
}

function setHoverRow(index: number | null) {
  if (T.hoverRow === index) return;
  T.hoverRow = index;
  invalidate('timeline');
}
function onMove(e: any) {
  const x = e.offsetX, y = e.offsetY;
  setHoverRow(hitRow(y)?.i ?? null);
  if (T.graph && x > T.gut && y >= T.ruler) {
    const graph = T._graph;
    const keys = graph?.series?.flatMap((axis: any) => axis.prop.kf) ?? [];
    const nearHandle = keys.some((key: any) => {
      const handles = keyHandles(key);
      return [handles?.ho, handles?.hi].some((point: any) => point && Math.hypot(x - point[0], y - point[1]) < 7);
    });
    const nearPoint = keys.some((key: any) => {
      const point = keyHandles(key)?.pt;
      return point && Math.hypot(x - point[0], y - point[1]) < 8;
    });
    T.cv.style.cursor = nearHandle ? 'crosshair'
      : pointInGraphSelection(graph?.selectionBounds, x, y) ? 'move'
      : nearPoint ? 'pointer' : 'crosshair';
    return;
  }
  let cur = 'default';
  let title = '';
  if (x > T.gut) {
    const workHit = workAreaHit(x, y);
    if (workHit) cur = workHit.kind === 'handle' ? 'ew-resize' : 'grab';
    else if (y < T.ruler) cur = 'ew-resize';
    const hr = hitRow(y);
    if (!workHit && hr && hr.row.kind === 'layer') {
      const L = hr.row.L;
      const x0 = t2x(L.from), x1 = t2x(L.from + L.dur);
      if (Math.abs(x - x0) < 5 || Math.abs(x - x1) < 5) cur = 'ew-resize';
      else if (x > x0 && x < x1) cur = 'grab';
    } else if (!workHit && hr && hr.row.kind === 'prop') {
      const nearKey = hr.row.prop.kf.some((key: any) => Math.abs(t2x(hr.row.L.from + key.t) - x) < 6);
      cur = nearKey ? 'pointer' : 'crosshair';
    }
  }
  if (x < T.gut && y >= T.ruler) {
    const row = hitRow(y)?.row;
    if (row?.kind === 'layer' && layerSupportsTransform(row.L.type) && x >= T.gut - 40) {
      cur = x < T.gut - 20 ? 'crosshair' : 'pointer';
      const parent = row.L.parent ? api.model.layer(row.L.parent)?.name || 'Missing layer' : 'None';
      title = x < T.gut - 20 ? `Parent: ${parent} · Drag to a layer` : `Parent: ${parent} · Choose parent`;
    } else if (row?.kind === 'prop' && row.prop.kf.length && x >= 16 && x < 64) {
      cur = adjacentKeyframe(api, x < 40 ? -1 : 1, row, space3d) != null ? 'pointer' : 'default';
    } else if (row?.kind === 'prop' && !row.L.lock) {
      if (isScaleTrack(row) && x >= T.gut - 24) {
        cur = 'pointer';
        title = row.L.scaleLinked ? 'Adjust Scale X and Y separately' : 'Link Scale X and Y';
      } else if (x >= 76 && x < 96) cur = 'pointer';
      else if (x >= T.propertyValueX - 4) cur = trackChannels(row).every(axis => typeof api.anim.evP(row.L, axis.prop, api.transport.time(), axis.key) === 'number') ? 'ew-resize' : 'pointer';
    }
  }
  T.cv.title = title;
  T.cv.style.cursor = cur;
}

function workAreaHit(x: any, y: any) {
  const wa = api.project.get().work || [0, api.project.get().dur];
  const x0 = t2x(wa[0]), x1 = t2x(wa[1]);
  const d0 = Math.abs(x - x0), d1 = Math.abs(x - x1);
  /* Handles get a little vertical grace; the bar itself only owns its thin
     strip so the ruler below it stays a scrub surface. */
  if (y <= WORK_BAR.height + 5 && Math.min(d0, d1) <= WORK_BAR.hit) return { kind: 'handle', idx: d0 <= d1 ? 0 : 1 };
  if (y <= WORK_BAR.height + 1 && x > x0 && x < x1) return { kind: 'bar' };
  return null;
}

function releaseExternalFieldFocus() {
  const active = document.activeElement;
  if (!(active instanceof HTMLElement)) return;
  if (!active.matches('input, textarea, select') && !active.isContentEditable) return;
  active.blur();
}

function onDown(e: any) {
  if (e.button === 2) return;
  if (e.button === 1) { releaseExternalFieldFocus(); return panViewport(e); }
  if (e.button === 0) releaseExternalFieldFocus();
  const x = e.offsetX, y = e.offsetY;
  api.ui.closeMenus();
  if (y < T.ruler && x > T.gut) {
    const hit = workAreaHit(x, y);
    if (hit?.kind === 'handle') return workAreaDrag(e, hit.idx);
    if (hit?.kind === 'bar') return workAreaMove(e);
    return scrub(e);
  }
  if (x < T.gut) return gutterDown(e, x, y);
  if (T.graph) return graphDown(e, x, y);
  const hr = hitRow(y);
  if (!hr) return marquee(e, { additive: e.shiftKey || e.metaKey });
  const r = hr.row;
  if (r.kind === 'prop') return keyDown(e, r, x, y, hr.i);
  const L = r.L;
  const span = L.type === 'group' ? api.groups.span(L) : L;
  const x0 = t2x(span.from), x1 = t2x(span.from + span.dur);
  const onClip = x >= x0 - 5 && x <= x1 + 5;
  if (!onClip) return marquee(e, { additive: e.shiftKey || e.metaKey });
  if (quickOffsetModifiers(e) && api.selection.layers().includes(L.id) && quickOffsetLayerGroups().length > 1) {
    if (L.lock) return;
    return quickOffsetLayers(e);
  }
  if (!selectLayerForPointer(L, e) || L.lock) return;
  if (Math.abs(x - x0) < 5) return trim(e, 'in');
  if (Math.abs(x - x1) < 5) return trim(e, 'out');
  if (x > x0 && x < x1) return slide(e);
}

function panViewport(e: PointerEvent) {
  e.preventDefault();
  const start = T.scrollT;
  beginDrag(e, {
    cursor: 'grabbing',
    move: (dx: number) => {
      T.scrollT = Math.max(-.4, start - dx / T.pps);
      invalidate('timeline');
    },
  });
}

let layerSelectionAnchor: string | null = null;
function quickOffsetModifiers(event: any) {
  return !!event?.altKey && !!(event?.metaKey || event?.ctrlKey);
}

function quickOffsetLayerGroups() {
  const selected = new Set(api.selection.layers());
  return api.selection.layers().map((id: string) => api.model.layer(id)).filter((layer: any) => layer
    && !api.groups.ancestors(layer).some((group: any) => selected.has(group.id)));
}

function quickOffsetLayers(e: any) {
  const roots = quickOffsetLayerGroups();
  if (roots.length < 2) return;
  const groups = roots.map((root: any) => {
    const ids = root.type === 'group' ? api.groups.expand([root.id]) : [root.id];
    return ids.map((id: string) => api.model.layer(id)).filter(Boolean);
  });
  if (groups.some((members: any[]) => members.some((layer: any) => layer.lock
      || api.groups.ancestors(layer).some((group: any) => group.lock)))) return;
  const timing = groups.flatMap((members: any[], offsetIndex: number) => members.map((L: any) => ({
    L, time: Number(L.from) || 0, offsetIndex, offsetCount: groups.length, minTime: 0,
  })));
  api.edit.begin('Quick offset layers', { origin: 'timeline' });
  let moved = false;
  const clear = () => { T.quickOffset = null; invalidate('timeline'); };
  beginDrag(e, {
    cursor: 'ew-resize',
    move: (dx: number, _dy: number, event: PointerEvent) => {
      if (!moved && Math.abs(dx) < 3) return;
      moved = true;
      const plan = planQuickOffsetTiming(timing, dx / T.pps);
      timing.forEach((item: any, index: number) => api.edit.dispatch({
        type: 'set_layer', target: item.L.id, patch: { from: api.util.round(plan.times[index] ?? item.time, 6) },
      }));
      const rect = T.cv.getBoundingClientRect();
      T.quickOffset = {
        total: plan.total, perGroup: plan.perGroup,
        x: event.clientX - rect.left, y: event.clientY - rect.top,
      };
      invalidate('timeline');
    },
    up: () => { clear(); moved ? api.edit.commit('Quick offset layers') : api.edit.cancel(); },
    cancel: () => { clear(); api.edit.cancel(); },
  });
}

function selectLayerForPointer(L: any, event: PointerEvent) {
  T.keySelectionActive = false;
  api.selection.set({ keys: [] });
  const selected = api.selection.layers().includes(L.id);
  const toggle = event.metaKey || event.ctrlKey;
  if (event.shiftKey) {
    const ids = buildRows().filter((row: any) => row.kind === 'layer').map((row: any) => row.L.id);
    // Keep the original anchor while the user extends or shrinks the range.
    if (!layerSelectionAnchor || !ids.includes(layerSelectionAnchor) || !api.selection.layers().includes(layerSelectionAnchor)) {
      layerSelectionAnchor = api.selection.layers().find((id: string) => ids.includes(id)) ?? L.id;
    }
    const from = ids.indexOf(layerSelectionAnchor), to = ids.indexOf(L.id);
    const range = ids.slice(Math.min(from, to), Math.max(from, to) + 1);
    api.selection.select(toggle ? [...new Set([...api.selection.layers(), ...range])] : range);
    return true;
  }
  layerSelectionAnchor = L.id;
  if (!toggle) {
    if (!selected) api.selection.select(L.id);
    return true;
  }
  api.selection.select(selected ? api.selection.layers().filter((id: any) => id !== L.id) : [...api.selection.layers(), L.id]);
  return !selected;
}

function workAreaDrag(e: any, idx: any) {
  const startWork = [...(api.project.get().work || [0, api.project.get().dur])];
  const startDuration = api.project.get().dur;
  const frame = 1 / Math.max(1, api.project.get().fps);
  api.edit.begin('Work area', { origin: 'timeline' });
  beginDrag(e, {
    cursor: 'ew-resize',
    move: (dx: any, dy: any, ev: any) => {
      const r = T.cv.getBoundingClientRect();
      const t = Math.max(0, api.util.snapF(x2t(ev.clientX - r.left), api.project.get().fps));
      const patch = WorkArea.resize(startWork, idx, t, Math.max(startDuration, api.project.get().dur), frame);
      api.edit.dispatch({ type: 'set_composition', patch });
    },
    up: () => api.edit.commit('Work area'),
    cancel: () => api.edit.cancel(),
  });
}

function workAreaMove(e: any) {
  const startWork = [...(api.project.get().work || [0, api.project.get().dur])];
  const duration = api.project.get().dur;
  const frame = 1 / Math.max(1, api.project.get().fps);
  api.edit.begin('Move work area', { origin: 'timeline' });
  let moved = false;
  beginDrag(e, {
    cursor: 'grabbing',
    move: (dx: any) => {
      if (!moved && Math.abs(dx) < 2) return;
      moved = true;
      const delta = api.util.snapF(dx / T.pps, api.project.get().fps);
      api.edit.dispatch({ type: 'set_composition', patch: WorkArea.move(startWork, delta, duration, frame) });
    },
    up: () => moved ? api.edit.commit('Move work area') : api.edit.cancel(),
    cancel: () => api.edit.cancel(),
  });
}

function scrub(e: any) {
  // Most drags do not snap. Enumerate the project's property/keyframe graph
  // only if Shift is actually pressed, including midway through a drag.
  let targets: number[] | undefined;
  let rawTime = 0;
  let lockedTarget: number | null = null;
  const apply = () => {
    const time = clamp(rawTime, 0, api.project.get().dur);
    if (!shiftSnapping()) { lockedTarget = null; api.transport.setTime(time); return; }
    if (!targets) {
      targets = [];
      for (const L of api.project.get().layers) {
        targets.push(L.from, L.from + L.dur);
        for (const item of api.anim.allProps(L) || []) {
          for (const key of item.prop?.kf || []) targets.push(L.from + key.t);
        }
      }
  }
    const tolerance = 10 / Math.max(1, T.pps);
    const resolved = resolveTimelineSnap(time, targets, tolerance, lockedTarget, 15 / Math.max(1, T.pps));
    lockedTarget = resolved.target;
    api.transport.setTime(resolved.time);
  };
  const set = (ev: any) => {
    const r = T.cv.getBoundingClientRect();
    rawTime = x2t(ev.clientX - r.left);
    if (ev?.shiftKey) updateShiftHeld(true);
    apply();
  };
  const refresh = () => apply();
  const cleanup = () => { if (refreshActiveScrub === refresh) refreshActiveScrub = null; };
  refreshActiveScrub = refresh;
  set(e);
  beginDrag(e, {
    move: (dx: any, dy: any, ev: any) => set(ev),
    up: cleanup,
    cancel: cleanup,
  });
}

function togglePropertyAnimation(row: any) {
  const axes = trackChannels(row);
  const remove = axes.every(axis => api.anim.hasKeyAt(row.L, axis.prop, api.transport.time()));
  api.history.do(remove ? 'Remove keyframe' : 'Add keyframe', () => {
    for (const axis of axes) {
      const at = api.anim.hasKeyAt(row.L, axis.prop, api.transport.time());
      if (remove && at) api.anim.removeKey(axis.prop, at);
      else if (!at) {
        const value = api.anim.evP(row.L, axis.prop, api.transport.time(), axis.key);
        if (value != null) api.anim.setKeyOn(axis.prop, api.transport.time() - row.L.from, value, 'linear', api.project.get().fps);
      }
    }
  });
  T.reveal(row.L, axes.map(axis => axis.key));
  rowsDirty = true; invalidate();
}

function isScaleTrack(row: any) {
  return row?.key === 'scale' && trackChannels(row).length === 2;
}

function dragPropertyValue(event: any, row: any, rowIndex: number) {
  let axes = trackChannels(row).map(axis => ({ ...axis, value: api.anim.evP(row.L, axis.prop, api.transport.time(), axis.key), meta: propertyMetadata(api, row.L, axis.key) }));
  if (!axes.every(axis => typeof axis.value === 'number')) {
    editPropertyValue(row, rowIndex); return;
  }
  if (row.channels && !row.L.scaleLinked) {
    const index = Math.min(axes.length - 1, Math.max(0, Math.floor((event.offsetX - T.propertyValueX) / ((T.gut - T.propertyValueX - 8) / axes.length))));
    axes = [axes[index]];
  }
  const time = api.transport.time();
  let editing = false;
  let control: any;
  const cancelOnEscape = (event: KeyboardEvent) => {
    if (event.key !== 'Escape') return;
    event.preventDefault(); event.stopImmediatePropagation(); control?.cancel();
  };
  const cleanup = () => window.removeEventListener('keydown', cancelOnEscape, true);
  window.addEventListener('keydown', cancelOnEscape, true);
  control = beginDrag(event, {
    infinite: true,
    cursor: 'ew-resize',
    move: (dx: number, _dy: number, ev: any) => {
      if (!editing && Math.abs(dx) < 3) return;
      if (!editing) { api.edit.begin(`Adjust ${row.label}`, { origin: 'timeline' }); editing = true; }
      axes.forEach((axis, index) => {
        const delta = draggedPropertyValue(axes[0].value, dx, axes[0].meta, !!ev.altKey, !!ev.shiftKey) - axes[0].value;
        const value = row.channels && row.L.scaleLinked && index > 0
          ? axis.value + delta * (axes[0].value ? axis.value / axes[0].value : 1)
          : draggedPropertyValue(axis.value, dx, axis.meta, !!ev.altKey, !!ev.shiftKey);
        api.edit.dispatch({ type: 'set_property', target: row.L.id, path: axis.key, value, time, mode: 'auto', preserveHandEdits: false });
      });
      invalidate();
    },
    up: () => { cleanup(); if (editing) api.edit.commit(`Adjust ${row.label}`); else editPropertyValue(row, rowIndex); },
    cancel: () => { cleanup(); if (editing) api.edit.cancel(); }
  });
}

function editPropertyValue(row: any, rowIndex: number) {
  if (row.L.lock) return;
  const axes = trackChannels(row);
  const values = axes.map(axis => api.anim.evP(row.L, axis.prop, api.transport.time(), axis.key));
  const time = api.transport.time();
  const choices: Record<string, readonly string[]> = {
    'l.blend': api.model.BLENDS, 'c.align': ['left', 'center', 'right'], 'c.shape': ['rect', 'ellipse', 'polygon', 'star', 'line'],
    'c.fit': ['cover', 'contain', 'stretch']
  };
  const options = typeof values[0] === 'boolean' ? ['true', 'false'] : choices[row.key]
    ?? (/^m\..+\.shape$/.test(row.key) ? ['rect', 'ellipse'] : /^m\..+\.mode$/.test(row.key) ? ['add', 'subtract'] : null);
  const input = h(options ? 'select' : 'input', { value: values.join(', '), 'aria-label': row.label,
    style: { position: 'absolute', left: `${T.propertyValueX}px`, top: `${rowY(rowIndex) + 3}px`,
      width: `${T.gut - T.propertyValueX - 6}px`, height: `${T.row - 6}px`, background: 'var(--bg-row)',
      border: '1px solid var(--accent)', color: 'var(--tx)', padding: '0 4px', zIndex: 9 } });
  if (options) { options.forEach(value => input.appendChild(h('option', { value }, value))); input.value = String(values[0]); }
  else if (typeof values[0] === 'string' && /^#[0-9a-f]{6}$/i.test(values[0])) input.type = 'color';
  document.querySelector<HTMLElement>('#tl-canvas-wrap')?.appendChild(input); input.focus(); input.select?.();
  let closed = false;
  const finish = (save: boolean) => {
    if (closed) return;
    closed = true;
    if (save) {
      const parts = axes.length > 1 ? input.value.split(',').map((value: string) => value.trim()) : [input.value];
      const next = values.map((value, index) => typeof value === 'number' ? Number(parts[index] ?? parts[0]) : typeof value === 'boolean' ? parts[index] === 'true' : parts[index]);
      if (next.every(value => typeof value !== 'number' || Number.isFinite(value))) api.edit.apply(axes.map((axis, index) => ({
        type: 'set_property', target: row.L.id, path: axis.key, value: next[index], time, mode: 'auto', preserveHandEdits: false
      })), { label: `Edit ${row.label}`, origin: 'timeline' });
    }
    input.remove(); invalidate();
  };
  if (options) input.onchange = () => finish(true);
  input.onblur = () => finish(true);
  input.onkeydown = (event: KeyboardEvent) => { event.stopPropagation(); if (event.key === 'Enter' || event.key === 'Escape') { event.preventDefault(); finish(event.key === 'Enter'); } };
  canvasCleanups.push(() => { input.onblur = null; input.remove(); });
}

function gutterDown(e: any, x: any, y: any) {
  const hr = hitRow(y);
  if (!hr) { if (!e.shiftKey && !e.metaKey) clearTimelineSelection(); return; }
  const r = hr.row;
  if (r.kind !== 'layer') {
    api.selection.set({ chan: r.key });
    T.focusGraph(r.L, r.key);
    api.selection.select(r.L.id); T.keySelectionActive = true; invalidate('timeline');
    if (r.prop.kf.length && x >= 16 && x < 64) { navigateKeyframe(x < 40 ? -1 : 1, r); return; }
    if (r.L.lock) return;
    if (isScaleTrack(r) && x >= T.gut - 24) { toggleTimelineScaleLink(api, r.L); return; }
    if (x >= 76 && x < 96) { togglePropertyAnimation(r); return; }
    if (x >= T.propertyValueX - 4) dragPropertyValue(e, r, hr.i);
    return;
  }
  T.keySelectionActive = false;
  api.selection.set({ keys: [] });
  const L = r.L;
  if (layerSupportsTransform(L.type) && x >= T.gut - 40) {
    const ids = api.selection.layers().includes(L.id) ? api.selection.layers() : [L.id];
    if (x < T.gut - 20) api.ui.beginParentPick(e, ids);
    else api.ui.showParentMenu(ids, e);
    return;
  }
  if (x < 22) { }
  else if (x < 40) { api.edit.apply({ type: 'set_layer', target: L.id, patch: { visible: !evaluatedValue(L, L.on, api.transport.time(), 'l.on') } }, { label: 'Toggle visibility', origin: 'timeline' }); return; }
  else if (x < 58) { api.edit.apply({ type: 'set_layer', target: L.id, patch: { locked: !L.lock } }, { label: 'Toggle lock', origin: 'timeline' }); return; }
  else if (x >= 58 + Math.min(48, (r.depth || 0) * 12) && x < 74 + Math.min(48, (r.depth || 0) * 12)) {
    const collapsed = toggleTimelineDisclosure(api, L);
    rowsDirty = true;
    if (!collapsed) {
      buildRows();
      let kids = visibleProps(L).length;
      if (L.type === 'group') {
        kids = 0;
        for (let index = hr.i + 1; index < T.rows.length; index++) {
          const child = T.rows[index];
          if (child.kind === 'layer' && (child.depth || 0) <= (r.depth || 0)) break;
          kids++;
        }
      }
      keepRowsVisible(hr.i, kids);
    }
    invalidate('timeline');
    return;
  }
  if (!selectLayerForPointer(L, e)) return;
  const selected = api.selection.layers().slice();
  const moving = api.groups.expand(selected) || selected;
  if (moving.some((id: string) => {
    const layer = api.model.layer(id);
    return !layer || layer.lock || api.groups.ancestors(layer).some((group) => group.lock);
  })) return;
  let active = false;
  const clear = () => { T.reorder = null; invalidate('timeline'); };
  beginDrag(e, {
    cursor: 'grabbing',
    move: (dx: number, dy: number, ev: PointerEvent) => {
      if (!active && Math.hypot(dx, dy) < 5) return;
      active = true;
      const rect = T.cv.getBoundingClientRect(), py = ev.clientY - rect.top;
      const drop = layerDrop(T.rows, (py - T.ruler + T.scrollY) / T.row, ev.clientX - rect.left);
      const target = drop && api.model.layer(drop.target);
      T.reorder = drop && target && !moving.includes(drop.target) && !target.lock && !api.groups.ancestors(target).some((g: any) => g.lock) ? drop : null;
      invalidate('timeline');
    },
    up: () => {
      const drop = T.reorder; clear();
      if (!drop) return;
      const roots = api.project.get().layers.filter((layer: any) => selected.includes(layer.id) && !api.groups.ancestors(layer).some((g: any) => selected.includes(g.id)));
      api.edit.mutate('Move layers', () => {
        const changing = roots.filter((layer: any) => (layer.group || null) !== drop.group);
        if (changing.length) api.groups.moveToGroup(changing.map((layer: any) => layer.id), drop.group);
        const block = api.project.get().layers.filter((layer: any) => moving.includes(layer.id));
        const remaining = api.project.get().layers.filter((layer: any) => !moving.includes(layer.id));
        let index = remaining.findIndex((layer: any) => layer.id === drop.target);
        if (drop.mode !== 'before') {
          index++;
          if (drop.mode === 'after') while (index < remaining.length && remaining[index] && api.groups.ancestors(remaining[index]!).some((g: any) => g.id === drop.target)) index++;
        }
        remaining.splice(index, 0, ...block); api.project.get().layers = remaining;
        api.groups.normalizeStack(); api.anim.touch();
      }, { origin: 'timeline' });
      invalidate();
    },
    cancel: clear,
  });
}

function slide(e: any) {
  const selectedIds = api.groups.expand(api.selection.layers()) || api.selection.layers();
  if (selectedLayers(api).some((layer: any) => layer.type === 'group') && api.project.get().layers.some((layer: any) => selectedIds.includes(layer.id) && layer.lock)) return;
  const layers = api.project.get().layers.filter((l: any) => (api.groups.expand(api.selection.layers()) || api.selection.layers()).includes(l.id) && l.type !== 'group' && !l.lock && !(api.groups.ancestors(l) || []).some((g: any) => g.lock));
  const groupIds = api.groups.expand(api.selection.layers()) || api.selection.layers();
  const groups = api.project.get().layers.filter((L: any) => groupIds.includes(L.id) && L.type === 'group' && !L.lock && !(api.groups.ancestors(L) || []).some((g: any) => g.lock));
  const groupStart = groups.map((L: any) => ({L, from:L.from}));
  const start = layers.map((L: any) => ({ L, from: L.from }));
  api.edit.begin('Move clip', { origin: 'timeline' });
  let moved = false;
  beginDrag(e, {
    cursor: 'grabbing',
    move: (dx: any, dy: any, ev: any) => {
      moved = true;
      let dt = dx / T.pps;
      if (shiftSnapping(ev)) dt = snapDelta(start, dt);
      dt = Math.max(dt, -Math.min(...start.map((item: any) => item.from)));
      groupStart.forEach((s: any) => api.edit.dispatch({type:'set_layer',target:s.L.id,patch:{from:api.util.snapF(s.from+dt,api.project.get().fps)}}));
      start.forEach((s: any) => api.edit.dispatch({ type: 'set_layer', target: s.L.id, patch: { from: Math.max(0, api.util.snapF(s.from + dt, api.project.get().fps)) } }));
    },
    up: () => { moved ? api.edit.commit('Move clip') : api.edit.cancel(); },
    cancel: () => api.edit.cancel(),
  });
}
function snapDelta(start: any, dt: any, side?: 'in' | 'out') {
  const pts = [api.transport.time(), 0, api.project.get().dur, ...(api.project.get().work || []), ...api.project.get().markers.map((m: any) => m.t)];
  api.project.get().layers.forEach((L: any) => { if (!start.some((s: any) => s.L === L)) { pts.push(L.from, L.from + L.dur); } });
  const tol = 8 / T.pps;
  let best = dt, bd = tol;
  for (const s of start) for (const edge of (side === 'in' ? [s.from + dt] : side === 'out' ? [s.from + s.dur + dt] : [s.from + dt, s.from + s.L.dur + dt])) {
    for (const p of pts) { const d = Math.abs(edge - p); if (d < bd) { bd = d; best = dt + (p - edge); } }
  }
  return best;
}

function trim(e: any, side: any) {
  const layers = api.project.get().layers.filter((l: any) => (api.groups.expand(api.selection.layers()) || api.selection.layers()).includes(l.id) && l.type !== 'group' && !l.lock && !(api.groups.ancestors(l) || []).some((g: any) => g.lock));
  const start = layers.map((L: any) => ({ L, from: L.from, dur: L.dur, trim: Number(L.d && L.d.trim) || 0 }));
  api.edit.begin('Trim clip', { origin: 'timeline' });
  let moved = false;
  beginDrag(e, {
    cursor: 'ew-resize',
    move: (dx: any, _dy: any, ev: any) => {
      moved = true;
      let dt = dx / T.pps;
      if (shiftSnapping(ev)) dt = snapDelta(start, dt, side);
      dt = api.util.snapF(dt, api.project.get().fps);
      start.forEach((s: any) => {
        if (side === 'in') {
          let nf = clamp(s.from + dt, 0, s.from + s.dur - 1 / api.project.get().fps);
          if (api.media.timing.isTimed(s.L)) nf = Math.max(nf, api.media.timing.earliestStart({ ...s.L, from: s.from, d: { ...s.L.d, trim: s.trim } }));
          api.edit.dispatch({ type: 'set_layer', target: s.L.id, patch: { from: nf, duration: s.dur + (s.from - nf) } });
          if (api.media.timing.isTimed(s.L)) {
            const rate = api.media.timing.rate(s.L);
            api.edit.dispatch({ type: 'set_content', target: s.L.id, patch: { trim: Math.max(0, s.trim + (nf - s.from) * rate) } });
          }
        } else api.edit.dispatch({ type: 'set_layer', target: s.L.id, patch: { duration: Math.max(1 / api.project.get().fps, s.dur + dt) } });
      });
    },
    up: () => { moved ? api.edit.commit('Trim clip') : api.edit.cancel(); },
    cancel: () => api.edit.cancel(),
  });
}

function captureKeyframeGesture(entries: any[]) {
  const selected = new Set(entries.map((entry: any) => entry.key));
  const entryLayerIds = new Set(entries.map((entry: any) => entry.L.id));
  const layerOrder = [
    ...api.selection.layers().filter((id: string) => entryLayerIds.has(id)),
    ...api.project.get().layers.map((layer: any) => layer.id).filter((id: string) => entryLayerIds.has(id)),
  ].filter((id: string, index: number, ids: string[]) => ids.indexOf(id) === index);
  const offsetIndex = new Map(layerOrder.map((id: string, index: number) => [id, index]));
  const layerByProperty = new Map(entries.map((entry: any) => [entry.prop, entry.L]));
  const props = [...new Set(entries.map((entry: any) => entry.prop))];
  const properties = props.map((prop: any) => ({
    prop,
    keys: prop.kf.map((key: any, order: number) => ({ key, time: key.t, value: key.v, order })),
  }));
  const items = properties.flatMap(({ prop, keys }: any) => keys.map((entry: any) => ({
    id: entry.key.i,
    property: prop,
    time: entry.time,
    compositionTime: entry.time + (Number(layerByProperty.get(prop)?.from) || 0),
    minTime: Math.min(
      entry.time,
      -(Number(layerByProperty.get(prop)?.from) || 0),
    ),
    /* Key times are layer-local. Existing files can contain keys beyond the
       current composition; let those move left, but never teleport them left
       merely because a drag began or allow the group to move farther right. */
    maxTime: Math.max(
      entry.time,
      Math.max(0, api.project.get().dur - (Number(layerByProperty.get(prop)?.from) || 0)),
    ),
    selected: selected.has(entry.key),
    order: entry.order,
    offsetIndex: offsetIndex.get(layerByProperty.get(prop)?.id) ?? 0,
    offsetCount: layerOrder.length,
    key: entry.key,
    value: entry.value,
  })));
  return { properties, items };
}

function keyframeSnapTargets(snapshot: any) {
  const selected = new Set(snapshot.items.filter((item: any) => item.selected).map((item: any) => item.key));
  const targets: number[] = [api.transport.time(), 0, api.project.get().dur, ...(api.project.get().work || [])];
  targets.push(...(api.project.get().markers || []).map((marker: any) => marker.t));
  api.project.get().layers.forEach((L: any) => {
    targets.push(L.from, L.from + L.dur);
    api.anim.allProps(L).forEach(({ prop }: any) => prop.kf.forEach((key: any) => {
      if (!selected.has(key)) targets.push(L.from + key.t);
    }));
  });
  return [...new Set(targets.filter(Number.isFinite).map((time: number) => api.util.round(time, 6)))];
}

function snapKeyframeGesture(snapshot: any, requestedDelta: number, lock: KeyframeGroupSnapLock | null) {
  return resolveKeyframeGroupSnap(
    snapshot.items.filter((item: any) => item.selected).map((item: any) => item.compositionTime),
    requestedDelta,
    keyframeSnapTargets(snapshot),
    8 / T.pps,
    lock,
    12 / T.pps,
  );
}

function restoreKeyframeGesture(snapshot: any) {
  snapshot.properties.forEach(({ prop, keys }: any) => {
    keys.forEach(({ key, time, value }: any) => { key.t = time; key.v = value; });
    prop.kf = keys.map(({ key }: any) => key);
  });
}

/** Rebuild from the pointer-down snapshot on every move. This is what makes a
    key removed by a temporary collision come back when the pointer moves away. */
function applyKeyframeGesture(
  snapshot: any, requestedDelta: number,
  valueForItem: ((item: any) => number | null) | null = null,
  planner: (items: any[], delta: number, fps: number) => any = planKeyframeMove,
) {
  restoreKeyframeGesture(snapshot);
  const plan = planner(snapshot.items, requestedDelta, api.project.get().fps);
  const removed = new Set(plan.removed.map((item: any) => item.key));
  plan.moves.forEach(({ item, time }: any) => { item.key.t = time; });
  if (valueForItem) {
    snapshot.items.forEach((item: any) => {
      if (!item.selected || typeof item.value !== 'number') return;
      const value = valueForItem(item);
      if (value != null && Number.isFinite(value)) item.key.v = api.util.round(value, 3);
    });
  }
  snapshot.properties.forEach(({ prop, keys }: any) => {
    const order = new Map(keys.map(({ key, order }: any) => [key, order]));
    prop.kf = keys.map(({ key }: any) => key)
      .filter((key: any) => !removed.has(key))
      .sort((a: any, b: any) => a.t - b.t || (order.get(a) as number) - (order.get(b) as number));
  });
  api.anim.touch(); invalidate();
  return plan;
}

function keyDown(e: any, r: any, x: any, y: any, rowIdx: any) {
  const hit = pickKeyframeHit(
    r.prop.kf, api.selection.keys(),
    (k: any) => Math.abs(t2x(r.L.from + k.t) - x), 6,
  );
  const quickModifier = quickOffsetModifiers(e);
  const additive = !quickModifier && (e.shiftKey || e.metaKey);
  if (!hit) return marquee(e, { additive });
  api.selection.set({ chan: r.key });
  T.focusGraph(r.L, r.key);
  const wasSelected = keySelected(hit);
  if (additive) api.selection.select(r.L.id, true);
  else if (!api.selection.layers().includes(r.L.id)) api.selection.select(r.L.id);
  const baseSelection = [...api.selection.keys()];
  const hitIds = uniqueKeyIds([hit]);
  setSelectedKeys(selectionAfterKeyGesture(baseSelection, hitIds, additive, true));
  const entries = selectedKeyEntries();
  const snapshot = captureKeyframeGesture(entries);
  const quickOffset = quickModifier && new Set(snapshot.items
    .filter((item: any) => item.selected).map((item: any) => item.offsetIndex)).size > 1;
  let snapLock: KeyframeGroupSnapLock | null = null;
  let moved = false;
  const clearQuickOffset = () => { T.quickOffset = null; };
  beginDrag(e, {
    move: (dx: any, dy: any, event: PointerEvent) => {
      if (!moved && Math.hypot(dx, dy) < 3) return;
      if (!moved) { moved = true; api.history.begin(quickOffset ? 'Quick offset keyframes' : 'Move keyframe'); }
      let delta = dx / T.pps;
      if (!quickOffset && shiftSnapping(event)) {
        const snap = snapKeyframeGesture(snapshot, delta, snapLock);
        delta = snap.delta; snapLock = snap.lock;
      } else snapLock = null;
      const plan = applyKeyframeGesture(snapshot, delta, null, quickOffset ? planQuickOffsetKeyframes : planKeyframeMove);
      if (quickOffset) {
        const count = Math.max(0, ...snapshot.items.map((item: any) => item.offsetCount || 0));
        const rect = T.cv.getBoundingClientRect();
        T.quickOffset = {
          total: plan.delta, perGroup: count > 1 ? plan.delta / (count - 1) : 0,
          x: event.clientX - rect.left, y: event.clientY - rect.top,
        };
      }
    },
    up: () => {
      clearQuickOffset();
      if (moved) api.history.commit(quickOffset ? 'Quick offset keyframes' : 'Move keyframe');
      else if (additive && wasSelected) setSelectedKeys(selectionAfterKeyGesture(baseSelection, hitIds, true, false));
      invalidate('timeline');
    },
    cancel: () => {
      clearQuickOffset();
      if (moved) { restoreKeyframeGesture(snapshot); api.history.cancel(); api.anim.touch(); }
      invalidate('timeline');
    },
  });
}

function graphDown(e: any, x: any, y: any) {
  const g = T._graph; if (!g) return;
  if (y < T.ruler + 28) return;
  let L = g.target.L;
  const series = g.series as any[];
  // Pick the closest visible point/handle, not whichever axis was iterated first.
  const points = series.flatMap((axis: any) => axis.prop.kf.map((key: any) => ({ axis, key })));
  const pointHits = points.map(item => {
    const pt = keyHandles(item.key)?.pt;
    return { ...item, i: item.key.i, distance: pt ? Math.hypot(x - pt[0], y - pt[1]) : Infinity };
  }).sort((a, b) => a.distance - b.distance);
  const closestDistance = pointHits[0]?.distance ?? Infinity;
  const pointHit = pickKeyframeHit(pointHits.filter(item => item.distance <= closestDistance + 1), api.selection.keys(), item => item.distance, 8) ?? pointHits[0];
  if (!pointHit || pointHit.distance >= 6) {
    const handles = points.flatMap(item => (['eo', 'ei'] as const).map(which => {
      const pt = keyHandles(item.key)?.[which === 'eo' ? 'ho' : 'hi'];
      return { ...item, which, distance: pt ? Math.hypot(x - pt[0], y - pt[1]) : Infinity };
    })).sort((a, b) => a.distance - b.distance);
    const nearest = handles[0]?.distance ?? Infinity;
    const handle = handles.find(item => item.distance <= nearest + 1 && api.selection.keys().includes(item.key.i)) ?? handles[0];
    if (handle && handle.distance < 7) return dragHandle(e, handle.key, handle.which, g, handle.axis.prop.kf, handle.axis.L, handle.axis);
  }
  if ((!pointHit || pointHit.distance >= 8) && pointInGraphSelection(g.selectionBounds, x, y)) {
    return dragGraphSelection(e, g, L);
  }
  if (!pointHit || pointHit.distance >= 8) return marquee(e, { additive: e.shiftKey || e.metaKey, graph: true });
  L = pointHit.axis.L;
  const hit = pointHit.key;
  const additive = e.shiftKey || e.metaKey;
  const wasSelected = keySelected(hit);
  if (additive) api.selection.select(L.id, true);
  else if (!api.selection.layers().includes(L.id)) api.selection.select(L.id);
  const baseSelection = [...api.selection.keys()];
  const hitIds = uniqueKeyIds([hit]);
  setSelectedKeys(selectionAfterKeyGesture(baseSelection, hitIds, additive, true));
  return dragGraphSelection(e, g, L, () => {
    if (additive && wasSelected) setSelectedKeys(selectionAfterKeyGesture(baseSelection, hitIds, true, false));
  });
}

function dragGraphSelection(e: any, g: any, L: any, click?: () => void) {
  const visibleProperties = new Set(g.series.map((axis: any) => axis.prop));
  const entries = selectedKeyEntries().filter((entry: any) => !entry.L.lock && visibleProperties.has(entry.prop));
  if (!entries.length) return;
  const snapshot = captureKeyframeGesture(entries);
  const speeds = entries.map((e:any)=>({key:e.key,inSpeed:e.key.inEase?.speed ?? 0,outSpeed:e.key.outEase?.speed ?? 0}));
  const canvasBounds = T.cv.getBoundingClientRect();
  const anchorY = e.clientY - canvasBounds.top;
  T.graphDragBounds = [g.vmin, g.vmax];
  const timeScale = T.pps;
  let snapLock: KeyframeGroupSnapLock | null = null;
  let moved = false;
  beginDrag(e, {
    cursor: 'move',
    move: (rawDx: any, rawDy: any, event: PointerEvent) => {
      let dx = rawDx, dy = rawDy;
      if (event?.shiftKey) {
        if (Math.abs(dx) >= Math.abs(dy)) dy = 0;
        else dx = 0;
      }
      if (!moved && Math.hypot(dx, dy) < 3) return;
      if (!moved) { moved = true; api.history.begin(entries.length > 1 ? 'Edit keyframes' : 'Edit curve'); }
      const dv = g.y2v(anchorY + dy) - g.y2v(anchorY);
      let delta = dx / timeScale;
      if (shiftSnapping(event) && dx !== 0) {
        const snap = snapKeyframeGesture(snapshot, delta, snapLock);
        delta = snap.delta; snapLock = snap.lock;
      } else snapLock = null;
      applyKeyframeGesture(snapshot, delta, (item: any) => item.value + (T.graphType==='speed'?0:dv), planGraphKeyframeMove);
      if(T.graphType==='speed'){ for(const item of speeds){ item.key.inInterp=item.key.outInterp='bezier'; item.key.inEase.speed=item.inSpeed+dv; item.key.outEase.speed=item.outSpeed+dv; } api.anim.touch(); invalidate(); }
    },
    up: () => {
      T.graphDragBounds = null;
      if (moved) api.history.commit('Edit curve');
      else click?.();
      invalidate();
    },
    cancel: () => {
      if (moved) { restoreKeyframeGesture(snapshot); api.history.cancel(); api.anim.touch(); }
      T.graphDragBounds = null;
      invalidate();
    },
  });
}
function dragHandle(e: any, k: any, which: 'eo' | 'ei', g: any, kf: any, L: any, clickedAxis: any) {
  if (L.lock) return;
  const trackKey = clickedAxis?.trackKey ?? clickedAxis?.key;
  const axes = g.series.filter((axis: any) => (axis.trackKey ?? axis.key) === trackKey && !axis.L.lock);
  const candidates = axes.flatMap((axis: any) => axis.prop.kf.map((key: any) => ({ axis, key })));
  const clickedIndex = kf.indexOf(k);
  const explicitlySelected = new Set(api.selection.keys());
  const owningKeys = candidates.filter((item: any) => item.axis.L === L).map((item: any) => item.key);
  const targetKeys = new Set([
    ...keysForBezierHandleDrag(owningKeys, k, api.selection.keys()),
    ...axes.filter((axis: any) => axis.L !== L).map((axis: any) => axis.prop.kf[clickedIndex])
      .filter((key: any) => key && keyMembers(key).some(member => explicitlySelected.has(member.key.i))),
  ]);
  const memberAxes = candidates.filter((item: any) => targetKeys.has(item.key));
  const clickedPrevious = which === 'eo' ? k : kf[clickedIndex - 1];
  const clickedNext = which === 'eo' ? kf[clickedIndex + 1] : k;
  if (!clickedPrevious || !clickedNext) return;
  const clickedStart: [number, number] = [t2x(L.from + clickedPrevious.t), g.v2y(clickedPrevious.v)];
  const clickedEnd: [number, number] = [t2x(L.from + clickedNext.t), g.v2y(clickedNext.v)];
  const start = visibleBezierHandle(k, which === 'eo' ? clickedNext : clickedPrevious, which);
  const originals = axes.flatMap((axis: any) => axis.prop.kf.map((key: any) => ({
    key, state: JSON.parse(JSON.stringify(key)),
  })));
  const split = Boolean(e.altKey || e.getModifierState?.('Alt'));
  const drags = memberAxes.map(({ axis, key }: any) => {
    const keys = axis.prop.kf;
    const index = keys.indexOf(key);
    const previous = which === 'eo' ? key : keys[index - 1];
    const next = which === 'eo' ? keys[index + 1] : key;
    if (!previous || !next) return null;
    const segmentStart: [number, number] = [t2x(axis.L.from + previous.t), g.v2y(previous.v)];
    const segmentEnd: [number, number] = [t2x(axis.L.from + next.t), g.v2y(next.v)];
    const oppositeWhich: 'eo' | 'ei' = which === 'eo' ? 'ei' : 'eo';
    const oppositeNeighbor = which === 'eo' ? keys[index - 1] : keys[index + 1];
    let opposite: any = null;
    if (oppositeNeighbor && key.bezierMode !== 'split' && !split) {
      const oppositeStart = which === 'eo'
        ? [t2x(axis.L.from + oppositeNeighbor.t), g.v2y(oppositeNeighbor.v)] as [number, number]
        : [t2x(axis.L.from + key.t), g.v2y(key.v)] as [number, number];
      const oppositeEnd = which === 'eo'
        ? [t2x(axis.L.from + key.t), g.v2y(key.v)] as [number, number]
        : [t2x(axis.L.from + oppositeNeighbor.t), g.v2y(oppositeNeighbor.v)] as [number, number];
      const oppositeHandle = visibleBezierHandle(key, oppositeNeighbor, oppositeWhich);
      opposite = { which: oppositeWhich, segmentStart: oppositeStart, segmentEnd: oppositeEnd,
        previous: which === 'eo' ? oppositeNeighbor : key,
        next: which === 'eo' ? key : oppositeNeighbor,
        point: pointForBezierHandle(oppositeHandle, oppositeStart, oppositeEnd), fallback: oppositeHandle };
    }
    return { axis, key, previous, next, segmentStart, segmentEnd, opposite };
  }).filter(Boolean);
  api.history.begin('Adjust easing');
  T.graphDragBounds = [g.vmin, g.vmax];
  let moved = false, prepared = false;
  beginDrag(e, {
    move: (dx: any, dy: any, event: PointerEvent) => {
      if (!moved && Math.hypot(dx, dy) < 3) return;
      moved = true;
      if (!prepared) {
        prepared = true;
        drags.forEach((drag: any) => {
          materializeLinearBezierSegment(drag.previous, drag.next);
          if (drag.opposite) {
            materializeLinearBezierSegment(drag.opposite.previous, drag.opposite.next);
          }
          if (split) drag.key.bezierMode = 'split';
        });
      }
      const handle = moveBezierHandle(start, [dx, dy], clickedStart, clickedEnd, shiftSnapping(event) ? which : undefined);
      drags.forEach((drag: any) => {
        drag.key[which] = handle.map((value: number) => api.util.round(value, 4));
        if (!drag.opposite) return;
        const keyPoint: [number, number] = [t2x(drag.axis.L.from + drag.key.t), g.v2y(drag.key.v)];
        const draggedPoint = pointForBezierHandle(handle, drag.segmentStart, drag.segmentEnd);
        const mirroredPoint = mirroredBezierHandlePoint(keyPoint, draggedPoint, drag.opposite.point);
        const oppositeHandle = bezierHandleAtPoint(
          mirroredPoint, drag.opposite.segmentStart, drag.opposite.segmentEnd, drag.opposite.fallback,
        );
        oppositeHandle[0] = Math.max(0, Math.min(1, oppositeHandle[0]));
        drag.key[drag.opposite.which] = oppositeHandle.map((value: number) => api.util.round(value, 4));
      });
      api.anim.touch(); invalidate();
    },
    up: () => { T.graphDragBounds = null; moved ? api.history.commit('Adjust easing') : api.history.cancel(); invalidate(); },
    cancel: () => {
      originals.forEach(({ key, state }: any) => {
        for (const field of Object.keys(key)) if (!(field in state)) delete key[field];
        Object.assign(key, state);
      });
      T.graphDragBounds = null; api.history.cancel(); api.anim.touch(); invalidate();
    },
  });
}

function keySelected(key: any) { return keyMembers(key).some(member => api.selection.keys().includes(member.key.i)); }
function uniqueKeyIds(keys: any) {
  return expandScaleKeyIds(api, keys);
}
function setSelectedKeys(keys: any, layerIds?: any) {
  const selection: { keys: string[]; layers?: string[] } = { keys: uniqueKeyIds(keys) };
  T.keySelectionActive = true;
  if (layerIds) selection.layers = [...new Set(layerIds)] as string[];
  api.selection.set(selection);
  invalidate();
}
function selectedKeyEntries() {
  const selected = new Set(api.selection.resolveSelectedKeys());
  const entries: any[] = [];
  api.project.get().layers.forEach((L: any) => api.anim.allProps(L).forEach(({ prop }: any) => prop.kf.forEach((key: any) => {
    if (selected.has(key)) {temporalKeys(prop.kf);entries.push({ key, prop, L });}
  })));
  return entries;
}
function keysInMarquee(m: any, graph: any = false) {
  const picked: any[] = [];
  if (graph) {
    const target = T._graph && T._graph.target;
    (T._graph?.series?.flatMap((axis:any) => axis.prop.kf) ?? []).forEach((key: any) => {
      const pt = keyHandles(key)?.pt;
      if (pt && pt[0] >= m.x0 && pt[0] <= m.x1 && pt[1] >= m.y0 && pt[1] <= m.y1) picked.push(key);
    });
    return picked;
  }
  T.rows.forEach((row: any, i: any) => {
    if (row.kind !== 'prop') return;
    const cy = rowY(i) + T.row / 2;
    if (cy < m.y0 || cy > m.y1) return;
    row.prop.kf.forEach((key: any) => {
      const x = t2x(row.L.from + key.t);
      if (x >= m.x0 && x <= m.x1) picked.push(key);
    });
  });
  return picked;
}
function layersForKeys(keys: any) {
  const ids = new Set(uniqueKeyIds(keys)), layers: any[] = [];
  api.project.get().layers.forEach((L: any) => {
    if (api.anim.allProps(L).some(({ prop }: any) => prop.kf.some((key: any) => ids.has(key.i)))) layers.push(L.id);
  });
  return layers;
}

function marquee(e: any, opt: any = {}) {
  const r = T.cv.getBoundingClientRect();
  const x0 = e.clientX - r.left, y0 = e.clientY - r.top;
  const additive = !!opt.additive;
  const baseKeys = additive ? [...api.selection.keys()] : [];
  const baseLayers = additive ? [...api.selection.layers()] : [];
  const originalKeys = [...api.selection.keys()], originalLayers = [...api.selection.layers()];
  const graphPoints = opt.graph ? [...(T._graph?.points ?? [])] : null;
  if (graphPoints) T.graphMarqueeIds = originalKeys;
  let graphPickedIds: string[] = [];
  let dragged = false;
  beginDrag(e, {
    move: (dx: any, dy: any) => {
      if (!dragged && Math.hypot(dx, dy) < 3) return;
      dragged = true;
      T.marquee = { x0: Math.min(x0, x0 + dx), y0: Math.min(y0, y0 + dy), x1: Math.max(x0, x0 + dx), y1: Math.max(y0, y0 + dy) };
      const picked = graphPoints
        ? graphPoints.filter((point: any) => point.x >= T.marquee.x0 && point.x <= T.marquee.x1 && point.y >= T.marquee.y0 && point.y <= T.marquee.y1).map((point: any) => point.key)
        : keysInMarquee(T.marquee);
      const pickedIds = uniqueKeyIds(picked);
      graphPickedIds = pickedIds;
      // Keep the graph selection visible until the box is complete.
      const nextKeys = graphPoints && !additive
        ? [...new Set([...originalKeys, ...pickedIds])]
        : selectionAfterMarquee(baseKeys, pickedIds, additive);
      setSelectedKeys(nextKeys, [...baseLayers, ...layersForKeys(nextKeys)]);
      invalidate('timeline');
    },
    up: () => {
      if (!dragged) {
        // Empty track space selects; only the ruler scrubs the playhead.
        if (!additive) clearTimelineSelection();
      } else if (graphPoints && !additive) {
        const nextKeys = graphPickedIds.length ? graphPickedIds : originalKeys;
        setSelectedKeys(nextKeys, layersForKeys(nextKeys));
      } else if (T.marquee && !api.selection.keys().length) {
        const m = T.marquee;
        const picked: any[] = [];
        T.rows.forEach((row: any, i: any) => {
          const y = rowY(i);
          if (y + T.row < m.y0 || y > m.y1) return;
          if (row.kind === 'layer') {
            const a = t2x(row.L.from), b = t2x(row.L.from + row.L.dur);
            if (b > m.x0 && a < m.x1) picked.push(row.L.id);
          }
        });
        api.selection.select([...baseLayers, ...picked]);
      }
      T.graphMarqueeIds = null; T.marquee = null; invalidate('timeline');
    },
    cancel: () => {
      if (graphPoints) { T.graphMarqueeIds = null; setSelectedKeys(originalKeys, originalLayers); }
      T.marquee = null; invalidate('timeline');
    },
  });
}

function onDbl(e: any) {
  const x = e.offsetX, y = e.offsetY;
  const workHit: any = x > T.gut && workAreaHit(x, y);
  if (workHit?.kind === 'bar') {
    api.edit.apply({ type: 'set_composition', patch: { workArea: [0, api.project.get().dur] } }, { label: 'Reset work area', origin: 'timeline' });
    return;
  }
  if (x < T.gut && y > T.ruler) {
    const hr = hitRow(y);
    if (hr?.row.kind === 'prop' && x >= T.propertyValueX - 4) { editPropertyValue(hr.row, hr.i); return; }
    if (hr && hr.row.kind === 'layer' && x > 90) renameLayer(hr.row.L, hr.i);
  }
}
function renameLayer(L: any, rowIdx: any) {
  const wrap = document.querySelector<HTMLElement>('#tl-canvas-wrap');
  const inp = h('input', {
    value: L.name,
    style: {
      position: 'absolute', left: '94px', top: (rowY(rowIdx) + 5) + 'px', width: (T.gut - 110) + 'px',
      height: '20px', background: '#000', border: '1px solid var(--accent)', borderRadius: '4px',
      color: 'var(--tx)', fontSize: '11.5px', padding: '0 5px', zIndex: 9,
    },
  });
  if (!wrap) return;
  wrap.appendChild(inp); inp.focus(); inp.select();
  const disposeInput = () => { inp.onblur = null; inp.onkeydown = null; inp.remove(); };
  canvasCleanups.push(disposeInput);
  const done = (ok: any) => {
    if (ok && inp.value.trim()) api.edit.apply({ type: 'set_layer', target: L.id, patch: { name: inp.value.trim() } }, { label: 'Rename layer', origin: 'timeline' });
    disposeInput(); invalidate();
  };
  inp.onblur = () => done(true);
  inp.onkeydown = (ev: any) => { ev.stopPropagation(); if (ev.key === 'Enter') done(true); if (ev.key === 'Escape') done(false); };
}

function pushKeyframeMenu(items: any[], clickedEntries: any[]) {
  const entries = keyframeContextEntries(clickedEntries, api.selection.keys(), selectedKeyEntries());
  entries.forEach((entry:any)=>temporalKeys(entry.prop.kf));
  const keys = entries.map((entry: any) => entry.key);
  const multiple = keys.length > 1;
  items.push({ header: multiple ? `${keys.length} keyframes` : 'Keyframe' });
  items.push({label:'Keyframe Velocity…',run:()=>velocityDialog(api,entries)}, {label:'Scale keyframes…',run:()=>scaleGraphDialog(api,selectedKeyEntries())});
  const labels: Record<string, string> = {
    linear: 'Linear', power: 'Power', easeOut: 'Ease out', easeInOut: 'Ease in / out',
    expoOut: 'Exponential out', backOut: 'Overshoot', snap: 'Snap', glide: 'Glide',
  };
  ['linear', 'power', 'easeOut', 'easeInOut', 'expoOut', 'backOut', 'snap', 'glide'].forEach((name: any) =>
    items.push({
      label: labels[name], curve: api.ease.PRESETS[name],
      on: keys.every((target: any) => !target.hold && api.ease.nameOf(target.eo, target.ei) === name),
      run: () => api.history.do(multiple ? 'Ease keyframes' : 'Ease', () => api.anim.applyEaseTo(keys, name)),
    }));
  const allHold = keys.every((target: any) => target.hold);
  items.push({
    label: allHold ? 'Remove hold' : multiple ? 'Hold keyframes' : 'Toggle hold',
    run: () => api.history.do(multiple ? 'Hold keyframes' : 'Hold', () => {
      keys.forEach((target: any) => { target.hold = !allHold; });
      api.anim.touch();
    }),
  });
  items.push('-', {
    label: multiple ? `Delete ${keys.length} keyframes` : 'Delete keyframe',
    run: () => api.history.do(multiple ? 'Delete keyframes' : 'Delete keyframe', () => {
      const values = new Map<any, any>();
      for (const layer of api.project.get().layers) for (const { key, prop } of api.anim.allProps(layer)) {
        if (entries.some((entry: any) => entry.prop === prop)) values.set(prop, api.anim.evP(layer, prop, api.transport.time(), key));
      }
      entries.forEach((entry: any) => api.anim.removeKey(entry.prop, entry.key));
      for (const [prop, value] of values) if (!prop.kf.length) prop.v = value;
    }),
  });
}

T.layerAtPoint = (clientX: number, clientY: number) => {
  const rect = T.cv?.getBoundingClientRect();
  if (!rect || clientX < rect.left || clientX > rect.right || clientY < rect.top + T.ruler || clientY > rect.bottom) return null;
  return hitRow(clientY - rect.top)?.row?.L || null;
};

function onCtx(e: any) {
  e.preventDefault();
  const x = e.offsetX, y = e.offsetY;
  const hr = hitRow(y);
  const items: any[] = [];
  const graphPoint = T.graph && x > T.gut
    ? [...(T._graph?.points || [])]
      .map((point: any) => ({ ...point, distance: Math.hypot(x - point.x, y - point.y) }))
      .sort((a: any, b: any) => a.distance - b.distance)[0]
    : null;
  if (graphPoint?.distance < 8) {
    pushKeyframeMenu(items, [{ key: graphPoint.key, prop: graphPoint.axis.prop }]);
  } else if (!T.graph && hr && hr.row.kind === 'prop') {
    const r = hr.row;
    const key = r.prop.kf.find((k: any) => Math.abs(t2x(r.L.from + k.t) - x) < 7);
    if (key) {
      const members = key.members ?? [{ key, prop: r.prop }];
      pushKeyframeMenu(items, members);
    } else {
      items.push({ label: 'Add keyframe here', icon: 'diamond', run: () => api.history.do('Add keyframe', () => {
        const values = trackChannels(r).map(axis => ({ ...axis, value: api.anim.evP(r.L, axis.prop, x2t(x), axis.key) }));
        values.forEach(axis => api.anim.setKeyOn(axis.prop, x2t(x) - r.L.from, axis.value, 'linear', api.project.get().fps));
      }) });
      items.push({ label: 'Clear all keyframes', icon: 'x', disabled: !r.prop.kf.length, run: () => api.history.do('Clear keys', () => { trackChannels(r).forEach(axis => { axis.prop.v = api.anim.evP(r.L, axis.prop, api.transport.time(), axis.key); axis.prop.kf = []; }); api.anim.touch(); }) });
    }
  } else if (!T.graph && hr && hr.row.kind === 'layer') {
    api.ui.showLayerMenu(hr.row.L, e, 'timeline');
    return;
  } else if (!T.graph || y < T.ruler) {
    items.push({ label: 'Set work area start', icon: 'frame', run: () => api.edit.apply({ type: 'set_composition', patch: { workArea: [Math.min(api.transport.time(), api.project.get().work[1] - 1 / api.project.get().fps), api.project.get().work[1]] } }, { label: 'Work area', origin: 'timeline' }) },
      { label: 'Set work area end', icon: 'frame', run: () => api.edit.apply({ type: 'set_composition', patch: { workArea: [api.project.get().work[0], Math.max(api.transport.time(), api.project.get().work[0] + 1 / api.project.get().fps)] } }, { label: 'Work area', origin: 'timeline' }) },
      { label: 'Reset work area', icon: 'undo', run: () => api.edit.apply({ type: 'set_composition', patch: { workArea: [0, api.project.get().dur] } }, { label: 'Work area', origin: 'timeline' }) });
  }
  /* Extension contributions land at the end, so the positions a user has
     learned for the built-in rows never move. `layer:context` only fires over a
     layer row; `timeline:context` fires for every row kind. */
  const rowKind = graphPoint?.distance < 8 ? 'keyframe' : T.graph ? 'graph' : hr ? hr.row.kind : 'empty';
  const layerId = graphPoint?.distance < 8
    ? T._graph?.target?.L?.id ?? null
    : hr && hr.row.kind === 'layer' ? hr.row.L.id : null;
  const contributed = [
    ...(layerId ? api.menus.collect('layer:context', { layerId }) : []),
    ...api.menus.collect('timeline:context', { kind: rowKind, layerId, time: x2t(x) }),
  ];
  if (contributed.length) items.push('-' as any, ...contributed);
  if (items.length) api.ui.menu({ x: e.clientX, y: e.clientY }, items);
}

/* ── edge navigation ───────────────────────────────────── */
function edges() {
  const e = new Set([0, api.project.get().dur]);
  api.project.get().layers.forEach((L: any) => {
    e.add(api.util.round(L.from, 4)); e.add(api.util.round(L.from + L.dur, 4));
    api.anim.allProps(L).forEach((p: any) => p.prop.kf.forEach((k: any) => e.add(api.util.round(L.from + k.t, 4))));
  });
  return [...e].sort((a: any, b: any) => a - b);
}
function nextEdge() { const e = edges(); return e.find((t: any) => t > api.transport.time() + 1e-4) ?? api.project.get().dur; }
function prevEdge() { const e = edges(); return [...e].reverse().find((t: any) => t < api.transport.time() - 1e-4) ?? 0; }
T.nextEdge = nextEdge; T.prevEdge = prevEdge;
T.frameView = () => { T.scrollT = 0; T.pps = clamp((T.w - T.gut - 40) / Math.max(.5, api.project.get().dur), 4, 4000); invalidate('timeline'); };
T.reveal = (L: any, keys: any) => {
  api.uiState.setLayerCollapsed(L, false);
  const current = api.uiState.getReveal(L);
  const alreadyVisible = current == null
    ? timelineProperties(api, L, space3d).flatMap((row: any) => trackChannels(row)
      .filter((axis: any) => axis.prop.kf.length || axis.prop.expr)
      .map((axis: any) => axis.key))
    : current;
  api.uiState.setReveal(L, alreadyVisible.includes('*') ? alreadyVisible : [...new Set([...alreadyVisible, ...keys])]);
  rowsDirty = true;
  buildRows();
  const idx = T.rows.findIndex((r: any) => r.kind === 'layer' && r.L === L);
  if (idx >= 0) {
    const kids = visibleProps(L).length;
    keepRowsVisible(idx, kids);
  }
  invalidate('timeline');
};
/* Direct module HMR can replace this runtime without rebuilding the panel.
   Rebind the already-mounted hosts immediately; kernel reloads rebuild the
   panel afterward and call these same idempotent attach methods. */
const liveHead = previousHead && previousHead.isConnected !== false ? previousHead : document.querySelector('#tl-head');
const liveWrap = previousWrap && previousWrap.isConnected !== false ? previousWrap : document.querySelector('#tl-canvas-wrap');
if (liveHead) T.attachHead(liveHead);
if (liveWrap) T.attachCanvas(liveWrap);
return T;
}
