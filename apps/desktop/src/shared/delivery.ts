/*
 * Delivery formats, presets, and comfort features (plan section 17, "Formats at
 * cutover", "Presets and the render queue", "Comfort"), shared by the export
 * sheet, the render queue, main's job submission, and agent exports.
 *
 * Formats are named by the sheet's ids; `wireFormat` turns one into the job
 * engine's `ExportFormat`. Formats marked `engine` need the native engine
 * (behind "Use New Engine" until cutover); the current exporter writes the
 * others (`legacyFormatId`).
 */
import type {
  AlphaOutput,
  AudioCoding,
  ExportFormat as WireFormat,
  OutputColor
} from './engine-protocol';

export type DeliveryFormat =
  | 'prores-proxy'
  | 'prores-lt'
  | 'prores-422'
  | 'prores-hq'
  | 'prores-4444'
  | 'prores-4444xq'
  | 'h264'
  | 'hevc'
  | 'webm-vp9'
  | 'webm-vp8'
  | 'png'
  | 'still';

/** The sheet's formats, including code and project export (not render jobs). */
export type SheetFormat = DeliveryFormat | 'web' | 'json';

export type DeliveryCategory = 'video' | 'images' | 'code' | 'project';

export interface FormatInfo {
  id: SheetFormat;
  label: string;
  category: DeliveryCategory;
  /** Only the native engine writes it (the current exporter cannot). */
  engine: boolean;
}

export const FORMATS: readonly FormatInfo[] = [
  { id: 'prores-proxy', label: 'QuickTime · ProRes 422 Proxy', category: 'video', engine: true },
  { id: 'prores-lt', label: 'QuickTime · ProRes 422 LT', category: 'video', engine: true },
  { id: 'prores-422', label: 'QuickTime · ProRes 422', category: 'video', engine: true },
  { id: 'prores-hq', label: 'QuickTime · ProRes 422 HQ', category: 'video', engine: true },
  { id: 'prores-4444', label: 'QuickTime · ProRes 4444', category: 'video', engine: false },
  { id: 'prores-4444xq', label: 'QuickTime · ProRes 4444 XQ', category: 'video', engine: true },
  { id: 'h264', label: 'MP4 · H.264', category: 'video', engine: false },
  { id: 'hevc', label: 'HEVC', category: 'video', engine: true },
  { id: 'webm-vp9', label: 'WebM · VP9', category: 'video', engine: false },
  { id: 'webm-vp8', label: 'WebM · VP8', category: 'video', engine: true },
  { id: 'png', label: 'PNG sequence', category: 'images', engine: false },
  { id: 'still', label: 'Still frame (PNG)', category: 'images', engine: false },
  { id: 'web', label: 'Web animation · Code + assets (.zip)', category: 'code', engine: false },
  { id: 'json', label: 'Project file (.pmv)', category: 'project', engine: false }
];

const FORMAT_IDS = new Set<string>(FORMATS.map((f) => f.id));

/** Today's stored format ids, migrated. "WebM · realtime capture" becomes the
 *  frame-exact VP9 export (section 17). */
const LEGACY_FORMATS: Record<string, SheetFormat> = {
  prores: 'prores-4444',
  mp4: 'h264',
  webm: 'webm-vp9',
  rec: 'webm-vp9'
};

export function formatInfo(id: SheetFormat): FormatInfo {
  return FORMATS.find((f) => f.id === id) ?? FORMATS.find((f) => f.id === 'h264')!;
}

/** A stored or legacy format id as a sheet format, or `fallback`. */
export function migrateFormat(value: unknown, fallback: SheetFormat = 'h264'): SheetFormat {
  if (typeof value !== 'string') return fallback;
  if (FORMAT_IDS.has(value)) return value as SheetFormat;
  return LEGACY_FORMATS[value] ?? fallback;
}

/** The current exporter's id for a format it writes, or null. */
export function legacyFormatId(format: SheetFormat): 'prores' | 'mp4' | 'webm' | 'png' | 'still' | 'web' | 'json' | null {
  switch (format) {
    case 'prores-4444': return 'prores';
    case 'h264': return 'mp4';
    case 'webm-vp9': return 'webm';
    case 'png': case 'still': case 'web': case 'json': return format;
    default: return null;
  }
}

