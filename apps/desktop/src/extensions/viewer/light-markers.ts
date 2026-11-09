import type { LightViewerGuides, LightViewerState, Point2D, PowermoveAPI } from 'powermove';

/*
 * Light markers: every 3D light layer is drawn on the viewer overlay, like
 * After Effects' light icons. Unselected lights stay quiet; a selected light
 * gets an accent ring that doubles as its drag handle, plus its cone or range.
 * Lights outside the stage or behind the camera pin to the stage edge with a
 * chevron pointing toward them, so a light is never simply gone.
 *
 * Everything here is screen space (stage-relative CSS px). It draws only on
 * the viewer's overlay canvas, so renders and exports never contain it.
 */

export const LIGHT_DEFINITION = 'powermove.3d.light';
export const MARKER_RADIUS = 10;
export const SELECTED_RADIUS = 11.5;
export const EDGE_RADIUS = 9;
export const EDGE_INSET = 18;
const HIT_SLOP = 3;

export type LightMarker = {
  id: string;
  name: string;
  type: LightViewerState['type'];
  color: string;
  intensity: number;
  selected: boolean;
  locked: boolean;
  /** On-stage marker centre, or the pinned edge position. */
  x: number;
  y: number;
  /** Set when the light is outside the stage or behind the camera. Radians, pointing toward the light. */
  edge: number | null;
  behind: boolean;
  /** The light's true projected position when it is in front of the camera (guides start here). */
  apex: Point2D | null;
  /** Screen angle the spot glyph points along (radians). */
  heading: number | null;
  guides: LightViewerGuides | null;
};

export type StageSize = { width: number; height: number };
export type ViewerRect = { x: number; y: number; width: number; height: number };

/** Pin a point that is off-stage (or a mirrored behind-camera projection) to the stage edge. */
export function edgePlacement(point: Point2D, behind: boolean, stage: StageSize, inset = EDGE_INSET): { x: number; y: number; angle: number } {
  const cx = stage.width / 2, cy = stage.height / 2;
  let dx = point.x - cx, dy = point.y - cy;
  // Perspective division mirrors a point behind the camera through the centre.
  if (behind) { dx = -dx; dy = -dy; }
  if (Math.abs(dx) < 1e-9 && Math.abs(dy) < 1e-9) dy = 1;
  const hx = Math.max(1, cx - inset), hy = Math.max(1, cy - inset);
  const scale = Math.min(hx / Math.max(1e-9, Math.abs(dx)), hy / Math.max(1e-9, Math.abs(dy)));
  return { x: cx + dx * scale, y: cy + dy * scale, angle: Math.atan2(dy, dx) };
}

export function onStage(point: Point2D, stage: StageSize, margin = 2): boolean {
  return point.x >= margin && point.y >= margin && point.x <= stage.width - margin && point.y <= stage.height - margin;
}

/** The two rim points whose lines from the apex enclose the projected cone. */
export function coneSilhouette(apex: Point2D, rim: (Point2D | null)[]): [Point2D, Point2D] | null {
  const points = rim.filter((p): p is Point2D => !!p);
  if (points.length < 3) return null;
  const cx = points.reduce((sum, p) => sum + p.x, 0) / points.length;
  const cy = points.reduce((sum, p) => sum + p.y, 0) / points.length;
  const base = Math.atan2(cy - apex.y, cx - apex.x);
  let low: Point2D | null = null, high: Point2D | null = null, min = Infinity, max = -Infinity;
  for (const p of points) {
    let a = Math.atan2(p.y - apex.y, p.x - apex.x) - base;
    while (a > Math.PI) a -= Math.PI * 2;
    while (a < -Math.PI) a += Math.PI * 2;
    if (a < min) { min = a; low = p; }
    if (a > max) { max = a; high = p; }
  }
  // Looking straight down the cone: the apex sits inside the rim, so no silhouette.
  if (!low || !high || max - min >= Math.PI * .98) return null;
  return [low, high];
}

