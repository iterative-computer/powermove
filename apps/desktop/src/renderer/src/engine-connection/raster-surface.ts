/*
 * `render.raster()` with "Use New Engine" on (the Phase 3c render
 * transport, RFC 16): the engine paints the layer's raster and this module
 * gives it today's `RasterSurface` shape (`legacy/gl/raster.ts`):
 *
 *   text      { cv, w, h, anchorX, anchorY, selection, blank, sourceWindow?, fontOffset? }
 *   captions  { cv, w, h, anchorX, anchorY, selection, blank }
 *   paths     { cv, w, h, anchorX, anchorY, selection }
 *   shapes    { cv, w, h, anchorX, anchorY, selection, uv }
 *
 * `cv` is a canvas holding the engine's premultiplied pixels (as a 2D
 * canvas holds today's), `w`/`h` the raster box in layer pixels, the
 * anchor where the layer origin sits in it, `selection` today's selection
 * box. A crop comes back as today's does: plain text with `sourceWindow`
 * (`[x, y, canvas width, canvas height]`), shapes with `uv`; the others
 * ignore it. The engine never paints a blank stand-in for a loading font,
 * so `blank` is always false.
 *
 * The layer is the document's when the caller passes the document's own
 * object; otherwise (a wrapper's modified copy) its JSON goes with the
 * request and the engine draws that object in the document's place.
 */
import type { EngineRenderTransport, EngineSnapshot } from './render-transport';

/** Today's crop window (`legacy/gl/shape-raster-window.ts`); the kernel's
 *  typing names it `{ x, y, w, h }`, so both spellings are read. */
export type RasterCrop = { x?: number; y?: number; width?: number; height?: number; w?: number; h?: number } | null | undefined;

export type EngineRasterSurface = {
  cv: HTMLCanvasElement;
  w: number;
  h: number;
  anchorX: number;
  anchorY: number;
  selection?: { x0: number; y0: number; x1: number; y1: number; w: number; h: number };
  blank?: boolean;
  sourceWindow?: [number, number, number, number];
  uv?: [number, number, number, number];
  /** Typography anchoring's shift of a text layer (today's `fontOffset`). */
  fontOffset?: { x: number; y: number };
  /** The engine's tags: the revision and quality the pixels show. */
  revision: number;
};

export type RasterHost = {
  /** The composition a layer object belongs to (the open one or a nested
   *  composition) and whether it is the document's own object. */
  locate(layer: unknown): { composition: string | null; own: boolean };
  /** Bounds of a captions layer at a time (today's caption block). */
  captionBounds?(layer: unknown, time: number): { x0: number; y0: number; x1: number; y1: number } | null;
};

/** The crop as the engine takes it, or null (none, or not a window). */
export function engineCrop(crop: RasterCrop): { x: number; y: number; width: number; height: number } | null {
  if (!crop || typeof crop !== 'object') return null;
  const width = Number(crop.width ?? crop.w), height = Number(crop.height ?? crop.h);
  const x = Number(crop.x ?? 0), y = Number(crop.y ?? 0);
  if (!(Number.isFinite(width) && Number.isFinite(height) && Number.isFinite(x) && Number.isFinite(y))) return null;
  if (width < 1 || height < 1) return null;
  return { x: Math.max(0, Math.round(x)), y: Math.max(0, Math.round(y)), width: Math.round(width), height: Math.round(height) };
}

const encoder = new TextEncoder();

function kindOf(layer: any): 'captions' | 'paths' | 'text' | 'shape' {
  if (layer?.type === 'captions') return 'captions';
  if (Array.isArray(layer?.d?.paths) && layer.d.paths.length) return 'paths';
  if (layer?.type === 'text') return 'text';
  return 'shape';
}

/** Builds today's surface from an engine raster. */
export function toRasterSurface(snapshot: EngineSnapshot, layer: any, crop: ReturnType<typeof engineCrop>, canvas: (snapshot: EngineSnapshot) => HTMLCanvasElement, host?: RasterHost, time = 0): EngineRasterSurface {
  const kind = kindOf(layer);
  const [w, h] = snapshot.layerBox ?? [snapshot.width, snapshot.height];
  const geometry = snapshot.raster;
  const box = (s: [number, number, number, number]) => ({ x0: s[0], y0: s[1], x1: s[2], y1: s[3], w: s[2] - s[0], h: s[3] - s[1] });
  let selection = geometry?.selection ? box(geometry.selection) : undefined;
  if (!selection && kind === 'captions') {
    const b = host?.captionBounds?.(layer, time);
    if (b) selection = box([b.x0, b.y0, b.x1, b.y1]);
  }
  const surface: EngineRasterSurface = { cv: canvas(snapshot), w, h, anchorX: snapshot.origin[0], anchorY: snapshot.origin[1], revision: snapshot.revision };
  if (selection) surface.selection = selection;
  if (kind === 'text' || kind === 'captions') surface.blank = false;
  if (kind === 'text' && geometry?.fontOffset) surface.fontOffset = { x: geometry.fontOffset[0], y: geometry.fontOffset[1] };
  const [cw, ch] = geometry?.canvas ?? [snapshot.width, snapshot.height];
  if (kind === 'text' && crop && geometry?.cropped) surface.sourceWindow = [crop.x, crop.y, cw, ch];
  if (kind === 'shape') surface.uv = crop && geometry?.cropped ? [-crop.x / crop.width, -crop.y / crop.height, cw / crop.width, ch / crop.height] : undefined;
  return surface;
}

