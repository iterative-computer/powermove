/* Track height zoom: Alt + wheel (Option + wheel on macOS) over the rows.

   The workspace's timeline chrome sets the base row height; the zoom is a
   view multiplier on top of it, so switching to a denser or roomier
   workspace keeps the user's relative choice. Each view clamps its own
   height: layer rows must still fit the 16px type swatch, tracks the 18px
   name chip, and neither grows past a few clips' worth of screen. */
import type { TimelineMode } from './track-view';

export const ROW_HEIGHT_LIMITS: Record<TimelineMode, { min: number; max: number }> = {
  layers: { min: 20, max: 96 },
  tracks: { min: 26, max: 120 },
};

/** Per wheel pixel; a 100px mouse notch changes height by about 16%. */
const ZOOM_PER_PIXEL = .0015;
/** A wheel "line" (deltaMode 1), as Chromium and WebView2 report a notch of 3 lines as ~100px. */
const LINE_PIXELS = 33;

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

export const normalizeRowZoom = (value: unknown): number =>
  typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : 1;

/** Track lanes carry a header chip, so they sit a little taller than layer rows. */
const unzoomed = (base: number, mode: TimelineMode) =>
  mode === 'tracks' ? Math.max(34, Math.round(base) + 6) : Math.round(base);

export function rowHeight(base: number, zoom: number, mode: TimelineMode): number {
  const { min, max } = ROW_HEIGHT_LIMITS[mode];
  return clamp(Math.round(unzoomed(base, mode) * normalizeRowZoom(zoom)), min, max);
}

/** The zoom range over which the height actually changes, so reversing at a bound responds at once. */
export function rowZoomLimits(base: number, mode: TimelineMode): [number, number] {
  const { min, max } = ROW_HEIGHT_LIMITS[mode];
  const height = Math.max(1, unzoomed(base, mode));
  return [min / height, max / height];
}

type WheelLike = Pick<WheelEvent, 'deltaX' | 'deltaY' | 'deltaMode'>;

/** Wheel travel in pixels. Some Windows drivers turn Alt + wheel into a
    horizontal wheel, so the dominant axis counts. */
export function wheelPixels(event: WheelLike, page: number): number {
  const delta = Math.abs(event.deltaY) >= Math.abs(event.deltaX) ? event.deltaY : event.deltaX;
  return (Number(delta) || 0) * (event.deltaMode === 1 ? LINE_PIXELS : event.deltaMode === 2 ? Math.max(100, page) : 1);
}

/** Next zoom for a wheel event: wheel up / swipe up grows the rows. */
export function wheelRowZoom(zoom: number, event: WheelLike, view: { base: number; mode: TimelineMode; page: number }): number {
  const [min, max] = rowZoomLimits(view.base, view.mode);
  const factor = clamp(Math.exp(-wheelPixels(event, view.page) * ZOOM_PER_PIXEL), .5, 2);
  return clamp(clamp(normalizeRowZoom(zoom), min, max) * factor, min, max);
}

/** Where a point `offset` px below a row's top lands after the row's header
    height changes. Below the header (unfolded property lanes) keeps its
    fixed-height offset. */
export const anchoredOffset = (offset: number, before: number, after: number) =>
  offset < before ? offset * after / Math.max(1, before) : offset - before + after;