/** Topmost marker under the point. Selected markers win, then the nearest. */
export function markerAt(markers: readonly LightMarker[], x: number, y: number): LightMarker | null {
  let best: LightMarker | null = null, bestScore = Infinity;
  for (const marker of markers) {
    if (marker.locked) continue;
    const radius = (marker.edge != null ? EDGE_RADIUS : marker.selected ? SELECTED_RADIUS : MARKER_RADIUS) + HIT_SLOP;
    const distance = Math.hypot(x - marker.x, y - marker.y);
    if (distance > radius) continue;
    const score = distance - (marker.selected ? 100 : 0);
    if (score < bestScore) { bestScore = score; best = marker; }
  }
  return best;
}

/** Collect markers for the current composition. Returns [] when there are no lights. */
export function collectLightMarkers(api: PowermoveAPI, rect: ViewerRect, stage: StageSize, time: number,
  options: { isLocked(layer: any): boolean }): LightMarker[] {
  const scene = api.scene3d;
  if (!scene?.lightViewerState) return [];
  const layers = api.model.curComp().layers.filter((layer: any) => layer?.d?.definition === LIGHT_DEFINITION);
  if (!layers.length) return [];
  const selected = new Set(api.selection.layers());
  const out: LightMarker[] = [];
  for (const layer of layers) {
    let state: LightViewerState | null = null;
    try { state = scene.lightViewerState(layer.id, time, rect); } catch { state = null; }
    if (!state || !(api.anim.active(layer, time) ?? true)) continue;
    // Ambient and Environment light come from everywhere: no place to mark.
    if (['ambient', 'environment'].includes(String(state.type ?? (layer as any).d?.data?.light?.type))) continue;
    const isSelected = selected.has(layer.id);
    let guides: LightViewerGuides | null = null;
    try { guides = scene.lightViewerGuides?.(layer.id, time, rect) ?? null; } catch { guides = null; }
    const base = { id: layer.id, name: String(layer.name || 'Light'), type: state.type, color: state.color,
      intensity: Number(state.intensity) || 0, selected: isSelected, locked: options.isLocked(layer) };
    const position = state.viewerPosition;
    if (!position) continue;
    if (state.inFront && onStage(position, stage)) {
      const aim = guides?.aim;
      const heading = aim && Math.hypot(aim.x - position.x, aim.y - position.y) > 2 ? Math.atan2(aim.y - position.y, aim.x - position.x) : null;
      out.push({ ...base, x: position.x, y: position.y, edge: null, behind: false, apex: position, heading, guides });
    } else {
      const pin = edgePlacement(position, !state.inFront, stage);
      // A light just past the edge can still aim its cone into view.
      out.push({ ...base, x: pin.x, y: pin.y, edge: pin.angle, behind: !state.inFront,
        apex: state.inFront ? position : null, heading: null, guides: state.inFront ? guides : null });
    }
  }
  // Draw selected markers last so they sit on top.
  return out.sort((a, b) => Number(a.selected) - Number(b.selected));
}

/* ── drawing ─────────────────────────────────────────────── */

export type MarkerPalette = { accent: string };

function parseHex(color: string): [number, number, number] {
  const hex = /^#([0-9a-f]{6})/i.exec(color)?.[1] ?? 'ffffff';
  return [0, 2, 4].map(i => parseInt(hex.slice(i, i + 2), 16)) as [number, number, number];
}

/** The light's colour, lifted so very dark lights still read on the dark puck. */
export function glyphColor(color: string): string {
  const [r, g, b] = parseHex(color);
  const luminance = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
  const lift = luminance < .45 ? (.45 - luminance) / .45 * .7 : 0;
  const mix = (v: number) => Math.round(v + (255 - v) * lift);
  return `rgb(${mix(r)},${mix(g)},${mix(b)})`;
}

const withAlpha = (rgb: string, alpha: number) => rgb.replace(/^rgb\(/, 'rgba(').replace(/\)$/, `,${alpha})`);

function haloStroke(c: CanvasRenderingContext2D, ink: string, width: number, path: () => void) {
  c.save();
  c.lineJoin = 'round'; c.lineCap = 'round';
  c.strokeStyle = 'rgba(0,0,0,.4)'; c.lineWidth = width + 1.5; path(); c.stroke();
  c.strokeStyle = ink; c.lineWidth = width; path(); c.stroke();
  c.restore();
}

