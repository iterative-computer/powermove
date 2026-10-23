/**
 * Per-project export defaults.
 *
 * The Export dialog opens with these values, and the Settings › Project tab
 * edits them, so a project remembers how it is meant to be delivered instead of
 * relying on one global preference shared by every project.
 *
 * Formats, delivery options, presets, and estimates come from
 * `shared/delivery.ts` (plan section 17). Today's stored format ids migrate:
 * `prores` is ProRes 4444, `mp4` H.264, and `webm` and the realtime capture
 * `rec` the frame-exact VP9 export.
 */
import {
  DEFAULT_DELIVERY,
  estimateBytes,
  fieldSupport,
  formatBytes,
  FORMATS,
  formatInfo,
  isRenderFormat,
  migrateFormat,
  normalizeDelivery,
  storedFormatId,
  type DeliveryOptions,
  type ExportQuality as SharedQuality,
  type ExportRange as SharedRange,
  type FieldSupport,
  type SheetFormat
} from '../../../shared/delivery';

export type ExportFormat = SheetFormat;
export type ExportQuality = SharedQuality;
export type ExportRange = SharedRange;

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
  /** Format-specific delivery options (depth, container, audio coding,
   *  timecode, reel, tags, file name, overwrite policy). */
  delivery: DeliveryOptions;
}

export interface ExportOption<T> {
  v: T;
  label: string;
}

/** Every format the sheet offers; formats only the native engine writes are
 *  filtered with `exportFormatOptions(engine)`. */
export const EXPORT_FORMAT_OPTIONS: Array<ExportOption<ExportFormat>> = FORMATS.map((f) => ({ v: f.id, label: f.label }));

/** The formats this launch can write: all of them with the native engine, and
 *  today's otherwise. */
export function exportFormatOptions(engine: boolean): Array<ExportOption<ExportFormat>> {
  return FORMATS.filter((f) => engine || !f.engine).map((f) => ({ v: f.id, label: f.label }));
}

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

export const EXPORT_FRAME_RATES = [24000/1001, 24, 25, 30000/1001, 30, 50, 60000/1001, 60];

/** Which delivery settings a format actually reads; the rest are noise. */
export type ExportFieldSupport = FieldSupport;

/** The settings `format` reads; with `engine` off only today's settings
 *  (the current exporter ignores the delivery options). */
export function exportFieldSupport(format: ExportFormat, delivery: Pick<DeliveryOptions, 'container' | 'highDepth'> = DEFAULT_DELIVERY, engine = false): ExportFieldSupport {
  return fieldSupport(format, delivery, engine);
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
    case 'web': return 'Export code';
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
  /** Estimated bytes of a render format (section 17, "Comfort"). */
  bytes: number | null;
}

/** The headline numbers the Export dialog shows before committing to a render. */
export function planExport(opts: ExportDefaults, comp: ExportCompositionInfo): ExportPlan {
  const image = opts.format === 'png' || opts.format === 'still';
  // Video is even-sized (subsampled chroma); PNG takes any size.
  const width = image ? Math.max(1, Math.round(comp.w * opts.scale)) : Math.max(2, Math.round(comp.w * opts.scale / 2) * 2);
  const height = image ? Math.max(1, Math.round(comp.h * opts.scale)) : Math.max(2, Math.round(comp.h * opts.scale / 2) * 2);
  const work = comp.work;
  const useWork = opts.range === 'work' && !!work && work[1] > work[0];
  const seconds = Math.round(((useWork ? work![1] - work![0] : comp.dur)) * 1000) / 1000;
  const frames = opts.format === 'still' ? 1 : Math.max(1, Math.round(seconds * opts.fps));
  const delivery = opts.delivery ?? DEFAULT_DELIVERY;
  const bytes = isRenderFormat(opts.format)
    ? estimateBytes({ format: opts.format, delivery, width, height, fps: opts.fps, seconds, frames, quality: opts.quality, alpha: opts.alpha, audio: opts.audio })
    : null;
  let note = '';
  const size = bytes === null ? '' : formatBytes(bytes);
  switch (opts.format) {
    case 'h264': case 'hevc': case 'webm-vp9': case 'webm-vp8': {
      const codec = opts.format === 'h264' ? 'H.264' : opts.format === 'hevc' ? `HEVC${delivery.highDepth ? ' 10-bit' : ''}` : opts.format === 'webm-vp9' ? 'VP9' : 'VP8';
      note = `${size} · ${exportBitrateMbps(opts.quality)} Mbps ${codec}`;
      break;
    }
    case 'png':
      note = `${frames} PNG file${frames === 1 ? '' : 's'}${opts.alpha ? ' · transparent' : ''} · ${size}`;
      break;
    case 'still':
      note = `Single PNG${opts.alpha ? ' · transparent' : ''}`;
      break;
    case 'web':
      note = 'Live animation · JavaScript player, React component, assets and agent handoff · full composition';
      break;
    case 'json':
      note = 'Packs the composition and its media into one .pmv file';
      break;
    default:
      // ProRes.
      note = `${size} · ${formatInfo(opts.format).label.replace('QuickTime · ', '')}${opts.alpha ? ' · alpha' : ''}`;
  }
  return { width, height, seconds, frames, note, bytes };
}

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
  const baseFps = Number.isFinite(fallbackFps) && fallbackFps > 0 ? Math.min(240, fallbackFps) : 30;
  return {
    format: migrateFormat(source.format, 'h264'),
    scale: clampExportScale(source.scale),
    fps: Number.isFinite(fps) && fps > 0 ? Math.min(240, fps) : baseFps,
    range: pick(RANGES, source.range, 'work'),
    quality: pick(QUALITIES, source.quality, 'high'),
    mblur: bool(source.mblur, true),
    audio: bool(source.audio, true),
    alpha: bool(source.alpha, false),
    delivery: normalizeDelivery(source.delivery)
  };
}

/**
 * Export defaults as the project document stores them: `format` keeps today's
 * id where one exists (issue P1: apiVersion 1 to 3 extensions read
 * `project.exportDefaults` as today's app wrote it). Read them back through
 * `normalizeExportDefaults`, which migrates the id. `previous` is the stored
 * value being replaced, so an alias of the same format is kept.
 */
export type StoredExportDefaults = Omit<ExportDefaults, 'format'> & { format: string };

export function storedExportDefaults(defaults: ExportDefaults, previous?: unknown): StoredExportDefaults {
  const before = previous && typeof previous === 'object' ? (previous as { format?: unknown }).format : previous;
  return { ...defaults, format: storedFormatId(defaults.format, before) };
}
