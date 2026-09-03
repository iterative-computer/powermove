/**
 * Per-project export defaults.
 *
 * The Export dialog opens with these values, and the Settings › Project tab
 * edits them, so a project remembers how it is meant to be delivered instead of
 * relying on one global preference shared by every project.
 */

export type ExportFormat = 'mp4' | 'webm' | 'rec' | 'png' | 'still' | 'json';
export type ExportQuality = 'draft' | 'high' | 'max';
export type ExportRange = 'work' | 'all';

export interface ExportDefaults {
  format: ExportFormat;
  /** Multiplier applied to the composition size when rendering. */
  scale: number;
  fps: number;
  range: ExportRange;
  quality: ExportQuality;
  mblur: boolean;
  audio: boolean;
  alpha: boolean;
}

export interface ExportOption<T> {
  v: T;
  label: string;
}

export const EXPORT_FORMAT_OPTIONS: Array<ExportOption<ExportFormat>> = [
  { v: 'mp4', label: 'MP4 · H.264' },
  { v: 'webm', label: 'WebM · VP9' },
  { v: 'rec', label: 'WebM · realtime capture' },
  { v: 'png', label: 'PNG sequence' },
  { v: 'still', label: 'Still frame (PNG)' },
  { v: 'json', label: 'Project file (.pmv)' }
];

export const EXPORT_SCALE_OPTIONS: Array<ExportOption<number>> = [
  { v: 0.5, label: 'Half' },
  { v: 1, label: 'Full' },
  { v: 2, label: '2×' }
];

export const EXPORT_QUALITY_OPTIONS: Array<ExportOption<ExportQuality>> = [
  { v: 'draft', label: 'Draft · 4 Mbps' },
  { v: 'high', label: 'High · 16 Mbps' },
  { v: 'max', label: 'Max · 40 Mbps' }
];

export const EXPORT_RANGE_OPTIONS: Array<ExportOption<ExportRange>> = [
  { v: 'work', label: 'Work area' },
  { v: 'all', label: 'Full composition' }
];

export const EXPORT_FRAME_RATES = [24, 25, 30, 50, 60];

/** Which delivery settings a format actually reads; the rest are noise. */
export interface ExportFieldSupport {
  scale: boolean;
  fps: boolean;
  range: boolean;
  quality: boolean;
  mblur: boolean;
  audio: boolean;
  alpha: boolean;
}

const NO_FIELDS: ExportFieldSupport = {
  scale: false, fps: false, range: false, quality: false, mblur: false, audio: false, alpha: false
};

const FIELD_SUPPORT: Record<ExportFormat, ExportFieldSupport> = {
  mp4: { ...NO_FIELDS, scale: true, fps: true, range: true, quality: true, mblur: true, audio: true },
  webm: { ...NO_FIELDS, scale: true, fps: true, range: true, quality: true, mblur: true, audio: true },
  rec: { ...NO_FIELDS, scale: true, fps: true, range: true, quality: true, mblur: true, audio: true },
  png: { ...NO_FIELDS, scale: true, fps: true, range: true, mblur: true, alpha: true },
  still: { ...NO_FIELDS, scale: true, mblur: true, alpha: true },
  json: NO_FIELDS
};

export function exportFieldSupport(format: ExportFormat): ExportFieldSupport {
  return FIELD_SUPPORT[format] ?? FIELD_SUPPORT.webm;
}

/** Video bitrate each quality tier encodes at, in Mbps. */
export function exportBitrateMbps(quality: ExportQuality): number {
  return quality === 'draft' ? 4 : quality === 'high' ? 16 : 40;
}

/** What the primary action of the Export dialog should promise. */
export function exportActionLabel(format: ExportFormat): string {
  switch (format) {
    case 'png': return 'Export frames';
    case 'still': return 'Export frame';
    case 'json': return 'Save project file';
    default: return 'Export video';
  }
}

export interface ExportCompositionInfo {
  w: number;
  h: number;
  dur: number;
  /** Work-area bounds in seconds, when one is set. */
  work?: readonly [number, number] | null;
}

export interface ExportPlan {
  /** Output size, rounded to even pixels the way the renderer does. */
  width: number;
  height: number;
  seconds: number;
  frames: number;
  /** Format-specific line: estimated size, file count, or what gets packed. */
  note: string;
}

/** The headline numbers the Export dialog shows before committing to a render. */
export function planExport(opts: ExportDefaults, comp: ExportCompositionInfo): ExportPlan {
  const width = Math.max(2, Math.round(comp.w * opts.scale / 2) * 2);
  const height = Math.max(2, Math.round(comp.h * opts.scale / 2) * 2);
  const work = comp.work;
  const useWork = opts.range === 'work' && !!work && work[1] > work[0];
  const seconds = Math.round(((useWork ? work![1] - work![0] : comp.dur)) * 1000) / 1000;
  const frames = opts.format === 'still' ? 1 : Math.max(1, Math.round(seconds * opts.fps));
  let note = '';
  if (opts.format === 'mp4' || opts.format === 'webm' || opts.format === 'rec') {
    const mb = exportBitrateMbps(opts.quality) * seconds / 8;
    const codec = opts.format === 'mp4' ? 'H.264' : 'VP9/VP8';
    note = `≈ ${Math.round(mb * 10) / 10} MB · ${exportBitrateMbps(opts.quality)} Mbps ${codec}`;
  } else if (opts.format === 'png') {
    note = `${frames} PNG file${frames === 1 ? '' : 's'}${opts.alpha ? ' · transparent' : ''}`;
  } else if (opts.format === 'still') {
    note = `Single PNG${opts.alpha ? ' · transparent' : ''}`;
  } else {
    note = 'Packs the composition and its media into one .pmv file';
  }
  return { width, height, seconds, frames, note };
}

const FORMATS = new Set(EXPORT_FORMAT_OPTIONS.map((option) => option.v));
const QUALITIES = new Set(EXPORT_QUALITY_OPTIONS.map((option) => option.v));
const RANGES = new Set(EXPORT_RANGE_OPTIONS.map((option) => option.v));

/** Any output size is allowed; these bound the resulting scale factor. */
export const MIN_EXPORT_SCALE = 0.01;
export const MAX_EXPORT_SCALE = 16;

export function clampExportScale(value: unknown, fallback = 1): number {
  const scale = Number(value);
  if (!Number.isFinite(scale) || scale <= 0) return fallback;
  return Math.min(MAX_EXPORT_SCALE, Math.max(MIN_EXPORT_SCALE, scale));
}

function pick<T>(allowed: Set<T>, value: unknown, fallback: T): T {
  return allowed.has(value as T) ? (value as T) : fallback;
}

function bool(value: unknown, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback;
}

/** Coerce stored or partial export options into a complete, safe set. */
export function normalizeExportDefaults(raw: unknown, fallbackFps = 30): ExportDefaults {
  const source = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {};
  const fps = Number(source.fps);
  const baseFps = Number.isFinite(fallbackFps) && fallbackFps > 0 ? Math.round(fallbackFps) : 30;
  return {
    format: pick(FORMATS, source.format, 'mp4'),
    scale: clampExportScale(source.scale),
    fps: Number.isFinite(fps) && fps > 0 ? Math.min(240, Math.round(fps)) : baseFps,
    range: pick(RANGES, source.range, 'work'),
    quality: pick(QUALITIES, source.quality, 'high'),
    mblur: bool(source.mblur, true),
    audio: bool(source.audio, true),
    alpha: bool(source.alpha, false)
  };
}
