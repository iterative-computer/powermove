/* Caption cues on a timeline lane. Shared by the layer timeline (a captions
   layer's strip) and the track timeline (the caption tracks above V1).

   A captions layer is one clip that holds many cues. Its strip draws the cues
   as blocks carrying their text; pointer gestures act on cues: click selects,
   drag moves (never across a neighbour), edges trim (neighbours stay put),
   double-click edits the text in place or adds a cue in a gap. Every change
   is an `edit_captions` command, so Undo and provenance work as for any edit.
   Only the cues in view are visited (binary search), so a lane with thousands
   of cues paints in constant time. */
import type { CaptionCue, CaptionSelection, CaptionsService, PowermoveAPI } from 'powermove';
import { MIN_CUE_DURATION, cueRange, moveLimits } from 'powermove';
import { createDirectionalSnapper } from './directional-snap';

export interface CaptionTrackHost {
  api: PowermoveAPI;
  T: any;
  t2x(time: number): number;
  x2t(x: number): number;
  theme(): any;
  roundRect(c: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number): void;
  clipText(c: CanvasRenderingContext2D, text: string, x: number, y: number, max: number): void;
  clipPalette(layer: any): { body: string; primary: string; foreground: string; ring: string };
  mix(color: string, toward: [number, number, number], amount: number): string;
  fui(): string;
  beginDrag(event: PointerEvent, options: any): any;
  invalidate(what?: string): void;
  edgeSnapping(event?: any): boolean;
  /** The element inline editors are placed in (the canvas wrap). */
  wrap(): HTMLElement | null;
  /** Register teardown with the canvas binding. */
  cleanup(dispose: () => void): void;
}

export interface CueHit { cue: CaptionCue; index: number; edge: 'start' | 'end' | null }

const EDGE = 5;
const SNAP_RELEASE_PX = 15;
const cuesOf = (layer: any): CaptionCue[] => Array.isArray(layer?.d?.cues) ? layer.d.cues : [];

