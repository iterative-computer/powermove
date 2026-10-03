/* Track timeline: the Premiere-style presentation of the layer stack.

   Video tracks stack upward from V1, audio tracks downward from A1, and clips
   that never overlap share a track. The layer stack remains the source of
   truth; see track-layout.ts for how lanes and stack order stay in step. */
import type { PowermoveAPI } from 'powermove';
import {
  layoutTracks, planTrackMove,
  type TrackArea, type TrackItem, type TrackLayer, type TrackLayout, type TrackLayoutOptions, type TrackMovePlan,
} from './track-layout';
import type { EdgeSnapProbe } from './timeline';

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
export const TRACK_GUTTER = 196;

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
  fui(): string;
  fmono(): string;
  niceStep(pps: number): number;
  icoEye(c: CanvasRenderingContext2D, x: number, y: number, on: boolean): void;
  icoLock(c: CanvasRenderingContext2D, x: number, y: number, on: boolean): void;
  icoSpeaker(c: CanvasRenderingContext2D, x: number, y: number, on: boolean): void;
  visible(layer: any): boolean;
  beginDrag(event: PointerEvent, options: any): any;
  invalidate(what?: string): void;
  edgeSnapping(event?: any): boolean;
  layerEdgeSnapper(ids: string[]): {
    resolve(probes: EdgeSnapProbe[], requested: number): { delta: number; snapped: boolean };
    release(requested: number): { delta: number; snapped: boolean };
  };
  trim(event: PointerEvent, side: 'in' | 'out'): void;
}

interface Lane { area: TrackArea; lane: number; y: number }
interface TrackDrag { plan: TrackMovePlan; layout: TrackLayout; moving: Set<string>; video: number; audio: number }

