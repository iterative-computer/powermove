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
type WorldBounds = Bounds & { cx: number; cy: number };
type Corner = readonly [number, number];
type AffineMatrix = readonly [number, number, number, number, number, number];
export type LinearMatrix = readonly [number, number, number, number];

export interface SelectionGeometry {
  mode: 'single' | 'common';
  layers: any[];
  roots: any[];
  bounds: Bounds;
  matrix: AffineMatrix;
  corners: Point[];
  handles: Record<ResizeHandle, Point>;
  pivotWorld: Point;
  transformable: boolean;
}

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
const VIEWER_RUNTIME_TOKEN = Symbol('powermove.viewer.runtime');

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

function invertDirection(m: AffineMatrix, direction: Point): Point | null {
  const det = m[0] * m[3] - m[1] * m[2];
  if (Math.abs(det) < 1e-9) return null;
  return {
    x: (direction.x * m[3] - direction.y * m[2]) / det,
    y: (direction.y * m[0] - direction.x * m[1]) / det,
  };
}

/** Resolve an absolute local rotation whose local X axis points in the desired
    world direction, even through a skewed/non-uniform parent transform. */
export function localRotationForWorldDirection(parent: AffineMatrix | null, worldDirection: Point): number | null {
  const local = parent ? invertDirection(parent, worldDirection) : worldDirection;
  if (!local || Math.hypot(local.x, local.y) < 1e-9) return null;
  return Math.atan2(local.y, local.x) * 180 / Math.PI;
}

export function multiplyLinear(left: LinearMatrix, right: LinearMatrix): LinearMatrix {
  return [
    left[0] * right[0] + left[2] * right[1],
    left[1] * right[0] + left[3] * right[1],
    left[0] * right[2] + left[2] * right[3],
    left[1] * right[2] + left[3] * right[3],
  ];
}

export function rotateLinear(matrix: LinearMatrix, degrees: number): LinearMatrix {
  const angle = degrees * Math.PI / 180, cos = Math.cos(angle), sin = Math.sin(angle);
  return [
    cos * matrix[0] - sin * matrix[1], sin * matrix[0] + cos * matrix[1],
    cos * matrix[2] - sin * matrix[3], sin * matrix[2] + cos * matrix[3],
  ];
}

export interface DecomposedLocalLinear {
  rotation: number;
  scaleX: number;
  scaleY: number;
  skew: number;
}

export function composeLocalLinear(transform: DecomposedLocalLinear): LinearMatrix {
  const rotation = transform.rotation * Math.PI / 180;
  const skew = Math.tan(transform.skew * Math.PI / 180);
  const cos = Math.cos(rotation), sin = Math.sin(rotation);
  return [
    cos * transform.scaleX,
    sin * transform.scaleX,
    transform.scaleY * (cos * skew - sin),
    transform.scaleY * (sin * skew + cos),
  ];
}

/** Solve the local R·Kx·S channels needed to produce a complete desired world
    linear transform. Returning null for singular matrices avoids NaN/∞ edits. */
export function solveLocalTransformForWorldLinear(
  parent: LinearMatrix | null, desiredWorld: LinearMatrix,
  preferred: { scaleX?: number; rotation?: number } = {},
): DecomposedLocalLinear | null {
  let local = desiredWorld;
  if (parent) {
    const det = parent[0] * parent[3] - parent[1] * parent[2];
    if (!Number.isFinite(det) || Math.abs(det) < 1e-8) return null;
    local = [
      (parent[3] * desiredWorld[0] - parent[2] * desiredWorld[1]) / det,
      (-parent[1] * desiredWorld[0] + parent[0] * desiredWorld[1]) / det,
      (parent[3] * desiredWorld[2] - parent[2] * desiredWorld[3]) / det,
      (-parent[1] * desiredWorld[2] + parent[0] * desiredWorld[3]) / det,
    ];
  }
  const scaleX = Math.hypot(local[0], local[1]);
  if (!Number.isFinite(scaleX) || scaleX < 1e-8) return null;
  const cos = local[0] / scaleX, sin = local[1] / scaleX;
  const skewTimesScaleY = cos * local[2] + sin * local[3];
  const scaleY = -sin * local[2] + cos * local[3];
  if (!Number.isFinite(scaleY) || Math.abs(scaleY) < 1e-8) return null;
  let skewRadians = Math.atan2(skewTimesScaleY, scaleY);
  if (skewRadians > Math.PI / 2) skewRadians -= Math.PI;
  else if (skewRadians < -Math.PI / 2) skewRadians += Math.PI;
  let rotation = Math.atan2(local[1], local[0]) * 180 / Math.PI;
  let signedScaleX = scaleX, signedScaleY = scaleY;
  if ((preferred.scaleX ?? 1) < 0) {
    rotation += 180;
    signedScaleX = -signedScaleX;
    signedScaleY = -signedScaleY;
  }
  if (Number.isFinite(preferred.rotation)) {
    rotation += Math.round((preferred.rotation! - rotation) / 360) * 360;
  }
  const result = {
    rotation,
    scaleX: signedScaleX, scaleY: signedScaleY,
    skew: skewRadians * 180 / Math.PI,
  };
  const check = composeLocalLinear(result);
  const magnitude = Math.max(1, ...local.map(Math.abs));
  if (check.some((value, index) => Math.abs(value - local[index]!) > magnitude * 1e-7)) return null;
  return result;
}

function pointInBounds(bounds: Bounds, corner: Corner): Point {
  return {
    x: bounds.x0 + bounds.w * corner[0],
    y: bounds.y0 + bounds.h * corner[1],
  };
}

function midpoint(a: Point, b: Point): Point {
  return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
}

