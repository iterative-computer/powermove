/* Ported from js/ui/viewer.js — behavior-preserving. */
/* TODO(extensions): `menus.contribute('viewer:context', …)` has no host here.
   src/extensions/viewer/viewer.ts never opens a context menu — the canvas
   has no `contextmenu` listener — so there is no single obvious place to append
   contributions. Wire it when the viewer grows a right-click menu of its own.
   (`timeline:context` and `layer:context` are hosted in extensions/timeline;
   `panel:context` in layout/menu.ts; `titlebar:right` in shell/Titlebar.svelte.) */
export const viewerPanelOptions = {
  title: 'Composition', flush: true, noscroll: true, headless: true, hideMoveHandle: true,
} as const;

export type ResizeHandle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w';

type Point = { x: number; y: number };
type Bounds = { x0: number; y0: number; x1: number; y1: number; w: number; h: number };
type Corner = readonly [number, number];
type AffineMatrix = readonly [number, number, number, number, number, number];

const HANDLE_NAMES: readonly ResizeHandle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];
const RESIZE_CURSORS = ['ew-resize', 'nwse-resize', 'ns-resize', 'nesw-resize'] as const;
const HANDLE_CURSOR_ANGLE: Record<ResizeHandle, number> = {
  e: 0, se: 45, s: 90, sw: 135, w: 180, nw: 225, n: 270, ne: 315,
};
const ROTATE_CURSOR = `url('data:image/svg+xml,${encodeURIComponent(
  "<svg xmlns='http://www.w3.org/2000/svg' width='26' height='26' viewBox='0 0 26 26'>" +
  "<path d='M13 4a9 9 0 1 0 9 9' fill='none' stroke='black' stroke-width='4.5' stroke-linecap='round'/>" +
  "<path d='M13 4a9 9 0 1 0 9 9' fill='none' stroke='white' stroke-width='2' stroke-linecap='round'/>" +
  "<path d='M13 0l-.3 8.4 6-3.9z' fill='white' stroke='black' stroke-width='1.4' stroke-linejoin='round'/>" +
  '</svg>',
)}') 13 13, auto`;

function applyMatrix(m: AffineMatrix, point: Point): Point {
  return {
    x: m[0] * point.x + m[2] * point.y + m[4],
    y: m[1] * point.x + m[3] * point.y + m[5],
  };
}

function invertPoint(m: AffineMatrix, point: Point): Point | null {
  const det = m[0] * m[3] - m[1] * m[2];
  if (Math.abs(det) < 1e-9) return null;
  const dx = point.x - m[4], dy = point.y - m[5];
  return {
    x: (dx * m[3] - dy * m[2]) / det,
    y: (dy * m[0] - dx * m[1]) / det,
  };
}

function pointInBounds(bounds: Bounds, corner: Corner): Point {
  return {
    x: bounds.x0 + bounds.w * corner[0],
    y: bounds.y0 + bounds.h * corner[1],
  };
}

/** Figma-style resize cursor projected through the selection's on-screen angle. */
export function resizeCursorForHandle(matrix: AffineMatrix, handle: ResizeHandle): string {
  const axis = Math.atan2(matrix[1], matrix[0]) * 180 / Math.PI;
  let angle = (HANDLE_CURSOR_ANGLE[handle] + axis) % 180;
  if (angle < 0) angle += 180;
  return RESIZE_CURSORS[Math.round(angle / 45) % RESIZE_CURSORS.length]!;
}

/** Canvas-handle policy mirrors Figma/Motioner semantics: typography and
    intrinsic media never stretch through ordinary selection handles. */
export function resizeLocksAspect(type: string, shiftKey = false): boolean {
  return shiftKey || type === 'text' || type === 'image' || type === 'video';
}

export interface ResizeCalculation {
  scaleX: number;
  scaleY: number;
  pivotLocal: Point;
}

/**
 * Resolve a resize in the transform's original local coordinate system.
 * The caller keeps pivotLocal fixed in world space after applying the scales.
 */
