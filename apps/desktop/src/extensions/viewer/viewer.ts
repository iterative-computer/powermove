import { is3DLayer, planeMatrix, planeContains, projectPoint, inversePlane } from 'powermove';
export function visualSelection(PM: any): any[] {
  const selected = PM.selLayers?.() || [];
  const ids = new Set(selected.map((layer: any) => layer.id));
  return selected.filter((layer: any) => !(PM.groupAncestors?.(layer) || []).some((group: any) => ids.has(group.id)));
}

export function selectionOutlineColor(project: any): string {
  const fill = project.backgroundFill;
  const color = fill?.type !== 'none' && fill?.stops?.[0]?.color || project.bg || '#000000';
  const hex = /^#[0-9a-f]{6}$/i.test(color) ? color.slice(1) : '000000';
  return '#' + [0, 2, 4].map(i => (255 - parseInt(hex.slice(i, i + 2), 16)).toString(16).padStart(2, '0')).join('');
}

import { editCanvasText } from './canvas-text';
import { drawEditablePaths, startPathEdit } from './path-editing';
import { resolveContent } from 'powermove';
/* Ported from js/ui/viewer.js — behavior-preserving. */
export const viewerPanelOptions = {
  title: 'Composition', flush: true, noscroll: true, headless: true, hideMoveHandle: false,
  library: { width: 640, height: 480 },
} as const;

export function previewRenderSize(width: number, height: number, zoom: number, dpr: number, quality: number) {
  const safeWidth = Math.max(2, Number(width) || 2);
  const safeHeight = Math.max(2, Number(height) || 2);
  const displayScale = Math.min(1, Math.max(0.05, Number(zoom) || 0.05) * Math.min(2, Math.max(1, Number(dpr) || 1)));
  const scale = Math.max(0.05, Math.min(1, displayScale * Math.max(0.25, Math.min(1, Number(quality) || 1))));
  return {
    width: Math.max(2, Math.round(safeWidth * scale)),
    height: Math.max(2, Math.round(safeHeight * scale)),
    scale,
  };
}

export type PreviewViewport = {
  x: number; y: number; width: number; height: number;
  compWidth: number; compHeight: number;
  cssLeft: number; cssTop: number; cssWidth: number; cssHeight: number;
  renderWidth: number; renderHeight: number;
};

/** Render only the visible portion of a magnified composition. A full 8×
 * 4K buffer is hundreds of megapixels; a viewport-sized buffer gives vector
 * edges one backing pixel per screen pixel without that memory cost. */
export function previewRenderViewport(
  compWidth: number, compHeight: number, zoom: number,
  stageWidth: number, stageHeight: number, frameX: number, frameY: number,
  dpr: number, quality: number, overscan = 128, previous?: PreviewViewport | null,
): PreviewViewport | null {
  const z = Number(zoom) || 0;
  if (!(z > 1 && compWidth > 0 && compHeight > 0 && stageWidth > 0 && stageHeight > 0)) return null;
  const displayWidth = compWidth * z, displayHeight = compHeight * z;
  const visibleLeft = Math.max(0, -frameX), visibleTop = Math.max(0, -frameY);
  const visibleRight = Math.min(displayWidth, stageWidth - frameX), visibleBottom = Math.min(displayHeight, stageHeight - frameY);
  if (visibleRight <= visibleLeft || visibleBottom <= visibleTop) return null;
  const pad = Math.max(0, Number(overscan) || 0);
  // Keep allocation independent of pan, including at composition edges.
  // Trimming each overscan margin to the visible intersection changed the
  // canvas size on every input event near an edge, clearing/reallocating GPU
  // buffers. Shift the bounded window inward instead of shrinking it.
  const cssWidth = Math.min(displayWidth, stageWidth + pad * 2);
  const cssHeight = Math.min(displayHeight, stageHeight + pad * 2);
  const cssLeft = Math.max(0, Math.min(displayWidth - cssWidth, visibleLeft - pad));
  const cssTop = Math.max(0, Math.min(displayHeight - cssHeight, visibleTop - pad));
  const density = Math.min(2, Math.max(1, Number(dpr) || 1)) * Math.max(.25, Math.min(1, Number(quality) || 1));
  // The overscan is a pan buffer. Retain its composition coordinates while
  // it covers the view, so input can move the already-presented image instead
  // of pinning old pixels to the screen until a replacement frame arrives.
  if (previous && previous.compWidth === compWidth && previous.compHeight === compHeight
      && Math.abs(previous.cssWidth / previous.width - z) < 1e-9
      && Math.abs(previous.cssHeight / previous.height - z) < 1e-9
      && previous.renderWidth === Math.max(2, Math.round(previous.cssWidth * density))
      && previous.renderHeight === Math.max(2, Math.round(previous.cssHeight * density))
      && Math.abs(previous.cssWidth - cssWidth) < 1e-7 && Math.abs(previous.cssHeight - cssHeight) < 1e-7
      && visibleLeft >= previous.cssLeft && visibleTop >= previous.cssTop
      && visibleRight <= previous.cssLeft + previous.cssWidth
      && visibleBottom <= previous.cssTop + previous.cssHeight) return previous;
  return {
    x: cssLeft / z, y: cssTop / z, width: cssWidth / z, height: cssHeight / z,
    compWidth, compHeight, cssLeft, cssTop, cssWidth, cssHeight,
    renderWidth: Math.max(2, Math.round(cssWidth * density)),
    renderHeight: Math.max(2, Math.round(cssHeight * density)),
  };
}

export type ResizeHandle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w';

type Point = { x: number; y: number };
type Bounds = { x0: number; y0: number; x1: number; y1: number; w: number; h: number };
type WorldBounds = Bounds & { cx: number; cy: number };
type Corner = readonly [number, number];
type AffineMatrix = readonly [number, number, number, number, number, number];
export type LinearMatrix = readonly [number, number, number, number];

export interface DragBox extends Bounds {}

export interface ViewportRect {
  left: number;
  top: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
}

/** True only when the composition frame has no visible overlap with the
    viewer. Touching an edge is treated as out of view: there is no useful
    composition pixel left to navigate from. */
export function compositionIsOutOfView(stage: ViewportRect, frame: ViewportRect): boolean {
  return frame.right <= stage.left || frame.left >= stage.right
    || frame.bottom <= stage.top || frame.top >= stage.bottom;
}

/** Explicit frame coordinates avoid CSS alignment changing semantics when a
    zoomed composition crosses from smaller than the stage to larger than it. */
export function compositionFramePosition(
  stage: { width: number; height: number },
  composition: { width: number; height: number },
  zoom: number,
  pan: Point,
): Point {
  return {
    x: (stage.width - composition.width * zoom) / 2 + pan.x,
    y: (stage.height - composition.height * zoom) / 2 + pan.y,
  };
}

/** AE shape tools draw from one corner; Option changes the origin to the
    center, and Shift constrains the result to a square/circle. */
export function shapeBoxFromDrag(
  start: Point, pointer: Point,
  options: { fromCenter?: boolean; constrain?: boolean } = {},
): DragBox {
  let dx = pointer.x - start.x;
  let dy = pointer.y - start.y;
  if (options.constrain) {
    const size = Math.max(Math.abs(dx), Math.abs(dy));
    dx = (dx < 0 ? -1 : 1) * size;
    dy = (dy < 0 ? -1 : 1) * size;
  }
  const opposite = options.fromCenter
    ? { x: start.x - dx, y: start.y - dy }
    : start;
  const far = options.fromCenter
    ? { x: start.x + dx, y: start.y + dy }
    : { x: start.x + dx, y: start.y + dy };
  const x0 = Math.min(opposite.x, far.x), x1 = Math.max(opposite.x, far.x);
  const y0 = Math.min(opposite.y, far.y), y1 = Math.max(opposite.y, far.y);
  return { x0, y0, x1, y1, w: x1 - x0, h: y1 - y0 };
}

/** Pan needed after a magnification change so the same composition point
    remains beneath the pointer, matching AE's Follow Cursor magnification. */
export function zoomPanForPoint(
  stage: { width: number; height: number },
  pointer: Point,
  compositionPoint: Point,
  composition: { width: number; height: number },
  zoom: number,
): Point {
  return {
    x: pointer.x - stage.width / 2 - (compositionPoint.x - composition.width / 2) * zoom,
    y: pointer.y - stage.height / 2 - (compositionPoint.y - composition.height / 2) * zoom,
  };
}

export type ViewerWheelMode = 'zoom' | 'pan';