/**
 * The raster of `layer` from the engine, synchronously (`render.raster` at
 * apiVersion 1 to 3). Null when it draws nothing; throws
 * `EngineSnapshotError` on failure.
 */
export function engineRasterSync(transport: EngineRenderTransport, host: RasterHost, layer: any, scale: number, time: number, crop: RasterCrop): EngineRasterSurface | null {
  const window = engineCrop(crop);
  const where = host.locate(layer);
  const snapshot = transport.rasterSync(String(layer?.id ?? ''), scale, time, window, rasterOptions(layer, where));
  return snapshot ? toRasterSurface(withoutInk(snapshot, layer, scale), layer, window, (s) => transport.toCanvas(s), host, time) : null;
}

/** The same, asynchronously (`render.rasterAsync`, apiVersion 4). */
export async function engineRaster(transport: EngineRenderTransport, host: RasterHost, layer: any, scale: number, time: number, crop: RasterCrop): Promise<EngineRasterSurface | null> {
  const window = engineCrop(crop);
  const where = host.locate(layer);
  const snapshot = await transport.raster(String(layer?.id ?? ''), scale, time, window, rasterOptions(layer, where));
  return snapshot ? toRasterSurface(withoutInk(snapshot, layer, scale), layer, window, (s) => transport.toCanvas(s), host, time) : null;
}

/** Playback's base pixels, asynchronously prepared at dependency priority. */
export async function engineRasterPrepared(transport: EngineRenderTransport, host: RasterHost, layer: any, scale: number, time: number, crop: RasterCrop): Promise<EngineRasterSurface | null> {
  const window = engineCrop(crop);
  const where = host.locate(layer);
  const snapshot = await transport.rasterForCompat(String(layer?.id ?? ''), scale, time, window, rasterOptions(layer, where));
  return snapshot ? toRasterSurface(withoutInk(snapshot, layer, scale), layer, window, s => transport.toCanvas(s), host, time) : null;
}

/** Today's raster at zero, a negative scale, or NaN draws through a
 *  degenerate density onto a canvas of at most one pixel: nothing lands on
 *  it (the captions rasterizer alone floors its density at 0.05). The
 *  engine draws such a request at a vanishing density (`layerRasterScale`),
 *  where its text rasterizer can still leave a faint pixel, so the
 *  surface is cleared to today's transparent canvas. */
function withoutInk(snapshot: EngineSnapshot, layer: any, scale: number): EngineSnapshot {
  if (scale > 0 || kindOf(layer) === 'captions') return snapshot;
  snapshot.data.fill(0);
  return snapshot;
}

function rasterOptions(layer: any, where: { composition: string | null; own: boolean }): { composition?: string; content: Uint8Array | null } {
  return {
    composition: where.composition ?? undefined,
    content: where.own ? null : encoder.encode(JSON.stringify(layer))
  };
}

/** Where a layer object is in `PM`'s document: its composition, and whether
 *  it is the document's own object (`unwrap` sees through kernel proxies). */
export function locateLayer(PM: any, layer: unknown, unwrap: (v: any) => any = (v) => v): { composition: string | null; own: boolean } {
  const raw = unwrap(layer);
  const proj = unwrap(PM?.proj);
  if (!proj || !raw || typeof raw !== 'object') return { composition: null, own: false };
  const open = String(proj.compId ?? proj.id ?? '');
  const has = (layers: unknown) => Array.isArray(layers) && layers.some((l) => unwrap(l) === raw);
  const current = unwrap(PM?.curComp?.() ?? proj);
  const currentId = current === proj ? open : typeof current?.id === 'string' ? current.id : open;
  if (has(current?.layers)) return { composition: currentId, own: true };
  if (has(proj.layers)) return { composition: open, own: true };
  for (const [id, comp] of Object.entries((proj.comps ?? {}) as Record<string, any>)) {
    if (has(unwrap(comp)?.layers)) return { composition: id, own: true };
  }
  return { composition: currentId, own: false };
}