export function calculateResize(
  bounds: Bounds,
  corner: Corner,
  pointerLocal: Point,
  grabOffset: Point,
  startScale: Point,
  options: { fromCenter?: boolean; lockAspect?: boolean; minScale?: number } = {},
): ResizeCalculation {
  const activeX = corner[0] !== .5;
  const activeY = corner[1] !== .5;
  const center = pointInBounds(bounds, [.5, .5]);
  const pivotCorner: Corner = options.fromCenter
    ? [.5, .5]
    : [activeX ? 1 - corner[0] : .5, activeY ? 1 - corner[1] : .5];
  const pivotLocal = pointInBounds(bounds, pivotCorner);
  const handleLocal = pointInBounds(bounds, corner);
  const grabbed = { x: pointerLocal.x - grabOffset.x, y: pointerLocal.y - grabOffset.y };
  const dx = handleLocal.x - pivotLocal.x;
  const dy = handleLocal.y - pivotLocal.y;
  let factorX = activeX && Math.abs(dx) > 1e-9 ? (grabbed.x - pivotLocal.x) / dx : 1;
  let factorY = activeY && Math.abs(dy) > 1e-9 ? (grabbed.y - pivotLocal.y) / dy : 1;

  if (options.lockAspect) {
    let uniform: number;
    if (activeX && activeY) {
      /* Orthogonally project onto the original corner diagonal. This behaves
         continuously for both growth and shrinkage instead of choosing an axis. */
      uniform = ((grabbed.x - pivotLocal.x) * dx + (grabbed.y - pivotLocal.y) * dy) /
        Math.max(1e-9, dx * dx + dy * dy);
    } else uniform = activeX ? factorX : factorY;
    factorX = uniform;
    factorY = uniform;
  }

  const minimum = Math.max(.001, options.minScale ?? .1);
  const scaleX = Math.max(minimum, startScale.x * factorX);
  const scaleY = Math.max(minimum, startScale.y * factorY);
  return { scaleX, scaleY, pivotLocal: options.fromCenter ? center : pivotLocal };
}

/** Compatibility entry point for registry-based tests and legacy installers. */
export function install(PM: any): void {
  createViewerRuntime(PM);
}

/** Geometry hit test for selection-preserving direct manipulation. The active
    selection owns a drag inside its visible transform box even when a
    full-frame layer is stacked above it. */
export function layerContainsPoint(PM: any, L: any, x: number, y: number, T: number): boolean {
  const b = PM.GL.bounds(L, T); if (!b) return false;
  const m = PM.worldMatrix(L, T);
  const det = m[0] * m[3] - m[1] * m[2]; if (Math.abs(det) < 1e-9) return false;
  const dx = x - m[4], dy = y - m[5];
  const lx = (dx * m[3] - dy * m[2]) / det;
  const ly = (dy * m[0] - dx * m[1]) / det;
  return lx >= b.x0 && lx <= b.x1 && ly >= b.y0 && ly <= b.y1;
}