export function isRenderFormat(format: SheetFormat): format is DeliveryFormat {
  return format !== 'web' && format !== 'json';
}

export function isProRes(format: SheetFormat): boolean {
  return format.startsWith('prores-');
}

/* ── Settings the sheet keeps per project ─────────────────────────────── */

export type ExportQuality = 'draft' | 'high' | 'max';
export type ExportRange = 'work' | 'all';
export type AudioChoice = 'auto' | 'pcm16' | 'pcm24' | 'float' | 'aac';
export type VideoColor = 'quicktime' | 'broadcast';
export type ImageColor = 'srgb' | 'rec709';
export type OverwritePolicy = 'keep-both' | 'replace' | 'stop';

export interface DeliveryOptions {
  /** HEVC 10-bit, PNG 16-bit. */
  highDepth: boolean;
  /** HEVC container. */
  container: 'mp4' | 'mov';
  /** ProRes 4444 alpha. */
  alphaMode: 'straight' | 'premultiplied';
  audioCoding: AudioChoice;
  videoColor: VideoColor;
  imageColor: ImageColor;
  /** Start timecode `HH:MM:SS:FF` (`;` for drop-frame); empty: the composition's. */
  timecode: string;
  reel: string;
  /** File name template with tokens ({project}, {composition}, …). */
  fileName: string;
  /** Render queue jobs: what happens when the file exists. */
  overwrite: OverwritePolicy;
}

export const DEFAULT_DELIVERY: DeliveryOptions = {
  highDepth: false,
  container: 'mp4',
  alphaMode: 'straight',
  audioCoding: 'auto',
  videoColor: 'quicktime',
  imageColor: 'srgb',
  timecode: '',
  reel: '',
  fileName: '{project}',
  overwrite: 'keep-both'
};

/** Which settings a format reads (the sheet hides the rest). */
export interface FieldSupport {
  scale: boolean;
  fps: boolean;
  range: boolean;
  quality: boolean;
  mblur: boolean;
  audio: boolean;
  alpha: boolean;
  alphaMode: boolean;
  highDepth: boolean;
  container: boolean;
  audioCoding: boolean;
  timecode: boolean;
  videoColor: boolean;
  imageColor: boolean;
}

const NONE: FieldSupport = {
  scale: false, fps: false, range: false, quality: false, mblur: false, audio: false, alpha: false,
  alphaMode: false, highDepth: false, container: false, audioCoding: false, timecode: false,
  videoColor: false, imageColor: false
};

/** The settings `format` reads; `engine` says whether the native engine writes
 *  it (the current exporter reads only today's settings). */
export function fieldSupport(format: SheetFormat, options: Pick<DeliveryOptions, 'container' | 'highDepth'>, engine: boolean): FieldSupport {
  const video: FieldSupport = { ...NONE, scale: true, fps: true, range: true, mblur: true, audio: true };
  const delivery = (s: FieldSupport): FieldSupport => engine ? s : {
    ...s, alphaMode: false, highDepth: false, container: false, audioCoding: false, timecode: false,
    videoColor: false, imageColor: false
  };
  switch (format) {
    case 'prores-proxy': case 'prores-lt': case 'prores-422': case 'prores-hq':
      return delivery({ ...video, audioCoding: true, timecode: true, videoColor: true });
    case 'prores-4444': case 'prores-4444xq':
      return delivery({ ...video, alpha: true, alphaMode: true, audioCoding: true, timecode: true, videoColor: true });
    case 'h264':
      return delivery({ ...video, quality: true, videoColor: true });
    case 'hevc': {
      const mov = options.container === 'mov';
      return delivery({ ...video, quality: true, alpha: !options.highDepth, highDepth: true, container: true, audioCoding: mov, timecode: mov, videoColor: true });
    }
    case 'webm-vp9': case 'webm-vp8':
      return delivery({ ...video, quality: true, alpha: engine, videoColor: true });
    case 'png':
      return delivery({ ...NONE, scale: true, fps: true, range: true, mblur: true, alpha: true, highDepth: true, imageColor: true });
    case 'still':
      return delivery({ ...NONE, scale: true, mblur: true, alpha: true, highDepth: true, imageColor: true });
    default:
      return NONE;
  }
}