function handlesFromCorners(corners: Point[]): Record<ResizeHandle, Point> {
  const [nw, ne, se, sw] = corners;
  return {
    nw: nw!, n: midpoint(nw!, ne!), ne: ne!, e: midpoint(ne!, se!),
    se: se!, s: midpoint(se!, sw!), sw: sw!, w: midpoint(sw!, nw!),
  };
}

function layerIsTransformable(PM: any, layer: any, T: number): boolean {
  return !!layer && !layer.lock && PM.active(layer, T) && PM.TYPE_META?.[layer.type]?.pickable !== false;
}

function visibleSelectionLayers(PM: any, selected: any[], T: number): any[] {
  return selected.filter((layer) => !!layer && PM.active(layer, T) && PM.GL.bounds(layer, T));
}

/** Selected parents own their descendants. Returning only roots prevents a
    parent and selected child from receiving the same transform twice. */
export function selectionTransformRoots(PM: any, layers: any[], T: number): any[] {
  const eligible = layers.filter((layer) => layerIsTransformable(PM, layer, T));
  const ids = new Set(eligible.map((layer) => layer.id));
  return eligible.filter((layer) => {
    let parentId = layer.parent, guard = 0;
    while (parentId && guard++ < 256) {
      if (ids.has(parentId)) return false;
      const parent = PM.L(parentId);
      parentId = parent?.parent;
    }
    return true;
  });
}

export function layerWorldBounds(PM: any, layer: any, T: number): WorldBounds | null {
  const bounds = PM.GL.bounds(layer, T); if (!bounds) return null;
  const matrix = PM.worldMatrix(layer, T);
  const points = [
    { x: bounds.x0, y: bounds.y0 }, { x: bounds.x1, y: bounds.y0 },
    { x: bounds.x1, y: bounds.y1 }, { x: bounds.x0, y: bounds.y1 },
  ].map((point) => applyMatrix(matrix, point));
  const xs = points.map((point) => point.x), ys = points.map((point) => point.y);
  const x0 = Math.min(...xs), x1 = Math.max(...xs), y0 = Math.min(...ys), y1 = Math.max(...ys);
  return { x0, x1, y0, y1, w: x1 - x0, h: y1 - y0, cx: (x0 + x1) / 2, cy: (y0 + y1) / 2 };
}

/** The real world-space anchor. Matrix translation is the transformed local
    origin, so it is not the rotation pivot when Anchor X/Y are non-zero. */
export function layerWorldPivot(PM: any, layer: any, T: number): Point {
  return applyMatrix(PM.worldMatrix(layer, T), {
    x: PM.ev(layer, 'anchor.x', T),
    y: PM.ev(layer, 'anchor.y', T),
  });
}

export function transformPointAround(
  point: Point, pivot: Point, scaleX: number, scaleY: number, rotationDegrees = 0,
): Point {
  const x = (point.x - pivot.x) * scaleX;
  const y = (point.y - pivot.y) * scaleY;
  if (!rotationDegrees) return { x: pivot.x + x, y: pivot.y + y };
  const angle = rotationDegrees * Math.PI / 180;
  const cos = Math.cos(angle), sin = Math.sin(angle);
  return { x: pivot.x + x * cos - y * sin, y: pivot.y + x * sin + y * cos };
}