/** Install the legacy viewer controller against the host registry. */
export function createViewerRuntime(PM: any): any {
if (PM.Viewer?.attach) return PM.Viewer;
const clamp = PM.clamp;

const V: any = { zoom: 1, fit: true, pan: [0, 0], el: null, ov: null, octx: null, inner: null, guides: null };
PM.Viewer = V;

V.attach = (stage: HTMLElement) => {
    if (V.stage === stage) return;
    if (V.stage) {
      /* The WebGL context is bound to the original #gl canvas and cannot move
         hosts; persist panels re-parent the same element in production, so a
         second attach only happens under HMR — keep the old wiring alive
         instead of leaving a dead panel behind a thrown build. */
      console.warn('[viewer] attach ignored: already bound to a stage (HMR requires a full reload)');
      return;
    }
    const inner = stage.querySelector<HTMLElement>(':scope > #stage-inner');
    const gl = inner?.querySelector<HTMLCanvasElement>(':scope > #gl');
    const ov = stage.querySelector<HTMLCanvasElement>('#overlay');
    if (!inner || !gl || !ov) throw new Error('Viewer host is missing the legacy canvas skeleton');
    V.el = gl; V.ov = ov; V.octx = ov.getContext('2d'); V.inner = inner; V.stage = stage;
    if (!PM.GL.gl) PM.GL.init(gl);
    bindStage(stage, inner);
    window.requestAnimationFrame(() => V.layout());
    new window.ResizeObserver(() => V.layout()).observe(stage);
};

/* ── layout / sizing ───────────────────────────────────── */
V.layout = () => {
  if (!V.el || !V.stage) return;
  const p = PM.proj;
  const r = V.stage.getBoundingClientRect();
  if (r.width < 8 || r.height < 8) return;
  V._sw = r.width; V._sh = r.height;
  const pad = 8;
  let z = V.fit ? Math.min((r.width - pad * 2) / p.w, (r.height - pad * 2) / p.h) : V.zoom;
  z = clamp(z, .02, 8);
  V.shown = z;
  const dw = Math.round(p.w * z), dh = Math.round(p.h * z);
  V.inner.style.width = dw + 'px'; V.inner.style.height = dh + 'px';
  /* apply pan (AE Hand tool) */
  if (!V.fit && (V.pan[0] || V.pan[1])) V.inner.style.transform = `translate(${V.pan[0]}px, ${V.pan[1]}px)`;
  else V.inner.style.transform = '';
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const rw = Math.max(2, Math.round(p.w * PM.quality)), rh = Math.max(2, Math.round(p.h * PM.quality));
  PM.GL.resize(rw, rh);
  V.ov.width = Math.round(r.width * dpr); V.ov.height = Math.round(r.height * dpr);
  V.ov.style.width = r.width + 'px'; V.ov.style.height = r.height + 'px';
  PM.invalidate();
};
PM.bus.on('quality', () => V.layout());
PM.bus.on('project', () => V.layout());
window.addEventListener('resize', () => V.layout());
PM.bus.on('layout:applied', () => V.layout());

/* comp px <-> screen px */
const toComp = (e: any): [number, number] => {
  const r = V.inner.getBoundingClientRect();
  return [(e.clientX - r.left) / V.shown, (e.clientY - r.top) / V.shown];
};

/* ── overlay drawing ───────────────────────────────────── */
PM.bus.on('overlay', drawOverlay);
PM.bus.on('sel', () => PM.invalidate('render'));

function drawOverlay() {
  const c = V.octx; if (!c) return;
  /* Self-heal after layout rebuilds: if the stage no longer matches the size
     V.layout() last computed, re-run layout before drawing overlays. */
  if (V.stage) {
    const r = V.stage.getBoundingClientRect();
    if (r.width >= 8 && r.height >= 8 &&
        (Math.abs(r.width - (V._sw || 0)) > .5 || Math.abs(r.height - (V._sh || 0)) > .5)) {
      V.layout();
    }
  }
  const p = PM.proj, dpr = V.ov.width / V.stage.getBoundingClientRect().width;
  c.setTransform(1, 0, 0, 1, 0, 0);
  c.clearRect(0, 0, V.ov.width, V.ov.height);
  const S = V.shown * dpr;
  const frame = V.inner.getBoundingClientRect(), stage = V.stage.getBoundingClientRect();
  c.save(); c.translate((frame.left - stage.left) * dpr, (frame.top - stage.top) * dpr); c.scale(S, S);
  c.lineWidth = 1 / S;

  drawSnapGuides(c, S, p);

  const sels = PM.selLayers().filter((L: any) => PM.active(L, PM.time));
  for (const L of sels) {
    const b = PM.GL.bounds(L, PM.time);
    if (!b) continue;
    const m = PM.worldMatrix(L, PM.time);
    const corners = ([[0, 0], [1, 0], [1, 1], [0, 1]] as const).map((corner) =>
      applyMatrix(m, pointInBounds(b, corner)));
    c.strokeStyle = 'rgba(255,107,26,.95)';
    c.lineWidth = 1.4 / Math.max(.02, V.shown);
    c.beginPath();
    c.moveTo(corners[0]!.x, corners[0]!.y);
    corners.slice(1).forEach((point) => c.lineTo(point.x, point.y));
    c.closePath();
    c.stroke();
    /* Handles are UI chrome, not layer content: keep them square and a
       constant screen size under zoom, rotation, skew, and nonuniform scale. */
    const hs = 5 / Math.max(.02, V.shown);
    c.fillStyle = '#fff';
    c.strokeStyle = 'rgba(0,0,0,.45)';
    c.lineWidth = 1 / Math.max(.02, V.shown);
    HANDLES.forEach(([hx, hy]: any) => {
      const point = applyMatrix(m, pointInBounds(b, [hx, hy]));
      c.fillRect(point.x - hs, point.y - hs, hs * 2, hs * 2);
      c.strokeRect(point.x - hs, point.y - hs, hs * 2, hs * 2);
    });
  }
  c.restore();
}
const HANDLES: readonly (readonly [number, number])[] = [[0, 0], [.5, 0], [1, 0], [1, .5], [1, 1], [.5, 1], [0, 1], [0, .5]];
const MOVE_DRAG_THRESHOLD_PX = 3;

function drawSnapGuides(c: any, S: any, p: any) {
  const guides = V.guides;
  if (!guides || (guides.x == null && guides.y == null)) return;
  c.save();
  c.strokeStyle = 'rgba(74,164,255,.96)';
  c.lineWidth = 1 / S;
  c.setLineDash([5 / S, 3 / S]);
  c.beginPath();
  if (guides.x != null) { c.moveTo(guides.x, 0); c.lineTo(guides.x, p.h); }
  if (guides.y != null) { c.moveTo(0, guides.y); c.lineTo(p.w, guides.y); }
  c.stroke();
  c.restore();
}

/* Screen-space alignment helpers. Bounds and guide coordinates stay in
   composition pixels, while the tolerance is converted from a constant
   on-screen distance by startMove(). */
function snapAxis(movingMarks: any, targetMarks: any, threshold: any) {
  let best = null;
  for (const moving of movingMarks || []) {
    const movingValue = typeof moving === 'number' ? moving : moving.value;
    if (!Number.isFinite(movingValue)) continue;
    for (const target of targetMarks || []) {
      const targetValue = typeof target === 'number' ? target : target.value;
      if (!Number.isFinite(targetValue)) continue;
      const delta = targetValue - movingValue;
      const distance = Math.abs(delta);
      if (distance > threshold || (best && distance >= best.distance - 1e-9)) continue;
      best = { delta, value: targetValue, distance };
    }
  }
  return best;
}

function worldBounds(L: any, T: any) {
  const b = PM.GL.bounds(L, T); if (!b) return null;
  const m = PM.worldMatrix(L, T);
  const points = [[b.x0, b.y0], [b.x1, b.y0], [b.x1, b.y1], [b.x0, b.y1]]
    .map(([x, y]: any) => [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]);
  const xs = points.map((point: any) => point[0]), ys = points.map((point: any) => point[1]);
  const x0 = Math.min(...xs), x1 = Math.max(...xs), y0 = Math.min(...ys), y1 = Math.max(...ys);
  return { x0, x1, y0, y1, cx: (x0 + x1) / 2, cy: (y0 + y1) / 2 };
}

function unionBounds(layers: any, T: any) {
  const bounds = layers.map((L: any) => worldBounds(L, T)).filter(Boolean);
  if (!bounds.length) return null;
  const x0 = Math.min(...bounds.map((b: any) => b.x0)), x1 = Math.max(...bounds.map((b: any) => b.x1));
  const y0 = Math.min(...bounds.map((b: any) => b.y0)), y1 = Math.max(...bounds.map((b: any) => b.y1));
  return { x0, x1, y0, y1, cx: (x0 + x1) / 2, cy: (y0 + y1) / 2 };
}

function hasSelectedAncestor(L: any, selectedIds: any) {
  let parentId = L.parent, guard = 0;
  while (parentId && guard++ < 256) {
    if (selectedIds.has(parentId)) return true;
    const parent = PM.L(parentId);
    parentId = parent && parent.parent;
  }
  return false;
}

function snapshotSnapTargets(T: any, selectedIds: any) {
  const x = [{ value: 0 }, { value: PM.proj.w / 2 }, { value: PM.proj.w }];
  const y = [{ value: 0 }, { value: PM.proj.h / 2 }, { value: PM.proj.h }];
  const soloOn = PM.proj.layers.some((L: any) => L.solo);
  for (const L of PM.proj.layers) {
    if (selectedIds.has(L.id) || hasSelectedAncestor(L, selectedIds) || !PM.active(L, T) || (soloOn && !L.solo)) continue;
    const b = worldBounds(L, T); if (!b) continue;
    x.push({ value: b.x0 }, { value: b.cx }, { value: b.x1 });
    y.push({ value: b.y0 }, { value: b.cy }, { value: b.y1 });
  }
  return { x, y };
}

function alignmentSnap(bounds: any, targets: any, threshold: any, axes: any = { x: true, y: true }) {
  if (!bounds) return { dx: 0, dy: 0, x: null, y: null };
  const x = axes.x ? snapAxis([bounds.x0, bounds.cx, bounds.x1], targets.x, threshold) : null;
  const y = axes.y ? snapAxis([bounds.y0, bounds.cy, bounds.y1], targets.y, threshold) : null;
  return { dx: x ? x.delta : 0, dy: y ? y.delta : 0, x, y };
}

function guideValue(guide: any) {
  return guide ? guide.value : null;
}

function alignmentGuideChanged(previous: any, next: any) {
  if (!next || (next.x == null && next.y == null)) return false;
  if (!previous) return true;
  return (next.x != null && next.x !== previous.x) ||
    (next.y != null && next.y !== previous.y);
}

function passedMoveDragThreshold(dx: any, dy: any) {
  return Math.hypot(dx, dy) >= MOVE_DRAG_THRESHOLD_PX;
}

function worldDeltaToLocal(L: any, T: any, dx: any, dy: any) {
  if (!L.parent) return [dx, dy];
  const parent = PM.L(L.parent); if (!parent) return [dx, dy];
  const m = PM.worldMatrix(parent, T), det = m[0] * m[3] - m[1] * m[2];
  if (Math.abs(det) < 1e-9) return [0, 0];
  return [(dx * m[3] - dy * m[2]) / det, (dy * m[0] - dx * m[1]) / det];
}

Object.assign(V, {
  snapAxis, worldBounds, unionBounds, snapshotSnapTargets, alignmentSnap,
  alignmentGuideChanged, passedMoveDragThreshold, calculateResize, resizeCursorForHandle,
  resizeLocksAspect,
  layerContainsPoint: (L: any, x: any, y: any, T: any) => layerContainsPoint(PM, L, x, y, T),
});

/* ── direct manipulation ───────────────────────────────── */
function bindStage(stage: any, inner: any) {
  stage.addEventListener('pointerdown', onDown);
  inner.addEventListener('pointermove', updateStageCursor);
  inner.addEventListener('pointerleave', () => { inner.style.cursor = ''; });
  stage.addEventListener('wheel', (e: any) => {
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      V.fit = false;
      V.zoom = clamp((V.shown || 1) * (1 - e.deltaY * .0035), .05, 8);
      V.layout();
    }
  }, { passive: false });
  inner.addEventListener('dblclick', (e: any) => {
    const [x, y] = toComp(e);
    const L = PM.GL.pick(x, y, PM.time);
    if (L && L.type === 'text') PM.Inspector?.focusText?.(L);
  });
}

