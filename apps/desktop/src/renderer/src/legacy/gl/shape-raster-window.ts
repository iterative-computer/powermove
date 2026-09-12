export type RasterWindow = { x: number; y: number; width: number; height: number };

/** Keep the crop on the original bitmap's pixel grid, including ceil rounding. */
export function shapeRasterGeometry(d: any, scale: number) {
  const pad = Math.ceil((d.stroke || 0) / 2) + 4;
  const w = d.w + pad * 2, h = d.h + pad * 2;
  const density = Math.min(scale, 8192 / w, 8192 / h);
  const stroke = Math.max(0, Number(d.stroke) || 0) / 2;
  const selection = {
    x0: -d.w / 2 - stroke, y0: -d.h / 2 - stroke,
    x1: d.w / 2 + stroke, y1: d.h / 2 + stroke,
    w: d.w + stroke * 2, h: d.h + stroke * 2,
  };
  return { pad, w, h, density, width: Math.max(1, Math.ceil(w * density)), height: Math.max(1, Math.ceil(h * density)), selection };
}

type Plan = { kind: 'full' | 'outside' | 'solid' } | { kind: 'crop'; window: RasterWindow };

/** Conservative visible source region for a plain 2D primitive. Complex
 * compositing and export never use this preview-only optimization. */
export function previewShapeRaster(d: any, scale: number, m: readonly number[], width: number, height: number): Plan {
  if (!(d.w > 0 && d.h > 0) || m.length < 6 || !m.every(Number.isFinite)) return { kind: 'full' };
  const [a, b, c, e, tx, ty] = m as [number, number, number, number, number, number];
  const det = a * e - b * c;
  if (Math.abs(det) < 1e-12) return { kind: 'full' };
  const geometry = shapeRasterGeometry(d, scale);
  // Include interpolation/antialiasing support outside the visible viewport.
  const points = [[-2, -2], [width + 2, -2], [-2, height + 2], [width + 2, height + 2]].map(([x, y]) => {
    const dx = x! - tx, dy = y! - ty;
    return { x: (e * dx - c * dy) / det, y: (-b * dx + a * dy) / det };
  });
  const x0 = Math.min(...points.map(p => p.x)), x1 = Math.max(...points.map(p => p.x));
  const y0 = Math.min(...points.map(p => p.y)), y1 = Math.max(...points.map(p => p.y));
  if (x1 < -geometry.w / 2 || x0 > geometry.w / 2 || y1 < -geometry.h / 2 || y0 > geometry.h / 2) return { kind: 'outside' };

  // A viewport wholly inside a rounded rectangle sees only its uniform fill.
  // Exclude strokes, corners and raster sampling margins conservatively.
  const inset = Math.max(0, Math.min(Number(d.radius) || 0, d.w / 2, d.h / 2))
    + Math.max(0, Number(d.stroke) || 0) / 2 + 8 / geometry.density;
  if (d.shape === 'rect' && /^#[0-9a-f]{6}$/i.test(d.color)
      && x0 > -d.w / 2 + inset && x1 < d.w / 2 - inset
      && y0 > -d.h / 2 + inset && y1 < d.h / 2 - inset) return { kind: 'solid' };

  // Buckets keep small pans from allocating a new source on every wheel event.
  const step = 256;
  const left = Math.max(0, Math.floor(((x0 + geometry.w / 2) * geometry.width / geometry.w - 2) / step) * step);
  const top = Math.max(0, Math.floor(((y0 + geometry.h / 2) * geometry.height / geometry.h - 2) / step) * step);
  const right = Math.min(geometry.width, Math.ceil(((x1 + geometry.w / 2) * geometry.width / geometry.w + 2) / step) * step);
  const bottom = Math.min(geometry.height, Math.ceil(((y1 + geometry.h / 2) * geometry.height / geometry.h + 2) / step) * step);
  if (right <= left || bottom <= top) return { kind: 'outside' };
  if (!left && !top && right === geometry.width && bottom === geometry.height) return { kind: 'full' };
  return { kind: 'crop', window: { x: left, y: top, width: right - left, height: bottom - top } };
}

/** Conservative visibility for a complete source bitmap, including its
 * transparent padding. Text keeps the original bitmap and pixel placement. */
export function rasterIntersectsViewport(g: { w: number; h: number; anchorX: number; anchorY: number }, m: readonly number[], width: number, height: number): boolean {
  if (!(g.w > 0 && g.h > 0) || m.length < 6 || !m.every(Number.isFinite)) return true;
  const [a, b, c, d, tx, ty] = m as [number, number, number, number, number, number];
  const det = a * d - b * c;
  if (Math.abs(det) < 1e-12) return true;
  const points = [[-2, -2], [width + 2, -2], [-2, height + 2], [width + 2, height + 2]].map(([x, y]) => {
    const dx = x! - tx, dy = y! - ty;
    return { x: (d * dx - c * dy) / det + g.anchorX, y: (-b * dx + a * dy) / det + g.anchorY };
  });
  const x0 = Math.min(...points.map(p => p.x)), x1 = Math.max(...points.map(p => p.x));
  const y0 = Math.min(...points.map(p => p.y)), y1 = Math.max(...points.map(p => p.y));
  return !(x1 < 0 || y1 < 0 || x0 > g.w || y0 > g.h);
}

/** Integer scissor regions outside a guaranteed opaque rectangle. Round the
 * occluder inward so partially covered edge pixels always retain their source. */
export function uncoveredRasterRegions(width: number, height: number, cover: RasterWindow): RasterWindow[] {
  const left = Math.max(0, Math.min(width, Math.ceil(cover.x)));
  const top = Math.max(0, Math.min(height, Math.ceil(cover.y)));
  const right = Math.max(0, Math.min(width, Math.floor(cover.x + cover.width)));
  const bottom = Math.max(0, Math.min(height, Math.floor(cover.y + cover.height)));
  if (right <= left || bottom <= top) return [{x:0,y:0,width,height}];
  return [
    {x:0,y:0,width,height:top},
    {x:0,y:bottom,width,height:height-bottom},
    {x:0,y:top,width:left,height:bottom-top},
    {x:right,y:top,width:width-right,height:bottom-top},
  ].filter(region => region.width > 0 && region.height > 0);
}