/** One geometry source for overlay drawing, hit-testing, and transforms. */
export function resolveSelectionGeometry(PM: any, selected: any[], T: number): SelectionGeometry | null {
  const layers = visibleSelectionLayers(PM, selected, T);
  if (!layers.length) return null;
  /* A selection is one atomic transform target. Locked or unpickable visual
     members keep their chrome, but disable interaction for the entire box. */
  const transformable = layers.every((layer) => layerIsTransformable(PM, layer, T));
  const roots = transformable ? selectionTransformRoots(PM, layers, T) : [];

  if (layers.length === 1) {
    const layer = layers[0]!;
    const bounds = PM.GL.bounds(layer, T); if (!bounds) return null;
    const matrix = PM.worldMatrix(layer, T) as AffineMatrix;
    const corners = ([[0, 0], [1, 0], [1, 1], [0, 1]] as const)
      .map((corner) => applyMatrix(matrix, pointInBounds(bounds, corner)));
    return {
      mode: 'single', layers, roots, transformable, bounds, matrix, corners,
      handles: handlesFromCorners(corners), pivotWorld: layerWorldPivot(PM, layer, T),
    };
  }

  const measured = layers.map((layer) => layerWorldBounds(PM, layer, T)).filter(Boolean) as WorldBounds[];
  if (!measured.length) return null;
  const x0 = Math.min(...measured.map((bounds) => bounds.x0));
  const x1 = Math.max(...measured.map((bounds) => bounds.x1));
  const y0 = Math.min(...measured.map((bounds) => bounds.y0));
  const y1 = Math.max(...measured.map((bounds) => bounds.y1));
  const bounds = { x0, x1, y0, y1, w: x1 - x0, h: y1 - y0 };
  const corners = [
    { x: x0, y: y0 }, { x: x1, y: y0 }, { x: x1, y: y1 }, { x: x0, y: y1 },
  ];
  return {
    mode: 'common', layers, roots, transformable, bounds, matrix: [1, 0, 0, 1, 0, 0], corners,
    handles: handlesFromCorners(corners), pivotWorld: { x: (x0 + x1) / 2, y: (y0 + y1) / 2 },
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
const existing = PM.Viewer;
if (existing?._runtimeToken === VIEWER_RUNTIME_TOKEN) return existing;
const existingStage = existing?.stage as HTMLElement | undefined;
if (existingStage && typeof existing?._disposeRuntime !== 'function') {
  existing._legacyListenerFence = true;
  /* The legacy runtime did not retain bus disposers. Its overlay callback is
     the one behavioral stale closure; remove only that named viewer handler. */
  const overlayHandlers = PM.bus?.m?.get?.('overlay');
  if (overlayHandlers instanceof Set) {
    for (const handler of overlayHandlers) if (handler?.name === 'drawOverlay') overlayHandlers.delete(handler);
  }
}
/* The pre-disposer runtime's listeners cannot be removed retroactively. Keep
   the capture fence on the retained Viewer object across every later HMR. */
const legacyFence = !!existing?._legacyListenerFence;
existing?._disposeRuntime?.();
const clamp = PM.clamp;

const V: any = existing || { zoom: 1, fit: true, pan: [0, 0], el: null, ov: null, octx: null, inner: null, snapLines: null };
PM.Viewer = V;
V._runtimeToken = VIEWER_RUNTIME_TOKEN;
let disposed = false;
let unbindStage: (() => void) | null = null;
let resizeObserver: ResizeObserver | null = null;
let activeDrag: { cancel(): void } | null = null;
const busOffs: Array<() => void> = [];

const disposeRuntime = () => {
  if (disposed) return;
  disposed = true;
  activeDrag?.cancel(); activeDrag = null;
  unbindStage?.(); unbindStage = null;
  resizeObserver?.disconnect(); resizeObserver = null;
  busOffs.splice(0).forEach((off) => off());
  window.removeEventListener('resize', onWindowResize);
  setStageCursor('');
  if (V._runtimeToken === VIEWER_RUNTIME_TOKEN) V._runtimeToken = null;
};
V._disposeRuntime = disposeRuntime;
V.dispose = disposeRuntime;

const beginDrag = (event: any, options: any) => {
  activeDrag?.cancel();
  let control: any;
  const finish = (kind: 'up' | 'cancel', args: any[]) => {
    if (activeDrag === control) activeDrag = null;
    return options[kind]?.(...args);
  };
  control = PM.drag(event, {
    ...options,
    up: (...args: any[]) => finish('up', args),
    cancel: (...args: any[]) => finish('cancel', args),
  });
  activeDrag = control;
  return control;
};

V.attach = (stage: HTMLElement) => {
    if (disposed || V._boundStage === stage) return V;
    if (V.stage && V.stage !== stage) {
      /* Never replace the live WebGL canvas. Panel rebuilds re-parent V.stage;
         attaching a genuinely different host would orphan the GL context. */
      console.warn('[viewer] attach ignored: a different live WebGL stage already exists');
      return;
    }
    const inner = stage.querySelector<HTMLElement>(':scope > #stage-inner');
    const gl = inner?.querySelector<HTMLCanvasElement>(':scope > #gl');
    const ov = stage.querySelector<HTMLCanvasElement>('#overlay');
    if (!inner || !gl || !ov) throw new Error('Viewer host is missing the legacy canvas skeleton');
    V.el = gl; V.ov = ov; V.octx = ov.getContext('2d'); V.inner = inner; V.stage = stage;
    if (!PM.GL.gl) PM.GL.init(gl);
    unbindStage?.();
    resizeObserver?.disconnect();
    unbindStage = bindStage(stage, inner, legacyFence);
    V._boundStage = stage;
    window.requestAnimationFrame(() => V.layout());
    resizeObserver = new window.ResizeObserver(() => V.layout());
    resizeObserver.observe(stage);
    return V;
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
const onWindowResize = () => V.layout();
window.addEventListener('resize', onWindowResize);
for (const event of ['quality', 'project', 'layout:applied']) {
  const off = PM.bus.on(event, () => V.layout());
  if (typeof off === 'function') busOffs.push(off);
}

/* comp px <-> screen px */
const toComp = (e: any): [number, number] => {
  const r = V.inner.getBoundingClientRect();
  return [(e.clientX - r.left) / V.shown, (e.clientY - r.top) / V.shown];
};

/* ── overlay drawing ───────────────────────────────────── */
for (const [event, handler] of [
  ['overlay', drawOverlay],
  ['sel', () => PM.invalidate('render')],
] as const) {
  const off = PM.bus.on(event, handler);
  if (typeof off === 'function') busOffs.push(off);
}

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

  drawSnapLines(c, S);

  const sels = PM.selLayers().filter((L: any) => PM.active(L, PM.time));
  const selection = resolveSelectionGeometry(PM, sels, PM.time);
  /* Multi-selection keeps light per-layer outlines for orientation, but owns
     exactly one common transform box and one set of controls. */
  if (sels.length > 1 || !selection) for (const L of sels) {
    const b = PM.GL.bounds(L, PM.time);
    if (!b) continue;
    const m = PM.worldMatrix(L, PM.time);
    const corners = ([[0, 0], [1, 0], [1, 1], [0, 1]] as const).map((corner) =>
      applyMatrix(m, pointInBounds(b, corner)));
    c.strokeStyle = 'rgba(255,107,26,.5)';
    c.lineWidth = 1 / Math.max(.02, V.shown);
    c.beginPath();
    c.moveTo(corners[0]!.x, corners[0]!.y);
    corners.slice(1).forEach((point) => c.lineTo(point.x, point.y));
    c.closePath();
    c.stroke();
  }
  if (selection) {
    c.strokeStyle = 'rgba(255,107,26,.95)';
    c.lineWidth = 1.4 / Math.max(.02, V.shown);
    c.beginPath();
    c.moveTo(selection.corners[0]!.x, selection.corners[0]!.y);
    selection.corners.slice(1).forEach((point) => c.lineTo(point.x, point.y));
    c.closePath();
    c.stroke();
    if (!selection.transformable) { c.restore(); return; }
    const rotate = rotationHandlePoint(selection);
    c.beginPath();
    c.moveTo(selection.handles.n.x, selection.handles.n.y);
    c.lineTo(rotate.x, rotate.y);
    c.stroke();
    const rotateRadius = 4 / Math.max(.02, V.shown);
    c.fillStyle = '#fff';
    c.beginPath();
    c.arc(rotate.x, rotate.y, rotateRadius, 0, Math.PI * 2);
    c.fill();
    c.strokeStyle = 'rgba(0,0,0,.45)';
    c.lineWidth = 1 / Math.max(.02, V.shown);
    c.stroke();
    /* Handles are UI chrome, not layer content: keep them square and a
       constant screen size under zoom, rotation, skew, and nonuniform scale. */
    const hs = 5 / Math.max(.02, V.shown);
    c.fillStyle = '#fff';
    c.strokeStyle = 'rgba(0,0,0,.45)';
    c.lineWidth = 1 / Math.max(.02, V.shown);
    HANDLE_NAMES.forEach((handle) => {
      const point = selection.handles[handle];
      c.fillRect(point.x - hs, point.y - hs, hs * 2, hs * 2);
      c.strokeRect(point.x - hs, point.y - hs, hs * 2, hs * 2);
    });
  }
  c.restore();
}
const HANDLES: readonly (readonly [number, number])[] = [[0, 0], [.5, 0], [1, 0], [1, .5], [1, 1], [.5, 1], [0, 1], [0, .5]];
const MOVE_DRAG_THRESHOLD_PX = 3;

function rotationHandlePoint(selection: SelectionGeometry): Point {
  const nw = selection.corners[0]!, ne = selection.corners[1]!;
  const dx = ne.x - nw.x, dy = ne.y - nw.y, length = Math.hypot(dx, dy) || 1;
  const distance = 22 / Math.max(.02, V.shown);
  return {
    x: selection.handles.n.x + dy / length * distance,
    y: selection.handles.n.y - dx / length * distance,
  };
}

function drawSnapLines(c: any, S: any) {
  const lines: SnapLine[] = V.snapLines;
  if (!lines || !lines.length) return;
  const cross = 3 / S;
  c.save();
  c.strokeStyle = SNAP_COLOR;
  c.lineWidth = 1 / S;
  c.beginPath();
  for (const line of lines) { c.moveTo(line.from.x, line.from.y); c.lineTo(line.to.x, line.to.y); }
  c.stroke();
  c.beginPath();
  for (const line of lines) for (const point of [line.from, line.to]) {
    c.moveTo(point.x - cross, point.y - cross); c.lineTo(point.x + cross, point.y + cross);
    c.moveTo(point.x - cross, point.y + cross); c.lineTo(point.x + cross, point.y - cross);
  }
  c.stroke();
  c.restore();
}

/* Alignment snapping. Candidates are points, not bare axis values: the
   corners and center of every layer a gesture can align with, so a snap knows
   which point it landed on and the guide can be drawn from the dragged point
   to the target instead of across the whole composition. Everything stays in
   composition pixels; the threshold is converted from a constant on-screen
   distance by the caller. */
type SnapAxisCandidate = { value: number; point: Point };
type SnapCandidates = { x: SnapAxisCandidate[]; y: SnapAxisCandidate[] };
type SnapLine = { from: Point; to: Point };
type SnapTarget = { offset: number; distance: number; sourcePoint: Point; targetPoint: Point };
type SnapResult = { dx: number; dy: number; lines: SnapLine[] };

const SNAP_COLOR = '#F43535';
/** How close, in CSS pixels, a candidate has to be before a gesture snaps to it. */
const SNAP_DISTANCE = 6;

function snapCandidatesFromPoints(points: Point[]): SnapCandidates {
  return {
    x: points.map((point) => ({ value: point.x, point })),
    y: points.map((point) => ({ value: point.y, point })),
  };
}

/** The four corners of an axis-aligned box plus its center. */
function boxSnapPoints(b: { x0: number; x1: number; y0: number; y1: number }): Point[] {
  return [
    { x: b.x0, y: b.y0 }, { x: b.x1, y: b.y0 }, { x: b.x1, y: b.y1 }, { x: b.x0, y: b.y1 },
    { x: (b.x0 + b.x1) / 2, y: (b.y0 + b.y1) / 2 },
  ];
}

/**
 * The nearest candidate within `threshold` on one axis, ties broken by the
 * shorter guide: of two equally close snaps, the one whose line is shorter is
 * the one the user meant.
 */
function findSnapTarget(
  source: SnapAxisCandidate[], targets: SnapAxisCandidate[], threshold: number, axis: 'x' | 'y',
): SnapTarget | null {
  const cross = (a: Point, b: Point) => (axis === 'x' ? Math.abs(b.y - a.y) : Math.abs(b.x - a.x));
  let best: SnapTarget | null = null;
  for (const candidate of source) {
    for (const target of targets) {
      const offset = target.value - candidate.value;
      const distance = Math.abs(offset);
      if (distance > threshold) continue;
      if (!best || distance < best.distance ||
          (distance === best.distance && cross(candidate.point, target.point) < cross(best.sourcePoint, best.targetPoint))) {
        best = { offset, distance, sourcePoint: candidate.point, targetPoint: target.point };
      }
    }
  }
  return best;
}

/**
 * Snaps a moving box against the candidates on each unlocked axis. Each guide
 * runs from where the dragged point ends up (both axes' corrections applied,
 * so the line meets the box) to the candidate it snapped to.
 */
function snapBox(
  moving: SnapCandidates, candidates: SnapCandidates, threshold: number,
  axes: { x: boolean; y: boolean } = { x: true, y: true },
): SnapResult {
  const xSnap = axes.x ? findSnapTarget(moving.x, candidates.x, threshold, 'x') : null;
  const ySnap = axes.y ? findSnapTarget(moving.y, candidates.y, threshold, 'y') : null;
  const lines: SnapLine[] = [];
  if (xSnap) lines.push({
    from: { x: xSnap.sourcePoint.x + xSnap.offset, y: xSnap.sourcePoint.y + (ySnap?.offset ?? 0) },
    to: xSnap.targetPoint,
  });
  if (ySnap) lines.push({
    from: { x: ySnap.sourcePoint.x + (xSnap?.offset ?? 0), y: ySnap.sourcePoint.y + ySnap.offset },
    to: ySnap.targetPoint,
  });
  return { dx: xSnap?.offset ?? 0, dy: ySnap?.offset ?? 0, lines };
}

/** True when the guides now point somewhere new, which is when the haptic fires. */
function snapLinesChanged(previous: SnapLine[] | null, next: SnapLine[] | null): boolean {
  if (!next || !next.length) return false;
  const key = (lines: SnapLine[]) => lines.map((l) => `${l.to.x},${l.to.y}`).sort().join('|');
  return !previous || key(previous) !== key(next);
}

function worldBounds(L: any, T: any) {
  const bounds = layerWorldBounds(PM, L, T);
  if (!bounds) return null;
  const { x0, x1, y0, y1, cx, cy } = bounds;
  return { x0, x1, y0, y1, cx, cy };
}

function unionBounds(layers: any, T: any) {
  const bounds = layers.map((L: any) => worldBounds(L, T)).filter(Boolean);
  if (!bounds.length) return null;
  const x0 = Math.min(...bounds.map((b: any) => b.x0)), x1 = Math.max(...bounds.map((b: any) => b.x1));
  const y0 = Math.min(...bounds.map((b: any) => b.y0)), y1 = Math.max(...bounds.map((b: any) => b.y1));
  return { x0, x1, y0, y1, cx: (x0 + x1) / 2, cy: (y0 + y1) / 2 };
}

/**
 * The points a gesture can snap to, taken once at dragstart so a layer cannot
 * snap to where it has just been dragged. A single layer snaps within its own
 * frame: its siblings plus its parent. A multi-selection spans frames, so the
 * composition's top level is the only frame they share. The composition
 * itself is always a candidate.
 */
function snapshotSnapCandidates(T: any, selectionLayers: any[]): SnapCandidates {
  const selectedIds = new Set(selectionLayers.map((L: any) => L.id));
  const points: Point[] = boxSnapPoints({ x0: 0, y0: 0, x1: PM.proj.w, y1: PM.proj.h });
  const only = selectionLayers.length === 1 ? selectionLayers[0] : null;
  const parentId = only ? (only.parent || null) : null;
  const visible = (L: any) => PM.active(L, T);
  for (const L of PM.proj.layers) {
    if ((L.parent || null) !== parentId || selectedIds.has(L.id) || !visible(L)) continue;
    const b = worldBounds(L, T); if (b) points.push(...boxSnapPoints(b));
  }
  const parent = parentId ? PM.L(parentId) : null;
  if (parent && visible(parent)) {
    const b = worldBounds(parent, T); if (b) points.push(...boxSnapPoints(b));
  }
  return snapCandidatesFromPoints(points);
}

function passedMoveDragThreshold(dx: any, dy: any) {
  return Math.hypot(dx, dy) >= MOVE_DRAG_THRESHOLD_PX;
}

function worldDeltaToLocal(L: any, T: any, dx: any, dy: any): [number, number] {
  if (!L.parent) return [dx, dy];
  const parent = PM.L(L.parent); if (!parent) return [dx, dy];
  const m = PM.worldMatrix(parent, T), det = m[0] * m[3] - m[1] * m[2];
  if (Math.abs(det) < 1e-9) return [0, 0];
  return [(dx * m[3] - dy * m[2]) / det, (dy * m[0] - dx * m[1]) / det];
}

Object.assign(V, {
  worldBounds, unionBounds, snapshotSnapCandidates, findSnapTarget, snapBox, boxSnapPoints,
  snapCandidatesFromPoints, snapLinesChanged, passedMoveDragThreshold, calculateResize, resizeCursorForHandle,
  resizeLocksAspect,
  resolveSelectionGeometry: (layers: any = PM.selLayers(), T: any = PM.time) =>
    resolveSelectionGeometry(PM, layers, T),
  selectionTransformRoots: (layers: any = PM.selLayers(), T: any = PM.time) =>
    selectionTransformRoots(PM, layers, T),
  layerWorldPivot: (layer: any, T: any = PM.time) => layerWorldPivot(PM, layer, T),
  layerContainsPoint: (L: any, x: any, y: any, T: any) => layerContainsPoint(PM, L, x, y, T),
});

/* ── direct manipulation ───────────────────────────────── */
function bindStage(stage: any, inner: any, fenceLegacyListeners = false): () => void {
  const listeners: Array<[any, string, EventListener, boolean | AddEventListenerOptions | undefined]> = [];
  const listen = (target: any, type: string, handler: EventListener, options?: boolean | AddEventListenerOptions) => {
    target.addEventListener(type, handler, options);
    listeners.push([target, type, handler, options]);
  };
  const guarded = (handler: (event: any) => void) => ((event: any) => {
    if (fenceLegacyListeners) event.stopImmediatePropagation();
    handler(event);
  }) as EventListener;
  const capture = fenceLegacyListeners ? true : undefined;
  listen(stage, 'pointerdown', guarded(onDown), capture);
  listen(stage, 'pointermove', guarded(updateStageCursor), capture);
  listen(stage, 'pointerleave', guarded(() => setStageCursor('')), capture);
  listen(stage, 'wheel', guarded((e: any) => {
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      V.fit = false;
      V.zoom = clamp((V.shown || 1) * (1 - e.deltaY * .0035), .05, 8);
      V.layout();
    }
  }), fenceLegacyListeners ? { capture: true, passive: false } : { passive: false });
  listen(fenceLegacyListeners ? stage : inner, 'dblclick', guarded((e: any) => {
    const [x, y] = toComp(e);
    const L = PM.GL.pick(x, y, PM.time);
    if (L && L.type === 'text') PM.Inspector?.focusText?.(L);
  }), capture);
  /* FX browser drops (effects / transitions). Non-fx drags (OS files) are left
     untouched so the window-level import handler keeps working. */
  const setDropOver = (on: boolean) => stage.classList.toggle('fx-drop-over', on);
  listen(stage, 'dragenter', (e: any) => {
    if (!PM.fxDrop?.hasFxDrag(e.dataTransfer)) return;
    e.preventDefault(); setDropOver(true);
  });
  listen(stage, 'dragover', (e: any) => {
    if (!PM.fxDrop?.hasFxDrag(e.dataTransfer)) return;
    e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; setDropOver(true);
  });
  listen(stage, 'dragleave', (e: any) => {
    if (e.relatedTarget && stage.contains(e.relatedTarget)) return;
    setDropOver(false);
  });
  listen(stage, 'drop', (e: any) => {
    const payload = PM.fxDrop?.readFxDrag(e.dataTransfer);
    setDropOver(false);
    if (!payload) return;
    e.preventDefault(); e.stopPropagation();
    const [x, y] = toComp(e);
    const hit = PM.GL.pick(x, y, PM.time);
    const target = hit || PM.firstSel?.();
    PM.fxDrop.applyFxDrop(payload, target?.id, undefined, PM);
  });
  return () => {
    for (const [target, type, handler, options] of listeners) target.removeEventListener(type, handler, options);
    if (V._boundStage === stage) V._boundStage = null;
  };
}

function setStageCursor(cursor: string) {
  if (V.stage) V.stage.style.cursor = cursor;
  if (V.inner) V.inner.style.cursor = cursor;
}

function cursorForHit(selection: SelectionGeometry, hit: any) {
  if (hit.rotate) return ROTATE_CURSOR;
  return resizeCursorForHandle(selection.matrix, hit.handle);
}

function updateStageCursor(e: any) {
  if (!V.inner) return;
  if (PM.tool === 'hand') { setStageCursor('grab'); return; }
  if (PM.tool === 'zoom') { setStageCursor(e.shiftKey ? 'zoom-out' : 'zoom-in'); return; }
  if (PM.tool && PM.tool !== 'select') { setStageCursor('crosshair'); return; }
  const [x, y] = toComp(e), T = PM.time;
  const selection = resolveSelectionGeometry(PM, PM.selLayers(), T);
  if (selection?.transformable) {
    const hit = handleAt(selection, x, y);
    if (hit) { setStageCursor(cursorForHit(selection, hit)); return; }
    if (pointInSelection(selection, x, y)) { setStageCursor('move'); return; }
  }
  setStageCursor('default');
}

function startPan(e: any) {
  const start = [V.pan[0], V.pan[1]];
  V.fit = false;
  beginDrag(e, {
    cursor: 'grabbing',
    move: (dx: any, dy: any) => { V.pan = [start[0] + dx, start[1] + dy]; V.layout(); },
  });
}

function onDown(e: any) {
  if (e.button !== 0) return;

  /* Explicit tools own the canvas. Command remains available to the active
     transform gesture for Motioner's 45-degree rotation snapping contract. */
  const tool = (PM.tool === 'hand' || PM.tool === 'zoom') ? PM.tool : 'select';
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

  const selection = resolveSelectionGeometry(PM, PM.selLayers(), T);
  if (selection?.transformable) {
    const hit = handleAt(selection, x, y);
    if (hit) return startTransform(e, selection, hit, T);
  }
  if (selection && !selection.transformable && pointInSelection(selection, x, y)) {
    /* Locked mixed selections own their visible geometry defensively. Do not
       let picking collapse the selection and start moving only an unlocked
       subset underneath the same press. */
    e.preventDefault();
    return;
  }
  const L = PM.GL.pick(x, y, T);
  if (selection?.transformable && pointInSelection(selection, x, y)) {
    const selectedIds = new Set(selection.layers.map((layer: any) => layer.id));
    return startMove(e, selection.roots, T, {
      selectionLayers: selection.layers,
      click: () => {
        if (L && !selectedIds.has(L.id)) PM.selectLayers(L.id, e.shiftKey);
        else if (!L && !e.shiftKey) PM.selectLayers([]);
      },
    });
  }
  if (!L) { if (!e.shiftKey) PM.selectLayers([]); return; }
  PM.selectLayers(L.id, e.shiftKey);
  const nextSelection = resolveSelectionGeometry(PM, PM.selLayers(), T);
  if (nextSelection) startMove(e, nextSelection.roots, T, { selectionLayers: nextSelection.layers });
}

function pointInSelection(selection: SelectionGeometry, x: number, y: number): boolean {
  if (selection.mode === 'common') {
    const bounds = selection.bounds;
    return x >= bounds.x0 && x <= bounds.x1 && y >= bounds.y0 && y <= bounds.y1;
  }
  return layerContainsPoint(PM, selection.layers[0], x, y, PM.time);
}

function handleAt(selection: SelectionGeometry, x: any, y: any) {
  const tolerance = 9 / Math.max(.02, V.shown);
  const rotate = rotationHandlePoint(selection);
  if (Math.hypot(x - rotate.x, y - rotate.y) <= tolerance * 1.2) return { rotate: true };
  for (let i = 0; i < HANDLE_NAMES.length; i++) {
    const handle = HANDLE_NAMES[i]!;
    const corner = HANDLES[i]!;
    const point = selection.handles[handle];
    if (Math.hypot(x - point.x, y - point.y) <= tolerance) {
      return { i, corner, handle };
    }
  }
  return null;
}

function startMove(e: any, layers: any, T: any, options: any = {}) {
  if (!layers.length) return;
  const start = layers.map((L: any) => ({ L, x: PM.ev(L, 'position.x', T), y: PM.ev(L, 'position.y', T) }));
  const selectionLayers = options.selectionLayers || layers;
  const candidates = snapshotSnapCandidates(T, selectionLayers);
  const clearGuides = () => { V.snapLines = null; PM.invalidate('render'); };
  PM.Edit.begin(selectionLayers.length > 1 ? 'Move selection' : 'Move layer', { origin: 'canvas' });
  let moved = false;
  beginDrag(e, {
    move: (dx: any, dy: any, ev: any) => {
      if (!moved && !passedMoveDragThreshold(dx, dy)) return;
      moved = true;
      let ddx = dx / V.shown, ddy = dy / V.shown;
      const axes = { x: true, y: true };
      if (ev.shiftKey) {
        if (Math.abs(ddx) > Math.abs(ddy)) { ddy = 0; axes.y = false; }
        else { ddx = 0; axes.x = false; }
      }
      const moveDeltas = new Map<any, [number, number]>();
      start.forEach((s: any) => {
        const delta = worldDeltaToLocal(s.L, T, ddx, ddy);
        moveDeltas.set(s.L, delta);
        setOrKey(s.L, 'position.x', s.x + delta[0], T);
        setOrKey(s.L, 'position.y', s.y + delta[1], T);
      });
      /* Shift enables snapping for the gesture. */
      let snap: SnapResult = { dx: 0, dy: 0, lines: [] };
      const box = ev.shiftKey ? unionBounds(selectionLayers, T) : null;
      if (box) {
        snap = snapBox(snapCandidatesFromPoints(boxSnapPoints(box)), candidates,
          SNAP_DISTANCE / Math.max(.02, V.shown), axes);
      }
      if (snap.dx || snap.dy) start.forEach((s: any) => {
        const [cx, cy] = worldDeltaToLocal(s.L, T, snap.dx, snap.dy);
        const [mx, my] = moveDeltas.get(s.L)!;
        if (cx) setOrKey(s.L, 'position.x', s.x + mx + cx, T);
        if (cy) setOrKey(s.L, 'position.y', s.y + my + cy, T);
      });
      const nextLines = snap.lines.length ? snap.lines : null;
      if (snapLinesChanged(V.snapLines, nextLines)) {
        window.powermove?.haptic.alignment();
      }
      V.snapLines = nextLines;
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

function worldPointToParentLocal(L: any, T: any, point: Point): Point | null {
  if (!L.parent) return point;
  const parent = PM.L(L.parent); if (!parent) return point;
  return invertPoint(PM.worldMatrix(parent, T), point);
}

function parentWorldMatrix(L: any, T: any): AffineMatrix | null {
  if (!L.parent) return null;
  const parent = PM.L(L.parent);
  return parent ? PM.worldMatrix(parent, T) as AffineMatrix : null;
}

function pointerAngleInParentSpace(L: any, T: any, pivotWorld: Point, pointerWorld: Point): number | null {
  const worldDirection = { x: pointerWorld.x - pivotWorld.x, y: pointerWorld.y - pivotWorld.y };
  const parent = parentWorldMatrix(L, T);
  const local = parent ? invertDirection(parent, worldDirection) : worldDirection;
  if (!local || Math.hypot(local.x, local.y) < 1e-9) return null;
  return Math.atan2(local.y, local.x);
}

function pointerComp(e: any): Point {
  const [x, y] = toComp(e);
  return { x, y };
}

function angleDeltaDegrees(next: number, start: number): number {
  let delta = (next - start) * 180 / Math.PI;
  while (delta > 180) delta -= 360;
  while (delta < -180) delta += 360;
  return delta;
}

function startTransform(e: any, selection: SelectionGeometry, hit: any, T: any) {
  if (selection.mode === 'common') return startCommonTransform(e, selection, hit, T);
  return startSingleTransform(e, selection, hit, T);
}

function startSingleTransform(e: any, selection: SelectionGeometry, hit: any, T: any) {
  const L = selection.layers[0];
  const b = PM.GL.bounds(L, T);
  if (!b) return;
  const s0 = {
    sx: PM.ev(L, 'scale.x', T), sy: PM.ev(L, 'scale.y', T),
    x: PM.ev(L, 'position.x', T), y: PM.ev(L, 'position.y', T),
    r: PM.ev(L, 'rotation', T),
  };
  const textSize0 = L.type === 'text' ? Math.max(4, Number(L.d?.size) || 4) : null;
  const m = [...PM.worldMatrix(L, T)] as [number, number, number, number, number, number];
  const pivotWorld = selection.pivotWorld;
  const pointerStart = pointerComp(e);
  const a0 = pointerAngleInParentSpace(L, T, pivotWorld, pointerStart);
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
  beginDrag(e, {
    cursor: cursorForHit(selection, hit),
    move: (dx: any, dy: any, ev: any) => {
      moved = true;
      if (hit.rotate) {
        const pointer = pointerComp(ev);
        const a = pointerAngleInParentSpace(L, T, pivotWorld, pointer);
        if (a == null || a0 == null) return;
        let deg = s0.r + angleDeltaDegrees(a, a0);
        if (ev.metaKey || ev.ctrlKey) deg = Math.round(deg / 45) * 45;
        else if (ev.shiftKey) deg = Math.round(deg / 15) * 15;
        setOrKey(L, 'rotation', PM.round(deg, 2), T);
      } else {
        const pointerWorld = pointerComp(ev);
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

function startCommonTransform(e: any, selection: SelectionGeometry, hit: any, T: any) {
  const bounds = selection.bounds;
  const pointerStart = pointerComp(e);
  const handleStart = hit.corner ? pointInBounds(bounds, hit.corner) : null;
  const grabOffset = handleStart
    ? { x: pointerStart.x - handleStart.x, y: pointerStart.y - handleStart.y }
    : { x: 0, y: 0 };
  const rotationStart = Math.atan2(
    pointerStart.y - selection.pivotWorld.y,
    pointerStart.x - selection.pivotWorld.x,
  );
  const snapshots = selection.roots.map((L: any) => ({
    L,
    x: PM.ev(L, 'position.x', T), y: PM.ev(L, 'position.y', T),
    sx: PM.ev(L, 'scale.x', T), sy: PM.ev(L, 'scale.y', T),
    rotation: PM.ev(L, 'rotation', T),
    skew: PM.ev(L, 'skew', T),
    pivotWorld: layerWorldPivot(PM, L, T),
    parentLinear: (() => {
      const matrix = parentWorldMatrix(L, T);
      return matrix ? [matrix[0], matrix[1], matrix[2], matrix[3]] as LinearMatrix : null;
    })(),
    worldLinear: (() => {
      const matrix = PM.worldMatrix(L, T);
      return [matrix[0], matrix[1], matrix[2], matrix[3]] as LinearMatrix;
    })(),
    textSize: L.type === 'text' ? Math.max(4, Number(L.d?.size) || 4) : null,
    textOwnsDescendants: L.type === 'text' && PM.proj.layers.some((layer: any) => layer.parent === L.id),
  }));
  if (!snapshots.length) return;

  const label = hit.rotate ? 'Rotate selection' : 'Resize selection';
  PM.Edit.begin(label, { origin: 'canvas' });
  let moved = false;
  beginDrag(e, {
    cursor: cursorForHit(selection, hit),
    move: (_dx: any, _dy: any, ev: any) => {
      moved = true;
      const pointer = pointerComp(ev);
      if (hit.rotate) {
        const angle = Math.atan2(pointer.y - selection.pivotWorld.y, pointer.x - selection.pivotWorld.x);
        let delta = angleDeltaDegrees(angle, rotationStart);
        if (ev.metaKey || ev.ctrlKey) delta = Math.round(delta / 45) * 45;
        else if (ev.shiftKey) delta = Math.round(delta / 15) * 15;
        for (const snapshot of snapshots) {
          const nextPivot = transformPointAround(
            snapshot.pivotWorld, selection.pivotWorld, 1, 1, delta,
          );
          const nextPosition = worldPointToParentLocal(snapshot.L, T, nextPivot);
          if (!nextPosition) continue;
          setOrKey(snapshot.L, 'position.x', PM.round(nextPosition.x, 3), T);
          setOrKey(snapshot.L, 'position.y', PM.round(nextPosition.y, 3), T);
          if (!snapshot.parentLinear) {
            /* With no parent, left-multiplying by R(delta) changes only the
               local rotation; preserve scale/skew channels and their keys. */
            setOrKey(snapshot.L, 'rotation', PM.round(snapshot.rotation + delta, 3), T);
            continue;
          }
          const desiredWorld = rotateLinear(snapshot.worldLinear, delta);
          const local = solveLocalTransformForWorldLinear(snapshot.parentLinear, desiredWorld, {
            scaleX: snapshot.sx / 100, rotation: snapshot.rotation,
          });
          if (!local) continue;
          const values = [
            ['rotation', local.rotation],
            ['scale.x', local.scaleX * 100],
            ['scale.y', local.scaleY * 100],
            ['skew', local.skew],
          ] as const;
          for (const [path, value] of values) setOrKey(snapshot.L, path, PM.round(value, 4), T);
        }
      } else {
        const next = calculateResize(
          bounds, hit.corner, pointer, grabOffset, { x: 1, y: 1 },
          { fromCenter: ev.altKey, lockAspect: true, minScale: .01 },
        );
        const scaleX = next.scaleX, scaleY = next.scaleY;
        const pivot = next.pivotLocal;
        for (const snapshot of snapshots) {
          if (snapshot.textSize != null && !snapshot.textOwnsDescendants) {
            PM.Edit.dispatch({
              type: 'set_content', target: snapshot.L.id,
              patch: { size: PM.round(Math.max(4, snapshot.textSize * scaleX), 2) },
            });
          } else {
            setOrKey(snapshot.L, 'scale.x', PM.round(snapshot.sx * scaleX, 3), T);
            setOrKey(snapshot.L, 'scale.y', PM.round(snapshot.sy * scaleY, 3), T);
          }
          const nextPivot = transformPointAround(snapshot.pivotWorld, pivot, scaleX, scaleY);
          const nextPosition = worldPointToParentLocal(snapshot.L, T, nextPivot);
          if (!nextPosition) continue;
          setOrKey(snapshot.L, 'position.x', PM.round(nextPosition.x, 3), T);
          setOrKey(snapshot.L, 'position.y', PM.round(nextPosition.y, 3), T);
        }
      }
      PM.invalidate();
    },
    up: () => { moved ? PM.Edit.commit(label) : PM.Edit.cancel(); PM.Inspector?.refresh?.(); },
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
/* Replacement activation receives the same viewer object and WebGL surface.
   Rebind the new closures immediately; the panel never needs to reopen. */
if (existingStage) V.attach(existingStage);
return V;
}