function polyline(c: CanvasRenderingContext2D, points: (Point2D | null)[], closed: boolean) {
  c.beginPath();
  let pen = false;
  const count = closed ? points.length + 1 : points.length;
  for (let i = 0; i < count; i++) {
    const p = points[i % points.length];
    if (!p) { pen = false; continue; }
    if (pen) c.lineTo(p.x, p.y); else c.moveTo(p.x, p.y);
    pen = true;
  }
}

function drawGuides(c: CanvasRenderingContext2D, marker: LightMarker) {
  const guides = marker.guides, apex = marker.apex;
  if (!guides || !apex) return;
  // Off-stage lights show guides only while selected.
  if (marker.edge != null && !marker.selected) return;
  const ink = glyphColor(marker.color);
  const strong = marker.selected;
  const alpha = strong ? .8 : .32;
  if (guides.rim) {
    const silhouette = coneSilhouette(apex, guides.rim);
    haloStroke(c, withAlpha(ink, alpha), 1, () => {
      polyline(c, guides.rim!, true);
      if (silhouette) for (const p of silhouette) { c.moveTo(apex.x, apex.y); c.lineTo(p.x, p.y); }
    });
    if (strong && guides.innerRim) {
      c.save(); c.setLineDash([3, 3]);
      haloStroke(c, withAlpha(ink, .45), 1, () => polyline(c, guides.innerRim!, true));
      c.restore();
    }
  }
  if (guides.aim && (marker.type === 'sun' || strong)) {
    const aim = guides.aim;
    c.save(); c.setLineDash([2, 3]);
    haloStroke(c, withAlpha(ink, marker.type === 'sun' ? alpha : .55), 1, () => { c.beginPath(); c.moveTo(apex.x, apex.y); c.lineTo(aim.x, aim.y); });
    c.restore();
    if (strong) haloStroke(c, withAlpha(ink, .85), 1.25, () => {
      c.beginPath(); c.moveTo(aim.x - 4, aim.y); c.lineTo(aim.x + 4, aim.y); c.moveTo(aim.x, aim.y - 4); c.lineTo(aim.x, aim.y + 4);
    });
  }
  // Range is a property of the selected light; on every light it would wall-paper the view.
  if (strong && guides.range && guides.range.radius > MARKER_RADIUS + 2) {
    const { center, radius } = guides.range;
    c.save(); c.setLineDash([4, 4]);
    haloStroke(c, withAlpha(ink, .6), 1, () => { c.beginPath(); c.arc(center.x, center.y, radius, 0, Math.PI * 2); });
    c.restore();
  }
}

function drawGlow(c: CanvasRenderingContext2D, marker: LightMarker) {
  if (marker.type !== 'point' || marker.intensity <= 0) return;
  const radius = Math.min(56, 16 + 9 * Math.log2(1 + marker.intensity));
  const ink = glyphColor(marker.color);
  const gradient = c.createRadialGradient(marker.x, marker.y, MARKER_RADIUS * .6, marker.x, marker.y, radius);
  gradient.addColorStop(0, withAlpha(ink, marker.selected ? .34 : .24));
  gradient.addColorStop(1, withAlpha(ink, 0));
  c.fillStyle = gradient;
  c.beginPath(); c.arc(marker.x, marker.y, radius, 0, Math.PI * 2); c.fill();
}

/** Light-type glyph centred at the origin, roughly 14 px across. */
function drawGlyph(c: CanvasRenderingContext2D, marker: LightMarker, scale: number) {
  const ink = glyphColor(marker.color);
  const dim = marker.intensity <= 0;
  c.save();
  c.scale(scale, scale);
  c.fillStyle = ink; c.strokeStyle = ink; c.lineCap = 'round'; c.lineJoin = 'round';
  if (dim) c.globalAlpha *= .55;
  if (marker.type === 'point') {
    // Bulb: globe and two base bands.
    c.beginPath(); c.arc(0, -1.3, 3.9, 0, Math.PI * 2);
    if (dim) { c.lineWidth = 1.2; c.stroke(); } else c.fill();
    c.lineWidth = 1.3;
    c.beginPath(); c.moveTo(-1.9, 3.7); c.lineTo(1.9, 3.7); c.moveTo(-1.3, 5.4); c.lineTo(1.3, 5.4); c.stroke();
  } else if (marker.type === 'sun') {
    c.beginPath(); c.arc(0, 0, 2.7, 0, Math.PI * 2);
    if (dim) { c.lineWidth = 1.2; c.stroke(); } else c.fill();
    c.lineWidth = 1.3;
    c.beginPath();
    for (let i = 0; i < 8; i++) {
      const a = i * Math.PI / 4;
      c.moveTo(Math.cos(a) * 4.7, Math.sin(a) * 4.7); c.lineTo(Math.cos(a) * 6.5, Math.sin(a) * 6.5);
    }
    c.stroke();
  } else {
    // Spot: a lamp body and the beam it throws toward where it points.
    c.rotate(marker.heading ?? Math.PI / 2);
    c.beginPath(); c.roundRect(-6, -2.7, 4.2, 5.4, 1.2);
    if (dim) { c.lineWidth = 1.1; c.stroke(); } else c.fill();
    c.globalAlpha *= .55;
    c.beginPath(); c.moveTo(-1, -1.5); c.lineTo(6, -4.6); c.lineTo(6, 4.6); c.lineTo(-1, 1.5); c.closePath(); c.fill();
  }
  c.restore();
}