function cursorForHit(L: any, hit: any, T: any) {
  if (hit.rotate) return ROTATE_CURSOR;
  return resizeCursorForHandle(PM.worldMatrix(L, T), hit.handle);
}

function updateStageCursor(e: any) {
  if (!V.inner) return;
  if (PM.tool === 'hand' || e.metaKey) { V.inner.style.cursor = 'grab'; return; }
  if (PM.tool === 'zoom') { V.inner.style.cursor = e.shiftKey ? 'zoom-out' : 'zoom-in'; return; }
  if (PM.tool && PM.tool !== 'select') { V.inner.style.cursor = 'crosshair'; return; }
  const [x, y] = toComp(e), T = PM.time;
  const selected = PM.firstSel();
  if (selected && !selected.lock && PM.active(selected, T)) {
    const hit = handleAt(selected, x, y, T);
    if (hit) { V.inner.style.cursor = cursorForHit(selected, hit, T); return; }
    if (layerContainsPoint(PM, selected, x, y, T)) { V.inner.style.cursor = 'move'; return; }
  }
  V.inner.style.cursor = 'default';
}

function startPan(e: any) {
  const start = [V.pan[0], V.pan[1]];
  V.fit = false;
  PM.drag(e, {
    cursor: 'grabbing',
    move: (dx: any, dy: any) => { V.pan = [start[0] + dx, start[1] + dy]; V.layout(); },
  });
}

