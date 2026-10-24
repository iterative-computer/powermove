import { execFileSync } from 'node:child_process';
import path from 'node:path';

import { expect } from '@playwright/test';

import { ffmpegBinary } from './ffmpeg';

const ffprobeBinary = ffmpegBinary.replace(/ffmpeg$/, 'ffprobe');
const MAX_DECODED_BYTES = 64 * 1024 * 1024;

type Stream = Record<string, any>;
type Probe = { streams: Stream[]; format: Record<string, any> };

function probe(file: string): Probe {
  return JSON.parse(execFileSync(ffprobeBinary, ['-v', 'error', '-count_frames', '-show_streams', '-show_format', '-of', 'json', file], {
    encoding: 'utf8', maxBuffer: 4 * 1024 * 1024,
  })) as Probe;
}

function one(streams: Stream[], type: string, file: string): Stream | null {
  const matching = streams.filter(stream => stream.codec_type === type);
  expect(matching.length, `${file}: ${type} track count`).toBeLessThanOrEqual(1);
  return matching[0] ?? null;
}

function decode(file: string, type: 'video' | 'audio'): Buffer {
  const args = type === 'video'
    ? ['-v', 'error', '-i', file, '-map', '0:v:0', '-fps_mode', 'passthrough', '-f', 'rawvideo', '-pix_fmt', 'rgba', 'pipe:1']
    : ['-v', 'error', '-i', file, '-map', '0:a:0', '-f', 'f32le', '-acodec', 'pcm_f32le', 'pipe:1'];
  return execFileSync(ffmpegBinary, args, { maxBuffer: MAX_DECODED_BYTES });
}

function equalFields(legacy: Stream, native: Stream, fields: readonly string[], label: string): void {
  for (const field of fields) {
    expect(native[field] ?? null, `${label}: ${field}`).toEqual(legacy[field] ?? null);
  }
}

/** EXPORT-1 compares media semantics. Container creation dates, track order, handlers,
 * vendor tags and atom layout are deliberately outside the equality assertion. */
export function expectExportParity(legacyFile: string, nativeFile: string, expectedFrames: number, expectAudio: boolean): void {
  const legacy = probe(legacyFile), native = probe(nativeFile);
  const oldVideo = one(legacy.streams, 'video', legacyFile);
  const newVideo = one(native.streams, 'video', nativeFile);
  expect(oldVideo, `${legacyFile}: video track`).not.toBeNull();
  expect(newVideo, `${nativeFile}: video track`).not.toBeNull();
  for (const [label, stream] of [['legacy', oldVideo!], ['native', newVideo!]] as const) {
    for (const field of ['codec_name', 'pix_fmt', 'r_frame_rate', 'avg_frame_rate', 'time_base'] as const) {
      expect(stream[field], `${label} video: ${field}`).toBeTruthy();
    }
  }
  equalFields(oldVideo!, newVideo!, [
    'codec_name', 'profile', 'codec_tag_string', 'pix_fmt', 'bits_per_raw_sample',
    'color_primaries', 'color_transfer', 'color_space', 'color_range',
    'width', 'height', 'r_frame_rate', 'avg_frame_rate', 'time_base',
    'start_time', 'nb_frames', 'nb_read_frames', 'duration', 'duration_ts',
  ], 'video');
  expect(Number(oldVideo!.nb_read_frames), 'legacy decoded frame count').toBe(expectedFrames);
  expect(Number(newVideo!.nb_read_frames), 'native decoded frame count').toBe(expectedFrames);
  expect(Number.isFinite(Number(legacy.format.duration)), 'legacy container duration').toBe(true);
  expect(Number.isFinite(Number(native.format.duration)), 'native container duration').toBe(true);
  expect(Number.isFinite(Number(legacy.format.start_time)), 'legacy container start').toBe(true);
  expect(Number.isFinite(Number(native.format.start_time)), 'native container start').toBe(true);
  expect(Number(native.format.duration), 'container duration').toBe(Number(legacy.format.duration));
  expect(Number(native.format.start_time), 'container start').toBe(Number(legacy.format.start_time));

  const frameBytes = Number(oldVideo!.width) * Number(oldVideo!.height) * 4;
  const oldFrames = decode(legacyFile, 'video'), newFrames = decode(nativeFile, 'video');
  expect(oldFrames.length, 'legacy decoded RGBA byte count').toBe(expectedFrames * frameBytes);
  expect(newFrames.length, 'native decoded RGBA byte count').toBe(expectedFrames * frameBytes);
  for (let frame = 0; frame < expectedFrames; frame++) {
    const start = frame * frameBytes;
    expect(newFrames.subarray(start, start + frameBytes).equals(oldFrames.subarray(start, start + frameBytes)),
      `decoded RGBA frame ${frame}`).toBe(true);
  }

  const oldAudio = one(legacy.streams, 'audio', legacyFile);
  const newAudio = one(native.streams, 'audio', nativeFile);
  expect(!!oldAudio, 'legacy audio track present').toBe(expectAudio);
  expect(!!newAudio, 'native audio track present').toBe(expectAudio);
  if (oldAudio && newAudio) {
    equalFields(oldAudio, newAudio, [
      'codec_name', 'profile', 'sample_fmt', 'sample_rate', 'channels',
      'channel_layout', 'ch_layout', 'bits_per_sample', 'bits_per_raw_sample',
      'start_time', 'duration', 'duration_ts',
    ], 'audio');
    const oldSamples = decode(legacyFile, 'audio'), newSamples = decode(nativeFile, 'audio');
    expect(oldSamples.length, 'decoded audio sample count').toBeGreaterThan(0);
    expect(newSamples.equals(oldSamples), 'decoded f32 audio samples').toBe(true);
  }
}

export type AvFoundationImport = { readable: boolean; playable: boolean; frames: number; audioSamples: number; firstPTS: number; duration: number; videoDuration: number; timecodeTrackStart: number | null; timecodeTrackDuration: number | null };

/** AVAssetReader consumes every picture, and AVURLAsset checks QuickTime import. */
export function avFoundationImport(file: string): AvFoundationImport {
  const script = path.join(__dirname, 'export-avfoundation.swift');
  return JSON.parse(execFileSync('/usr/bin/swift', [script, file], { encoding: 'utf8', maxBuffer: 1024 * 1024 })) as AvFoundationImport;
}

/** RDD 36 §5.1.1: byte 21 is aspect ratio (high nibble) and frame-rate code
 * (low nibble). Code 0 is unspecified, code 2 is 24 fps. */
export function proResFrameHeaderCodes(file: string): { aspectRatio: number; frameRate: number } {
  const bitstream = execFileSync(ffmpegBinary, [
    '-v', 'error', '-i', file, '-map', '0:v:0', '-c:v', 'copy', '-f', 'data', 'pipe:1',
  ], { maxBuffer: MAX_DECODED_BYTES });
  expect(bitstream.length).toBeGreaterThan(21);
  expect(bitstream.toString('ascii', 4, 8)).toBe('icpf');
  return { aspectRatio: bitstream[21]! >> 4, frameRate: bitstream[21]! & 0x0f };
}