/** Chromium exposes trackpad pinch as ctrl+wheel. Plain two-finger scrolling
    is continuous (often fractional and/or two-axis), while a physical mouse
    wheel retains discrete line/page or legacy 120-step deltas. */
export function viewerWheelMode(event: {
  ctrlKey?: boolean;
  shiftKey?: boolean;
  deltaMode?: number;
  deltaX: number;
  deltaY: number;
  wheelDeltaY?: number;
}): ViewerWheelMode {
  if (event.ctrlKey) return 'zoom';
  if (event.shiftKey || Math.abs(event.deltaX) > .01) return 'pan';
  if ((event.deltaMode || 0) !== 0) return 'zoom';
  const legacy = Math.abs(Number(event.wheelDeltaY) || 0);
  if (legacy >= 119 && Math.abs(legacy / 120 - Math.round(legacy / 120)) < .05) return 'zoom';
  return 'pan';
}

function wheelZoomDelta(event: WheelEvent): number {
  if (event.deltaMode === WheelEvent.DOM_DELTA_LINE) return event.deltaY * 40;
  if (event.deltaMode === WheelEvent.DOM_DELTA_PAGE) {
    return event.deltaY * Math.max(100, event.currentTarget instanceof HTMLElement
      ? event.currentTarget.clientHeight : 600);
  }
  return event.deltaY;
}

/** Resolve a Pan Behind drag in the layer's parent space. `linear` is the
    layer's rotation/skew/scale matrix. Compensating Position keeps every
    rendered pixel fixed while Anchor Point moves; Option disables that. */
export function anchorMoveValues(
  linear: LinearMatrix,
  anchor: Point,
  position: Point,
  pointerInParent: Point,
  options: { moveLayer?: boolean } = {},
): { anchor: Point; position: Point } | null {
  const det = linear[0] * linear[3] - linear[1] * linear[2];
  if (Math.abs(det) < 1e-9) return null;
  const dx = pointerInParent.x - position.x;
  const dy = pointerInParent.y - position.y;
  const localDx = (dx * linear[3] - dy * linear[2]) / det;
  const localDy = (dy * linear[0] - dx * linear[1]) / det;
  const nextAnchor = { x: anchor.x + localDx, y: anchor.y + localDy };
  const moveLayer = options.moveLayer !== false;
  return {
    anchor: nextAnchor,
    position: moveLayer ? {
      x: position.x + linear[0] * localDx + linear[2] * localDy,
      y: position.y + linear[1] * localDx + linear[3] * localDy,
    } : { ...position },
  };
}

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

export function selectionBoundsCenter(selection: Pick<SelectionGeometry, 'corners'>): Point {
  return midpoint(selection.corners[0]!, selection.corners[2]!);
}

function layerIsTransformable(PM: any, layer: any, T: number): boolean {
  return !!layer && !layer.lock && !(PM.groupAncestors?.(layer) || []).some((group: any) => group.lock) && PM.active(layer, T) && (layer.type === 'group' || PM.TYPE_META?.[layer.type]?.pickable !== false);
}

function visibleSelectionLayers(PM: any, selected: any[], T: number): any[] {
  return selected.filter((layer) => !!layer && PM.active(layer, T) && PM.GL.bounds(layer, T));
}

/** Selected parents own their descendants. Returning only roots prevents a
    parent and selected child from receiving the same transform twice. */
