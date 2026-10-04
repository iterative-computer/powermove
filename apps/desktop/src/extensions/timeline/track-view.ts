/* Track timeline: the Premiere-style presentation of the layer stack.

   Video tracks stack upward from V1, audio tracks downward from A1, and clips
   that never overlap share a track. The layer stack remains the source of
   truth; see track-layout.ts for how lanes and stack order stay in step.

   Keyframes live with their clip. An opened clip unfolds its track: one lane
   per animated property, labelled in the track header, with the clip's keys
   and the shape of the value drawn directly beneath it. Opening a clip is the
   layer timeline's disclosure, so U / P / S reveals and twirls carry over.

   Captions layers get caption tracks above the video tracks (C1, C2…), one
   per layer, each showing its cues; see caption-track.ts. */
import type { PowermoveAPI } from 'powermove';
import {
  layoutTracks, planTrackMove,
  type TrackArea, type TrackItem, type TrackLayer, type TrackLayout, type TrackLayoutOptions, type TrackMovePlan,
} from './track-layout';
import type { EdgeSnapProbe } from './timeline';
import { trackChannels } from './property-tracks';
import type { CaptionTrack } from './caption-track';

export type TimelineMode = 'layers' | 'tracks';
export const TIMELINE_MODES: ReadonlyArray<{ id: TimelineMode; label: string; icon: string; title: string }> = [
  { id: 'layers', label: 'Layers', icon: 'layers', title: 'Layer timeline (After Effects style)' },
  { id: 'tracks', label: 'Tracks', icon: 'film', title: 'Track timeline (Premiere style)' },
];
export const normalizeTimelineMode = (value: unknown): TimelineMode => value === 'tracks' ? 'tracks' : 'layers';

/** Minimum empty tracks shown, as in a new Premiere sequence. */
const MIN_VIDEO_TRACKS = 3;
const MIN_AUDIO_TRACKS = 2;
const DIVIDER = 8;
const EDGE = 5;
/** Height of one property lane in an unfolded track. */
export const PROPERTY_LANE = 24;
/** Wide enough for the ruler gutter's transport, view switch and magnet. */
export const TRACK_GUTTER = 272;
/** Pointer reach of a clip's disclosure arrow, from its visible left edge. */
const DISCLOSURE = 18;

export interface TrackViewHost {
  api: PowermoveAPI;
  T: any;
  t2x(time: number): number;
  x2t(x: number): number;
  theme(): any;
  ink: any;
  drawClip(c: CanvasRenderingContext2D, layer: any, y: number, height: number): void;
  clipPalette(layer: any): { body: string; primary: string; foreground: string; ring: string };
  roundRect(c: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number): void;
  clipText(c: CanvasRenderingContext2D, text: string, x: number, y: number, max: number): void;
  rgba(hex: string, alpha: number): string;
  captions: CaptionTrack;
  fui(): string;
  fmono(): string;
  niceStep(pps: number): number;
  icoEye(c: CanvasRenderingContext2D, x: number, y: number, on: boolean): void;
  icoLock(c: CanvasRenderingContext2D, x: number, y: number, on: boolean): void;
  icoSpeaker(c: CanvasRenderingContext2D, x: number, y: number, on: boolean): void;
  icoAnimationDiamond(c: CanvasRenderingContext2D, x: number, y: number, animated: boolean, current: boolean): void;
  fitValue(c: CanvasRenderingContext2D, value: any, unit: string, width: number): string;
  visible(layer: any): boolean;
  beginDrag(event: PointerEvent, options: any): any;
  invalidate(what?: string): void;
  edgeSnapping(event?: any): boolean;
  layerEdgeSnapper(ids: string[]): {
    resolve(probes: EdgeSnapProbe[], requested: number): { delta: number; snapped: boolean };
    release(requested: number): { delta: number; snapped: boolean };
  };
  trim(event: PointerEvent, side: 'in' | 'out'): void;
  /** Distinct keyframe times of a layer, local to its start, sorted. */
  keyTimes(layer: any): number[];
  /** The property rows the layer timeline lists for an opened layer. */
  props(layer: any): any[];
  keyframeSize(): number;
  keySelected(key: any): boolean;
  rowSelected(row: any): boolean;
  /** Layer-timeline key gesture: select, drag, Alt-stretch, or a key marquee. */
  keyDown(event: PointerEvent, row: any, x: number, y: number): void;
  keyMarquee(event: PointerEvent): void;
  editKeyAt(row: any, x: number): boolean;
  focusRow(row: any): void;
  toggleAnimation(row: any): void;
}

interface Lane { area: TrackArea; lane: number; y: number; h: number }
interface TrackDrag { plan: TrackMovePlan; layout: TrackLayout; moving: Set<string>; video: number; audio: number; captions: number }
/** An unfolded track: its property lanes, and each open clip's row for them. */
interface Drawer { lanes: Array<{ key: string; label: string }>; rows: Map<string, Map<string, any>> }
interface DrawerHit { lane: Lane; index: number; key: string; label: string; cy: number; layer: any | null; row: any | null }