export function createTrackView(host: TrackViewHost) {
  const { api, T } = host;
  const rowH = () => Math.max(34, Math.round(T.row) + 6);

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
    };
  }
  function laneY(area: TrackArea, lane: number) {
    const { video } = counts();
    const index = area === 'video' ? video - 1 - lane : video + lane;
    return T.ruler - T.scrollY + index * rowH() + (area === 'audio' ? DIVIDER : 0);
  }
  function lanes(): Lane[] {
    const { video, audio } = counts();
    const result: Lane[] = [];
    for (let lane = video - 1; lane >= 0; lane--) result.push({ area: 'video', lane, y: laneY('video', lane) });
    for (let lane = 0; lane < audio; lane++) result.push({ area: 'audio', lane, y: laneY('audio', lane) });
    return result;
  }
  function contentHeight() {
    const { video, audio } = counts();
    return (video + audio) * rowH() + DIVIDER;
  }
  function laneAt(y: number): Lane | null {
    if (y < T.ruler) return null;
    return lanes().find(lane => y >= lane.y && y < lane.y + rowH()) ?? null;
  }
  /** Nearest lane in an area, used while dragging past the divider. */
  function laneIndexAt(area: TrackArea, y: number) {
    const { video } = counts();
    const index = (y + T.scrollY - T.ruler - (area === 'audio' ? DIVIDER : 0)) / rowH();
    return area === 'video' ? video - 1 - Math.floor(index) : Math.floor(index) - video;
  }
  function itemsIn(area: TrackArea, lane: number) {
    return layout().items.filter(item => item.area === area && item.lane === lane);
  }
  function hitItem(x: number, y: number): { item: TrackItem; edge: 'in' | 'out' | null } | null {
    const lane = laneAt(y);
    if (!lane || x < T.gut) return null;
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
  const layerOf = (id: string) => api.model.layer(id);

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
      if (lane.y + h < T.ruler || lane.y > H) continue;
      if (T.trackHover && T.trackHover.area === lane.area && T.trackHover.lane === lane.lane) {
        c.fillStyle = ink.over; c.globalAlpha = .7; c.fillRect(T.gut, lane.y, W - T.gut, h); c.globalAlpha = 1;
      }
      c.strokeStyle = ink.grid; c.beginPath(); c.moveTo(T.gut, Math.round(lane.y + h) - .5); c.lineTo(W, Math.round(lane.y + h) - .5); c.stroke();
    }
    drawDivider(c, T.gut, W);

    // Expanded groups: a quiet band behind their members, edged in the bar's tint.
    for (const band of current.bands) {
      const group = layerOf(band.id);
      if (!group) continue;
      const pal = host.clipPalette(group);
      const top = laneY(band.area, band.laneEnd - 1), bottom = laneY(band.area, band.laneStart) + h;
      const x0 = host.t2x(band.from), x1 = host.t2x(band.from + band.dur);
      if (x1 < T.gut || x0 > W || bottom < T.ruler || top > H) continue;
      c.fillStyle = pal.primary; c.globalAlpha = .1;
      c.fillRect(x0, top, x1 - x0, bottom - top);
      c.globalAlpha = .55;
      c.fillRect(x0, top, 2, bottom - top - 1);
      c.globalAlpha = 1;
    }

    for (const item of current.items) {
      const layer = layerOf(item.id);
      if (!layer) continue;
      const y = laneY(item.area, item.lane);
      if (y + h < T.ruler || y > H) continue;
      const from = planned?.get(item.id)?.from ?? layer.from;
      if (item.kind === 'group') {
        const collapsed = api.uiState.getGroupCollapsed(layer);
        host.drawClip(c, { ...layer, from: item.from, dur: item.dur, name: `${collapsed ? '▸' : '▾'}  ${layer.name}` }, y, h);
      } else {
        if (drag?.moving.has(item.id)) c.globalAlpha = .85;
        host.drawClip(c, { ...layer, from }, y, h);
        c.globalAlpha = 1;
      }
    }
    drawDropGhost(c, W, H);
    c.restore();
    drawHeaders(c, W, H);
  }

  function drawDivider(c: CanvasRenderingContext2D, left: number, right: number) {
    const theme = host.theme();
    const y = laneY('audio', 0) - DIVIDER;
    c.fillStyle = theme.panel; c.fillRect(left, y, right - left, DIVIDER);
    c.strokeStyle = theme.line; c.beginPath();
    c.moveTo(left, y + .5); c.lineTo(right, y + .5);
    c.moveTo(left, y + DIVIDER - .5); c.lineTo(right, y + DIVIDER - .5); c.stroke();
  }

  function laneState(area: TrackArea, lane: number) {
    const layers = itemsIn(area, lane).map(item => layerOf(item.id)).filter((layer): layer is NonNullable<typeof layer> => !!layer);
    return {
      layers,
      visible: !layers.length || layers.some(layer => host.visible(layer)),
      locked: layers.length > 0 && layers.every(layer => layer.lock),
      selected: layers.some(layer => api.selection.layers().includes(layer.id)),
    };
  }

  function drawHeaders(c: CanvasRenderingContext2D, _W: number, H: number) {
    const theme = host.theme(), ink = host.ink, h = rowH();
    c.fillStyle = theme.panel; c.fillRect(0, T.ruler, T.gut, H - T.ruler);
    c.save(); c.beginPath(); c.rect(0, T.ruler, T.gut, H - T.ruler); c.clip();
    c.textBaseline = 'middle';
    for (const lane of lanes()) {
      if (lane.y + h < T.ruler || lane.y > H) continue;
      const state = laneState(lane.area, lane.lane);
      const cy = lane.y + h / 2;
      const hovered = T.trackHover && T.trackHover.area === lane.area && T.trackHover.lane === lane.lane;
      if (state.selected) { c.fillStyle = ink.over2; c.fillRect(0, lane.y, T.gut, h); }
      else if (hovered) { c.fillStyle = ink.over; c.fillRect(0, lane.y, T.gut, h); }
      c.strokeStyle = ink.grid; c.beginPath(); c.moveTo(0, Math.round(lane.y + h) - .5); c.lineTo(T.gut, Math.round(lane.y + h) - .5); c.stroke();
      host.icoLock(c, 16, cy, state.locked);
      // Track name chip, Premiere's "V1" / "A1".
      const label = `${lane.area === 'video' ? 'V' : 'A'}${lane.lane + 1}`;
      c.font = '500 11px ' + host.fmono();
      const chipW = Math.max(28, c.measureText(label).width + 12);
      c.fillStyle = state.selected ? host.rgba(theme.accent, .18) : ink.over2;
      host.roundRect(c, 30, cy - 9, chipW, 18, 4); c.fill();
      c.fillStyle = state.selected ? theme.accent : theme.tx2;
      c.fillText(label, 36, cy + .5);
      if (state.layers.length) {
        c.font = '400 11px ' + host.fui(); c.fillStyle = theme.tx3;
        const names = state.layers.length === 1 ? state.layers[0]!.name : `${state.layers.length} clips`;
        host.clipText(c, names, 38 + chipW, cy + .5, T.gut - 38 - chipW - 34);
      }
      if (lane.area === 'video') host.icoEye(c, T.gut - 18, cy, state.visible);
      else host.icoSpeaker(c, T.gut - 18, cy, state.visible);
    }
    drawDivider(c, 0, T.gut);
    c.restore();
    c.strokeStyle = theme.line; c.beginPath(); c.moveTo(T.gut - .5, T.ruler); c.lineTo(T.gut - .5, H); c.stroke();
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

  /* ── gestures ───────────────────────────────────────────── */
  function down(event: PointerEvent, x: number, y: number) {
    if (x < T.gut) return headerDown(event, x, y);
    const hit = hitItem(x, y);
    if (!hit) return marquee(event);
    const layer = layerOf(hit.item.id);
    if (!layer) return;
    if (hit.item.kind === 'group' && x - Math.max(host.t2x(hit.item.from), T.gut) < 18) {
      api.uiState.setGroupCollapsed(layer, !api.uiState.getGroupCollapsed(layer));
      host.invalidate('timeline');
      return;
    }
    if (!selectForPointer(layer.id, event)) return;
    if (hit.edge && !layer.lock) return host.trim(event, hit.edge);
    return moveClips(event);
  }

  function headerDown(_event: PointerEvent, x: number, y: number) {
    const lane = laneAt(y);
    if (!lane) return;
    const state = laneState(lane.area, lane.lane);
    if (!state.layers.length) return;
    if (x < 26) {
      api.edit.apply(state.layers.map((layer: any) => ({ type: 'set_layer', target: layer.id, patch: { locked: !state.locked } })),
        { label: state.locked ? 'Unlock track' : 'Lock track', origin: 'timeline' });
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
        const rows = Math.round(dy / rowH());
        plan = planTrackMove(source, layoutOptions, { ids, dt, lanes: { video: -rows, audio: rows } });
        const preview = layoutTracks(plan.layers, layoutOptions);
        T.trackDrag = {
          plan, layout: preview, moving,
          video: Math.max(start.video, preview.video + 1), audio: Math.max(start.audio, preview.audio + 1),
        } satisfies TrackDrag;
        const rect = T.cv.getBoundingClientRect();
        T.trackHover = { area: startArea, lane: laneIndexAt(startArea, ev.clientY - rect.top) };
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

  function hover(x: number, y: number): { cursor: string; title: string } {
    const lane = laneAt(y);
    const next = lane ? { area: lane.area, lane: lane.lane } : null;
    if (!T.trackDrag && (next?.area !== T.trackHover?.area || next?.lane !== T.trackHover?.lane)) {
      T.trackHover = next; host.invalidate('timeline');
    }
    if (!lane) return { cursor: 'default', title: '' };
    if (x < T.gut) {
      const state = laneState(lane.area, lane.lane);
      if (!state.layers.length) return { cursor: 'default', title: '' };
      if (x < 26) return { cursor: 'pointer', title: state.locked ? 'Unlock track' : 'Lock track' };
      if (x >= T.gut - 30) return { cursor: 'pointer', title: lane.area === 'audio' ? 'Mute track' : 'Toggle track output' };
      return { cursor: 'pointer', title: 'Select all clips on this track' };
    }
    const hit = hitItem(x, y);
    if (!hit) return { cursor: 'default', title: '' };
    const layer = layerOf(hit.item.id);
    if (hit.item.kind === 'group' && x - Math.max(host.t2x(hit.item.from), T.gut) < 18) {
      return { cursor: 'pointer', title: api.uiState.getGroupCollapsed(layer!) ? 'Expand group' : 'Collapse group' };
    }
    if (hit.edge && !layer?.lock) return { cursor: 'ew-resize', title: '' };
    return { cursor: 'grab', title: layer?.name ?? '' };
  }

  function doubleClick(x: number, y: number) {
    const hit = hitItem(x, y);
    const layer = hit && layerOf(hit.item.id);
    if (!layer) return false;
    if (layer.type === 'group') {
      api.uiState.setGroupCollapsed(layer, !api.uiState.getGroupCollapsed(layer));
      host.invalidate('timeline');
      return true;
    }
    if (layer.type === 'precomp' && layer.d?.comp) { api.commands.run('openComposition', layer.d.comp); return true; }
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
    if (!added || lane <= 0) return;
    if ((layout().byId.get(added.id)?.lane ?? 0) >= lane) return;
    api.edit.mutate('Place clip on track', () => { (added as any).track = lane; api.anim.touch(); }, { origin: 'timeline' });
  }

  return {
    draw, hover, down, doubleClick, contentHeight, laneAt, laneY, setDrop, dropTarget, placeOnTrack,
    layerAt(x: number, y: number) { const hit = hitItem(x, y); return hit ? layerOf(hit.item.id) : null; },
    layout,
    rowHeight: rowH,
  };
}
