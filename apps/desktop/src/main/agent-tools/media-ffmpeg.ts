/*
 * Pure pieces of the agent's media tools: ffmpeg argument builders and the
 * parsers for what ffmpeg prints. Only `ffmpeg` ships with Powermove (no
 * ffprobe), so probing reads the `-i` banner, and analysis reads the
 * silencedetect / ebur128 / astats log lines. Everything here is
 * deterministic and unit tested; media-tools.ts runs the processes.
 */

import { formatClock } from '../../shared/media-tools';

/* ── probe ───────────────────────────────────────────────── */

export interface ProbedVideoStream {
  index: number;
  codec: string;
  profile?: string;
  pixelFormat?: string;
  width?: number;
  height?: number;
  fps?: number;
  bitrateKbps?: number;
  alpha: boolean;
  /** Display rotation in degrees (ffmpeg applies it when decoding). */
  rotation?: number;
  language?: string;
}

export interface ProbedAudioStream {
  index: number;
  codec: string;
  profile?: string;
  sampleRate?: number;
  channels?: number;
  layout?: string;
  sampleFormat?: string;
  bitrateKbps?: number;
  language?: string;
}

export interface MediaProbe {
  format: string | null;
  duration: number | null;
  start: number;
  bitrateKbps: number | null;
  video: ProbedVideoStream[];
  audio: ProbedAudioStream[];
  /** Subtitle, data and attachment streams. */
  otherStreams: number;
}

const FORMATS: Record<string, string> = {
  'mov,mp4,m4a,3gp,3g2,mj2': 'mov/mp4',
  'matroska,webm': 'matroska/webm'
};

const LAYOUT_CHANNELS: Record<string, number> = {
  mono: 1, stereo: 2, '2.1': 3, '3.0': 3, '3.0(back)': 3, '4.0': 4, quad: 4, 'quad(side)': 4, '3.1': 4,
  '5.0': 5, '5.0(side)': 5, '4.1': 5, '5.1': 6, '5.1(side)': 6, '6.0': 6, '6.1': 7, '7.0': 7, '7.1': 8, '7.1(wide)': 8, octagonal: 8
};

/** Split on top-level commas: "yuv420p(tv, bt709), 1920x1080" keeps its parentheses. */
export function splitTopLevel(text: string): string[] {
  const parts: string[] = [];
  let depth = 0, current = '';
  for (const char of text) {
    if (char === '(' || char === '[') depth++;
    else if ((char === ')' || char === ']') && depth > 0) depth--;
    if (char === ',' && depth === 0) { parts.push(current.trim()); current = ''; continue; }
    current += char;
  }
  if (current.trim()) parts.push(current.trim());
  return parts;
}

function codecAndProfile(head: string): { codec: string; profile?: string } {
  const codec = /^([A-Za-z0-9_.-]+)/.exec(head)?.[1] ?? 'unknown';
  const groups = [...head.matchAll(/\(([^()]*)\)/g)].map((match) => match[1]!.trim());
  const profile = groups.find((group) => !/\/\s*0x[0-9a-f]+/i.test(group) && group.length > 0);
  return { codec, ...(profile ? { profile } : {}) };
}

const ALPHA_FORMATS = /^(yuva|rgba|bgra|argb|abgr|gbrap|ya8|ya16|rgba64|bgra64)/;

function number(text: string | undefined): number | undefined {
  if (text === undefined) return undefined;
  const value = Number(text);
  return Number.isFinite(value) ? value : undefined;
}

