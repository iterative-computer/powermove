import { execFileSync } from 'node:child_process';
import { readdirSync } from 'node:fs';
import path from 'node:path';

import { ffmpegBinary } from './ffmpeg';

const ffprobeBinary = ffmpegBinary.replace(/ffmpeg$/, 'ffprobe');
const MAX_BUFFER = 256 * 1024 * 1024;

type Stream = Record<string, unknown>;
type Probe = { streams: Stream[]; format: Record<string, unknown> };

export type Check = { name: string; expected: unknown; actual: unknown; equal: boolean };
export type ParityReport = { checks: Check[]; metrics: Record<string, unknown> };

/** Stream properties an export keeps from today's (G-X1). Codec ids,
 *  container framing and colour tags may change with the engine's encoders
 *  (WebM now carries the export's colour description). */
const VIDEO_FIELDS = ['codec_name', 'pix_fmt', 'width', 'height', 'r_frame_rate', 'nb_read_frames'] as const;
const AUDIO_FIELDS = ['sample_rate', 'channels', 'channel_layout'] as const;

function json<T>(binary: string, args: string[]): T {
  return JSON.parse(execFileSync(binary, args, { encoding: 'utf8', maxBuffer: MAX_BUFFER })) as T;
}

function probe(file: string): Probe {
  return json<Probe>(ffprobeBinary, ['-v', 'error', '-count_frames', '-show_streams', '-show_format', '-of', 'json', file]);
}

function add(checks: Check[], name: string, expected: unknown, actual: unknown): void {
  checks.push({ name, expected, actual, equal: Object.is(expected, actual) });
}

function one(probeResult: Probe, kind: 'video' | 'audio'): Stream | null {
  return probeResult.streams.find(stream => stream.codec_type === kind) ?? null;
}

/** Decoded frames as 16-bit RGBA (alpha as coded; opaque formats at full).
 *  zimg converts with the exact BT.709 limited-range matrix (bilinear chroma
 *  at the tagged siting); swscale's tables are off by up to a code, more than
 *  the difference this gate measures. */
function frames16(input: string): Buffer {
  return execFileSync(ffmpegBinary, ['-v', 'error', '-i', input, '-map', '0:v:0', '-fps_mode', 'passthrough',
    '-vf', 'zscale=matrixin=709:rangein=limited:range=full:filter=bilinear,format=gbrap16le',
    '-f', 'rawvideo', '-pix_fmt', 'rgba64le', 'pipe:1'], { maxBuffer: MAX_BUFFER });
}

/** Decoded audio length in sample frames (after priming, pre-skip and edits). */
function decodedAudioFrames(input: string): number {
  return execFileSync(ffmpegBinary, ['-v', 'error', '-i', input, '-map', '0:a:0', '-ac', '1', '-f', 'f32le', '-acodec', 'pcm_f32le',
    'pipe:1'], { maxBuffer: MAX_BUFFER }).length / 4;
}

/** A PNG sequence folder's frames, in name order, as one input. */
function sequenceFrames16(folder: string): Buffer {
  const first = readdirSync(folder).filter(n => n.endsWith('.png')).sort()[0];
  if (!first) throw new Error(`no PNG frames in ${folder}`);
  const digits = /_(\d+)\.png$/.exec(first)?.[1]?.length ?? 5;
  const pattern = path.join(folder, first.replace(/_\d+\.png$/, `_%0${digits}d.png`));
  return execFileSync(ffmpegBinary, ['-v', 'error', '-start_number', String(Number(/_(\d+)\.png$/.exec(first)![1])),
    '-i', pattern, '-f', 'rawvideo', '-pix_fmt', 'rgba64le', 'pipe:1'], { maxBuffer: MAX_BUFFER });
}

/** PSNR in dB of the color channels of `actual` against `reference`
 *  (16-bit RGBA, color over black where `opaque`). */
function psnr(reference: Buffer, actual: Buffer, opaque: boolean): number {
  const n = Math.min(reference.length, actual.length) / 2;
  let sum = 0, count = 0;
  for (let i = 0; i < n; i += 4) {
    const ra = opaque ? reference.readUInt16LE((i + 3) * 2) / 65535 : 1;
    for (let c = 0; c < 3; c++) {
      const r = reference.readUInt16LE((i + c) * 2) / 65535 * ra;
      const a = actual.readUInt16LE((i + c) * 2) / 65535;
      sum += (r - a) ** 2;
      count++;
    }
  }
  const mse = sum / Math.max(count, 1);
  return mse === 0 ? Infinity : 10 * Math.log10(1 / mse);
}

/** G-X1 (docs/engine/gate-policy.md): the engine's export keeps today's
 *  stream properties, and its decoded pictures are at least as close to the
 *  reference (the engine's lossless 16-bit PNG render of the same frames)
 *  as today's. Every field is a separate check. */
export function compareExports(today: string, engine: string, reference: string | null, expectedFrames: number,
  opaqueReference: boolean): ParityReport {
  const old = probe(today), current = probe(engine);
  const checks: Check[] = [];
  const metrics: Record<string, unknown> = {};
  for (const kind of ['video', 'audio'] as const) {
    const a = one(old, kind), b = one(current, kind);
    add(checks, `${kind}.present`, a !== null, b !== null);
    for (const field of kind === 'video' ? VIDEO_FIELDS : AUDIO_FIELDS) add(checks, `${kind}.${field}`, a?.[field] ?? null, b?.[field] ?? null);
  }
  add(checks, 'video.engine_decoded_frames', expectedFrames, Number(one(current, 'video')?.nb_read_frames));
  // Audio: the same family (PCM, AAC, Opus) and the same length.
  const family = (s: Stream | null) => String(s?.codec_name ?? '').replace(/^pcm_.*/, 'pcm');
  add(checks, 'audio.codec_family', family(one(old, 'audio')), family(one(current, 'audio')));
  const lengths = ([[today, old], [engine, current]] as const).map(([file, p]) => (one(p, 'audio') ? decodedAudioFrames(file) : 0));
  metrics.audioSamples = lengths;
  add(checks, 'audio.length_within_one_opus_frame', true, Math.abs(lengths[0]! - lengths[1]!) <= 960);
  const video = one(current, 'video');
  const colour = [video?.color_primaries, video?.color_transfer, video?.color_space, video?.color_range].map(String).join('/');
  metrics.colour = colour;
  add(checks, 'video.colour_tags', 'bt709/iec61966-2-1/bt709/tv', colour);
  if (reference) {
    const want = sequenceFrames16(reference);
    const t = psnr(want, frames16(today), opaqueReference), e = psnr(want, frames16(engine), opaqueReference);
    metrics.psnrDb = { today: t, engine: e };
    add(checks, 'video.psnr_at_least_todays', true, e >= t);
  }
  return { checks, metrics };
}
