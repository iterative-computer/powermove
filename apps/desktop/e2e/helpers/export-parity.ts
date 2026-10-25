import { execFileSync } from 'node:child_process';
import path from 'node:path';

import { expect } from '@playwright/test';

import { ffmpegBinary } from './ffmpeg';

const MAX_DECODED_BYTES = 64 * 1024 * 1024;

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