export function createTrackView(host: TrackViewHost) {
  const { api, T } = host;
  const rowH = () => Math.max(34, Math.round(T.row) + 6);
  const scroll = () => Number(T.trackScrollY) || 0;

  /* ── layout ─────────────────────────────────────────────── */
  let cached: { signature: unknown[]; layout: TrackLayout } | null = null;
  function options(layers: any[]): TrackLayoutOptions {
    const byId = new Map(layers.map(layer => [layer.id, layer]));
    return {
      duration: Number(api.project.get().dur) || 0,
      collapsed: (id) => { const group = byId.get(id); return !!group && api.uiState.getGroupCollapsed(group); },
    };
  }
  function snapshot(layers: any[]): TrackLayer[] {
    return layers.map(layer => ({
      id: layer.id, type: layer.type, from: Number(layer.from) || 0, dur: Number(layer.dur) || 0,
      group: layer.group ?? null, ...(Number.isInteger(layer.track) ? { track: layer.track } : {}),
    }));
  }
  function layout(): TrackLayout {
    const drag: TrackDrag | null = T.trackDrag;
    if (drag) return drag.layout;
    const layers = api.project.get().layers;
    const signature: unknown[] = [layers, layers.length, api.project.get().dur, api.uiState.timelineVersion?.()];
    for (const layer of layers) signature.push(layer.from, layer.dur, layer.group, layer.track, layer.type);
    if (!cached || cached.signature.length !== signature.length || cached.signature.some((value, i) => value !== signature[i])) {
      cached = { signature, layout: layoutTracks(snapshot(layers), options(layers)) };
    }
    return cached.layout;
  }
  function counts() {
    const current = layout(), drag: TrackDrag | null = T.trackDrag;
    return {
      video: Math.max(MIN_VIDEO_TRACKS, current.video + 1, drag?.video ?? 0),
      audio: Math.max(MIN_AUDIO_TRACKS, current.audio + 1, drag?.audio ?? 0),
      // Caption tracks exist only while there are captions, as in Premiere.
      captions: Math.max(current.captions, drag?.captions ?? 0),
    };
  }
  const layerOf = (id: string) => api.model.layer(id);
  const laneKey = (area: TrackArea, lane: number) => `${area}:${lane}`;
  const isOpen = (layer: any) => !api.uiState.getLayerCollapsed(layer);
  /** A clip worth unfolding: it is animated, or a reveal asked for its rows. */
  const openable = (layer: any) => host.keyTimes(layer).length > 0 || isOpen(layer);

  /* ── unfolded tracks ────────────────────────────────────── */
  let drawerCache: { signature: unknown[]; drawers: Map<string, Drawer> } | null = null;
  function drawers(): Map<string, Drawer> {
    // Mid-drag the tracks keep the shape they had when the drag began.
    const base = T.trackDrag && cached ? cached.layout : layout();
    const signature: unknown[] = [base, api.anim.version(), api.uiState.timelineVersion?.()];
    if (drawerCache && api.uiState.timelineVersion && drawerCache.signature.every((value, i) => value === signature[i])) {
      return drawerCache.drawers;
    }
    const result = new Map<string, Drawer>();
    for (const item of base.items) {
      if (item.kind !== 'clip') continue;
      const layer = layerOf(item.id);
      if (!layer || !isOpen(layer)) continue;
      const rows = host.props(layer);
      if (!rows.length) continue;
      const key = laneKey(item.area, item.lane);
      let drawer = result.get(key);
      if (!drawer) result.set(key, drawer = { lanes: [], rows: new Map() });
      const byKey = new Map<string, any>();
      for (const row of rows) {
        byKey.set(row.key, row);
        if (!drawer.lanes.some(lane => lane.key === row.key)) drawer.lanes.push({ key: row.key, label: row.label });
      }
      drawer.rows.set(layer.id, byKey);
    }
    drawerCache = { signature, drawers: result };
    return result;
  }
  const drawerOf = (lane: { area: TrackArea; lane: number }) => drawers().get(laneKey(lane.area, lane.lane));

  /* ── geometry ───────────────────────────────────────────── */
  let laneCache: { signature: unknown[]; lanes: Lane[] } | null = null;
  function lanes(): Lane[] {
    const { video, audio, captions } = counts();
    const map = drawers();
    const signature: unknown[] = [map, video, audio, captions, scroll(), T.ruler, rowH()];
    if (laneCache && laneCache.signature.every((value, i) => value === signature[i])) return laneCache.lanes;
    const result: Lane[] = [];
    let y = T.ruler - scroll();
    const push = (area: TrackArea, lane: number) => {
      const h = rowH() + (map.get(laneKey(area, lane))?.lanes.length ?? 0) * PROPERTY_LANE;
      result.push({ area, lane, y, h });
      y += h;
    };
    for (let lane = captions - 1; lane >= 0; lane--) push('captions', lane);
    if (captions) y += DIVIDER;
    for (let lane = video - 1; lane >= 0; lane--) push('video', lane);
    y += DIVIDER;
    for (let lane = 0; lane < audio; lane++) push('audio', lane);
    laneCache = { signature, lanes: result };
    return result;
  }
  function laneY(area: TrackArea, lane: number) {
    const found = lanes().find(entry => entry.area === area && entry.lane === lane);
    return found ? found.y : laneYIn(lanes(), area, lane);
  }
  /** Extrapolates past the drawn tracks with plain track heights. */
  function laneYIn(list: Lane[], area: TrackArea, lane: number) {
    const own = list.filter(entry => entry.area === area);
    if (area !== 'audio') {
      const top = own[0];
      return top ? top.y - (lane - top.lane) * rowH() : T.ruler - scroll();
    }
    const last = own.at(-1);
    return last ? last.y + last.h + (lane - last.lane - 1) * rowH() : T.ruler - scroll();
  }
  function contentHeight() {
    const list = lanes(), last = list.at(-1);
    return last ? last.y + last.h + scroll() - T.ruler : 0;
  }
  function laneAt(y: number): Lane | null {
    if (y < T.ruler) return null;
    return lanes().find(lane => y >= lane.y && y < lane.y + lane.h) ?? null;
  }
  /** Nearest track index in an area, extrapolated past the drawn tracks. */
  function laneIndexIn(list: Lane[], area: TrackArea, y: number) {
    const own = list.filter(entry => entry.area === area);
    const hit = own.find(entry => y >= entry.y && y < entry.y + entry.h);
    if (hit) return hit.lane;
    const first = own[0], last = own.at(-1);
    if (!first || !last) return 0;
    if (area !== 'audio') {
      // own[0] is the top track; own.at(-1) is V1 (or C1).
      if (y < first.y) return first.lane + Math.ceil((first.y - y) / rowH());
      return -1 - Math.floor((y - last.y - last.h) / rowH());
    }
    if (y < first.y) return -1 - Math.floor((first.y - y) / rowH());
    return last.lane + 1 + Math.floor((y - last.y - last.h) / rowH());
  }
  const laneIndexAt = (area: TrackArea, y: number) => laneIndexIn(lanes(), area, y);
  function itemsIn(area: TrackArea, lane: number) {
    return layout().items.filter(item => item.area === area && item.lane === lane);
  }
  function hitItem(x: number, y: number): { item: TrackItem; edge: 'in' | 'out' | null } | null {
    const lane = laneAt(y);
    if (!lane || x < T.gut || y >= lane.y + rowH()) return null;
    let best: { item: TrackItem; edge: 'in' | 'out' | null; distance: number } | null = null;
    for (const item of itemsIn(lane.area, lane.lane)) {
      const x0 = host.t2x(item.from), x1 = host.t2x(item.from + item.dur);
      if (x < x0 - EDGE || x > x1 + EDGE) continue;
      const inside = x >= x0 && x <= x1;
      const edge = item.kind === 'clip' && x1 - x0 > EDGE * 3
        ? Math.abs(x - x0) <= EDGE ? 'in' : Math.abs(x - x1) <= EDGE ? 'out' : null : null;
      const distance = inside ? 0 : Math.min(Math.abs(x - x0), Math.abs(x - x1));
      if (!best || distance < best.distance) best = { item, edge, distance };
    }
    return best;
  }
  /** Horizontal reach of an open clip's lanes: the clip, plus any keys past it. */
  function rowSpan(layer: any, row: any): [number, number] {
    let first = 0, last = Number(layer.dur) || 0;
    for (const axis of trackChannels(row)) {
      const kf = axis.prop.kf;
      if (kf.length) { first = Math.min(first, kf[0].t); last = Math.max(last, kf.at(-1).t); }
    }
    return [host.t2x(layer.from + first), host.t2x(layer.from + last)];
  }
  function drawerAt(x: number, y: number): DrawerHit | null {
    const lane = laneAt(y);
    if (!lane || y < lane.y + rowH()) return null;
    const drawer = drawerOf(lane);
    if (!drawer) return null;
    const index = Math.floor((y - lane.y - rowH()) / PROPERTY_LANE);
    const entry = drawer.lanes[index];
    if (!entry) return null;
    const cy = lane.y + rowH() + index * PROPERTY_LANE + PROPERTY_LANE / 2;
    for (const [id, rows] of drawer.rows) {
      const layer = layerOf(id), row = rows.get(entry.key);
      if (!layer || !row) continue;
      const [x0, x1] = rowSpan(layer, row);
      if (x >= x0 - 8 && x <= x1 + 8) return { lane, index, key: entry.key, label: entry.label, cy, layer, row };
    }
    return { lane, index, key: entry.key, label: entry.label, cy, layer: null, row: null };
  }
  /** The clip a track's property label speaks for: under the playhead, else selected, else first. */
  function readoutRow(drawer: Drawer, key: string) {
    const time = api.transport.time(), selected = api.selection.layers();
    let fallback: any = null;
    for (const [id, rows] of drawer.rows) {
      const layer = layerOf(id), row = rows.get(key);
      if (!layer || !row) continue;
      if (time >= layer.from && time < layer.from + layer.dur) return row;
      if (!fallback || (selected.includes(id) && !selected.includes(fallback.L.id))) fallback = row;
    }
    return fallback;
  }

  /* ── drawing ────────────────────────────────────────────── */
  function draw(c: CanvasRenderingContext2D, W: number, H: number) {
    const theme = host.theme(), ink = host.ink, h = rowH();
    const current = layout();
    const drag: TrackDrag | null = T.trackDrag;
    const planned = drag ? new Map(drag.plan.layers.map(layer => [layer.id, layer])) : null;

    // Lanes, work area and second gridlines.
    c.save(); c.beginPath(); c.rect(T.gut, T.ruler, W - T.gut, H - T.ruler); c.clip();
    c.fillStyle = theme.sunken; c.fillRect(T.gut, T.ruler, W - T.gut, H - T.ruler);
    const project = api.project.get();
    const work = project.work || [0, project.dur];
    const w0 = host.t2x(work[0]), w1 = host.t2x(work[1]);
    c.fillStyle = ink.over;
    c.fillRect(T.gut, T.ruler, Math.max(0, w0 - T.gut), H - T.ruler);
    c.fillRect(w1, T.ruler, Math.max(0, W - w1), H - T.ruler);
    const end = host.t2x(project.dur);
    if (end < W) { c.fillStyle = ink.over; c.fillRect(Math.max(T.gut, end), T.ruler, W - Math.max(T.gut, end), H - T.ruler); }
    const step = host.niceStep(T.pps);
    c.strokeStyle = ink.grid; c.lineWidth = 1; c.beginPath();
    for (let t = Math.floor(T.scrollT / step) * step; host.t2x(t) < W; t += step) {
      const x = host.t2x(t) + .5;
      if (x >= T.gut) { c.moveTo(x, T.ruler); c.lineTo(x, H); }
    }
    c.stroke();
    for (const lane of lanes()) {
      if (lane.y + lane.h < T.ruler || lane.y > H) continue;
      if (T.trackHover && T.trackHover.area === lane.area && T.trackHover.lane === lane.lane) {
        c.fillStyle = ink.over; c.globalAlpha = .7; c.fillRect(T.gut, lane.y, W - T.gut, h); c.globalAlpha = 1;
      }
      c.strokeStyle = ink.grid; c.beginPath(); c.moveTo(T.gut, Math.round(lane.y + lane.h) - .5); c.lineTo(W, Math.round(lane.y + lane.h) - .5); c.stroke();
    }
    drawDivider(c, T.gut, W);

    // Expanded groups: a quiet band behind their members, edged in the bar's tint.
    for (const band of current.bands) {
      const group = layerOf(band.id);
      if (!group) continue;
      const pal = host.clipPalette(group);
      const top = laneY(band.area, band.laneEnd - 1);
      const bottomLane = lanes().find(lane => lane.area === band.area && lane.lane === band.laneStart);
      const bottom = bottomLane ? bottomLane.y + bottomLane.h : laneY(band.area, band.laneStart) + h;
      const x0 = host.t2x(band.from), x1 = host.t2x(band.from + band.dur);
      if (x1 < T.gut || x0 > W || bottom < T.ruler || top > H) continue;
      c.fillStyle = pal.primary; c.globalAlpha = .1;
      c.fillRect(x0, top, x1 - x0, bottom - top);
      c.globalAlpha = .55;
      c.fillRect(x0, top, 2, bottom - top - 1);
      c.globalAlpha = 1;
    }

    drawDrawers(c, W, H, drag);

    for (const item of current.items) {
      const layer = layerOf(item.id);
      if (!layer) continue;
      const y = laneY(item.area, item.lane);
      if (y + h < T.ruler || y > H) continue;
      const from = planned?.get(item.id)?.from ?? layer.from;
      if (item.area === 'captions' && item.kind === 'clip') {
        if (drag?.moving.has(item.id)) c.globalAlpha = .85;
        host.drawClip(c, { ...layer, from }, y, h);
        host.captions.draw(c, { ...layer, from }, y, h, W);
        c.globalAlpha = 1;
        continue;
      }
      if (item.kind === 'group') {
        const collapsed = api.uiState.getGroupCollapsed(layer);
        host.drawClip(c, { ...layer, from: item.from, dur: item.dur, name: `${collapsed ? '▸' : '▾'}  ${layer.name}` }, y, h);
      } else {
        if (drag?.moving.has(item.id)) c.globalAlpha = .85;
        const open = isOpen(layer);
        // Animated clips carry the same disclosure arrow as group bars.
        host.drawClip(c, openable(layer) ? { ...layer, from, name: `${open ? '▾' : '▸'}  ${layer.name}` } : { ...layer, from }, y, h);
        if (!open || !drawerOf(item)) drawClipKeys(c, layer, from, y, h);
        c.globalAlpha = 1;
      }
    }
    drawDropGhost(c, W, H);
    c.restore();
    drawHeaders(c, W, H);
  }

  /* Each open clip hangs a tinted sheet below itself, as if the clip had
     unfolded: per property, the value's shape and the keys that make it. */
  function drawDrawers(c: CanvasRenderingContext2D, W: number, H: number, drag: TrackDrag | null) {
    const ink = host.ink, h = rowH();
    for (const lane of lanes()) {
      const drawer = drawerOf(lane);
      if (!drawer || lane.y + lane.h < T.ruler || lane.y > H) continue;
      const top = lane.y + h;
      drawer.lanes.forEach((_entry, index) => {
        const y = top + index * PROPERTY_LANE;
        c.strokeStyle = ink.grid; c.lineWidth = 1; c.beginPath();
        c.moveTo(T.gut, Math.round(y + PROPERTY_LANE / 2) + .5); c.lineTo(W, Math.round(y + PROPERTY_LANE / 2) + .5); c.stroke();
      });
      for (const [id, rows] of drawer.rows) {
        const layer = layerOf(id);
        if (!layer || drag?.moving.has(id)) continue;
        const pal = host.clipPalette(layer);
        const x0 = host.t2x(layer.from), x1 = host.t2x(layer.from + layer.dur);
        if (x1 >= T.gut && x0 <= W) {
          c.save();
          c.fillStyle = pal.body; c.globalAlpha = .45;
          host.roundRect(c, x0, top - 4, Math.max(4, x1 - x0), drawer.lanes.length * PROPERTY_LANE + 2, 4); c.fill();
          c.restore();
        }
        drawer.lanes.forEach((entry, index) => {
          const row = rows.get(entry.key);
          if (!row) return;
          const y = top + index * PROPERTY_LANE;
          if (host.rowSelected(row)) {
            c.fillStyle = ink.over2; c.fillRect(Math.max(T.gut, x0), y, Math.max(0, Math.min(W, x1) - Math.max(T.gut, x0)), PROPERTY_LANE);
          }
          drawCurve(c, layer, row, y, x0, x1, W, pal.primary);
          drawKeys(c, layer, row, y + PROPERTY_LANE / 2, W);
        });
      }
    }
  }

  /* A faint line of the value across the clip, normalised to its lane. */
  const curves = new Map<string, { signature: string; points: Array<Array<[number, number]>> | null }>();
  function drawCurve(c: CanvasRenderingContext2D, layer: any, row: any, y: number, x0: number, x1: number, W: number, color: string) {
    const left = Math.max(T.gut, x0), right = Math.min(W, x1);
    if (right - left < 4) return;
    const id = `${layer.id}:${row.key}`;
    const signature = [api.anim.version(), T.pps, T.scrollT, Math.round(left), Math.round(right), layer.from].join(':');
    let entry = curves.get(id);
    if (!entry || entry.signature !== signature) {
      const channels = trackChannels(row).filter(axis => axis.prop.kf.length > 1);
      const series: Array<Array<[number, number]>> = channels.map(() => []);
      let min = Infinity, max = -Infinity;
      for (let x = left; x <= right + .5; x += 3) {
        const time = host.x2t(Math.min(x, right));
        channels.forEach((axis, index) => {
          const value = api.anim.evP(layer, axis.prop, time, axis.key);
          if (typeof value !== 'number' || !Number.isFinite(value)) return;
          series[index]!.push([Math.min(x, right), value]);
          min = Math.min(min, value); max = Math.max(max, value);
        });
      }
      entry = { signature, points: max - min > 1e-6 ? series.map(points => points.map(([x, value]) => [x, (value - min) / (max - min)] as [number, number])) : null };
      if (curves.size > 256) curves.clear();
      curves.set(id, entry);
    }
    if (!entry.points) return;
    const pad = 4, span = PROPERTY_LANE - pad * 2;
    c.save();
    c.strokeStyle = color; c.globalAlpha = .55; c.lineWidth = 1.25; c.lineJoin = 'round';
    for (const points of entry.points) {
      c.beginPath();
      points.forEach(([x, value], index) => {
        const py = y + pad + (1 - value) * span;
        if (index) c.lineTo(x, py); else c.moveTo(x, py);
      });
      c.stroke();
    }
    c.restore();
  }

  /* Same key glyphs as the layer timeline: first/last half-filled toward the
     animated span, holds square, selection in the accent. */
  function drawKeys(c: CanvasRenderingContext2D, layer: any, row: any, cy: number, W: number) {
    const theme = host.theme(), ink = host.ink;
    const kf = row.prop.kf;
    const r = host.keyframeSize() / 2;
    const first = kf[0]?.t, last = kf.at(-1)?.t;
    c.lineWidth = 1;
    for (const k of kf) {
      const x = host.t2x(layer.from + k.t);
      if (x < T.gut - 6 || x > W + 6) continue;
      const color = host.keySelected(k) ? theme.accent : ink.key;
      c.fillStyle = color; c.strokeStyle = color;
      if (k.hold) { c.fillRect(x - r, cy - r, r * 2, r * 2); continue; }
      c.beginPath(); c.moveTo(x, cy - r); c.lineTo(x + r, cy); c.lineTo(x, cy + r); c.lineTo(x - r, cy); c.closePath();
      c.stroke();
      const isFirst = k.t === first, isLast = k.t === last;
      if (isFirst === isLast) { c.fill(); continue; }
      c.beginPath(); c.moveTo(x, cy - r);
      c.lineTo(x + (isFirst ? r : -r), cy);
      c.lineTo(x, cy + r); c.closePath(); c.fill();
    }
  }

  /* A closed clip still shows where it is animated: small diamonds along its
     foot, edge keys tucked just inside it, clear of the trim handles. */
  function drawClipKeys(c: CanvasRenderingContext2D, layer: any, from: number, y: number, h: number) {
    const times = host.keyTimes(layer);
    if (!times.length || h < 24) return;
    const start = host.t2x(from), x0 = Math.max(start, T.gut), x1 = host.t2x(from + (Number(layer.dur) || 0));
    if (x1 - x0 < 8) return;
    const pal = host.clipPalette(layer), r = 3, cy = y + h - 7;
    c.save(); c.beginPath(); c.rect(x0, y, x1 - x0, h); c.clip();
    c.fillStyle = pal.foreground; c.globalAlpha *= .85;
    let last = -Infinity;
    for (const time of times) {
      const at = host.t2x(from + time);
      if (at < start - .5 || at > x1 + .5 || at < T.gut - r) continue;
      const x = Math.min(x1 - r - 2, Math.max(x0 + r + 2, at));
      if (x - last < r * 2) continue;
      last = x;
      c.beginPath(); c.moveTo(x, cy - r); c.lineTo(x + r, cy); c.lineTo(x, cy + r); c.lineTo(x - r, cy); c.closePath(); c.fill();
    }
    c.restore();
  }

  function drawDivider(c: CanvasRenderingContext2D, left: number, right: number) {
    const theme = host.theme();
    const rule = (y: number) => {
      c.fillStyle = theme.panel; c.fillRect(left, y, right - left, DIVIDER);
      c.strokeStyle = theme.line; c.beginPath();
      c.moveTo(left, y + .5); c.lineTo(right, y + .5);
      c.moveTo(left, y + DIVIDER - .5); c.lineTo(right, y + DIVIDER - .5); c.stroke();
    };
    rule(laneY('audio', 0) - DIVIDER);
    // Caption tracks sit apart from the picture tracks, above V tracks.
    const lastCaption = lanes().find(lane => lane.area === 'captions' && lane.lane === 0);
    if (lastCaption) rule(lastCaption.y + lastCaption.h);
  }

  function laneState(area: TrackArea, lane: number) {
    const layers = itemsIn(area, lane).map(item => layerOf(item.id)).filter((layer): layer is NonNullable<typeof layer> => !!layer);
    const animated = layers.filter(layer => layer.type !== 'group' && openable(layer));
    return {
      layers,
      animated,
      open: animated.some(layer => isOpen(layer)),
      visible: !layers.length || layers.some(layer => host.visible(layer)),
      locked: layers.length > 0 && layers.every(layer => layer.lock),
      selected: layers.some(layer => api.selection.layers().includes(layer.id)),
    };
  }

  function drawTwirl(c: CanvasRenderingContext2D, x: number, y: number, open: boolean, color: string) {
    c.save();
    c.translate(x, y); c.rotate(open ? Math.PI / 2 : 0);
    c.strokeStyle = color; c.lineWidth = 1.4; c.beginPath();
    c.moveTo(-1.6, -3.4); c.lineTo(2, 0); c.lineTo(-1.6, 3.4); c.stroke();
    c.restore();
  }

  function drawHeaders(c: CanvasRenderingContext2D, _W: number, H: number) {
    const theme = host.theme(), ink = host.ink, h = rowH();
    c.fillStyle = theme.panel; c.fillRect(0, T.ruler, T.gut, H - T.ruler);
    c.save(); c.beginPath(); c.rect(0, T.ruler, T.gut, H - T.ruler); c.clip();
    c.textBaseline = 'middle';
    for (const lane of lanes()) {
      if (lane.y + lane.h < T.ruler || lane.y > H) continue;
      const state = laneState(lane.area, lane.lane);
      const cy = lane.y + h / 2;
      const hovered = T.trackHover && T.trackHover.area === lane.area && T.trackHover.lane === lane.lane;
      if (state.selected) { c.fillStyle = ink.over2; c.fillRect(0, lane.y, T.gut, h); }
      else if (hovered) { c.fillStyle = ink.over; c.fillRect(0, lane.y, T.gut, h); }
      c.strokeStyle = ink.grid; c.beginPath(); c.moveTo(0, Math.round(lane.y + lane.h) - .5); c.lineTo(T.gut, Math.round(lane.y + lane.h) - .5); c.stroke();
      host.icoLock(c, 16, cy, state.locked);
      if (state.animated.length) drawTwirl(c, 34, cy, state.open, state.open || hovered ? theme.tx2 : theme.tx3);
      // Track name chip, Premiere's "V1" / "A1".
      const label = `${lane.area === 'video' ? 'V' : lane.area === 'captions' ? 'C' : 'A'}${lane.lane + 1}`;
      c.font = '500 11px ' + host.fmono();
      const chipW = Math.max(28, c.measureText(label).width + 12);
      c.fillStyle = state.selected ? host.rgba(theme.accent, .18) : ink.over2;
      host.roundRect(c, 44, cy - 9, chipW, 18, 4); c.fill();
      c.fillStyle = state.selected ? theme.accent : theme.tx2;
      c.fillText(label, 50, cy + .5);
      if (state.layers.length) {
        c.font = '400 11px ' + host.fui(); c.fillStyle = theme.tx3;
        const names = state.layers.length === 1 ? state.layers[0]!.name : `${state.layers.length} clips`;
        host.clipText(c, names, 52 + chipW, cy + .5, T.gut - 52 - chipW - 34);
      }
      if (lane.area !== 'audio') host.icoEye(c, T.gut - 18, cy, state.visible);
      else host.icoSpeaker(c, T.gut - 18, cy, state.visible);
      drawPropertyLabels(c, lane);
    }
    drawDivider(c, 0, T.gut);
    c.restore();
    c.strokeStyle = theme.line; c.beginPath(); c.moveTo(T.gut - .5, T.ruler); c.lineTo(T.gut - .5, H); c.stroke();
  }

  /* An unfolded track names its lanes in the header, with the value at the
     playhead and the stopwatch diamond, as the layer timeline does. */
  function drawPropertyLabels(c: CanvasRenderingContext2D, lane: Lane) {
    const drawer = drawerOf(lane);
    if (!drawer) return;
    const theme = host.theme(), ink = host.ink, time = api.transport.time();
    drawer.lanes.forEach((entry, index) => {
      const y = lane.y + rowH() + index * PROPERTY_LANE, cy = y + PROPERTY_LANE / 2;
      const row = readoutRow(drawer, entry.key);
      const selected = !!row && host.rowSelected(row);
      if (selected) { c.fillStyle = ink.over2; c.fillRect(0, y, T.gut, PROPERTY_LANE); }
      if (row) {
        const channels = trackChannels(row);
        host.icoAnimationDiamond(c, 52, cy, channels.some(axis => axis.prop.kf.length > 0),
          channels.some(axis => !!api.anim.hasKeyAt(row.L, axis.prop, time)));
      }
      c.font = '400 11px ' + host.fui(); c.fillStyle = selected ? theme.accent : theme.tx3;
      const valueX = T.gut - 12, labelMax = Math.max(0, T.gut - 64 - 92);
      host.clipText(c, entry.label, 64, cy, labelMax);
      if (!row) return;
      const active = time >= row.L.from && time < row.L.from + row.L.dur;
      const values = trackChannels(row).map(axis => api.anim.evP(row.L, axis.prop, time, axis.key));
      c.font = '400 10px ' + host.fmono(); c.textAlign = 'right';
      c.fillStyle = active ? theme.accent : theme.tx3;
      const text = values.map(value => host.fitValue(c, value, row.channels ? '%' : '', 44)).join('  ');
      host.clipText(c, text, valueX, cy, 88);
      c.textAlign = 'left';
    });
  }

  function drawDropGhost(c: CanvasRenderingContext2D, W: number, H: number) {
    const drop = T.trackDrop;
    if (!drop) return;
    const theme = host.theme(), h = rowH();
    const type = drop.kind === 'audio' ? 'audio' : drop.kind === 'video' ? 'video' : drop.kind === 'image' ? 'image' : null;
    const pal = host.clipPalette({ type });
    const y = laneY(drop.area, drop.lane);
    if (y + h < T.ruler || y > H) return;
    const x0 = host.t2x(drop.at);
    const width = Math.max(6, (drop.dur || Math.max(1, Math.min(4, api.project.get().dur - drop.at))) * T.pps);
    c.save();
    c.globalAlpha = .55; host.roundRect(c, x0, y + 1, width, h - 2, 4); c.fillStyle = pal.body; c.fill();
    c.globalAlpha = 1; c.setLineDash([4, 3]); c.strokeStyle = theme.accent; c.lineWidth = 1.25;
    host.roundRect(c, x0 + .5, y + 1.5, width - 1, h - 3, 4); c.stroke(); c.setLineDash([]);
    c.font = '500 11px ' + host.fui(); c.textBaseline = 'middle'; c.fillStyle = pal.foreground;
    if (width > 30) host.clipText(c, drop.name, x0 + 7, y + h / 2, Math.min(width, W - x0) - 14);
    c.restore();
  }

  /* ── selection ──────────────────────────────────────────── */
  function selectForPointer(id: string, event: PointerEvent) {
    T.keySelectionActive = false;
    api.selection.set({ keys: [] });
    const selected = api.selection.layers();
    if (event.shiftKey || event.metaKey || event.ctrlKey) {
      api.selection.select(selected.includes(id) ? selected.filter(other => other !== id) : [...selected, id]);
      return !selected.includes(id);
    }
    if (!selected.includes(id)) api.selection.select([id]);
    return true;
  }

  function lockedLayer(layer: any) {
    return !layer || layer.lock || api.groups.ancestors(layer).some(group => group.lock);
  }

  function toggleOpen(layers: any[], open = !layers.some(layer => isOpen(layer))) {
    for (const layer of layers) api.uiState.setLayerCollapsed(layer, !open);
    host.invalidate('timeline');
  }
  const onDisclosure = (item: TrackItem, x: number) => x - Math.max(host.t2x(item.from), T.gut) < DISCLOSURE;

  /* ── gestures ───────────────────────────────────────────── */
  function down(event: PointerEvent, x: number, y: number) {
    if (x < T.gut) return headerDown(event, x, y);
    const drawerHit = drawerAt(x, y);
    if (drawerHit) {
      if (drawerHit.row) return host.keyDown(event, drawerHit.row, x, y);
      return host.keyMarquee(event);
    }
    const hit = hitItem(x, y);
    if (!hit) return marquee(event);
    const layer = layerOf(hit.item.id);
    if (!layer) return;
    if (hit.item.kind === 'group' && onDisclosure(hit.item, x)) {
      api.uiState.setGroupCollapsed(layer, !api.uiState.getGroupCollapsed(layer));
      host.invalidate('timeline');
      return;
    }
    if (hit.item.area === 'captions' && host.captions.hit(layer, x)) {
      const additive = event.shiftKey || event.metaKey || event.ctrlKey;
      if (!additive || api.selection.layers().includes(layer.id)) { host.captions.down(event, layer, x); return; }
    }
    if (hit.item.kind === 'clip' && !hit.edge && openable(layer) && onDisclosure(hit.item, x)) return toggleOpen([layer]);
    if (!selectForPointer(layer.id, event)) return;
    if (hit.edge && !layer.lock) return host.trim(event, hit.edge);
    return moveClips(event);
  }

  function headerDown(_event: PointerEvent, x: number, y: number) {
    const lane = laneAt(y);
    if (!lane) return;
    if (y >= lane.y + rowH()) {
      const hit = drawerAt(T.gut + 1, y);
      const drawer = drawerOf(lane);
      const row = hit && drawer ? readoutRow(drawer, hit.key) : null;
      if (!row) return;
      if (x >= 44 && x < 60 && !row.L.lock) host.toggleAnimation(row);
      else host.focusRow(row);
      host.invalidate('timeline');
      return;
    }
    const state = laneState(lane.area, lane.lane);
    if (!state.layers.length) return;
    if (x < 26) {
      api.edit.apply(state.layers.map((layer: any) => ({ type: 'set_layer', target: layer.id, patch: { locked: !state.locked } })),
        { label: state.locked ? 'Unlock track' : 'Lock track', origin: 'timeline' });
    } else if (x < 42 && state.animated.length) {
      toggleOpen(state.animated);
      return;
    } else if (x >= T.gut - 30) {
      api.edit.apply(state.layers.map((layer: any) => ({ type: 'set_layer', target: layer.id, patch: { visible: !state.visible } })),
        { label: lane.area === 'audio' ? (state.visible ? 'Mute track' : 'Unmute track') : (state.visible ? 'Hide track' : 'Show track'), origin: 'timeline' });
    } else {
      // Clicking the track name selects everything on the track.
      T.keySelectionActive = false;
      api.selection.set({ keys: [] });
      api.selection.select(state.layers.map((layer: any) => layer.id));
    }
    host.invalidate('timeline');
  }

  function moveClips(event: PointerEvent) {
    const project = api.project.get();
    const selected = new Set(api.selection.layers());
    const roots = project.layers.filter((layer: any) => selected.has(layer.id)
      && !api.groups.ancestors(layer).some(group => selected.has(group.id)) && !lockedLayer(layer));
    if (!roots.length) return;
    const ids = roots.map((layer: any) => layer.id);
    const moving = new Set(api.groups.expand(ids));
    const source = snapshot(project.layers);
    const layoutOptions = options(project.layers);
    const start = counts();
    // Track moves follow the pointer over the tracks as drawn when the drag began.
    const startLanes = lanes().map(lane => ({ ...lane }));
    const rect = T.cv.getBoundingClientRect();
    const startY = event.clientY - rect.top;
    const clips = project.layers.filter((layer: any) => moving.has(layer.id) && layer.type !== 'group');
    const earliest = Math.min(...clips.map((layer: any) => Number(layer.from) || 0), Infinity);
    const probes: EdgeSnapProbe[] = clips.flatMap((layer: any) => [
      { time: layer.from, edge: 'in' as const }, { time: layer.from + layer.dur, edge: 'out' as const },
    ]);
    const snapper = host.layerEdgeSnapper([...moving]);
    const fps = Math.max(1, Number(project.fps) || 30);
    const startArea = layout().byId.get(ids[0]!)?.area ?? 'video';
    let active = false;
    let plan: TrackMovePlan | null = null;
    const clear = () => { T.trackDrag = null; T.snapGuide = null; host.invalidate('timeline'); };
    host.beginDrag(event, {
      cursor: 'grabbing',
      move: (dx: number, dy: number, ev: PointerEvent) => {
        if (!active && Math.hypot(dx, dy) < 3) return;
        active = true;
        const requested = dx / T.pps;
        const { delta, snapped } = host.edgeSnapping(ev) ? snapper.resolve(probes, requested) : snapper.release(requested);
        let dt = snapped ? delta : Math.round(delta * fps) / fps;
        if (Number.isFinite(earliest)) dt = Math.max(dt, -earliest);
        const y = startY + dy;
        const shift = (area: TrackArea) => laneIndexIn(startLanes, area, y) - laneIndexIn(startLanes, area, startY);
        plan = planTrackMove(source, layoutOptions, { ids, dt, lanes: { video: shift('video'), audio: shift('audio'), captions: shift('captions') } });
        const preview = layoutTracks(plan.layers, layoutOptions);
        T.trackDrag = {
          plan, layout: preview, moving,
          video: Math.max(start.video, preview.video + 1), audio: Math.max(start.audio, preview.audio + 1),
          captions: Math.max(start.captions, preview.captions),
        } satisfies TrackDrag;
        T.trackHover = { area: startArea, lane: laneIndexIn(startLanes, startArea, y) };
        host.invalidate('timeline');
      },
      up: () => {
        const result = plan;
        clear();
        if (!active || !result) return;
        applyPlan('Move clips', result);
      },
      cancel: clear,
    });
  }

  function applyPlan(label: string, plan: TrackMovePlan) {
    api.edit.mutate(label, () => {
      const project = api.project.get();
      const byId = new Map(project.layers.map((layer: any) => [layer.id, layer]));
      const ordered: any[] = [];
      for (const entry of plan.layers) {
        const layer: any = byId.get(entry.id);
        if (!layer) continue;
        layer.from = api.util.round(entry.from, 6);
        layer.track = entry.track;
        ordered.push(layer);
      }
      project.layers = ordered;
      api.groups.normalizeStack();
      api.anim.touch();
    }, { origin: 'timeline' });
    host.invalidate();
  }

  function marquee(event: PointerEvent) {
    const rect = T.cv.getBoundingClientRect();
    const x0 = event.clientX - rect.left, y0 = event.clientY - rect.top;
    const additive = event.shiftKey || event.metaKey || event.ctrlKey;
    const base = additive ? [...api.selection.layers()] : [];
    let dragged = false;
    const pick = () => {
      const m = T.marquee;
      return layout().items.filter(item => {
        const y = laneY(item.area, item.lane);
        const a = host.t2x(item.from), b = host.t2x(item.from + item.dur);
        return b > m.x0 && a < m.x1 && y + rowH() > m.y0 && y < m.y1;
      }).map(item => item.id);
    };
    host.beginDrag(event, {
      move: (dx: number, dy: number) => {
        if (!dragged && Math.hypot(dx, dy) < 3) return;
        dragged = true;
        T.marquee = { x0: Math.min(x0, x0 + dx), y0: Math.min(y0, y0 + dy), x1: Math.max(x0, x0 + dx), y1: Math.max(y0, y0 + dy) };
        api.selection.select([...new Set([...base, ...pick()])]);
        host.invalidate('timeline');
      },
      up: () => {
        if (!dragged && !additive) api.selection.set({ chan: null, keys: [], layers: [] });
        T.marquee = null; host.invalidate('timeline');
      },
      cancel: () => { T.marquee = null; host.invalidate('timeline'); },
    });
  }

  /** Keys inside a marquee, across every unfolded track. */
  function keysIn(m: { x0: number; y0: number; x1: number; y1: number }) {
    const picked: any[] = [];
    for (const lane of lanes()) {
      const drawer = drawerOf(lane);
      if (!drawer) continue;
      drawer.lanes.forEach((entry, index) => {
        const cy = lane.y + rowH() + index * PROPERTY_LANE + PROPERTY_LANE / 2;
        if (cy < m.y0 || cy > m.y1) return;
        for (const [id, rows] of drawer.rows) {
          const layer = layerOf(id), row = rows.get(entry.key);
          if (!layer || !row) continue;
          for (const key of row.prop.kf) {
            const x = host.t2x(layer.from + key.t);
            if (x >= m.x0 && x <= m.x1) picked.push(key);
          }
        }
      });
    }
    return picked;
  }

  function hover(x: number, y: number): { cursor: string; title: string } {
    const lane = laneAt(y);
    const next = lane ? { area: lane.area, lane: lane.lane } : null;
    if (!T.trackDrag && (next?.area !== T.trackHover?.area || next?.lane !== T.trackHover?.lane)) {
      T.trackHover = next; host.invalidate('timeline');
    }
    if (!lane) return { cursor: 'default', title: '' };
    if (y >= lane.y + rowH()) {
      const hit = drawerAt(x < T.gut ? T.gut + 1 : x, y);
      if (!hit) return { cursor: 'default', title: '' };
      if (x < T.gut) return x >= 44 && x < 60 ? { cursor: 'pointer', title: 'Animate this property' } : { cursor: 'pointer', title: hit.label };
      if (!hit.row) return { cursor: 'crosshair', title: '' };
      const layer = hit.layer;
      const nearKey = hit.row.prop.kf.some((key: any) => Math.abs(host.t2x(layer.from + key.t) - x) < 6);
      return { cursor: nearKey ? 'pointer' : 'crosshair', title: '' };
    }
    if (x < T.gut) {
      const state = laneState(lane.area, lane.lane);
      if (!state.layers.length) return { cursor: 'default', title: '' };
      if (x < 26) return { cursor: 'pointer', title: state.locked ? 'Unlock track' : 'Lock track' };
      if (x < 42 && state.animated.length) return { cursor: 'pointer', title: state.open ? 'Fold keyframes' : 'Unfold keyframes' };
      if (x >= T.gut - 30) return { cursor: 'pointer', title: lane.area === 'audio' ? 'Mute track' : lane.area === 'captions' ? 'Show or hide captions' : 'Toggle track output' };
      return { cursor: 'pointer', title: 'Select all clips on this track' };
    }
    const hit = hitItem(x, y);
    if (!hit) return { cursor: 'default', title: '' };
    const layer = layerOf(hit.item.id);
    if (hit.item.area === 'captions' && layer) {
      const cue = host.captions.hover(layer, x);
      if (cue) return cue;
    }
    if (hit.item.kind === 'group' && onDisclosure(hit.item, x)) {
      return { cursor: 'pointer', title: api.uiState.getGroupCollapsed(layer!) ? 'Expand group' : 'Collapse group' };
    }
    if (hit.item.kind === 'clip' && !hit.edge && layer && openable(layer) && onDisclosure(hit.item, x)) {
      return { cursor: 'pointer', title: isOpen(layer) ? 'Fold keyframes' : 'Unfold keyframes' };
    }
    if (hit.edge && !layer?.lock) return { cursor: 'ew-resize', title: '' };
    return { cursor: 'grab', title: layer?.name ?? '' };
  }

  function doubleClick(x: number, y: number) {
    const drawerHit = drawerAt(x, y);
    if (drawerHit) return drawerHit.row ? host.editKeyAt(drawerHit.row, x) : false;
    const hit = hitItem(x, y);
    const layer = hit && layerOf(hit.item.id);
    if (!layer) return false;
    if (layer.type === 'group') {
      api.uiState.setGroupCollapsed(layer, !api.uiState.getGroupCollapsed(layer));
      host.invalidate('timeline');
      return true;
    }
    if (layer.type === 'precomp' && layer.d?.comp) { api.commands.run('openComposition', layer.d.comp); return true; }
    if (layer.type === 'captions' && hit) return host.captions.doubleClick(layer, x, laneY(hit.item.area, hit.item.lane), rowH());
    // Double-clicking an animated clip unfolds or folds its keyframes.
    if (openable(layer)) { toggleOpen([layer]); return true; }
    return false;
  }

  /* ── media drops ────────────────────────────────────────── */
  function dropTarget(x: number, y: number, kind: string) {
    const area: TrackArea = kind === 'audio' ? 'audio' : 'video';
    const { video, audio } = counts();
    const lane = Math.max(0, Math.min(area === 'video' ? video - 1 : audio - 1, laneIndexAt(area, y)));
    const fps = Math.max(1, Number(api.project.get().fps) || 30);
    return { area, lane, at: Math.max(0, Math.round(host.x2t(Math.max(x, T.gut)) * fps) / fps) };
  }
  function setDrop(next: any) {
    const prev = T.trackDrop;
    if (!next && !prev) return;
    if (next && prev && next.at === prev.at && next.lane === prev.lane && next.area === prev.area && next.name === prev.name) return;
    T.trackDrop = next;
    host.invalidate('timeline');
  }
  /** After a drop lands at the top of the stack, lift it to the hovered track. */
  function placeOnTrack(before: Set<string>, lane: number) {
    const added = api.project.get().layers.find((layer: any) => !before.has(layer.id) && !layer.group);
    // Imported subtitles get a caption track of their own instead.
    if (!added || lane <= 0 || added.type === 'captions') return;
    if ((layout().byId.get(added.id)?.lane ?? 0) >= lane) return;
    api.edit.mutate('Place clip on track', () => { (added as any).track = lane; api.anim.touch(); }, { origin: 'timeline' });
  }

  return {
    draw, hover, down, doubleClick, contentHeight, laneAt, laneY, setDrop, dropTarget, placeOnTrack, keysIn,
    layerAt(x: number, y: number) { const hit = hitItem(x, y); return hit ? layerOf(hit.item.id) : null; },
    /** The property row under a point in an unfolded track, for menus. */
    rowAt(x: number, y: number) { return drawerAt(x, y)?.row ?? null; },
    /** True while any track is unfolded: its readouts follow the playhead. */
    unfolded() { return drawers().size > 0; },
    layout,
    rowHeight: rowH,
  };
}