/* ── The job engine's settings ────────────────────────────────────────── */

/** The job engine's format for a sheet format. */
export function wireFormat(format: DeliveryFormat, options: Pick<DeliveryOptions, 'highDepth' | 'container'>): WireFormat {
  switch (format) {
    case 'prores-proxy': return { kind: 'proRes', profile: 'proxy' };
    case 'prores-lt': return { kind: 'proRes', profile: 'lt' };
    case 'prores-422': return { kind: 'proRes', profile: 'standard' };
    case 'prores-hq': return { kind: 'proRes', profile: 'hq' };
    case 'prores-4444': return { kind: 'proRes', profile: 'p4444' };
    case 'prores-4444xq': return { kind: 'proRes', profile: 'p4444Xq' };
    case 'h264': return { kind: 'h264' };
    case 'hevc': return { kind: 'hevc', tenBit: options.highDepth, container: options.container };
    case 'webm-vp9': return { kind: 'webm', codec: 'vp9' };
    case 'webm-vp8': return { kind: 'webm', codec: 'vp8' };
    case 'png': return { kind: 'pngSequence', sixteenBit: options.highDepth };
    case 'still': return { kind: 'pngStill', sixteenBit: options.highDepth };
  }
}

/** The file extension of a format (`null`: a PNG sequence is a folder). */
export function extensionOf(format: DeliveryFormat, options: Pick<DeliveryOptions, 'container'>): string | null {
  if (isProRes(format)) return 'mov';
  switch (format) {
    case 'h264': return 'mp4';
    case 'hevc': return options.container;
    case 'webm-vp9': case 'webm-vp8': return 'webm';
    case 'still': return 'png';
    default: return null;
  }
}

/** Whether the wire format carries alpha (section 17: HEVC alpha at 8 bits). */
export function supportsAlpha(format: DeliveryFormat, options: Pick<DeliveryOptions, 'highDepth'>): boolean {
  if (format === 'prores-4444' || format === 'prores-4444xq') return true;
  if (format === 'hevc') return !options.highDepth;
  return format === 'webm-vp9' || format === 'webm-vp8' || format === 'png' || format === 'still';
}

export function wireAlpha(format: DeliveryFormat, alpha: boolean, options: Pick<DeliveryOptions, 'highDepth' | 'alphaMode'>): AlphaOutput {
  if (!alpha || !supportsAlpha(format, options)) return 'none';
  return isProRes(format) && options.alphaMode === 'premultiplied' ? 'premultiplied' : 'straight';
}

export function wireColor(format: DeliveryFormat, options: Pick<DeliveryOptions, 'videoColor' | 'imageColor'>): OutputColor {
  if (format === 'png' || format === 'still') return options.imageColor === 'rec709' ? { kind: 'rec709Image' } : { kind: 'srgbImage' };
  return options.videoColor === 'broadcast' ? { kind: 'rec709Broadcast' } : { kind: 'rec709QuickTime', candidate: 'cicp1_13_1' };
}

/** Whether the output is a QuickTime movie (timecode, PCM audio). */
export function isMov(format: DeliveryFormat, options: Pick<DeliveryOptions, 'container'>): boolean {
  return extensionOf(format, options) === 'mov';
}

export function wireAudioCoding(format: DeliveryFormat, options: Pick<DeliveryOptions, 'audioCoding' | 'container'>): AudioCoding | null {
  if (!isMov(format, options)) return null;
  switch (options.audioCoding) {
    case 'pcm16': return 'pcm16';
    case 'pcm24': return 'pcm24';
    case 'float': return 'pcmFloat';
    case 'aac': return 'aac';
    default: return null;
  }
}

/** Codec depth for dither defaults: 8-bit outputs dither (section 17). */
export function outputDepth(format: DeliveryFormat, options: Pick<DeliveryOptions, 'highDepth'>): number {
  switch (format) {
    case 'prores-4444': case 'prores-4444xq': return 12;
    case 'prores-proxy': case 'prores-lt': case 'prores-422': case 'prores-hq': return 10;
    case 'hevc': return options.highDepth ? 10 : 8;
    case 'png': case 'still': return options.highDepth ? 16 : 8;
    default: return 8;
  }
}