function onDown(e: any) {
  if (e.button !== 0) return;

  /* AE-style tool override: Hand pans, Zoom zooms. Space/⌘ temporarily = Hand. */
  const tool = (PM.tool === 'hand' || PM.tool === 'zoom') ? PM.tool : (e.metaKey ? 'hand' : 'select');
  if (tool === 'hand') return startPan(e);
  if (tool === 'zoom') {
    const dir = e.shiftKey ? -1 : 1;
    V.fit = false;
    V.zoom = clamp((V.shown || 1) * (dir > 0 ? 1.25 : .8), .05, 8);
    V.layout();
    return;
  }

  const [x, y] = toComp(e);
  const T = PM.time;

  /* handle grab on the selected layer */
  const sel = PM.firstSel();
  if (sel && !sel.lock && PM.active(sel, T)) {
    const hit = handleAt(sel, x, y, T);
    if (hit) return startTransform(e, sel, hit, T);
  }
  const L = PM.GL.pick(x, y, T);
  const selected = PM.selLayers().filter((layer: any) =>
    !layer.lock && PM.active(layer, T) && PM.TYPE_META?.[layer.type]?.pickable !== false);
  if (selected.some((layer: any) => layerContainsPoint(PM, layer, x, y, T))) {
    const selectedIds = new Set(selected.map((layer: any) => layer.id));
    return startMove(e, selected, T, {
      click: () => {
        if (L && !selectedIds.has(L.id)) PM.selectLayers(L.id, e.shiftKey);
      },
    });
  }
  if (!L) { if (!e.shiftKey) PM.selectLayers([]); return; }
  PM.selectLayers(L.id, e.shiftKey);
  startMove(e, PM.selLayers().filter((l: any) => !l.lock), T);
}

