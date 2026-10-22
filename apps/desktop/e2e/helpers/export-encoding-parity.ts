import { execFileSync } from 'node:child_process';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

import type { Page } from 'playwright';

import { ffmpegBinary } from './ffmpeg';

const ffprobeBinary = ffmpegBinary.replace(/ffmpeg$/, 'ffprobe');
const MAX_BUFFER = 96 * 1024 * 1024;

type Stream = Record<string, unknown>;
type Probe = { streams: Stream[]; format: Record<string, unknown> };
type Packet = { size?: string; data_hash?: string; pts?: number; dts?: number; duration?: number };

export type Check = { name: string; expected: unknown; actual: unknown; equal: boolean };
export type Difference = { compared: number; unequal: number; first: number | null; peak: number; rms: number };
export type ParityReport = { checks: Check[]; metrics: Record<string, unknown> };

const VIDEO_FIELDS = [
  'codec_name', 'profile', 'codec_tag_string', 'pix_fmt', 'bits_per_raw_sample',
  'color_primaries', 'color_transfer', 'color_space', 'color_range', 'width', 'height',
  'r_frame_rate', 'avg_frame_rate', 'time_base', 'start_time', 'nb_frames',
  'nb_read_frames', 'duration', 'duration_ts',
] as const;
const AUDIO_FIELDS = [
  'codec_name', 'profile', 'sample_fmt', 'sample_rate', 'channels', 'channel_layout',
  'ch_layout', 'bits_per_sample', 'bits_per_raw_sample', 'time_base', 'start_time',
  'duration', 'duration_ts', 'nb_frames', 'nb_read_frames',
] as const;

function json<T>(binary: string, args: string[]): T {
  return JSON.parse(execFileSync(binary, args, { encoding: 'utf8', maxBuffer: MAX_BUFFER })) as T;
}

function probe(file: string): Probe {
  return json<Probe>(ffprobeBinary, [
    '-v', 'error', '-count_frames', '-show_streams', '-show_format', '-of', 'json', file,
  ]);
}

function packets(file: string): Packet[] {
  return json<{ packets: Packet[] }>(ffprobeBinary, [
    '-v', 'error', '-select_streams', 'a:0', '-show_packets', '-show_data_hash', 'sha256',
    '-show_entries', 'packet=size,pts,dts,duration,data_hash', '-of', 'json', file,
  ]).packets ?? [];
}

function decode(file: string, kind: 'video' | 'audio' | 'wav-s16'): Buffer {
  const args = kind === 'video'
    ? ['-map', '0:v:0', '-fps_mode', 'passthrough', '-f', 'rawvideo', '-pix_fmt', 'rgba']
    : kind === 'audio'
      ? ['-map', '0:a:0', '-f', 'f32le', '-acodec', 'pcm_f32le']
      : ['-map', '0:a:0', '-f', 's16le', '-acodec', 'pcm_s16le'];
  return execFileSync(ffmpegBinary, ['-v', 'error', '-i', file, ...args, 'pipe:1'], { maxBuffer: MAX_BUFFER });
}

function encodedAudio(file: string): Buffer {
  // The data muxer concatenates packet payloads without container framing.
  // Comparing this stream does not mistake different packet grouping for
  // different encoded bytes (PCM in MOV may have different chunk boundaries).
  return execFileSync(ffmpegBinary, [
    '-v', 'error', '-i', file, '-map', '0:a:0', '-c', 'copy', '-f', 'data', 'pipe:1',
  ], { maxBuffer: MAX_BUFFER });
}

function add(checks: Check[], name: string, expected: unknown, actual: unknown): void {
  checks.push({ name, expected, actual, equal: Object.is(expected, actual) });
}

function one(probeResult: Probe, kind: 'video' | 'audio'): Stream | null {
  return probeResult.streams.find(stream => stream.codec_type === kind) ?? null;
}

function floatDifference(expected: Buffer, actual: Buffer): Difference {
  const n = Math.min(Math.floor(expected.length / 4), Math.floor(actual.length / 4));
  let unequal = 0, first: number | null = null, peak = 0, sumSquares = 0;
  for (let i = 0; i < n; i++) {
    if (expected.readUInt32LE(i * 4) === actual.readUInt32LE(i * 4)) continue;
    unequal++;
    first ??= i;
    const delta = expected.readFloatLE(i * 4) - actual.readFloatLE(i * 4);
    peak = Math.max(peak, Math.abs(delta));
    sumSquares += delta * delta;
  }
  return { compared: n, unequal, first, peak, rms: Math.sqrt(sumSquares / Math.max(n, 1)) };
}

