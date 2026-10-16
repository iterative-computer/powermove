/**
 * Generated video for the motion, 2.5D, and retiming cases
 * (docs/engine/motion.md): every frame differs (FFmpeg's `testsrc2`, a
 * frame counter, moving color bars, and a gradient), so the frame a
 * retimed clip or a motion-blur sample shows is visible in its pixels. The
 * fixture videos switch between two flat colors once and cannot show which
 * frame was selected.
 *
 * Encoded with Homebrew's FFmpeg (libx264, single-threaded, fixed settings)
 * as a test tool, never linked; the capture records each file's SHA-256 and
 * the engine tests refuse a file that differs. The files are untagged (SD:
 * BT.601 in Chromium and in section 9's rule alike); a file tagged only with
 * a BT.709 matrix (no primaries or transfer) is read with BT.601
 * coefficients by Chromium but BT.709 by the engine (up to 23 CV on
 * saturated red), a color-interpretation question outside this corpus.
 */
import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const FFMPEG = process.env.PM_CORPUS_FFMPEG ?? '/opt/homebrew/bin/ffmpeg';

function testsrc(fps: number, seconds: number): Uint8Array {
  if (!existsSync(FFMPEG)) throw new Error(`The corpus needs an FFmpeg with libx264 at ${FFMPEG} (brew install ffmpeg, or set PM_CORPUS_FFMPEG) to generate its frame-counter videos`);
  const dir = mkdtempSync(path.join(os.tmpdir(), 'pm-corpus-video-'));
  try {
    const out = path.join(dir, 'out.mp4');
    execFileSync(FFMPEG, [
      '-hide_banner', '-loglevel', 'error', '-y',
      '-f', 'lavfi', '-i', `testsrc2=size=320x180:rate=${fps}:duration=${seconds}`,
      '-c:v', 'libx264', '-preset', 'medium', '-crf', '10', '-g', String(fps), '-bf', '0',
      '-pix_fmt', 'yuv420p', '-threads', '1', '-x264-params', 'threads=1:sliced-threads=0',
      '-movflags', '+faststart', '-map_metadata', '-1', '-fflags', '+bitexact', '-flags:v', '+bitexact',
      out,
    ], { stdio: ['ignore', 'ignore', 'pipe'] });
    return new Uint8Array(readFileSync(out));
  } finally { rmSync(dir, { recursive: true, force: true }); }
}

export const GENERATED_VIDEOS: Record<string, () => Uint8Array> = {
  'corpus-frames-30.mp4': () => testsrc(30, 4),
  'corpus-frames-24.mp4': () => testsrc(24, 4),
};