const TIMECODE = /^\d{2}:[0-5]\d:[0-5]\d[:;.,]\d{2,3}$/;

/** A start timecode the engine parses (`HH:MM:SS:FF`, `;` for drop-frame), or
 *  an empty string for the composition's own. */
export function normalizeTimecode(value: unknown): string {
  const text = typeof value === 'string' ? value.trim() : '';
  return TIMECODE.test(text) ? text : '';
}

export function isDropFrameTimecode(text: string): boolean {
  return /[;.,]\d{2,3}$/.test(text);
}

/* ── Presets ──────────────────────────────────────────────────────────── */

export interface ExportPresetOptions {
  format: SheetFormat;
  scale?: number;
  fps?: number;
  range?: ExportRange;
  quality?: ExportQuality;
  mblur?: boolean;
  audio?: boolean;
  alpha?: boolean;
  captionsBurn?: boolean;
  captionsSidecar?: 'none' | 'srt' | 'vtt';
  delivery?: Partial<DeliveryOptions>;
}

export interface ExportPreset {
  id: string;
  name: string;
  /** Built-in presets cannot be overwritten or deleted. */
  builtIn?: boolean;
  options: ExportPresetOptions;
}

/** The five presets of section 17. */
export const BUILT_IN_PRESETS: readonly ExportPreset[] = [
  { id: 'builtin:master', name: 'Master · ProRes 422 HQ', builtIn: true, options: { format: 'prores-hq', alpha: false, quality: 'max', delivery: { audioCoding: 'pcm24' } } },
  { id: 'builtin:master-alpha', name: 'Master with alpha · ProRes 4444', builtIn: true, options: { format: 'prores-4444', alpha: true, delivery: { alphaMode: 'straight', audioCoding: 'pcm24' } } },
  { id: 'builtin:web-h264', name: 'Web · H.264', builtIn: true, options: { format: 'h264', alpha: false, quality: 'high' } },
  { id: 'builtin:web-hevc', name: 'Web · HEVC', builtIn: true, options: { format: 'hevc', alpha: false, quality: 'high', delivery: { container: 'mp4', highDepth: false } } },
  { id: 'builtin:png', name: 'Image sequence · PNG', builtIn: true, options: { format: 'png', alpha: true, delivery: { highDepth: false } } }
];

/** A stored preset (today's `renderPresets` entries hold the sheet's options
 *  under `options`, with today's format ids), migrated; null when unusable. */
export function migratePreset(value: unknown): ExportPreset | null {
  if (!value || typeof value !== 'object') return null;
  const raw = value as Record<string, unknown>;
  const name = typeof raw['name'] === 'string' ? raw['name'].trim() : '';
  const id = typeof raw['id'] === 'string' && raw['id'] ? raw['id'] : null;
  const options = raw['options'] && typeof raw['options'] === 'object' ? raw['options'] as Record<string, unknown> : null;
  if (!name || !id || !options || id.startsWith('builtin:')) return null;
  const out: ExportPresetOptions = { format: migrateFormat(options['format']) };
  for (const key of ['scale', 'fps'] as const) {
    const n = Number(options[key]);
    if (Number.isFinite(n) && n > 0) out[key] = n;
  }
  if (options['range'] === 'work' || options['range'] === 'all') out.range = options['range'];
  if (['draft', 'high', 'max'].includes(options['quality'] as string)) out.quality = options['quality'] as ExportQuality;
  for (const key of ['mblur', 'audio', 'alpha', 'captionsBurn'] as const) {
    if (typeof options[key] === 'boolean') out[key] = options[key] as boolean;
  }
  if (['none', 'srt', 'vtt'].includes(options['captionsSidecar'] as string)) out.captionsSidecar = options['captionsSidecar'] as 'none' | 'srt' | 'vtt';
  if (options['delivery'] && typeof options['delivery'] === 'object') out.delivery = normalizeDelivery(options['delivery']);
  return { id, name: name.slice(0, 120), options: out };
}