function byteDifference(expected: Buffer, actual: Buffer): Difference {
  const n = Math.min(expected.length, actual.length);
  let unequal = 0, first: number | null = null, peak = 0, sumSquares = 0;
  for (let i = 0; i < n; i++) {
    const delta = expected[i]! - actual[i]!;
    if (delta === 0) continue;
    unequal++;
    first ??= i;
    peak = Math.max(peak, Math.abs(delta));
    sumSquares += delta * delta;
  }
  return { compared: n, unequal, first, peak, rms: Math.sqrt(sumSquares / Math.max(n, 1)) };
}

/** Every field is a separate check, so a codec failure does not conceal timing or pixel results. */
export function compareExports(reference: string, candidate: string, expectedFrames: number): ParityReport {
  const old = probe(reference), current = probe(candidate);
  const checks: Check[] = [];
  const metrics: Record<string, unknown> = {};
  for (const kind of ['video', 'audio'] as const) {
    const a = old.streams.filter(stream => stream.codec_type === kind);
    const b = current.streams.filter(stream => stream.codec_type === kind);
    add(checks, `${kind}.legacy_count`, 1, a.length);
    add(checks, `${kind}.candidate_count`, 1, b.length);
    for (const field of kind === 'video' ? VIDEO_FIELDS : AUDIO_FIELDS) {
      add(checks, `${kind}.${field}`, a[0]?.[field] ?? null, b[0]?.[field] ?? null);
    }
  }
  const videoA = one(old, 'video'), videoB = one(current, 'video');
  add(checks, 'video.legacy_decoded_frames', expectedFrames, Number(videoA?.nb_read_frames));
  add(checks, 'video.candidate_decoded_frames', expectedFrames, Number(videoB?.nb_read_frames));
  for (const field of ['start_time', 'duration'] as const) {
    add(checks, `container.${field}`, old.format[field] ?? null, current.format[field] ?? null);
  }
  if (videoA && videoB) {
    const a = decode(reference, 'video'), b = decode(candidate, 'video');
    const diff = byteDifference(a, b);
    metrics.decodedVideo = { legacyBytes: a.length, candidateBytes: b.length, ...diff };
    add(checks, 'video.decoded_rgba_bytes', a.length, b.length);
    add(checks, 'video.decoded_rgba_identical', true, a.equals(b));
    const frameBytes = Number(videoA.width) * Number(videoA.height) * 4;
    add(checks, 'video.legacy_rgba_frame_bytes', expectedFrames * frameBytes, a.length);
    add(checks, 'video.candidate_rgba_frame_bytes', expectedFrames * frameBytes, b.length);
  }
  if (one(old, 'audio') && one(current, 'audio')) {
    const a = decode(reference, 'audio'), b = decode(candidate, 'audio');
    metrics.decodedAudio = { legacySamples: a.length / 4, candidateSamples: b.length / 4, ...floatDifference(a, b) };
    add(checks, 'audio.decoded_f32_bytes', a.length, b.length);
    add(checks, 'audio.decoded_f32_identical', true, a.equals(b));
    const oldPackets = packets(reference), newPackets = packets(candidate);
    const signature = (p: Packet) => `${p.size ?? ''}:${p.data_hash ?? ''}`;
    const timing = (p: Packet) => `${p.pts ?? ''}:${p.dts ?? ''}:${p.duration ?? ''}`;
    const encodedA = encodedAudio(reference), encodedB = encodedAudio(candidate);
    metrics.encodedAudio = { legacyPackets: oldPackets.length, candidatePackets: newPackets.length,
      legacyBytes: encodedA.length, candidateBytes: encodedB.length,
      differingPayloadPackets: Math.max(oldPackets.length, newPackets.length) - oldPackets.filter((p, i) => signature(p) === signature(newPackets[i] ?? {})).length,
      payloadByteDifference: byteDifference(encodedA, encodedB) };
    add(checks, 'audio.encoded_bytes', encodedA.length, encodedB.length);
    add(checks, 'audio.encoded_payload_identical', true, encodedA.equals(encodedB));
    add(checks, 'audio.packet_count', oldPackets.length, newPackets.length);
    add(checks, 'audio.encoded_packet_timing_identical', true,
      oldPackets.length === newPackets.length && oldPackets.every((p, i) => timing(p) === timing(newPackets[i]!)));
  }
  return { checks, metrics };
}