/** The puck outline; an edge pin grows a tip toward the off-screen light. */
function puckPath(c: CanvasRenderingContext2D, marker: LightMarker, radius: number) {
  c.beginPath();
  if (marker.edge == null) { c.arc(marker.x, marker.y, radius, 0, Math.PI * 2); return; }
  const a = marker.edge, spread = .62, tip = radius + 6.5;
  c.arc(marker.x, marker.y, radius, a + spread, a - spread + Math.PI * 2);
  c.lineTo(marker.x + Math.cos(a) * tip, marker.y + Math.sin(a) * tip);
  c.closePath();
}

function drawPuck(c: CanvasRenderingContext2D, marker: LightMarker, radius: number, palette: MarkerPalette, hovered: boolean) {
  c.save();
  c.shadowColor = 'rgba(0,0,0,.45)'; c.shadowBlur = 4; c.shadowOffsetY = 1;
  c.fillStyle = 'rgba(28,28,30,.84)';
  puckPath(c, marker, radius); c.fill();
  c.restore();
  c.save();
  c.lineJoin = 'round';
  if (marker.selected) {
    c.strokeStyle = palette.accent; c.lineWidth = 2;
    puckPath(c, marker, radius - 1); c.stroke();
    if (marker.edge == null) {
      c.globalAlpha = .28; c.lineWidth = 3;
      c.beginPath(); c.arc(marker.x, marker.y, radius + 2.5, 0, Math.PI * 2); c.stroke();
    }
  } else {
    c.strokeStyle = hovered ? 'rgba(255,255,255,.95)' : 'rgba(255,255,255,.6)'; c.lineWidth = 1;
    puckPath(c, marker, radius - .5); c.stroke();
  }
  c.restore();
}

/** Paint markers. The context must be in stage CSS px (the caller applies device pixel ratio). */
export function drawLightMarkers(c: CanvasRenderingContext2D, markers: readonly LightMarker[], palette: MarkerPalette, hoverId: string | null): void {
  if (!markers.length) return;
  c.save();
  for (const marker of markers) drawGuides(c, marker);
  for (const marker of markers) if (marker.edge == null) drawGlow(c, marker);
  for (const marker of markers) {
    const hovered = marker.id === hoverId;
    c.save();
    if (!marker.selected && !hovered) c.globalAlpha = marker.locked ? .45 : .82;
    if (marker.edge != null) {
      if (marker.behind && !marker.selected && !hovered) c.globalAlpha *= .8;
      drawPuck(c, marker, EDGE_RADIUS, palette, hovered);
      c.translate(marker.x, marker.y);
      drawGlyph(c, marker, .82);
    } else {
      const radius = marker.selected ? SELECTED_RADIUS : MARKER_RADIUS;
      drawPuck(c, marker, radius, palette, hovered);
      c.translate(marker.x, marker.y);
      drawGlyph(c, marker, marker.selected ? 1.05 : 1);
    }
    c.restore();
  }
  c.restore();
}

/** Short tooltip text for a hovered marker. */
export function markerTitle(marker: LightMarker): string {
  if (marker.edge == null) return marker.name;
  return `${marker.name} — ${marker.behind ? 'behind the camera' : 'outside the view'}`;
}