/** Built-ins first, then the user's presets (migrated, unique by name). */
export function allPresets(stored: unknown): ExportPreset[] {
  const user = (Array.isArray(stored) ? stored : []).map(migratePreset).filter((p): p is ExportPreset => !!p);
  const seen = new Set(BUILT_IN_PRESETS.map((p) => p.name));
  const unique: ExportPreset[] = [];
  for (const preset of user) {
    if (seen.has(preset.name)) continue;
    seen.add(preset.name);
    unique.push(preset);
  }
  return [...BUILT_IN_PRESETS, ...unique];
}

export function normalizeDelivery(raw: unknown): DeliveryOptions {
  const s = raw && typeof raw === 'object' ? raw as Record<string, unknown> : {};
  const pick = <T extends string>(value: unknown, allowed: readonly T[], fallback: T): T =>
    allowed.includes(value as T) ? value as T : fallback;
  const text = (value: unknown, max: number): string =>
    typeof value === 'string' ? value.replace(/[\u0000-\u001f]/g, '').slice(0, max) : '';
  return {
    highDepth: typeof s['highDepth'] === 'boolean' ? s['highDepth'] : DEFAULT_DELIVERY.highDepth,
    container: pick(s['container'], ['mp4', 'mov'] as const, DEFAULT_DELIVERY.container),
    alphaMode: pick(s['alphaMode'], ['straight', 'premultiplied'] as const, DEFAULT_DELIVERY.alphaMode),
    audioCoding: pick(s['audioCoding'], ['auto', 'pcm16', 'pcm24', 'float', 'aac'] as const, DEFAULT_DELIVERY.audioCoding),
    videoColor: pick(s['videoColor'], ['quicktime', 'broadcast'] as const, DEFAULT_DELIVERY.videoColor),
    imageColor: pick(s['imageColor'], ['srgb', 'rec709'] as const, DEFAULT_DELIVERY.imageColor),
    timecode: normalizeTimecode(s['timecode']),
    reel: text(s['reel'], 64).trim(),
    fileName: text(s['fileName'], 200).trim() || DEFAULT_DELIVERY.fileName,
    overwrite: pick(s['overwrite'], ['keep-both', 'replace', 'stop'] as const, DEFAULT_DELIVERY.overwrite)
  };
}

/* ── File name tokens ─────────────────────────────────────────────────── */

export interface NameContext {
  project: string;
  composition: string;
  preset?: string;
  format: SheetFormat;
  width: number;
  height: number;
  fps: number;
  /** When the name is made (dates and times are local). */
  now: Date;
}

export const FILE_NAME_TOKENS = ['project', 'composition', 'preset', 'format', 'size', 'width', 'height', 'fps', 'date', 'time'] as const;

const SHORT_FORMAT: Record<SheetFormat, string> = {
  'prores-proxy': 'ProRes Proxy', 'prores-lt': 'ProRes LT', 'prores-422': 'ProRes 422',
  'prores-hq': 'ProRes 422 HQ', 'prores-4444': 'ProRes 4444', 'prores-4444xq': 'ProRes 4444 XQ',
  h264: 'H.264', hevc: 'HEVC', 'webm-vp9': 'VP9', 'webm-vp8': 'VP8', png: 'PNG', still: 'PNG',
  web: 'Web', json: 'Project'
};

function pad(n: number, digits = 2): string {
  return String(n).padStart(digits, '0');
}

/** A file name from a template: `{project} {size}` → `Launch 1920x1080`.
 *  Unknown tokens stay as written; the result is a bare, safe file name part. */