export async function compareCapture(reference: string, candidate: string, kind: 'rgba' | 'wav' | 'f32', expectedBytes?: number): Promise<ParityReport> {
  const checks: Check[] = [];
  const metrics: Record<string, unknown> = {};
  const [a, b] = await Promise.all([readFile(reference).catch(() => null), readFile(candidate).catch(() => null)]);
  add(checks, `${kind}.legacy_capture_present`, true, a !== null);
  add(checks, `${kind}.native_capture_present`, true, b !== null);
  if (!a || !b) return { checks, metrics };
  const old = kind === 'wav' ? decode(reference, 'audio') : a;
  const current = kind === 'wav' ? decode(candidate, 'audio') : b;
  const diff = kind === 'rgba' ? byteDifference(old, current) : floatDifference(old, current);
  metrics[kind] = { legacyBytes: old.length, nativeBytes: current.length, ...diff };
  add(checks, `${kind}.capture_bytes`, old.length, current.length);
  if (expectedBytes !== undefined) {
    add(checks, `${kind}.legacy_expected_bytes`, expectedBytes, old.length);
    add(checks, `${kind}.native_expected_bytes`, expectedBytes, current.length);
  }
  add(checks, `${kind}.capture_identical`, true, old.equals(current));
  if (kind === 'wav') {
    const pcmA = decode(reference, 'wav-s16'), pcmB = decode(candidate, 'wav-s16');
    metrics.wavS16 = { legacyBytes: pcmA.length, nativeBytes: pcmB.length, ...byteDifference(pcmA, pcmB) };
    add(checks, 'wav.s16_bytes', pcmA.length, pcmB.length);
    add(checks, 'wav.s16_identical', true, pcmA.equals(pcmB));
  }
  return { checks, metrics };
}

/** Capture Chromium WebCodecs inputs without changing the frames handed to its encoders. */
export async function installLegacyWebmCapture(page: Page): Promise<void> {
  await page.evaluate(() => {
    const w = window as any;
    if (!w.VideoEncoder || !w.AudioEncoder) throw new Error('WebCodecs encoders are unavailable');
    const state = { frames: [] as Array<Uint8Array | null>, audio: [] as Uint8Array[], pending: [] as Promise<void>[], errors: [] as string[],
      videoEncode: w.VideoEncoder.prototype.encode, audioEncode: w.AudioEncoder.prototype.encode };
    w.__exportParityCapture = state;
    w.VideoEncoder.prototype.encode = function (frame: any, options: any) {
      const clone = frame.clone();
      const index = state.frames.push(null) - 1;
      const bytes = new Uint8Array(clone.displayWidth * clone.displayHeight * 4);
      state.pending.push(Promise.resolve(clone.copyTo(bytes, { format: 'RGBA', layout: [{ offset: 0, stride: clone.displayWidth * 4 }] }))
        .then(() => { state.frames[index] = bytes; })
        .catch((error: unknown) => { state.errors.push(`VideoFrame.copyTo: ${String(error)}`); })
        .finally(() => clone.close()));
      return state.videoEncode.call(this, frame, options);
    };
    w.AudioEncoder.prototype.encode = function (data: any) {
      try {
        const channels = data.numberOfChannels, frames = data.numberOfFrames;
        const planes = Array.from({ length: channels }, (_, c) => {
          const plane = new Float32Array(frames);
          data.copyTo(plane, { planeIndex: c, format: 'f32-planar' });
          return plane;
        });
        const interleaved = new Float32Array(channels * frames);
        for (let i = 0; i < frames; i++) for (let c = 0; c < channels; c++) interleaved[i * channels + c] = planes[c]![i]!;
        state.audio.push(new Uint8Array(interleaved.buffer));
      } catch (error) { state.errors.push(`AudioData.copyTo: ${String(error)}`); }
      return state.audioEncode.call(this, data);
    };
  });
}

export async function saveLegacyWebmCapture(page: Page, directory: string): Promise<string[]> {
  const result = await page.evaluate(async () => {
    const w = window as any, state = w.__exportParityCapture;
    if (!state) throw new Error('WebCodecs capture was not installed');
    w.VideoEncoder.prototype.encode = state.videoEncode;
    w.AudioEncoder.prototype.encode = state.audioEncode;
    delete w.__exportParityCapture;
    await Promise.all(state.pending);
    const base64 = (bytes: Uint8Array) => {
      let text = '';
      for (let i = 0; i < bytes.length; i += 0x8000) text += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
      return btoa(text);
    };
    return { video: state.frames.filter((frame: Uint8Array | null): frame is Uint8Array => frame !== null).map(base64),
      audio: state.audio.map(base64), errors: state.errors as string[] };
  });
  await Promise.all([
    writeFile(path.join(directory, 'legacy.rgba'), Buffer.concat(result.video.map((part: string) => Buffer.from(part, 'base64')))),
    writeFile(path.join(directory, 'legacy.f32'), Buffer.concat(result.audio.map((part: string) => Buffer.from(part, 'base64')))),
  ]);
  return result.errors;
}
