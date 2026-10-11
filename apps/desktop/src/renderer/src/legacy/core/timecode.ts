/*
 * Timecode helpers behind PM.tc, PM.parseTc and PM.tcDrop.
 *
 * Non-drop timecode labels frames with an integer base: the rate itself for
 * integer rates and the nearest integer for fractional ones (24 for 23.976,
 * 30 for 29.97, 60 for 59.94). The frame count is the number of real frames
 * elapsed at the actual rate, so at 29.97 the label runs slower than the wall
 * clock, as SMPTE non-drop timecode does. Integer rates keep the exact
 * arithmetic and output format the editor has always shown.
 *
 * Drop-frame timecode (29.97 and 59.94 only) skips labels 00 and 01 (00 to 03
 * at 59.94) at the start of every minute except every tenth, so the label
 * tracks the wall clock. It always shows hours, separates frames with ";",
 * and wraps at 24 hours.
 */

const DEFAULT_FPS = 30;

/** A usable rate; anything that is not a positive finite number reads as 30. */
function usableRate(fps: unknown): number {
  const rate = Number(fps);
  return Number.isFinite(rate) && rate > 0 ? rate : DEFAULT_FPS;
}

/** Frames per labelled second: the rate for integer rates, else the nearest integer. */
export function timecodeBase(fps: number): number {
  const rate = usableRate(fps);
  return Number.isInteger(rate) ? rate : Math.max(1, Math.round(rate));
}

/**
 * `[-][HH:]MM:SS[:FF]` non-drop timecode for a time in seconds. Hours appear
 * only when non-zero. The frame is the nearest frame start to `seconds`.
 */
export function formatTimecode(seconds: number, fps: number = DEFAULT_FPS, showFrames = true): string {
  const rate = usableRate(fps);
  const base = timecodeBase(rate);
  const negative = seconds < 0;
  let frames = Math.round(Math.abs(seconds) * rate);
  const ff = frames % base;
  frames = Math.floor(frames / base);
  const ss = frames % 60;
  const mm = Math.floor(frames / 60) % 60;
  const hh = Math.floor(frames / 3600);
  const pad = (value: number) => String(value).padStart(2, '0');
  const clock = (hh ? pad(hh) + ':' : '') + pad(mm) + ':' + pad(ss);
  return (negative ? '-' : '') + clock + (showFrames ? ':' + pad(ff) : '');
}

/**
 * Seconds for `HH:MM:SS:FF`, `MM:SS:FF`, `SS:FF`, or plain seconds, read as
 * non-drop timecode at `fps`. Returns null when a part is not a number.
 */
export function parseTimecode(value: unknown, fps: number = DEFAULT_FPS): number | null {
  const parts = String(value).trim().split(':').map(Number);
  if (parts.some(Number.isNaN)) return null;
  const rate = usableRate(fps);
  let fields: [number, number, number, number];
  if (parts.length === 4) fields = [parts[0]!, parts[1]!, parts[2]!, parts[3]!];
  else if (parts.length === 3) fields = [0, parts[0]!, parts[1]!, parts[2]!];
  else if (parts.length === 2) fields = [0, 0, parts[0]!, parts[1]!];
  else return parts[0]!;
  const [hh, mm, ss, ff] = fields;
  if (Number.isInteger(rate)) {
    // Integer rates keep the historical arithmetic bit for bit.
    if (parts.length === 4) return hh * 3600 + mm * 60 + ss + ff / rate;
    if (parts.length === 3) return mm * 60 + ss + ff / rate;
    return ss + ff / rate;
  }
  return ((hh * 3600 + mm * 60 + ss) * timecodeBase(rate) + ff) / rate;
}

export interface DropFrameRate {
  /** Frames per labelled second: 30 or 60. */
  base: number;
  /** Labels skipped at the start of each minute not divisible by ten: 2 or 4. */
  drop: number;
}

const NTSC_TOLERANCE = 0.0002;

/** The drop-frame scheme for 29.97 or 59.94 (within 0.02% of k × 1000/1001), else null. */
export function dropFrameRate(fps: number): DropFrameRate | null {
  const rate = Number(fps);
  if (!Number.isFinite(rate) || rate <= 0) return null;
  for (const [base, drop] of [[30, 2], [60, 4]] as const) {
    const exact = base * 1000 / 1001;
    if (Math.abs(rate - exact) <= exact * NTSC_TOLERANCE) return { base, drop };
  }
  return null;
}

function requireDropFrameRate(fps: number): DropFrameRate {
  const scheme = dropFrameRate(fps);
  if (!scheme) throw new RangeError(`Drop-frame timecode needs 29.97 or 59.94 fps, not ${fps}.`);
  return scheme;
}

function schemeSizes({ base, drop }: DropFrameRate) {
  const perMinute = base * 60 - drop;
  const perTenMinutes = base * 600 - drop * 9;
  return { perMinute, perTenMinutes, perDay: perTenMinutes * 6 * 24 };
}

/**
 * `HH:MM:SS;FF` drop-frame timecode for a frame number at 29.97 or 59.94.
 * Frames wrap at 24 hours in both directions (frame −1 is 23:59:59;29).
 * Throws a RangeError for other rates or a non-integer frame.
 */
export function formatDropFrameTimecode(frame: number, fps: number): string {
  if (!Number.isSafeInteger(frame)) throw new RangeError(`Frame must be an integer, not ${frame}.`);
  const scheme = requireDropFrameRate(fps);
  const { base, drop } = scheme;
  const { perMinute, perTenMinutes, perDay } = schemeSizes(scheme);
  const f = ((frame % perDay) + perDay) % perDay;
  const tens = Math.floor(f / perTenMinutes);
  const rest = f % perTenMinutes;
  const skipped = drop * 9 * tens + (rest > drop ? drop * Math.floor((rest - drop) / perMinute) : 0);
  const label = f + skipped;
  const ff = label % base;
  const totalSeconds = Math.floor(label / base);
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${pad(Math.floor(totalSeconds / 3600))}:${pad(Math.floor(totalSeconds / 60) % 60)}:${pad(totalSeconds % 60)};${pad(ff)}`;
}

/** Drop-frame timecode for a time in seconds; the frame is the nearest frame start. */
export function dropFrameTimecode(seconds: number, fps: number): string {
  requireDropFrameRate(fps);
  return formatDropFrameTimecode(Math.round(Number(seconds) * Number(fps)), fps);
}

const DROP_FRAME_PATTERN = /^(\d{2}):(\d{2}):(\d{2})[;.,](\d{2})$/;

/**
 * The frame number for an `HH:MM:SS;FF` drop-frame string (`;`, `.` or `,`
 * before the frames). Returns null for malformed strings, out-of-range fields,
 * labels the scheme skips (for example 00:01:00;00 at 29.97), and rates
 * without drop-frame timecode.
 */
export function parseDropFrameTimecode(value: unknown, fps: number): number | null {
  const scheme = dropFrameRate(fps);
  if (!scheme) return null;
  const match = DROP_FRAME_PATTERN.exec(String(value).trim());
  if (!match) return null;
  const [hh, mm, ss, ff] = match.slice(1).map(Number) as [number, number, number, number];
  const { base, drop } = scheme;
  if (hh > 23 || mm > 59 || ss > 59 || ff >= base) return null;
  if (ss === 0 && mm % 10 !== 0 && ff < drop) return null;
  const minutes = hh * 60 + mm;
  return (hh * 3600 + mm * 60 + ss) * base + ff - drop * (minutes - Math.floor(minutes / 10));
}