export function selectionTransformRoots(PM: any, layers: any[], T: number): any[] {
  const eligible = layers.filter((layer) => layerIsTransformable(PM, layer, T));
  if (PM.transformRoots) return PM.transformRoots(eligible.map(layer => layer.id));
  const ids = new Set(eligible.map((layer) => layer.id));
  return eligible.filter((layer) => {
    if ((PM.groupAncestors?.(layer) || []).some((group: any) => ids.has(group.id))) return false;
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
  if (layer.type === 'group' && is3DLayer(PM,layer)) {
    const members = PM.curComp().layers.filter((child: any) => child.type !== 'group' && child.type !== 'audio' && PM.active(child,T) && (PM.groupAncestors(child) || []).some((group: any) => group.id === layer.id));
    const boxes = members.map((child: any) => layerWorldBounds(PM,child,T)).filter(Boolean) as WorldBounds[];
    if (!boxes.length) return null;
    const x0=Math.min(...boxes.map(b=>b.x0)),y0=Math.min(...boxes.map(b=>b.y0));
    const x1=Math.max(...boxes.map(b=>b.x1)),y1=Math.max(...boxes.map(b=>b.y1));
    return {x0,y0,x1,y1,w:x1-x0,h:y1-y0,cx:(x0+x1)/2,cy:(y0+y1)/2};
  }
  const bounds = PM.GL.bounds(layer, T); if (!bounds) return null;
  const matrix = PM.worldMatrix(layer, T);
  const points = [
    { x: bounds.x0, y: bounds.y0 }, { x: bounds.x1, y: bounds.y0 },
    { x: bounds.x1, y: bounds.y1 }, { x: bounds.x0, y: bounds.y1 },
  ].map((point) => is3DLayer(PM,layer) ? projectPoint(planeMatrix(PM,layer,T),point) : applyMatrix(matrix, point));
  const xs = points.map((point) => point.x), ys = points.map((point) => point.y);
  const x0 = Math.min(...xs), x1 = Math.max(...xs), y0 = Math.min(...ys), y1 = Math.max(...ys);
  return { x0, x1, y0, y1, w: x1 - x0, h: y1 - y0, cx: (x0 + x1) / 2, cy: (y0 + y1) / 2 };
}

/** The real world-space anchor. Matrix translation is the transformed local
    origin, so it is not the rotation pivot when Anchor X/Y are non-zero. */
export function layerWorldPivot(PM: any, layer: any, T: number): Point {
  if (is3DLayer(PM,layer)) return projectPoint(planeMatrix(PM,layer,T,true), {x: PM.ev(layer,'position.x',T), y: PM.ev(layer,'position.y',T)});
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

  if (layers.length === 1 && layers[0].type === 'group' && is3DLayer(PM,layers[0])) {
    const box=layerWorldBounds(PM,layers[0],T);if(!box)return null;
    const corners=[{x:box.x0,y:box.y0},{x:box.x1,y:box.y0},{x:box.x1,y:box.y1},{x:box.x0,y:box.y1}];
    return {mode:'common',layers,roots,transformable,bounds:box,matrix:[1,0,0,1,0,0],corners,handles:handlesFromCorners(corners),pivotWorld:layerWorldPivot(PM,layers[0],T)};
  }
  if (layers.length === 1) {
    const layer = layers[0]!;
    const bounds = PM.GL.bounds(layer, T); if (!bounds) return null;
    const matrix = PM.worldMatrix(layer, T) as AffineMatrix;
    const corners = ([[0, 0], [1, 0], [1, 1], [0, 1]] as const)
      .map((corner) => is3DLayer(PM,layer) ? projectPoint(planeMatrix(PM,layer,T),pointInBounds(bounds,corner)) : applyMatrix(matrix, pointInBounds(bounds, corner)));
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
  return planeContains(PM, L, T, x, y, b);
}

/** Text editing follows the visible selection before the topmost pixel pick.
    This keeps a selected title editable even when a full-frame adjustment or
    overlay layer sits above it in the render stack. */
export function editableTextAtPoint(PM: any, x: number, y: number, T: number): any | null {
  const selected = visualSelection(PM).filter((layer: any) =>
    layer?.type === 'text' && !layer.lock && PM.active?.(layer, T) !== false);
  for (let index = selected.length - 1; index >= 0; index -= 1) {
    if (layerContainsPoint(PM, selected[index], x, y, T)) return selected[index];
  }
  const picked = PM.GL?.pick?.(x, y, T);
  return picked?.type === 'text' && !picked.lock ? picked : null;
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

const V: any = existing || {
  zoom: 1, fit: true, pan: [0, 0], el: null, ov: null, octx: null, inner: null,
  snapLines: null, toolRect: null, zoomRect: null, temporaryTool: null,
};
const ZOOM_LAYOUT_VERSION = 3;
if (V.zoomLayoutVersion !== ZOOM_LAYOUT_VERSION) {
  /* One-time migration away from the discarded transform/clamp zoom model.
     HMR returns the already-open viewer to a known frame without restarting. */
  V.zoomLayoutVersion = ZOOM_LAYOUT_VERSION;
  V.fit = true;
  V.pan = [0, 0];
}
PM.Viewer = V;
V._runtimeToken = VIEWER_RUNTIME_TOKEN;
let disposed = false;
let unbindStage: (() => void) | null = null;
let resizeObserver: ResizeObserver | null = null;
let activeDrag: { cancel(): void } | null = null;
let presentedViewport: PreviewViewport | null = null;
let requestedViewport: PreviewViewport | null = null;
let visibleRegion: { x: number; y: number; right: number; bottom: number } | null = null;
let zoomGestureUntil = 0;
let navigationUntil = 0;
V.isNavigating = () => !disposed && (!!activeDrag || window.performance.now() < navigationUntil);
let presentation: { time: number; version: number | undefined; project: any; quality: number } | null = null;
V.deferNavigationRender = (now: number) => {
  if (disposed || now >= zoomGestureUntil || PM.playing || !requestedViewport || !presentedViewport
      || requestedViewport === presentedViewport || !visibleRegion || !presentation
      || presentation.version === undefined || presentation.version !== PM.animVersion?.()
      || presentation.time !== PM.time || presentation.project !== PM.proj || presentation.quality !== PM.quality) return false;
  return visibleRegion.x >= presentedViewport.x && visibleRegion.y >= presentedViewport.y
    && visibleRegion.right <= presentedViewport.x + presentedViewport.width
    && visibleRegion.bottom <= presentedViewport.y + presentedViewport.height;
};
const displayViewport = (viewport: PreviewViewport, zoom = viewport.cssWidth / viewport.width) => {
  const gl = V.el as HTMLCanvasElement;
  gl.style.position = 'absolute'; gl.style.left = viewport.x * zoom + 'px'; gl.style.top = viewport.y * zoom + 'px';
  gl.style.width = viewport.width * zoom + 'px'; gl.style.height = viewport.height * zoom + 'px';
};
const busOffs: Array<() => void> = [];

const disposeRuntime = () => {
  if (disposed) return;
  disposed = true;
  activeDrag?.cancel(); activeDrag = null;
  unbindStage?.(); unbindStage = null;
  resizeObserver?.disconnect(); resizeObserver = null;
  V.previewOff?.(); V.previewOff=null;
  PM.finishCanvasText?.();
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
    let recovery = stage.querySelector<HTMLButtonElement>('#composition-recovery');
    if (!recovery) {
      recovery = document.createElement('button');
      recovery.id = 'composition-recovery';
      recovery.type = 'button';
      recovery.hidden = true;
      recovery.innerHTML = '<span>Composition is out of view</span><b>Fit composition</b>';
      stage.appendChild(recovery);
    }
    V.el = gl; V.ov = ov; V.octx = ov.getContext('2d'); V.inner = inner; V.stage = stage;
    V.recovery = recovery;
    const zoomHost = stage.closest<HTMLElement>('.panel') ?? stage;
    let zoomControl = zoomHost.querySelector<HTMLSelectElement>('#composition-zoom');
    if (!zoomControl) {
      zoomControl = document.createElement('select');
      zoomControl.id = 'composition-zoom';
      zoomControl.setAttribute('aria-label', 'Composition zoom');
      zoomControl.title = 'Composition zoom';
      for (const [value, label] of [['fit', 'Fit'], ...[5, 12.5, 25, 50, 100, 200, 400, 800].map(n => [String(n / 100), `${n}%`]), ['custom', 'Custom']] as [string, string][]) {
        const option = document.createElement('option'); option.value = value; option.textContent = label;
        if (value === 'custom') option.hidden = true;
        zoomControl.append(option);
      }

    }
    zoomControl.dataset.globalSpaceShortcut = '';
    // Share the panel chrome row with its move handle, outside the clipped stage.
    zoomHost.append(zoomControl);
    zoomControl.style.cssText = 'position:absolute;right:8px;top:7px;z-index:6;width:110px;height:24px;padding:0 26px 0 10px;border:0;border-radius:var(--r-sm);box-shadow:none;background-color:color-mix(in srgb,var(--tx) 5%,var(--bg-panel));color:var(--tx-2);font:var(--fs-md) var(--f-ui);cursor:pointer';
    V.zoomControl = zoomControl;
    let preview=zoomHost.querySelector<HTMLElement>('#preview-controls');
    if(!preview){preview=document.createElement('div');preview.id='preview-controls';zoomHost.append(preview);}
    preview.style.cssText='position:absolute;right:124px;top:7px;z-index:6;display:flex;align-items:center';
    preview.replaceChildren();
    const quality=document.createElement('select');quality.setAttribute('aria-label','Preview resolution');quality.style.cssText='width:82px;height:24px;padding:0 26px 0 10px;border:0;border-radius:var(--r-sm);box-shadow:none;background-color:color-mix(in srgb,var(--tx) 5%,var(--bg-panel));color:var(--tx-2);font:var(--fs-md) var(--f-ui);cursor:pointer';
    for(const [value,label] of [['auto','Auto'],['1','Full'],['0.5','Half'],['0.25','Quarter']]){const option=document.createElement('option');option.value=value!;option.textContent=label!;quality.append(option);}
    quality.value=PM.perf?.auto?'auto':String(PM.quality);quality.onchange=()=>{PM.perf.auto=quality.value==='auto';PM.quality=quality.value==='auto'?1:Number(quality.value);PM.previewResolution=quality.value;PM.bus.emit('quality');};preview.append(quality);
    V.previewOff?.(); V.previewOff = undefined;

    zoomControl.onchange = () => { if (zoomControl.value === 'fit') V.returnToComposition(); else V.setZoom(Number(zoomControl.value)); };

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
V.layout = (panOnly = false) => {
  if (panOnly === true) navigationUntil = window.performance.now() + 250;
  if (!V.el || !V.stage) return;
  const gl = V.el as HTMLCanvasElement;
  const p = PM.proj;
  const r = V.stage.getBoundingClientRect();
  if (r.width < 8 || r.height < 8) return;
  V._sw = r.width; V._sh = r.height;
  const pad = 8;
  let z = V.fit ? Math.min((r.width - pad * 2) / p.w, (r.height - pad * 2) / p.h) : V.zoom;
  z = clamp(z, .02, 8);
  V.shown = z;
  if (V.zoomControl) {
    const control = V.zoomControl as HTMLSelectElement;
    control.options[0]!.textContent = `Fit (${Math.round(z * 1000) / 10}%)`;
    const preset = [...control.options].find(option => Number(option.value) === z);
    const custom = control.querySelector<HTMLOptionElement>('option[value="custom"]')!;
    custom.textContent = `${Math.round(z * 1000) / 10}%`; custom.hidden = V.fit || !!preset;
    control.value = V.fit ? 'fit' : preset?.value ?? 'custom';
  }
  /* Keep display geometry fractional so its pixels and `shown` describe the
     same coordinate system. The GL backing buffer is rounded independently. */
  const dw = p.w * z, dh = p.h * z;
  V.inner.style.width = dw + 'px'; V.inner.style.height = dh + 'px';
  const position = compositionFramePosition(
    { width: r.width, height: r.height }, { width: p.w, height: p.h }, z,
    { x: V.fit ? 0 : V.pan[0], y: V.fit ? 0 : V.pan[1] },
  );
  V.inner.style.left = position.x + 'px';
  V.inner.style.top = position.y + 'px';
  V.inner.style.transform = '';
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  /* Cropped high-zoom rendering is exact for ordinary 2D source layers. Keep
     complex full-frame/3D pipelines on the established full-composition path
     until their coordinate-dependent effects can consume a viewport origin. */
  const viewportSafe = p.layers.every((layer: any) => !layer.threeD
    && !(layer.fx || []).some((effect: any) => effect?.on !== false)
    && !(layer.masks || []).length && !layer.matteSource && !layer.transitionIn && !layer.transitionOut
    && !['adjustment', 'shader', 'extension', 'precomp'].includes(layer.type));
  const viewport = viewportSafe
    ? previewRenderViewport(p.w, p.h, z, r.width, r.height, position.x, position.y, dpr, PM.quality, 128, PM.GL.previewViewport)
    : null;
  requestedViewport = viewport;
  visibleRegion = viewport ? { x: Math.max(0, -position.x) / z, y: Math.max(0, -position.y) / z,
    right: Math.min(p.w, (r.width - position.x) / z), bottom: Math.min(p.h, (r.height - position.y) / z) } : null;
  if (viewport) {
    // While a new crop is pending, keep the old pixels in their own source
    // coordinates. Commit the new CSS placement with its GL presentation.
    const retain = presentedViewport && presentedViewport.compWidth === p.w && presentedViewport.compHeight === p.h;
    displayViewport(retain ? presentedViewport! : viewport, z);
    PM.GL.resize(viewport.renderWidth, viewport.renderHeight, viewport);
  } else {
    presentedViewport = null;
    gl.style.position = ''; gl.style.left = ''; gl.style.top = ''; gl.style.width = '100%'; gl.style.height = '100%';
    const renderSize = PM.previewResolution && PM.previewResolution!=='auto' ? {width:Math.max(2,Math.round(p.w*PM.quality)),height:Math.max(2,Math.round(p.h*PM.quality))} : previewRenderSize(p.w, p.h, z, dpr, PM.quality);
    PM.GL.resize(renderSize.width, renderSize.height, null);
  }
  const overlayWidth = Math.round(r.width * dpr);
  const overlayHeight = Math.round(r.height * dpr);
  /* Assigning either canvas dimension clears the bitmap even when the value is
     unchanged. Zoom does not resize the stage, so retain its controls between
     frames and only clear for a real panel-size or density change. */
  if (V.ov.width !== overlayWidth) V.ov.width = overlayWidth;
  if (V.ov.height !== overlayHeight) V.ov.height = overlayHeight;
  V.ov.style.width = r.width + 'px'; V.ov.style.height = r.height + 'px';
  V.updateRecovery?.();
  /* Geometry changes immediately during zoom/pan. Repaint handles and paths in
     the same event so they never lag or blink while the GL frame catches up. */
  drawOverlay();
  /* Zoom, pan, and panel geometry are view-only. Avoid waking inspector/UI and
     timeline redraws for every wheel event. */
  // A pan inside the already-presented overscan only moves existing pixels.
  // Do not submit the entire layer stack again. This is deliberately scoped
  // to pan input: independent draw requests (assets, fonts, edits, context
  // recovery, etc.) remain pending and are never swallowed here.
  const reusePan = panOnly === true && !PM.playing && viewport && viewport === presentedViewport
    && presentation && presentation.version !== undefined && presentation.version === PM.animVersion?.()
    && presentation.time === PM.time && presentation.project === p && presentation.quality === PM.quality;
  if (!reusePan) PM.invalidate('render');
};
V.setZoom = (zoom: number) => {
  if (!Number.isFinite(zoom)) return false;
  if (V.fit) V.pan = [0, 0];
  V.fit = false; V.zoom = clamp(zoom, .05, 8); V.layout();
  return V.zoom;
};
V.returnToComposition = () => {
  V.fit = true;
  V.pan = [0, 0];
  V.layout();
};
V.updateRecovery = () => {
  if (!V.recovery || !V.stage || !V.inner) return false;
  const hiddenBySourcePreview = !!V.preview?.activeId;
  const out = !V.fit && !hiddenBySourcePreview
    && compositionIsOutOfView(V.stage.getBoundingClientRect(), V.inner.getBoundingClientRect());
  V.recovery.hidden = !out;
  return out;
};
const onWindowResize = () => V.layout();
window.addEventListener('resize', onWindowResize);
for (const event of ['quality', 'project', 'layout:applied']) {
  const off = PM.bus.on(event, () => V.layout());
  if (typeof off === 'function') busOffs.push(off);
}
{
  const off = PM.bus.on('source-preview', () => V.updateRecovery?.());
  if (typeof off === 'function') busOffs.push(off);
}

/* comp px <-> screen px */
const toComp = (e: any): [number, number] => {
  const r = V.inner.getBoundingClientRect();
  return [(e.clientX - r.left) / V.shown, (e.clientY - r.top) / V.shown];
};

const stagePoint = (e: any): Point => {
  const r = V.stage.getBoundingClientRect();
  return { x: e.clientX - r.left, y: e.clientY - r.top };
};

function setZoomAtPoint(compositionPoint: Point, pointer: Point, zoom: number): void {
  const stage = V.stage.getBoundingClientRect();
  V.fit = false;
  V.zoom = clamp(zoom, .05, 8);
  const pan = zoomPanForPoint(
    { width: stage.width, height: stage.height }, pointer, compositionPoint,
    { width: PM.proj.w, height: PM.proj.h }, V.zoom,
  );
  V.pan = [pan.x, pan.y];
  V.layout();
}

function zoomAtEvent(e: any, factor: number): void {
  const [x, y] = toComp(e);
  setZoomAtPoint({ x, y }, stagePoint(e), (V.shown || 1) * factor);
}

function leaveFitMode(): void {
  if (!V.fit) return;
  V.zoom = Number.isFinite(V.shown) ? V.shown : V.zoom;
  V.pan = [0, 0];
  V.fit = false;
}

/* ── overlay drawing ───────────────────────────────────── */
for (const [event, handler] of [
  ['overlay', drawOverlay],
  ['preview:presented', (frame: { viewport: PreviewViewport; time: number; version: number | undefined; project: any; quality: number }) => {
    if (frame.viewport && frame.viewport === requestedViewport && V.el) {
      presentedViewport = frame.viewport; presentation = frame; displayViewport(frame.viewport);
    }
  }],
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
  const selectionInk = selectionOutlineColor(PM.proj);
  const p = PM.proj, dpr = V.ov.width / V.stage.getBoundingClientRect().width;
  c.setTransform(1, 0, 0, 1, 0, 0);
  c.clearRect(0, 0, V.ov.width, V.ov.height);
  if (V.zoomRect) {
    const box = V.zoomRect as DragBox;
    c.save();
    c.scale(dpr, dpr);
    c.strokeStyle = 'rgba(255,255,255,.88)';
    c.fillStyle = 'rgba(255,255,255,.08)';
    c.setLineDash([4, 3]);
    c.lineWidth = 1;
    c.fillRect(box.x0, box.y0, box.w, box.h);
    c.strokeRect(box.x0 + .5, box.y0 + .5, Math.max(0, box.w - 1), Math.max(0, box.h - 1));
    c.restore();
  }
  const S = V.shown * dpr;
  const frame = V.inner.getBoundingClientRect(), stage = V.stage.getBoundingClientRect();
  c.save(); c.translate((frame.left - stage.left) * dpr, (frame.top - stage.top) * dpr); c.scale(S, S);
  c.lineWidth = 1 / S;

  drawSnapLines(c);
  if(V.showControls !== false && !visualSelection(PM).some((layer: any) => is3DLayer(PM,layer)))drawEditablePaths(PM,c,S);

  if (V.toolRect) {
    const box = V.toolRect.box as DragBox;
    c.save();
    c.strokeStyle = V.toolRect.kind === 'shape' ? '#fff' : selectionInk;
    c.fillStyle = V.toolRect.kind === 'shape' ? 'rgba(255,255,255,.12)' : 'rgba(255,255,255,.045)';
    c.setLineDash([5 / S, 4 / S]);
    c.lineWidth = 1 / S;
    if (V.toolRect.kind === 'shape' && PM.toolShape === 'ellipse') {
      c.beginPath();
      c.ellipse((box.x0 + box.x1) / 2, (box.y0 + box.y1) / 2, box.w / 2, box.h / 2, 0, 0, Math.PI * 2);
      c.fill(); c.stroke();
    } else {
      c.fillRect(box.x0, box.y0, box.w, box.h);
      c.strokeRect(box.x0, box.y0, box.w, box.h);
    }
    c.restore();
  }

  /* Command+Shift+H is After Effects' Show Layer Controls toggle. Drawing and
     tool feedback remain live; only selection boxes, handles, and paths hide. */
  if (V.showControls === false) {
    c.restore();
    return;
  }

  const sels = visualSelection(PM).filter((L: any) => PM.active(L, PM.time) && L.id !== PM.canvasTextEditing);
  const selection = resolveSelectionGeometry(PM, sels, PM.time);
  /* Multi-selection keeps light per-layer outlines for orientation, but owns
     exactly one common transform box and one set of controls. */
  if (sels.length > 1 || !selection) for (const L of sels) {
    const b = PM.GL.bounds(L, PM.time);
    if (!b) continue;
    const m = PM.worldMatrix(L, PM.time);
    const corners = ([[0, 0], [1, 0], [1, 1], [0, 1]] as const).map((corner) =>
      is3DLayer(PM,L) ? projectPoint(planeMatrix(PM,L,PM.time),pointInBounds(b,corner)) : applyMatrix(m, pointInBounds(b, corner)));
    c.strokeStyle = selectionInk;
    c.lineWidth = 1 / Math.max(.02, V.shown);
    c.beginPath();
    c.moveTo(corners[0]!.x, corners[0]!.y);
    corners.slice(1).forEach((point) => c.lineTo(point.x, point.y));
    c.closePath();
    c.stroke();
  }
  // Selected-layer anchors use the same accent as the transform box.
  // Evaluate the local anchor through the full parent chain.
  c.save();
  const anchorUnit = 1 / Math.max(.02, V.shown);
  for (const layer of sels) {
    if (!PM.GL.bounds(layer, PM.time)) continue;
    const pivot = layerWorldPivot(PM, layer, PM.time);
    if (!Number.isFinite(pivot.x) || !Number.isFinite(pivot.y)) continue;
    c.beginPath();
    c.arc(pivot.x, pivot.y, 5 * anchorUnit, 0, Math.PI * 2);
    c.moveTo(pivot.x - 8 * anchorUnit, pivot.y);
    c.lineTo(pivot.x + 8 * anchorUnit, pivot.y);
    c.moveTo(pivot.x, pivot.y - 8 * anchorUnit);
    c.lineTo(pivot.x, pivot.y + 8 * anchorUnit);
    c.strokeStyle = 'rgba(0,0,0,.8)';
    c.lineWidth = 3 * anchorUnit;
    c.stroke();
    c.strokeStyle = selectionInk;
    c.lineWidth = anchorUnit;
    c.stroke();
  }
  c.restore();
  /* Groups and multi-selections align by their visual bounds, which can be
     somewhere other than any layer's transform anchor. Mark that bounds
     center explicitly so the point used by snapping is visible before drag. */
  if (selection && (selection.mode === 'common' || selection.layers[0]?.type === 'group')) {
    const center = selectionBoundsCenter(selection);
    const unit = 1 / Math.max(.02, V.shown), radius = 5 * unit;
    const pivotDistance = Math.hypot(center.x - selection.pivotWorld.x, center.y - selection.pivotWorld.y);
    c.save();
    if (pivotDistance > 12 * unit) {
      c.setLineDash([3 * unit, 3 * unit]);
      c.strokeStyle = selectionInk;
      c.globalAlpha = .7;
      c.lineWidth = unit;
      c.beginPath(); c.moveTo(selection.pivotWorld.x, selection.pivotWorld.y); c.lineTo(center.x, center.y); c.stroke();
      c.setLineDash([]);
      c.globalAlpha = 1;
    }
    c.fillStyle = 'rgba(0,0,0,.72)';
    c.strokeStyle = 'rgba(0,0,0,.8)';
    c.lineWidth = 3 * unit;
    c.beginPath();
    c.moveTo(center.x, center.y - radius); c.lineTo(center.x + radius, center.y);
    c.lineTo(center.x, center.y + radius); c.lineTo(center.x - radius, center.y); c.closePath();
    c.stroke(); c.fill();
    c.strokeStyle = selectionInk;
    c.lineWidth = unit;
    c.stroke();
    c.restore();
  }
  if (selection) {
    c.strokeStyle = selectionInk;
    c.lineWidth = 1.4 / Math.max(.02, V.shown);
    c.beginPath();
    c.moveTo(selection.corners[0]!.x, selection.corners[0]!.y);
    selection.corners.slice(1).forEach((point) => c.lineTo(point.x, point.y));
    c.closePath();
    c.stroke();
    if (!selection.transformable || selection.layers.some((layer: any) => is3DLayer(PM,layer))) { c.restore(); return; }
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

function drawSnapLines(c: any) {
  const lines: SnapLine[] = V.snapLines;
  if (!lines || !lines.length) return;
  const unit = 1 / Math.max(.02, V.shown), cross = 3 * unit;
  const centerSnaps = lines.filter((line) =>
    line.targetScope === 'composition' && line.targetRole === 'center' && line.axis,
  );
  c.save();
  c.strokeStyle = SNAP_COLOR;
  c.lineWidth = unit;
  c.beginPath();
  for (const line of lines) {
    if (line.targetScope === 'composition' && line.targetRole === 'center' && line.axis === 'x') {
      c.moveTo(line.to.x, 0); c.lineTo(line.to.x, PM.proj.h);
    } else if (line.targetScope === 'composition' && line.targetRole === 'center' && line.axis === 'y') {
      c.moveTo(0, line.to.y); c.lineTo(PM.proj.w, line.to.y);
    } else {
      c.moveTo(line.from.x, line.from.y); c.lineTo(line.to.x, line.to.y);
    }
  }
  c.stroke();
  c.beginPath();
  for (const line of lines) for (const point of centerSnaps.includes(line) ? [line.from] : [line.from, line.to]) {
    c.moveTo(point.x - cross, point.y - cross); c.lineTo(point.x + cross, point.y + cross);
    c.moveTo(point.x - cross, point.y + cross); c.lineTo(point.x + cross, point.y - cross);
  }
  c.stroke();
  /* A center-to-center snap used to collapse into a zero-length segment. A
     fixed-size bullseye makes the composition center legible even when both
     axes and both points perfectly overlap. */
  const center = centerSnaps[0]?.to;
  if (center) {
    const radius = 6 * unit;
    c.fillStyle = 'rgba(0,0,0,.72)';
    c.beginPath(); c.arc(center.x, center.y, radius, 0, Math.PI * 2); c.fill();
    c.strokeStyle = SNAP_COLOR; c.lineWidth = 1.5 * unit; c.stroke();
    c.fillStyle = SNAP_COLOR;
    c.beginPath(); c.arc(center.x, center.y, 1.5 * unit, 0, Math.PI * 2); c.fill();
  }
  c.restore();
}

/* Alignment snapping. Candidates are points, not bare axis values: the
   corners and center of every layer a gesture can align with, so a snap knows
   which point it landed on and the guide can be drawn from the dragged point
   to the target instead of across the whole composition. Everything stays in
   composition pixels; the threshold is converted from a constant on-screen
   distance by the caller. */
type SnapPointRole = 'corner' | 'center';
type SnapPointScope = 'layer' | 'composition';
type SnapPoint = Point & { snapRole?: SnapPointRole; snapScope?: SnapPointScope };
type SnapAxisCandidate = { value: number; point: SnapPoint };
type SnapCandidates = { x: SnapAxisCandidate[]; y: SnapAxisCandidate[] };
type SnapLine = {
  from: Point; to: Point; axis?: 'x' | 'y';
  sourceRole?: SnapPointRole; targetRole?: SnapPointRole; targetScope?: SnapPointScope;
};
type SnapTarget = { offset: number; distance: number; sourcePoint: SnapPoint; targetPoint: SnapPoint };
type SnapResult = { dx: number; dy: number; lines: SnapLine[] };

const SNAP_COLOR = '#F43535';
/** How close, in CSS pixels, a candidate has to be before a gesture snaps to it. */
const SNAP_DISTANCE = 8;

function snapCandidatesFromPoints(points: SnapPoint[]): SnapCandidates {
  return {
    x: points.map((point) => ({ value: point.x, point })),
    y: points.map((point) => ({ value: point.y, point })),
  };
}

/** The four corners of an axis-aligned box plus its semantic center. */
function boxSnapPoints(
  b: { x0: number; x1: number; y0: number; y1: number },
  scope: SnapPointScope = 'layer',
): SnapPoint[] {
  return [
    { x: b.x0, y: b.y0, snapRole: 'corner', snapScope: scope },
    { x: b.x1, y: b.y0, snapRole: 'corner', snapScope: scope },
    { x: b.x1, y: b.y1, snapRole: 'corner', snapScope: scope },
    { x: b.x0, y: b.y1, snapRole: 'corner', snapScope: scope },
    { x: (b.x0 + b.x1) / 2, y: (b.y0 + b.y1) / 2, snapRole: 'center', snapScope: scope },
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
    axis: 'x', sourceRole: xSnap.sourcePoint.snapRole,
    targetRole: xSnap.targetPoint.snapRole, targetScope: xSnap.targetPoint.snapScope,
  });
  if (ySnap) lines.push({
    from: { x: ySnap.sourcePoint.x + (xSnap?.offset ?? 0), y: ySnap.sourcePoint.y + ySnap.offset },
    to: ySnap.targetPoint,
    axis: 'y', sourceRole: ySnap.sourcePoint.snapRole,
    targetRole: ySnap.targetPoint.snapRole, targetScope: ySnap.targetPoint.snapScope,
  });
  return { dx: xSnap?.offset ?? 0, dy: ySnap?.offset ?? 0, lines };
}

/** True when the guides now point somewhere new, which is when the haptic fires. */
function snapLinesChanged(previous: SnapLine[] | null, next: SnapLine[] | null): boolean {
  if (!next || !next.length) return false;
  const key = (lines: SnapLine[]) => lines.map((line) => [
    line.axis, line.sourceRole, line.targetScope, line.targetRole, line.to.x, line.to.y,
  ].join(':')).sort().join('|');
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
 * frame: its siblings plus its parent. A multi-selection keeps that same frame
 * when every selected layer shares a parent; only a cross-parent selection
 * falls back to the composition's top level. The composition itself is always
 * a candidate.
 */
function snapshotSnapCandidates(T: any, selectionLayers: any[]): SnapCandidates {
  const selectedIds = new Set(selectionLayers.map((L: any) => L.id));
  const points: SnapPoint[] = boxSnapPoints(
    { x0: 0, y0: 0, x1: PM.proj.w, y1: PM.proj.h }, 'composition',
  );
  const parentIds = new Set(selectionLayers.map((L: any) => L.parent || null));
  const parentId = parentIds.size === 1 ? [...parentIds][0] : null;
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
  const m = parentWorldMatrix(L, T);
  if (!m) return [dx, dy];
  const delta = invertDirection(m, {x:dx,y:dy});
  return delta ? [delta.x,delta.y] : [0,0];
}

Object.assign(V, {
  worldBounds, unionBounds, snapshotSnapCandidates, findSnapTarget, snapBox, boxSnapPoints,
  snapCandidatesFromPoints, snapLinesChanged, passedMoveDragThreshold, calculateResize, resizeCursorForHandle,
  resizeLocksAspect,
  resolveSelectionGeometry: (layers: any = visualSelection(PM), T: any = PM.time) =>
    resolveSelectionGeometry(PM, layers, T),
  selectionTransformRoots: (layers: any = visualSelection(PM), T: any = PM.time) =>
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
    if ((event.target as Element)?.closest?.('#composition-zoom, #preview-controls, [contenteditable]')) return;
    if (fenceLegacyListeners) event.stopImmediatePropagation();
    handler(event);
  }) as EventListener;
  const capture = fenceLegacyListeners ? true : undefined;
  listen(stage, 'contextmenu', guarded((event: MouseEvent) => {
    event.preventDefault();
    const point = pointerComp(event);
    const layer = PM.GL.pick(point.x, point.y, PM.time, { includeLocked: true });
    if (layer) PM.showLayerMenu?.(layer, event, 'viewer');
    else {
      const items = PM.Kernel?.collectMenu?.('viewer:context', { layerId: null, time: PM.time }) || [];
      if (items.length) PM.menu(document.body, items, { x: event.clientX, y: event.clientY });
    }
  }), capture);
  V.layerAtPoint = (clientX: number, clientY: number) => {
    const rect = stage.getBoundingClientRect();
    if (clientX < rect.left || clientX > rect.right || clientY < rect.top || clientY > rect.bottom) return null;
    const point = pointerComp({ clientX, clientY });
    return PM.GL.pick(point.x, point.y, PM.time);
  };
  listen(stage, 'pointerdown', guarded(onDown), capture);
  listen(stage, 'pointermove', guarded(updateStageCursor), capture);
  listen(stage, 'pointerenter', guarded(() => { V.pointerOver = true; }), capture);
  listen(stage, 'pointerleave', guarded(() => { V.pointerOver = false; setStageCursor(''); }), capture);
  listen(stage, 'wheel', guarded((e: any) => {
    e.preventDefault();
    if (viewerWheelMode(e) === 'zoom') {
      zoomGestureUntil = window.performance.now() + 80;
      navigationUntil = window.performance.now() + 250;
      const factor = clamp(Math.exp(-wheelZoomDelta(e) * .0015), .5, 2);
      zoomAtEvent(e, factor);
      return;
    }
    leaveFitMode();
    const dx = e.shiftKey && !e.deltaX ? e.deltaY : e.deltaX;
    const dy = e.shiftKey && !e.deltaX ? 0 : e.deltaY;
    V.pan = [V.pan[0] - dx, V.pan[1] - dy];
    V.layout(true);
  }), fenceLegacyListeners ? { capture: true, passive: false } : { passive: false });
  if (V.recovery) listen(V.recovery, 'pointerdown', ((e: any) => {
    e.preventDefault();
    e.stopPropagation();
  }) as EventListener);
  if (V.recovery) listen(V.recovery, 'click', ((e: any) => {
    e.preventDefault();
    e.stopPropagation();
    V.returnToComposition();
  }) as EventListener);
  listen(window, 'keydown', ((e: any) => {
    if (!V.pointerOver || e.code !== 'Space' || e.repeat || e.target?.closest?.('input,textarea,[contenteditable]')) return;
    /* Space remains the global transport shortcut even while the pointer is
       over the canvas. The viewer may temporarily expose Hand for a Space-drag,
       but must not swallow the same keydown before the timeline can play. */
    V.temporaryTool = 'hand';
    setStageCursor('grab');
  }) as EventListener, true);
  listen(window, 'keyup', ((e: any) => {
    if (e.code !== 'Space' || V.temporaryTool !== 'hand' || e.target?.closest?.('input,textarea,[contenteditable]')) return;
    e.preventDefault();
    V.temporaryTool = null;
    setStageCursor('default');
  }) as EventListener, true);
  listen(fenceLegacyListeners ? stage : inner, 'dblclick', guarded((e: any) => {
    if(PM.tool==='pen'){const L=PM.firstSel(),path=L?.d?.paths?.find((p:any)=>p.id===PM.activePath)||L?.masks?.find((m:any)=>m.path?.id===PM.activePath)?.path;if(path&&!L.lock){PM.Edit.mutate('Close path',()=>{if(path.p.closed.kf.length)PM.setKeyOn(path.p.closed,PM.time-L.from,true);else path.p.closed.v=true;PM.activePath=null;},{origin:'canvas'});PM.invalidate();PM.Inspector?.refresh?.();}e.preventDefault();e.stopPropagation();return;}
    const [x, y] = toComp(e);
    let L = editableTextAtPoint(PM, x, y, PM.time);
    const remembered = V.textDoubleClickCandidate;
    if (!L && remembered && Date.now() - remembered.at < 700) {
      const candidate = PM.L?.(remembered.id);
      if (candidate?.type === 'text' && !candidate.lock
        && PM.active?.(candidate, PM.time) !== false
        && layerContainsPoint(PM, candidate, x, y, PM.time)) L = candidate;
    }
    V.textDoubleClickCandidate = null;
    if (L) {
      e.preventDefault();
      e.stopPropagation();
      editCanvasText(PM, V, L, undefined, true);
    }
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
  const tool = V.temporaryTool || PM.tool;
  if (tool === 'hand') { setStageCursor('grab'); return; }
  if (tool === 'zoom') { setStageCursor(e.altKey ? 'zoom-out' : 'zoom-in'); return; }
  if (tool === 'rotate') { setStageCursor(ROTATE_CURSOR); return; }
  if (tool === 'anchor') { setStageCursor('crosshair'); return; }
  if (tool === 'pen') { setStageCursor('crosshair'); return; }
  if (tool === 'shape') { setStageCursor('crosshair'); return; }
  if (tool === 'text') { setStageCursor('text'); return; }
  const [x, y] = toComp(e), T = PM.time;
  const selection = resolveSelectionGeometry(PM, visualSelection(PM), T);
  if (selection?.transformable) {
    const hit = handleAt(selection, x, y);
    if (hit) { setStageCursor(cursorForHit(selection, hit)); return; }
    if (pointInSelection(selection, x, y)) { setStageCursor('move'); return; }
  }
  setStageCursor('default');
}

function startPan(e: any) {
  leaveFitMode();
  const start = [V.pan[0], V.pan[1]];
  beginDrag(e, {
    cursor: 'grabbing',
    move: (dx: any, dy: any, ev: any) => {
      const speed = ev.shiftKey ? 2 : 1;
      V.pan = [start[0] + dx * speed, start[1] + dy * speed];
      V.layout(true);
    },
  });
}

function onDown(e: any) {
  if (e.button === 1) return startPan(e);
  if (e.button !== 0) return;

  /* The first click of a double-click may legitimately select a full-frame
     layer above the text. Remember the selected text hit briefly so the
     ensuing dblclick can still enter that text editor and select all. */
  if ((V.temporaryTool || PM.tool || 'select') === 'select') {
    const remembered = V.textDoubleClickCandidate;
    if (remembered && Date.now() - remembered.at < 700
      && Math.hypot(e.clientX - remembered.clientX, e.clientY - remembered.clientY) < 6) {
      const layer = PM.L?.(remembered.id);
      if (layer?.type === 'text' && !layer.lock && PM.active?.(layer, PM.time) !== false) {
        V.textDoubleClickCandidate = null;
        e.preventDefault();
        e.stopPropagation();
        editCanvasText(PM, V, layer, undefined, true);
        return;
      }
    }
    const point = pointerComp(e);
    const candidate = visualSelection(PM).find((layer: any) =>
      layer?.type === 'text' && !layer.lock
      && PM.active?.(layer, PM.time) !== false
      && layerContainsPoint(PM, layer, point.x, point.y, PM.time));
    if (candidate) V.textDoubleClickCandidate = {
      id: candidate.id, at: Date.now(), clientX: e.clientX, clientY: e.clientY,
    };
  }

  const tool = V.temporaryTool || PM.tool || 'select';
  if (tool === 'hand') return startPan(e);
  if (tool === 'zoom') return startZoom(e);
  if (tool === 'pen' && visualSelection(PM).some((layer: any) => is3DLayer(PM,layer))) { PM.toast('Turn off 3D temporarily to edit path vertices'); return; }
  if (tool === 'pen') return startPathEdit(PM,e,pointerComp,beginDrag,V.shown);
  if (tool === 'shape') return startShape(e);
  if (tool === 'text') return startText(e);
  if (tool === 'rotate') return startRotationTool(e);
  if (tool === 'anchor') return startAnchorTool(e);

  const [x, y] = toComp(e);
  const T = PM.time;

  const selection = resolveSelectionGeometry(PM, visualSelection(PM), T);
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
        if (L && e.shiftKey && selectedIds.has(L.id)) {
          PM.selectLayers(PM.sel.layers.filter((id: any) => id !== L.id));
        } else if (L && !selectedIds.has(L.id)) PM.selectLayers(L.id, e.shiftKey);
        else if (!L && !e.shiftKey) PM.selectLayers([]);
      },
    });
  }
  if (!L) return startSelectionMarquee(e);
  if (e.shiftKey && PM.sel.layers.includes(L.id)) {
    PM.selectLayers(PM.sel.layers.filter((id: any) => id !== L.id));
    return;
  }
  PM.selectLayers(L.id, e.shiftKey);
  const nextSelection = resolveSelectionGeometry(PM, visualSelection(PM), T);
  if (nextSelection) startMove(e, nextSelection.roots, T, { selectionLayers: nextSelection.layers });
}

function clearToolRect(): void {
  V.toolRect = null;
  PM.invalidate('render');
}

function startZoom(e: any): void {
  const startScreen = stagePoint(e);
  const startComposition = pointerComp(e);
  let moved = false;
  beginDrag(e, {
    cursor: e.altKey ? 'zoom-out' : 'zoom-in',
    move: (dx: number, dy: number, ev: any) => {
      moved = passedMoveDragThreshold(dx, dy);
      V.zoomRect = moved && !ev.altKey
        ? shapeBoxFromDrag(startScreen, stagePoint(ev))
        : null;
      PM.invalidate('render');
    },
    up: (_dx: number, _dy: number, ev: any) => {
      const box = V.zoomRect as DragBox | null;
      V.zoomRect = null;
      if (!moved || ev.altKey || !box || box.w < 3 || box.h < 3) {
        zoomAtEvent(ev, ev.altKey ? .8 : 1.25);
        return;
      }
      const endComposition = pointerComp(ev);
      const compositionBox = shapeBoxFromDrag(startComposition, endComposition);
      const stage = V.stage.getBoundingClientRect();
      const zoom = Math.min(
        (stage.width - 16) / Math.max(.01, compositionBox.w),
        (stage.height - 16) / Math.max(.01, compositionBox.h),
      );
      setZoomAtPoint(
        { x: (compositionBox.x0 + compositionBox.x1) / 2, y: (compositionBox.y0 + compositionBox.y1) / 2 },
        { x: stage.width / 2, y: stage.height / 2 }, zoom,
      );
    },
    cancel: () => { V.zoomRect = null; PM.invalidate('render'); },
  });
}

function startSelectionMarquee(e: any): void {
  const start = pointerComp(e);
  const before = [...PM.sel.layers];
  let moved = false;
  beginDrag(e, {
    cursor: 'default',
    move: (dx: number, dy: number, ev: any) => {
      if (!passedMoveDragThreshold(dx, dy)) return;
      moved = true;
      V.toolRect = { kind: 'selection', box: shapeBoxFromDrag(start, pointerComp(ev)) };
      PM.invalidate('render');
    },
    up: () => {
      const box = V.toolRect?.box as DragBox | undefined;
      clearToolRect();
      if (!moved || !box) {
        if (!e.shiftKey) PM.selectLayers([]);
        return;
      }
      const enclosed = PM.proj.layers.filter((layer: any) => {
        if (!PM.active(layer, PM.time) || PM.TYPE_META?.[layer.type]?.pickable === false) return false;
        const bounds = layerWorldBounds(PM, layer, PM.time);
        return !!bounds && bounds.x0 >= box.x0 && bounds.x1 <= box.x1 && bounds.y0 >= box.y0 && bounds.y1 <= box.y1;
      }).map((layer: any) => layer.id);
      if (!e.shiftKey) PM.selectLayers(enclosed);
      else {
        const next = new Set(before);
        for (const id of enclosed) next.has(id) ? next.delete(id) : next.add(id);
        PM.selectLayers([...next]);
      }
    },
    cancel: clearToolRect,
  });
}

function startShape(e: any): void {
  const start = pointerComp(e);
  let moved = false;
  beginDrag(e, {
    cursor: 'crosshair',
    move: (dx: number, dy: number, ev: any) => {
      if (!passedMoveDragThreshold(dx, dy)) return;
      moved = true;
      V.toolRect = {
        kind: 'shape',
        box: shapeBoxFromDrag(start, pointerComp(ev), { fromCenter: ev.altKey, constrain: ev.shiftKey }),
      };
      PM.invalidate('render');
    },
    up: (_dx: number, _dy: number, ev: any) => {
      const box = moved
        ? shapeBoxFromDrag(start, pointerComp(ev), { fromCenter: ev.altKey, constrain: ev.shiftKey })
        : null;
      clearToolRect();
      if (!box || box.w < .5 || box.h < .5) return;
      createShape(box);
    },
    cancel: clearToolRect,
  });
}

function createShape(box: DragBox): void {
  const variant = ['rect', 'rounded', 'ellipse', 'polygon', 'star'].includes(PM.toolShape)
    ? PM.toolShape : 'rect';
  const selected = PM.firstSel?.();
  const canMask = selected && !selected.lock && selected.type !== 'shape'
    && PM.TYPE_META?.[selected.type]?.masks !== false
    && (variant === 'rect' || variant === 'ellipse');
  if (canMask) {
    const matrix = PM.worldMatrix(selected, PM.time) as AffineMatrix;
    const inv = is3DLayer(PM,selected) ? inversePlane(planeMatrix(PM,selected,PM.time)) : null;
    const first = inv ? projectPoint(inv, {x:box.x0,y:box.y0}) : invertPoint(matrix, { x: box.x0, y: box.y0 });
    const second = inv ? projectPoint(inv, {x:box.x1,y:box.y1}) : invertPoint(matrix, { x: box.x1, y: box.y1 });
    if (!first || !second) return;
    PM.Edit.mutate('Draw mask', () => {
      const mask = PM.mkMask(variant === 'ellipse' ? 'ellipse' : 'rect');
      mask.p.x.v = (first.x + second.x) / 2;
      mask.p.y.v = (first.y + second.y) / 2;
      mask.p.w.v = Math.abs(second.x - first.x);
      mask.p.h.v = Math.abs(second.y - first.y);
      mask.p.feather.v = 0;
      selected.masks.push(mask);
      return mask;
    }, { origin: 'canvas' });
    PM.invalidate();
    PM.Inspector?.refresh?.();
    return;
  }
  const shape = variant === 'rounded' ? 'rect' : variant;
  PM.Edit.apply({
    type: 'add_layer', layerType: 'shape', name: `${variant[0].toUpperCase()}${variant.slice(1)}`,
    from: PM.snapF(PM.time, PM.proj.fps), duration: Math.max(1 / PM.proj.fps, PM.proj.dur - PM.time),
    content: {
      /* The drawn dimensions live in ordinary animated Scale channels. The
         source shape is normalized to 100px, so a Scale value in percent is
         also the drawn size in composition pixels. */
      shape, w: 100, h: 100,
      radius: variant === 'rounded' ? 20 : 0,
    },
    properties: {
      'position.x': (box.x0 + box.x1) / 2,
      'position.y': (box.y0 + box.y1) / 2,
      'scale.x': box.w,
      'scale.y': box.h,
    },
    select: true,
  }, { label: `Draw ${variant}`, origin: 'canvas' });
  PM.invalidate();
  PM.Inspector?.refresh?.();
}

function startText(e: any): void {
  const [hitX, hitY] = toComp(e);
  const hit = PM.GL.pick(hitX, hitY, PM.time);
  if (hit?.type === 'text' && !e.shiftKey) {
    editCanvasText(PM,V,hit,e);
    return;
  }
  const start = pointerComp(e);
  let moved = false;
  beginDrag(e, {
    cursor: 'text',
    move: (dx: number, dy: number, ev: any) => {
      if (!passedMoveDragThreshold(dx, dy)) return;
      moved = true;
      V.toolRect = { kind: 'text', box: shapeBoxFromDrag(start, pointerComp(ev), { fromCenter: ev.altKey }) };
      PM.invalidate('render');
    },
    up: (_dx: number, _dy: number, ev: any) => {
      const box = moved ? shapeBoxFromDrag(start, pointerComp(ev), { fromCenter: ev.altKey }) : null;
      clearToolRect();
      const position = box ? { x: box.x0, y: box.y0 } : start;
      const result = PM.Edit.apply({
        type: 'add_layer', layerType: 'text', name: 'Text',
        from: PM.snapF(PM.time, PM.proj.fps), duration: Math.max(1 / PM.proj.fps, PM.proj.dur - PM.time),
        content: {
          text: '', align: 'left',
          boxWidth: PM.P(box?.w || 0), boxHeight: PM.P(box?.h || 0),
        },
        properties: { 'position.x': position.x, 'position.y': position.y },
        select: true,
      }, { label: box ? 'New paragraph text' : 'New point text', origin: 'canvas' });
      const layer = result?.ok && result.data?.results?.[0]?.data?.layer;
      if (layer) editCanvasText(PM,V,layer);
      PM.invalidate();
    },
    cancel: clearToolRect,
  });
}

function selectionForTransformTool(e: any): SelectionGeometry | null {
  const point = pointerComp(e);
  let selection = resolveSelectionGeometry(PM, visualSelection(PM), PM.time);
  if (selection && pointInSelection(selection, point.x, point.y)) return selection;
  const layer = PM.GL.pick(point.x, point.y, PM.time);
  if (!layer) return null;
  PM.selectLayers(layer.id, e.shiftKey);
  selection = resolveSelectionGeometry(PM, visualSelection(PM), PM.time);
  return selection;
}

function startRotationTool(e: any): void {
  const selection = selectionForTransformTool(e);
  if (selection?.layers.some((layer: any) => is3DLayer(PM,layer))) { PM.toast('Use X, Y and Z Rotation in Transform for 3D layers'); return; }
  if (selection?.transformable) startTransform(e, selection, { rotate: true }, PM.time);
}

function startAnchorTool(e: any): void {
  const selection = selectionForTransformTool(e);
  if (selection?.layers.some((layer: any) => is3DLayer(PM,layer))) { PM.toast('Use Anchor X, Y and Z in Transform for 3D layers'); return; }
  if (!selection?.transformable || selection.layers.length !== 1) return;
  const pointer = pointerComp(e);
  if (Math.hypot(pointer.x - selection.pivotWorld.x, pointer.y - selection.pivotWorld.y) > 12 / Math.max(.02, V.shown)) return;
  const layer = selection.layers[0];
  const T = PM.time;
  const local = PM.localMatrix(layer, T) as AffineMatrix;
  const linear: LinearMatrix = [local[0], local[1], local[2], local[3]];
  const anchor = { x: PM.ev(layer, 'anchor.x', T), y: PM.ev(layer, 'anchor.y', T) };
  const position = { x: PM.ev(layer, 'position.x', T), y: PM.ev(layer, 'position.y', T) };
  const parent = parentWorldMatrix(layer, T);
  const bounds = PM.GL.bounds(layer, T);
  let moved = false;
  PM.Edit.begin('Move anchor point', { origin: 'canvas' });
  beginDrag(e, {
    cursor: 'crosshair',
    move: (dx: number, dy: number, ev: any) => {
      if (!passedMoveDragThreshold(dx, dy)) return;
      moved = true;
      const world = pointerComp(ev);
      const pointerInParent = parent ? invertPoint(parent, world) : world;
      if (!pointerInParent) return;
      let next = anchorMoveValues(linear, anchor, position, pointerInParent, { moveLayer: !ev.altKey });
      if (!next) return;
      if ((ev.metaKey || ev.ctrlKey) && bounds) {
        const xs = [bounds.x0, (bounds.x0 + bounds.x1) / 2, bounds.x1];
        const ys = [bounds.y0, (bounds.y0 + bounds.y1) / 2, bounds.y1];
        next.anchor.x = xs.reduce((best, value) => Math.abs(value - next!.anchor.x) < Math.abs(best - next!.anchor.x) ? value : best);
        next.anchor.y = ys.reduce((best, value) => Math.abs(value - next!.anchor.y) < Math.abs(best - next!.anchor.y) ? value : best);
        if (!ev.altKey) {
          const dax = next.anchor.x - anchor.x, day = next.anchor.y - anchor.y;
          next.position = {
            x: position.x + linear[0] * dax + linear[2] * day,
            y: position.y + linear[1] * dax + linear[3] * day,
          };
        }
      }
      setOrKey(layer, 'anchor.x', PM.round(next.anchor.x, 3), T);
      setOrKey(layer, 'anchor.y', PM.round(next.anchor.y, 3), T);
      if (!ev.altKey) {
        setOrKey(layer, 'position.x', PM.round(next.position.x, 3), T);
        setOrKey(layer, 'position.y', PM.round(next.position.y, 3), T);
      }
      PM.invalidate();
    },
    up: () => { moved ? PM.Edit.commit('Move anchor point') : PM.Edit.cancel(); PM.Inspector?.refresh?.(); },
    cancel: () => { PM.Edit.cancel(); PM.Inspector?.refresh?.(); },
  });
}

function pointInSelection(selection: SelectionGeometry, x: number, y: number): boolean {
  if (selection.mode === 'common') {
    const bounds = selection.bounds;
    return x >= bounds.x0 && x <= bounds.x1 && y >= bounds.y0 && y <= bounds.y1;
  }
  return layerContainsPoint(PM, selection.layers[0], x, y, PM.time);
}

function handleAt(selection: SelectionGeometry, x: any, y: any) {
  if (selection.layers.some((layer: any) => is3DLayer(PM,layer))) return null;
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
  const start = layers.map((L: any) => {
    const x = PM.ev(L, 'position.x', T), y = PM.ev(L, 'position.y', T);
    const m = is3DLayer(PM,L) ? planeMatrix(PM,L,T,true) : null;
    return {L,x,y,inverse:m ? inversePlane(m) : null,pivot:m ? projectPoint(m,{x,y}) : null};
  });
  const moveDelta = (s: any, dx: number, dy: number): [number,number] => {
    if (!is3DLayer(PM,s.L)) return worldDeltaToLocal(s.L,T,dx,dy);
    if (!s.inverse) return [0,0];
    const p = projectPoint(s.inverse,{x:s.pivot.x+dx,y:s.pivot.y+dy});
    return [p.x-s.x,p.y-s.y];
  };
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
        const delta = moveDelta(s, ddx, ddy);
        moveDeltas.set(s.L, delta);
        setOrKey(s.L, 'position.x', s.x + delta[0], T);
        setOrKey(s.L, 'position.y', s.y + delta[1], T);
      });
      /* Alignment is part of ordinary direct manipulation. Shift only locks
         the dominant movement axis; Command/Control temporarily bypasses
         snapping for precise free movement. Keeping those jobs separate also
         lets an unconstrained drag align both axes at once. */
      let snap: SnapResult = { dx: 0, dy: 0, lines: [] };
      const box = !(ev.metaKey || ev.ctrlKey) && !selectionLayers.some((layer: any) => is3DLayer(PM,layer)) ? unionBounds(selectionLayers, T) : null;
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
  const parent = parentWorldMatrix(L, T);
  return parent ? invertPoint(parent, point) : point;
}

function parentWorldMatrix(L: any, T: any): AffineMatrix | null {
  if (PM.transformParentMatrix) return PM.transformParentMatrix(L, T);
  const parent = L.parent && PM.L(L.parent);
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
  const textSize0 = L.type === 'text' ? Math.max(4, Number(resolveContent(PM, L, PM.time).size) || 4) : null;
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
    textSize: L.type === 'text' ? Math.max(4, Number(resolveContent(PM, L, PM.time).size) || 4) : null,
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