export function expandFileName(template: string, ctx: NameContext): string {
  const fps = Number.isInteger(ctx.fps) ? String(ctx.fps) : ctx.fps.toFixed(3).replace(/0+$/, '');
  const values: Record<string, string> = {
    project: ctx.project || 'Untitled',
    composition: ctx.composition || ctx.project || 'Untitled',
    preset: ctx.preset ?? '',
    format: SHORT_FORMAT[ctx.format],
    size: `${ctx.width}x${ctx.height}`,
    width: String(ctx.width),
    height: String(ctx.height),
    fps,
    date: `${ctx.now.getFullYear()}-${pad(ctx.now.getMonth() + 1)}-${pad(ctx.now.getDate())}`,
    time: `${pad(ctx.now.getHours())}-${pad(ctx.now.getMinutes())}-${pad(ctx.now.getSeconds())}`
  };
  const expanded = template.replace(/\{([a-z]+)\}/g, (whole, key: string) => key in values ? values[key]! : whole);
  return safeFileName(expanded) || safeFileName(values.project!) || 'Powermove';
}

/** A bare file name part: no path separators, control characters, or colons;
 *  no leading dots; collapsed spaces; at most 180 bytes. */
export function safeFileName(name: string): string {
  let safe = name.replace(/[\\/\u0000-\u001f:]/g, '_').replace(/\s+/g, ' ').trim().replace(/^\.+/, '');
  const encoder = new TextEncoder();
  while (encoder.encode(safe).length > 180) safe = Array.from(safe).slice(0, -1).join('');
  return safe.trim();
}

/* ── Size estimate ────────────────────────────────────────────────────── */

/** Apple's ProRes target data rates at 1920×1080, 29.97 fps, in Mbps
 *  (Apple ProRes White Paper), scaled by pixels and frame rate. */
const PRORES_MBPS: Record<string, number> = {
  'prores-proxy': 45, 'prores-lt': 102, 'prores-422': 147, 'prores-hq': 220, 'prores-4444': 330, 'prores-4444xq': 500
};

/** Audio data rate in bits per second for a format and coding (stereo). */
export function audioBitsPerSecond(format: DeliveryFormat, options: Pick<DeliveryOptions, 'audioCoding' | 'container'>, sampleRate = 48_000): number {
  if (format === 'png' || format === 'still') return 0;
  if (format === 'webm-vp9' || format === 'webm-vp8') return 192_000;
  if (!isMov(format, options)) return 256_000;
  switch (wireAudioCoding(format, options) ?? 'pcm24') {
    case 'pcm16': return sampleRate * 2 * 16;
    case 'pcmFloat': return sampleRate * 2 * 32;
    case 'aac': return 256_000;
    default: return sampleRate * 2 * 24;
  }
}

/** Estimated bytes of an export. Bit rate codecs are close (their rate is the
 *  target); ProRes follows Apple's data rates; PNG assumes 2:1 compression. */
export function estimateBytes(input: {
  format: DeliveryFormat;
  delivery: Pick<DeliveryOptions, 'audioCoding' | 'container' | 'highDepth'>;
  width: number;
  height: number;
  fps: number;
  seconds: number;
  frames: number;
  quality: ExportQuality;
  alpha: boolean;
  audio: boolean;
}): number {
  const { format, width, height, fps, seconds } = input;
  const audio = input.audio ? audioBitsPerSecond(format, input.delivery) * seconds / 8 : 0;
  if (format === 'png' || format === 'still') {
    const channels = input.alpha ? 4 : 3;
    const bytes = input.delivery.highDepth ? 2 : 1;
    const frames = format === 'still' ? 1 : input.frames;
    return Math.round(width * height * channels * bytes * frames / 2);
  }
  if (isProRes(format)) {
    const mbps = PRORES_MBPS[format]! * (width * height) / (1920 * 1080) * fps / (30000 / 1001);
    return Math.round(mbps * 1e6 * seconds / 8 + audio);
  }
  const mbps = input.quality === 'draft' ? 4 : input.quality === 'high' ? 16 : 40;
  return Math.round(mbps * 1e6 * seconds / 8 + audio);
}

/** `≈ 1.2 GB`, `≈ 640 MB`, `≈ 900 KB`. */
export function formatBytes(bytes: number): string {
  if (bytes >= 1e9) return `≈ ${(bytes / 1e9).toFixed(bytes >= 1e10 ? 0 : 1)} GB`;
  if (bytes >= 1e6) return `≈ ${Math.round(bytes / 1e6)} MB`;
  return `≈ ${Math.max(1, Math.round(bytes / 1e3))} KB`;
}