function handleAt(L: any, x: any, y: any, T: any) {
  const b = PM.GL.bounds(L, T); if (!b) return null;
  const m = PM.worldMatrix(L, T);
  const tolerance = 9 / Math.max(.02, V.shown);
  for (let i = 0; i < HANDLES.length; i++) {
    const corner = HANDLES[i]!;
    const point = applyMatrix(m, pointInBounds(b, corner));
    if (Math.hypot(x - point.x, y - point.y) <= tolerance) {
      return { i, corner, handle: HANDLE_NAMES[i]! };
    }
  }
  /* Just outside a corner rotates; the selection interior remains a move hit. */
  if (!layerContainsPoint(PM, L, x, y, T)) {
    for (const corner of [[0, 0], [1, 0], [1, 1], [0, 1]] as any) {
      const point = applyMatrix(m, pointInBounds(b, corner));
      if (Math.hypot(x - point.x, y - point.y) <= tolerance * 2.4) return { rotate: true };
    }
  }
  return null;
}

function startMove(e: any, layers: any, T: any, options: any = {}) {
  if (!layers.length) return;
  const start = layers.map((L: any) => ({ L, x: PM.ev(L, 'position.x', T), y: PM.ev(L, 'position.y', T) }));
  const selectedIds = new Set(layers.map((L: any) => L.id));
  const correctionRoots = start.filter((s: any) => !hasSelectedAncestor(s.L, selectedIds));
  const targets = snapshotSnapTargets(T, selectedIds);
  const clearGuides = () => { V.guides = null; PM.invalidate('render'); };
  PM.Edit.begin('Move layer', { origin: 'canvas' });
  let moved = false;
  PM.drag(e, {
    move: (dx: any, dy: any, ev: any) => {
      if (!moved && !passedMoveDragThreshold(dx, dy)) return;
      moved = true;
      let ddx = dx / V.shown, ddy = dy / V.shown;
      const axes = { x: true, y: true };
      if (ev.shiftKey) {
        if (Math.abs(ddx) > Math.abs(ddy)) { ddy = 0; axes.y = false; }
        else { ddx = 0; axes.x = false; }
      }
      start.forEach((s: any) => {
        setOrKey(s.L, 'position.x', s.x + ddx, T);
        setOrKey(s.L, 'position.y', s.y + ddy, T);
      });
      let snap: any = { dx: 0, dy: 0, x: null, y: null };
      if (PM.snap) snap = alignmentSnap(unionBounds(layers, T), targets, 8 / Math.max(.02, V.shown), axes);
      correctionRoots.forEach((s: any) => {
        const [cx, cy] = worldDeltaToLocal(s.L, T, snap.dx, snap.dy);
        if (cx) setOrKey(s.L, 'position.x', s.x + ddx + cx, T);
        if (cy) setOrKey(s.L, 'position.y', s.y + ddy + cy, T);
      });
      const nextGuides = PM.snap && (snap.x || snap.y)
        ? { x: guideValue(snap.x), y: guideValue(snap.y) }
        : null;
      if (alignmentGuideChanged(V.guides, nextGuides)) {
        window.powermove?.haptic.alignment();
      }
      V.guides = nextGuides;
      PM.invalidate();
    },
    up: () => {
      clearGuides();
      if (moved) PM.Edit.commit('Move layer');
      else { PM.Edit.cancel(); options.click?.(); }
      PM.Inspector?.refresh?.();
    },
    cancel: () => { clearGuides(); PM.Edit.cancel(); PM.Inspector?.refresh?.(); },
  });
}