/** Parse `ffmpeg -hide_banner -i <file>` stderr. */
export function parseProbe(stderr: string): MediaProbe {
  const probe: MediaProbe = { format: null, duration: null, start: 0, bitrateKbps: null, video: [], audio: [], otherStreams: 0 };
  const lines = stderr.split(/\r?\n/);
  let inInput = false;
  let current: ProbedVideoStream | ProbedAudioStream | null = null;
  for (const line of lines) {
    const input = /^Input #0, (.+?), from /.exec(line);
    if (input) { inInput = true; probe.format = FORMATS[input[1]!] ?? input[1]!.split(',')[0]!; continue; }
    if (/^Input #[1-9]|^Output #|^Stream mapping/.test(line)) inInput = false;
    if (!inInput) continue;
    const duration = /^\s+Duration: (?:(\d+):(\d+):(\d+(?:\.\d+)?)|N\/A)(?:, start: (-?[\d.]+))?(?:, bitrate: (\d+) kb\/s)?/.exec(line);
    if (duration) {
      if (duration[1] !== undefined) probe.duration = Number(duration[1]) * 3600 + Number(duration[2]) * 60 + Number(duration[3]);
      probe.start = number(duration[4]) ?? 0;
      probe.bitrateKbps = number(duration[5]) ?? null;
      continue;
    }
    const stream = /^\s+Stream #0:(\d+)(?:\[0x[0-9a-f]+\])?(?:\(([A-Za-z]+)\))?: (Video|Audio|Subtitle|Data|Attachment): (.*)$/.exec(line);
    if (stream) {
      const index = Number(stream[1]);
      const language = stream[2] && stream[2] !== 'und' ? stream[2] : undefined;
      const parts = splitTopLevel(stream[4]!);
      const { codec, profile } = codecAndProfile(parts[0] ?? '');
      if (stream[3] === 'Video') {
        const video: ProbedVideoStream = { index, codec, ...(profile ? { profile } : {}), alpha: false, ...(language ? { language } : {}) };
        for (const part of parts.slice(1)) {
          const size = /^(\d+)x(\d+)/.exec(part);
          if (size) { video.width = Number(size[1]); video.height = Number(size[2]); continue; }
          const fps = /^([\d.]+)(k?) fps$/.exec(part);
          if (fps) { video.fps = Number(fps[1]) * (fps[2] ? 1000 : 1); continue; }
          const tbr = /^([\d.]+)(k?) tbr$/.exec(part);
          if (tbr && video.fps === undefined) { video.fps = Number(tbr[1]) * (tbr[2] ? 1000 : 1); continue; }
          const bitrate = /^(\d+) kb\/s/.exec(part);
          if (bitrate) { video.bitrateKbps = Number(bitrate[1]); continue; }
          const pix = /^([a-z0-9_]+)(\(.*\))?$/.exec(part);
          if (pix && video.pixelFormat === undefined && !/tb[rnc]$/.test(part)) video.pixelFormat = pix[1]!;
        }
        if (video.pixelFormat && ALPHA_FORMATS.test(video.pixelFormat)) video.alpha = true;
        probe.video.push(video); current = video;
      } else if (stream[3] === 'Audio') {
        const audio: ProbedAudioStream = { index, codec, ...(profile ? { profile } : {}), ...(language ? { language } : {}) };
        const rest = parts.slice(1);
        for (const [position, part] of rest.entries()) {
          const rate = /^(\d+) Hz$/.exec(part);
          if (rate) { audio.sampleRate = Number(rate[1]); continue; }
          const bitrate = /^(\d+) kb\/s/.exec(part);
          if (bitrate) { audio.bitrateKbps = Number(bitrate[1]); continue; }
          const counted = /^(\d+) channels?(?: \(.*\))?$/.exec(part);
          if (counted) { audio.channels = Number(counted[1]); audio.layout = part; continue; }
          if (audio.layout === undefined && LAYOUT_CHANNELS[part] !== undefined) { audio.layout = part; audio.channels = LAYOUT_CHANNELS[part]; continue; }
          if (audio.layout !== undefined && audio.sampleFormat === undefined && /^[a-z0-9]+$/.test(part) && position > 0) audio.sampleFormat = part;
        }
        probe.audio.push(audio); current = audio;
      } else { probe.otherStreams++; current = null; }
      continue;
    }
    if (current && 'alpha' in current) {
      if (/^\s+alpha_mode\s*:\s*1\s*$/.test(line)) current.alpha = true;
      const rotation = /displaymatrix: rotation of (-?[\d.]+) degrees/.exec(line) ?? /^\s+rotate\s*:\s*(-?\d+)\s*$/.exec(line);
      if (rotation) current.rotation = Number(rotation[1]);
    }
  }
  return probe;
}

/** The picture as displayed: ffmpeg autorotates 90/270-degree streams. */
export function displaySize(video: ProbedVideoStream | undefined): { width: number; height: number } | null {
  if (!video?.width || !video.height) return null;
  const quarter = Math.round(Math.abs(video.rotation ?? 0) / 90) % 2 === 1;
  return quarter ? { width: video.height, height: video.width } : { width: video.width, height: video.height };
}

/* ── frame sampling ──────────────────────────────────────── */

export const FRAME_QUALITY = { small: 384 * 384, medium: 768 * 768, large: 1536 * 1536 } as const;
export type FrameQuality = keyof typeof FRAME_QUALITY;

/** Even dimensions inside a pixel budget, aspect preserved, never upscaled. */
export function fitBudget(width: number, height: number, budget: number): { width: number; height: number } {
  const scale = Math.min(1, Math.sqrt(budget / Math.max(1, width * height)));
  const even = (value: number) => Math.max(2, Math.round(value * scale / 2) * 2);
  return { width: even(width), height: even(height) };
}

export interface SampleTimesOptions {
  duration: number;
  fps?: number;
  times?: number[];
  count?: number;
  start?: number;
  end?: number;
}

/**
 * Explicit times (negative counts back from the end), or `count` frames spread
 * evenly from the first to the last frame of [start, end]. Times land on a
 * frame that exists: the end of the media is its last frame.
 */
export function resolveSampleTimes({ duration, fps = 30, times, count, start, end }: SampleTimesOptions): number[] {
  const frame = 1 / Math.max(1, fps);
  const last = Math.max(0, duration - frame);
  const clampToFrame = (time: number) => Math.min(last, Math.max(0, time));
  if (times?.length) {
    return times.map((time) => {
      const resolved = time < 0 ? duration + time : time;
      if (!Number.isFinite(resolved) || resolved < -1e-6 || resolved > duration + 1e-6) {
        throw new Error(`Time ${time}s is outside the media (0–${round(duration, 3)}s; negative times count back from the end).`);
      }
      return round(clampToFrame(resolved), 3);
    });
  }
  const from = clampToFrame(start ?? 0);
  const to = clampToFrame(end ?? duration);
  if (to < from) throw new Error(`The window is empty: start ${round(from, 3)}s is after end ${round(to, 3)}s.`);
  const n = Math.max(1, Math.trunc(count ?? 1));
  if (n === 1) return [round(from + (to - from) / 2, 3)];
  return Array.from({ length: n }, (_, index) => round(from + (to - from) * index / (n - 1), 3));
}

export function round(value: number, places = 2): number {
  const factor = 10 ** places;
  return Math.round(value * factor) / factor;
}

/** A drawtext `text='…'` value: labels are clock times, so anything that is
 *  not a plain word character, dot, dash or space is dropped and colons are
 *  escaped for the filter's option parser. */
export function drawtextEscape(value: string): string {
  return value.replace(/[^0-9A-Za-z.:\- ]/g, '').replace(/:/g, '\\:');
}

function fontOption(fontFile: string | null): string {
  return fontFile ? `fontfile='${fontFile.replace(/\\/g, '/').replace(/'/g, '')}':` : '';
}

export interface FrameArgsOptions {
  input: string;
  time: number;
  width: number;
  height: number;
  /** Letterbox into exactly width×height (contact sheet cells). */
  pad?: boolean;
  label?: string;
  fontFile?: string | null;
  format: 'jpeg' | 'png';
  output: string;
}

/** One decoded frame at `time`: input seeking, then exact decode to that frame. */
export function frameArgs(options: FrameArgsOptions): string[] {
  const filters = [options.pad
    ? `scale=${options.width}:${options.height}:force_original_aspect_ratio=decrease,pad=${options.width}:${options.height}:(ow-iw)/2:(oh-ih)/2:color=0x1c1c1e`
    : `scale=${options.width}:${options.height}`];
  if (options.label && options.fontFile !== null) {
    const size = Math.max(11, Math.round(options.height / 13));
    filters.push(`drawtext=${fontOption(options.fontFile ?? null)}text='${drawtextEscape(options.label)}':x=${Math.round(size * 0.5)}:y=h-th-${Math.round(size * 0.5)}:fontsize=${size}:fontcolor=white:box=1:boxcolor=black@0.6:boxborderw=${Math.max(3, Math.round(size / 3))}`);
  }
  return [
    '-hide_banner', '-loglevel', 'error', '-nostdin', '-y',
    '-ss', String(Math.max(0, options.time)), '-i', options.input,
    '-frames:v', '1', '-an', '-sn', '-dn', '-vf', filters.join(','),
    ...(options.format === 'jpeg' ? ['-pix_fmt', 'yuvj420p', '-q:v', '3', '-f', 'image2', '-c:v', 'mjpeg'] : ['-f', 'image2', '-c:v', 'png']),
    options.output
  ];
}

/* ── auto frames: one per visual state ───────────────────── */

/* A port of dapi's frame triage (frame-triage.ts) to ffmpeg: the footage is
   scanned at 2 fps as a 32×32 grid of block lumas (ffmpeg's area scaler is
   the block average); a frame is kept when the picture settles into a state
   that differs from the last kept one, never mid-transition, and footage that
   never settles still yields its calmest frame every few seconds. */
export const TRIAGE_GRID = 32;
const TRIAGE_DIM = TRIAGE_GRID * TRIAGE_GRID;
const DEADZONE = 0.02;
const SATURATION = 0.1;
const THRESHOLD = 2 / TRIAGE_DIM;
const MOTION_EPS = 2 / TRIAGE_DIM;
const MAX_UNSETTLED = 4;

export function triageScanArgs(input: string, start: number, duration: number, scanFps: number, keyframesOnly = false): string[] {
  return [
    '-hide_banner', '-loglevel', 'error', '-nostdin',
    ...(keyframesOnly ? ['-skip_frame', 'nokey'] : []),
    '-ss', String(Math.max(0, start)), '-t', String(Math.max(0.01, duration)), '-i', input,
    '-an', '-sn', '-dn', '-vf', `fps=${scanFps},scale=${TRIAGE_GRID}:${TRIAGE_GRID}:flags=area,format=gray`,
    '-f', 'rawvideo', 'pipe:1'
  ];
}

export function fingerprintDistance(a: Uint8Array, b: Uint8Array): number {
  let sum = 0;
  for (let index = 0; index < TRIAGE_DIM; index++) {
    const delta = Math.abs((a[index] ?? 0) - (b[index] ?? 0)) / 255;
    if (delta > DEADZONE) sum += Math.min((delta - DEADZONE) / (SATURATION - DEADZONE), 1);
  }
  return sum / TRIAGE_DIM;
}

export interface TriagePick {
  time: number;
  novelty: number;
  /** The scan time the footage started changing into this state (a cut lands here). */
  since?: number;
}

/** `raw` is the concatenated 32×32 gray frames; frame i was sampled at start + i / scanFps. */
export function pickInformativeTimes(raw: Uint8Array, start: number, scanFps: number, max: number): { picks: TriagePick[]; states: number } {
  const frames = Math.floor(raw.length / TRIAGE_DIM);
  const kept: TriagePick[] = [];
  let lastKept: Uint8Array | undefined;
  let previous: Uint8Array | undefined;
  let runStart = 0;
  let calmest: { time: number; print: Uint8Array; motion: number } | undefined;
  let changeStart: number | null = null;
  const keep = (time: number, print: Uint8Array, novelty: number, since: number | null) => {
    kept.push({ time: round(time, 3), novelty, ...(since !== null && since < time ? { since: round(since, 3) } : {}) });
    lastKept = print;
  };
  for (let index = 0; index < frames; index++) {
    const print = raw.subarray(index * TRIAGE_DIM, (index + 1) * TRIAGE_DIM);
    const time = start + index / scanFps;
    const motion = previous ? fingerprintDistance(print, previous) : 0;
    previous = print;
    if (motion <= MOTION_EPS) {
      calmest = undefined;
      const novelty = lastKept ? fingerprintDistance(print, lastKept) : Infinity;
      if (novelty >= THRESHOLD) keep(time, print, novelty, changeStart);
      changeStart = null;
    } else {
      if (!calmest) runStart = time;
      changeStart ??= time;
      if (!calmest || motion < calmest.motion) calmest = { time, print, motion };
      if (time - runStart >= MAX_UNSETTLED) {
        const novelty = lastKept ? fingerprintDistance(calmest.print, lastKept) : Infinity;
        if (novelty >= THRESHOLD) keep(calmest.time, calmest.print, novelty, changeStart);
        calmest = undefined;
        changeStart = null;
      }
    }
  }
  const picks = [...kept].sort((a, b) => b.novelty - a.novelty).slice(0, Math.max(1, max)).sort((a, b) => a.time - b.time);
  return { picks, states: kept.length };
}

/* ── contact sheets ──────────────────────────────────────── */

export const SHEET_MAX_WIDTH = 1600;

export interface SheetPlan { columns: number; rows: number; cellWidth: number; cellHeight: number }

export function planSheet(count: number, aspect: number, columns?: number): SheetPlan {
  const n = Math.max(1, count);
  const auto = n <= 3 ? n : n <= 4 ? 2 : n <= 9 ? 3 : n <= 16 ? 4 : n <= 25 ? 5 : n <= 36 ? 6 : 8;
  const cols = Math.max(1, Math.min(n, Math.trunc(columns ?? auto), 10));
  const rows = Math.ceil(n / cols);
  const safeAspect = Number.isFinite(aspect) && aspect > 0 ? Math.min(4, Math.max(0.25, aspect)) : 16 / 9;
  const cellWidth = Math.max(120, Math.min(640, Math.floor((SHEET_MAX_WIDTH - 4 * (cols + 1)) / cols / 2) * 2));
  const cellHeight = Math.max(68, Math.round(cellWidth / safeAspect / 2) * 2);
  return { columns: cols, rows, cellWidth, cellHeight };
}

export function tileArgs(pattern: string, plan: SheetPlan, output: string): string[] {
  return [
    '-hide_banner', '-loglevel', 'error', '-nostdin', '-y',
    '-framerate', '1', '-i', pattern,
    '-vf', `tile=${plan.columns}x${plan.rows}:padding=4:margin=4:color=0x111113`,
    '-frames:v', '1', '-pix_fmt', 'yuvj420p', '-q:v', '3', '-f', 'image2', '-c:v', 'mjpeg', output
  ];
}

/* ── audio analysis ──────────────────────────────────────── */

export interface SilenceOptions { thresholdDb: number; minDuration: number }

export function analysisArgs(input: string, start: number, duration: number, silence: SilenceOptions): string[] {
  return [
    '-hide_banner', '-nostdin', '-nostats',
    '-ss', String(Math.max(0, start)), '-t', String(Math.max(0.01, duration)), '-i', input,
    '-map', '0:a:0', '-vn', '-sn', '-dn',
    '-af', `silencedetect=noise=${silence.thresholdDb}dB:d=${silence.minDuration},ebur128=peak=true+sample:framelog=verbose,astats=measure_perchannel=none:measure_overall=Peak_level+RMS_level+Flat_factor+Peak_count`,
    '-f', 'null', '-'
  ];
}

/** Silent ranges in media time (`offset` = the analysed window's start). Open silence runs to `end`. */
export function parseSilences(stderr: string, offset: number, end: number): Array<[number, number]> {
  const ranges: Array<[number, number]> = [];
  let open: number | null = null;
  for (const match of stderr.matchAll(/silence_(start|end): (-?[\d.]+)/g)) {
    const at = Number(match[2]) + offset;
    if (match[1] === 'start') open = Math.max(offset, at);
    else if (open !== null) { ranges.push([round(open, 3), round(Math.min(end, at), 3)]); open = null; }
  }
  if (open !== null && open < end) ranges.push([round(open, 3), round(end, 3)]);
  return ranges.filter(([start, stop]) => stop > start);
}

export interface Loudness {
  integratedLufs: number | null;
  loudnessRangeLu: number | null;
  truePeakDbfs: number | null;
  samplePeakDbfs: number | null;
  rmsDbfs: number | null;
  /** Peaks reach full scale and stay there (flat tops), or decoded samples exceed it. */
  clipping: boolean;
}

function dbValue(text: string | undefined): number | null {
  if (text === undefined) return null;
  if (/^-?inf$/i.test(text)) return -Infinity;
  const value = Number(text);
  return Number.isFinite(value) ? round(value, 1) : null;
}

export function parseLoudness(stderr: string): Loudness {
  const summary = stderr.slice(stderr.lastIndexOf('Summary:'));
  const hasSummary = stderr.includes('Summary:');
  const integrated = hasSummary ? /\bI:\s+(-?[\d.]+|-?inf) LUFS/.exec(summary)?.[1] : undefined;
  const range = hasSummary ? /\bLRA:\s+(-?[\d.]+) LU\b/.exec(summary)?.[1] : undefined;
  const truePeak = hasSummary ? /True peak:\s*\n\s*Peak:\s+(-?[\d.]+|-?inf) dBFS/.exec(summary)?.[1] : undefined;
  const peakLevel = [...stderr.matchAll(/Peak level dB: (-?[\d.]+|-?inf)/g)].at(-1)?.[1];
  const rms = [...stderr.matchAll(/RMS level dB: (-?[\d.]+|-?inf)/g)].at(-1)?.[1];
  const flat = Number([...stderr.matchAll(/Flat factor: ([\d.]+)/g)].at(-1)?.[1] ?? 0);
  const peakDb = peakLevel === undefined ? null : Number(peakLevel);
  const clipping = peakDb !== null && Number.isFinite(peakDb) && (peakDb > 0.1 || (peakDb >= -0.05 && flat > 1));
  const finiteOrNull = (value: number | null) => value === null || Number.isFinite(value) ? value : null;
  return {
    integratedLufs: finiteOrNull(dbValue(integrated)),
    loudnessRangeLu: dbValue(range),
    truePeakDbfs: finiteOrNull(dbValue(truePeak)),
    samplePeakDbfs: finiteOrNull(dbValue(peakLevel)),
    rmsDbfs: finiteOrNull(dbValue(rms)),
    clipping
  };
}

/* ── waveform image ──────────────────────────────────────── */

const TICK_STEPS = [0.25, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];

export function tickStep(duration: number, maxTicks = 10): number {
  return TICK_STEPS.find((step) => duration / step <= maxTicks) ?? 7200;
}

export interface WaveformArgsOptions {
  input: string;
  start: number;
  duration: number;
  width: number;
  height: number;
  /** Silent ranges in media time. */
  silences: Array<[number, number]>;
  fontFile?: string | null;
  output: string;
}

export const WAVEFORM_RULER = 22;

export function waveformArgs(options: WaveformArgsOptions): string[] {
  const { width, height, start, duration } = options;
  const x = (time: number) => Math.round((time - start) / duration * width);
  const shading = options.silences.slice(0, 200).map(([from, to]) => {
    const left = Math.max(0, x(from)), right = Math.min(width, x(to));
    return right > left ? `drawbox=x=${left}:y=0:w=${Math.max(1, right - left)}:h=${height}:color=0xff453a@0.30:t=fill` : '';
  }).filter(Boolean);
  const step = tickStep(duration);
  const ticks: string[] = [];
  const first = Math.ceil(start / step - 1e-9) * step;
  for (let time = first; time <= start + duration + 1e-9; time += step) {
    const left = x(time);
    if (left < 0 || left > width) continue;
    ticks.push(`drawbox=x=${Math.min(width - 1, left)}:y=${height}:w=1:h=6:color=0x8e8e93@1:t=fill`);
    if (options.fontFile !== null && left < width - 40) {
      ticks.push(`drawtext=${fontOption(options.fontFile ?? null)}text='${drawtextEscape(formatClock(time))}':x=${left + 3}:y=${height + 6}:fontsize=12:fontcolor=0xaeaeb2`);
    }
  }
  const graph = [
    `[0:a]aformat=channel_layouts=mono,showwavespic=s=${width}x${height}:colors=0xc7c7cc:scale=sqrt:draw=full[wave]`,
    `color=c=0x18181b:s=${width}x${height}[bg]`,
    `[bg][wave]overlay=shortest=1:format=auto${shading.length ? `,${shading.join(',')}` : ''},pad=${width}:${height + WAVEFORM_RULER}:0:0:color=0x111113${ticks.length ? `,${ticks.join(',')}` : ''}[out]`
  ].join(';');
  return [
    '-hide_banner', '-loglevel', 'error', '-nostdin', '-y',
    '-ss', String(Math.max(0, start)), '-t', String(Math.max(0.01, duration)), '-i', options.input,
    '-filter_complex', graph, '-map', '[out]', '-frames:v', '1', '-f', 'image2', '-c:v', 'png', options.output
  ];
}

/* ── composition ⇄ source time ───────────────────────────── */

/** Composition time → source time along piecewise-linear samples. */
export function compositionToSource(samples: Array<[number, number]>, time: number): number | null {
  if (!samples.length) return null;
  if (time <= samples[0]![0]) return samples[0]![1];
  for (let index = 1; index < samples.length; index++) {
    const [c0, s0] = samples[index - 1]!, [c1, s1] = samples[index]!;
    if (time <= c1) return c1 === c0 ? s1 : s0 + (time - c0) * (s1 - s0) / (c1 - c0);
  }
  return samples.at(-1)![1];
}

/** Source time → the first composition time the clip shows it, or null when it never does. */
export function sourceToComposition(samples: Array<[number, number]>, time: number, tolerance = 1e-3): number | null {
  for (let index = 1; index < samples.length; index++) {
    const [c0, s0] = samples[index - 1]!, [c1, s1] = samples[index]!;
    const low = Math.min(s0, s1) - tolerance, high = Math.max(s0, s1) + tolerance;
    if (time < low || time > high) continue;
    if (Math.abs(s1 - s0) < 1e-9) return c0;
    const fraction = Math.min(1, Math.max(0, (time - s0) / (s1 - s0)));
    return c0 + fraction * (c1 - c0);
  }
  return null;
}