export function createCaptionTrack(host: CaptionTrackHost) {
  const { api, T } = host;
  const service = () => api.services.get<CaptionsService>('captions');
  /* Without the host service (tests, forks) cue selection stays local. */
  let local: CaptionSelection = { layerId: null, cues: [] };
  let version = 0;
  const selection = (): CaptionSelection => service()?.selection() ?? local;
  const select = (layerId: string | null, cues: string[]) => {
    const host = service();
    if (host) host.select(layerId, cues);
    else { local = { layerId, cues }; if (layerId) api.selection.select([layerId]); }
    version++;
    hostInvalidate();
  };
  const hostInvalidate = () => host.invalidate('timeline');
  const unsubscribe = service()?.onChange(() => { version++; hostInvalidate(); });
  if (unsubscribe) host.cleanup(unsubscribe);
  const selectedIn = (layer: any) => {
    const current = selection();
    return current.layerId === layer.id ? new Set(current.cues) : new Set<string>();
  };

  /* ── drawing ────────────────────────────────────────────── */
  /** Cue blocks of a captions layer inside [y, y + h). */
  function draw(c: CanvasRenderingContext2D, layer: any, y: number, h: number, W: number) {
    const cues = cuesOf(layer);
    if (!cues.length) return;
    const left = Math.max(T.gut, host.t2x(layer.from)), right = Math.min(W, host.t2x(layer.from + layer.dur));
    if (right <= left) return;
    const [first, end] = cueRange(cues, host.x2t(left) - layer.from, host.x2t(right) - layer.from);
    if (first >= end) return;
    const theme = host.theme(), pal = host.clipPalette(layer);
    const dark = document.documentElement.dataset.theme === 'dark';
    const selected = selectedIn(layer);
    const top = y + 4, height = Math.max(6, h - 8);
    const fill = host.mix(pal.body, dark ? [255, 255, 255] : [0, 0, 0], dark ? .14 : .08);
    const seam = host.mix(pal.body, [0, 0, 0], dark ? .4 : .2);
    c.save();
    c.beginPath(); c.rect(left, y, right - left, h); c.clip();
    c.font = '500 11px ' + host.fui();
    c.textBaseline = 'middle';
    for (let index = first; index < end; index++) {
      const cue = cues[index]!;
      const x0 = host.t2x(layer.from + cue.start), x1 = host.t2x(layer.from + cue.end);
      const w = Math.max(1, x1 - x0 - 1);
      if (w < 3) { c.fillStyle = pal.primary; c.fillRect(x0, top, Math.max(1, w), height); continue; }
      host.roundRect(c, x0 + .5, top, w, height, 3);
      c.fillStyle = fill; c.fill();
      if (selected.has(cue.id)) {
        c.save(); c.clip(); c.strokeStyle = theme.accent; c.lineWidth = 4; c.stroke(); c.restore();
      } else {
        c.strokeStyle = seam; c.lineWidth = 1; c.stroke();
      }
      if (w > 22 && height >= 12) {
        const lx = Math.max(x0, left) + 6, max = Math.min(x1, right) - lx - 5;
        c.fillStyle = pal.foreground;
        if (max > 8) host.clipText(c, cue.text.replace(/\n/g, ' '), lx, top + height / 2 + .5, max);
      }
    }
    c.restore();
  }

  /* ── hit testing ────────────────────────────────────────── */
  function hit(layer: any, x: number): CueHit | null {
    const cues = cuesOf(layer);
    if (!cues.length || x < T.gut) return null;
    const t = host.x2t(x) - layer.from, reach = EDGE / Math.max(1e-6, T.pps);
    const [first, end] = cueRange(cues, t - reach, t + reach);
    let best: CueHit | null = null, distance = Infinity;
    for (let index = first; index < end; index++) {
      const cue = cues[index]!;
      const x0 = host.t2x(layer.from + cue.start), x1 = host.t2x(layer.from + cue.end);
      if (x < x0 - EDGE || x > x1 + EDGE) continue;
      const wide = x1 - x0 > EDGE * 3;
      const edge = wide && Math.abs(x - x0) <= EDGE ? 'start' : wide && Math.abs(x - x1) <= EDGE ? 'end' : null;
      const d = x >= x0 && x <= x1 ? 0 : Math.min(Math.abs(x - x0), Math.abs(x - x1));
      if (d < distance || (d === distance && edge)) { best = { cue, index, edge }; distance = d; }
    }
    return best;
  }

  function hover(layer: any, x: number): { cursor: string; title: string } | null {
    const found = hit(layer, x);
    if (!found) return null;
    if (found.edge && !layer.lock) return { cursor: 'ew-resize', title: '' };
    return { cursor: layer.lock ? 'default' : 'grab', title: found.cue.text };
  }

  /* ── snapping ───────────────────────────────────────────── */
  function snapper(layer: any, moving: Set<string>) {
    const project = api.project.get();
    const fixed: Array<{ time: number; edge?: 'in' | 'out'; weight?: number }> = [{ time: 0 }, { time: project.dur }];
    for (const time of project.work || []) fixed.push({ time });
    for (const marker of project.markers || []) fixed.push({ time: marker.t });
    for (const other of project.layers) {
      if (other.id === layer.id || !(other.dur > 0)) continue;
      fixed.push({ time: other.from }, { time: other.from + other.dur });
    }
    for (const cue of cuesOf(layer)) {
      if (moving.has(cue.id)) continue;
      fixed.push({ time: layer.from + cue.start, edge: 'out' }, { time: layer.from + cue.end, edge: 'in' });
    }
    const latch = createDirectionalSnapper();
    return (probes: Array<{ time: number; edge: 'in' | 'out' }>, requested: number, event: any) => {
      const frame = 1 / Math.max(1, project.fps);
      const head = api.transport.time();
      const resolved = latch.resolve(probes, requested,
        [...fixed, { time: head, weight: 1.5 }, { time: head + frame, edge: 'out', weight: 1.5 }],
        SNAP_RELEASE_PX / Math.max(1, T.pps), host.edgeSnapping(event));
      T.snapGuide = resolved.lock?.target ?? null;
      return { delta: resolved.delta, snapped: !!resolved.lock };
    };
  }

  /* ── gestures ───────────────────────────────────────────── */
  /** Pointer down on a captions strip. Returns false when no cue was hit. */
  function down(event: PointerEvent, layer: any, x: number): boolean {
    const found = hit(layer, x);
    if (!found) return false;
    T.keySelectionActive = false;
    api.selection.set({ keys: [] });
    const current = selectedIn(layer);
    const additive = event.shiftKey || event.metaKey || event.ctrlKey;
    if (additive) {
      const next = current.has(found.cue.id) ? [...current].filter(id => id !== found.cue.id) : [...current, found.cue.id];
      select(layer.id, next);
      return true;
    }
    if (!current.has(found.cue.id)) select(layer.id, [found.cue.id]);
    if (layer.lock) return true;
    if (found.edge) trimCue(event, layer, found.cue, found.edge);
    else moveCues(event, layer);
    return true;
  }

  const fps = () => Math.max(1, Number(api.project.get().fps) || 30);
  const frameRound = (time: number) => Math.round(time * fps()) / fps();

  function moveCues(event: PointerEvent, layer: any) {
    const ids = [...selectedIn(layer)];
    const cues = cuesOf(layer).filter(cue => ids.includes(cue.id)).map(cue => ({ ...cue }));
    if (!cues.length) return;
    const [min, max] = moveLimits(cuesOf(layer), ids);
    const snap = snapper(layer, new Set(ids));
    const probes = cues.flatMap(cue => [
      { time: layer.from + cue.start, edge: 'in' as const }, { time: layer.from + cue.end, edge: 'out' as const },
    ]);
    let active = false, began = false;
    const finish = (commit: boolean) => {
      T.snapGuide = null;
      if (began) { if (commit) api.edit.commit(ids.length === 1 ? 'Move caption' : 'Move captions'); else api.edit.cancel(); }
      host.invalidate();
    };
    host.beginDrag(event, {
      cursor: 'grabbing',
      move: (dx: number, _dy: number, ev: PointerEvent) => {
        if (!active && Math.abs(dx) < 3) return;
        active = true;
        const { delta, snapped } = snap(probes, dx / T.pps, ev);
        let dt = snapped ? delta : frameRound(cues[0]!.start + layer.from + delta) - (cues[0]!.start + layer.from);
        dt = Math.max(min, Math.min(max, dt));
        if (!began) { api.edit.begin('Move caption', { origin: 'timeline' }); began = true; }
        api.edit.dispatch({ type: 'edit_captions', target: layer.id, op: 'update',
          cues: cues.map(cue => ({ id: cue.id, start: layer.from + cue.start + dt, end: layer.from + cue.end + dt })) });
        host.invalidate();
      },
      up: () => finish(true),
      cancel: () => finish(false),
    });
  }

  function trimCue(event: PointerEvent, layer: any, cue: CaptionCue, edge: 'start' | 'end') {
    const all = cuesOf(layer), index = all.findIndex(item => item.id === cue.id);
    const previous = all[index - 1], next = all[index + 1];
    const lower = edge === 'start' ? (previous?.end ?? 0) : cue.start + MIN_CUE_DURATION;
    const upper = edge === 'start' ? cue.end - MIN_CUE_DURATION : (next?.start ?? Infinity);
    const origin = edge === 'start' ? cue.start : cue.end;
    const snap = snapper(layer, new Set([cue.id]));
    const probes = [{ time: layer.from + origin, edge: edge === 'start' ? 'in' as const : 'out' as const }];
    let began = false;
    const finish = (commit: boolean) => {
      T.snapGuide = null;
      if (began) { if (commit) api.edit.commit('Trim caption'); else api.edit.cancel(); }
      host.invalidate();
    };
    host.beginDrag(event, {
      cursor: 'ew-resize',
      move: (dx: number, _dy: number, ev: PointerEvent) => {
        const { delta, snapped } = snap(probes, dx / T.pps, ev);
        let time = origin + delta;
        if (!snapped) time = frameRound(layer.from + time) - layer.from;
        time = Math.max(lower, Math.min(upper, time));
        if (!began) { api.edit.begin('Trim caption', { origin: 'timeline' }); began = true; }
        api.edit.dispatch({ type: 'edit_captions', target: layer.id, op: 'update', cues: [{ id: cue.id, [edge]: layer.from + time }] });
        host.invalidate();
      },
      up: () => finish(true),
      cancel: () => finish(false),
    });
  }

  /* ── inline text editing ────────────────────────────────── */
  let editor: HTMLTextAreaElement | null = null;
  function closeEditor() { const current = editor; editor = null; current?.remove(); }
  host.cleanup(closeEditor);

  /** Edit a cue's text over its block. `rowTop`/`rowHeight` place the field. */
  function edit(layer: any, cueId: string, rowTop: number, rowHeight: number) {
    const cue = cuesOf(layer).find(item => item.id === cueId);
    const wrap = host.wrap();
    if (!cue || !wrap || layer.lock) return false;
    closeEditor();
    const x0 = Math.max(T.gut + 2, host.t2x(layer.from + cue.start));
    const width = Math.max(180, Math.min(420, host.t2x(layer.from + cue.end) - x0));
    const field = document.createElement('textarea');
    field.className = 'tl-caption-editor';
    field.value = cue.text;
    field.rows = 1;
    field.setAttribute('aria-label', 'Caption text');
    field.style.left = `${Math.min(x0, Math.max(T.gut + 2, wrap.clientWidth - width - 8))}px`;
    field.style.top = `${rowTop + 3}px`;
    field.style.width = `${width}px`;
    field.style.height = `${Math.max(20, rowHeight - 6)}px`;
    wrap.appendChild(field);
    editor = field;
    field.focus(); field.select();
    let done = false;
    const finish = (save: boolean) => {
      if (done) return;
      done = true;
      const text = field.value;
      closeEditor();
      if (save && text !== cue.text) {
        api.edit.apply({ type: 'edit_captions', target: layer.id, op: 'update', cues: [{ id: cue.id, text }] }, { label: 'Edit caption', origin: 'timeline' });
      }
      host.invalidate();
    };
    field.addEventListener('blur', () => finish(true));
    field.addEventListener('keydown', ev => {
      ev.stopPropagation();
      if (ev.key === 'Enter' && !ev.shiftKey) { ev.preventDefault(); finish(true); }
      else if (ev.key === 'Escape') { ev.preventDefault(); finish(false); }
    });
    return true;
  }

  /** Double-click: edit the cue under the pointer, or add one in a gap. */
  function doubleClick(layer: any, x: number, rowTop: number, rowHeight: number): boolean {
    const found = hit(layer, x);
    if (found) { select(layer.id, [found.cue.id]); return edit(layer, found.cue.id, rowTop, rowHeight); }
    const time = host.x2t(x);
    if (layer.lock || time < layer.from || time > layer.from + layer.dur) return false;
    const cues = cuesOf(layer), local = time - layer.from;
    const next = cues.find(cue => cue.start > local);
    const start = frameRound(time), end = Math.min(start + 2, next ? layer.from + next.start : Infinity, layer.from + layer.dur);
    if (end - start < MIN_CUE_DURATION) return false;
    const before = new Set(cues.map(cue => cue.id));
    const result = api.edit.apply({ type: 'edit_captions', target: layer.id, op: 'insert', cues: [{ start, end, text: 'Caption' }] }, { label: 'Add caption', origin: 'timeline' });
    if (result?.ok === false) return false;
    const added = cuesOf(api.model.layer(layer.id)).find(cue => !before.has(cue.id));
    if (!added) return false;
    select(layer.id, [added.id]);
    return edit(api.model.layer(layer.id), added.id, rowTop, rowHeight);
  }

  return {
    draw, hit, hover, down, doubleClick, edit, select, selection,
    /** Changes whenever the cue selection does, for paint caches. */
    version: () => version,
    editing: () => !!editor,
  };
}

export type CaptionTrack = ReturnType<typeof createCaptionTrack>;