function startTransform(e: any, L: any, hit: any, T: any) {
  const b = PM.GL.bounds(L, T);
  if (!b) return;
  const s0 = {
    sx: PM.ev(L, 'scale.x', T), sy: PM.ev(L, 'scale.y', T),
    x: PM.ev(L, 'position.x', T), y: PM.ev(L, 'position.y', T),
    r: PM.ev(L, 'rotation', T),
  };
  const textSize0 = L.type === 'text' ? Math.max(4, Number(L.d?.size) || 4) : null;
  const m = [...PM.worldMatrix(L, T)] as [number, number, number, number, number, number];
  const cx = m[4], cy = m[5];
  const r0 = V.inner.getBoundingClientRect();
  const a0 = Math.atan2((e.clientY - r0.top) / V.shown - cy, (e.clientX - r0.left) / V.shown - cx);
  const pointerStart = { x: (e.clientX - r0.left) / V.shown, y: (e.clientY - r0.top) / V.shown };
  const pointerStartLocal = invertPoint(m, pointerStart);
  const handleLocal = hit.corner ? pointInBounds(b, hit.corner) : null;
  const grabOffset = pointerStartLocal && handleLocal
    ? { x: pointerStartLocal.x - handleLocal.x, y: pointerStartLocal.y - handleLocal.y }
    : { x: 0, y: 0 };
  const applied = { sx: s0.sx, sy: s0.sy, x: s0.x, y: s0.y };
  let appliedTextSize = textSize0;
  const applyValue = (path: string, value: number, key: keyof typeof applied) => {
    if (Math.abs(value - applied[key]) < .0005) return;
    setOrKey(L, path, value, T);
    applied[key] = value;
  };
  const applyTextSize = (value: number) => {
    if (appliedTextSize == null || Math.abs(value - appliedTextSize) < .0005) return;
    PM.Edit.dispatch({ type: 'set_content', target: L.id, patch: { size: value } });
    appliedTextSize = value;
  };
  PM.Edit.begin(hit.rotate ? 'Rotate layer' : textSize0 == null ? 'Scale layer' : 'Resize text', { origin: 'canvas' });
  let moved = false;
  PM.drag(e, {
    cursor: cursorForHit(L, hit, T),
    move: (dx: any, dy: any, ev: any) => {
      moved = true;
      if (hit.rotate) {
        const a = Math.atan2((ev.clientY - r0.top) / V.shown - cy, (ev.clientX - r0.left) / V.shown - cx);
        let deg = s0.r + (a - a0) * 180 / Math.PI;
        if (ev.shiftKey) deg = Math.round(deg / 15) * 15;
        setOrKey(L, 'rotation', PM.round(deg, 2), T);
      } else {
        const pointerWorld = { x: (ev.clientX - r0.left) / V.shown, y: (ev.clientY - r0.top) / V.shown };
        const pointerLocal = invertPoint(m, pointerWorld);
        if (!pointerLocal) return;
        const next = calculateResize(b, hit.corner, pointerLocal, grabOffset,
          { x: s0.sx, y: s0.sy }, {
            fromCenter: ev.altKey,
            lockAspect: resizeLocksAspect(L.type, ev.shiftKey),
          });
        const fixedWorld = applyMatrix(m, next.pivotLocal);

        let renderedPivotLocal = next.pivotLocal;
        if (textSize0 != null) {
          /* Text owns its visual size through the typography model. Canvas
             resizing changes Size; Scale X/Y remain explicit transform
             overrides instead of becoming an accidental second font size. */
          const factor = next.scaleX / Math.max(.001, s0.sx);
          applyTextSize(PM.round(Math.max(4, textSize0 * factor), 2));
          const resizedBounds = PM.GL.bounds(L, T);
          if (resizedBounds) {
            const pivotRatio: Corner = [
              b.w ? (next.pivotLocal.x - b.x0) / b.w : .5,
              b.h ? (next.pivotLocal.y - b.y0) / b.h : .5,
            ];
            renderedPivotLocal = pointInBounds(resizedBounds, pivotRatio);
          }
        } else {
          applyValue('scale.x', PM.round(next.scaleX, 3), 'sx');
          applyValue('scale.y', PM.round(next.scaleY, 3), 'sy');
        }
        /* Every move starts from the gesture snapshot. This prevents the
           previous correction from accumulating as the pointer changes. */
        applyValue('position.x', s0.x, 'x');
        applyValue('position.y', s0.y, 'y');
        const resizedMatrix = PM.worldMatrix(L, T);
        const renderedPivot = applyMatrix(resizedMatrix, renderedPivotLocal);
        const [positionDx, positionDy] = worldDeltaToLocal(
          L, T, fixedWorld.x - renderedPivot.x, fixedWorld.y - renderedPivot.y,
        );
        applyValue('position.x', PM.round(s0.x + positionDx, 3), 'x');
        applyValue('position.y', PM.round(s0.y + positionDy, 3), 'y');
      }
      PM.invalidate();
    },
    up: () => { moved ? PM.Edit.commit() : PM.Edit.cancel(); PM.Inspector?.refresh?.(); },
    cancel: () => { PM.Edit.cancel(); PM.Inspector?.refresh?.(); },
  });
}

/** Write a value: sets a keyframe when the channel is animated, otherwise the static value. */
function setOrKey(L: any, key: any, v: any, T: any) {
  return PM.Edit.dispatch({
    type: 'set_property', target: L.id, path: key, value: v, time: T,
    mode: 'auto', preserveHandEdits: false, markIntent: 'human',
  });
}
PM.setOrKey = setOrKey;
return V;
}
