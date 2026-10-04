import path from 'node:path';

/*
 * ffmpeg turns any audio or video file into what the recognizer eats: 16 kHz
 * mono 32-bit float, streamed on stdout. The bundled ffmpeg-static binary is
 * the one media-proxy and render-encoder use.
 */

export interface DecodeSpan {
  start?: number;
  end?: number;
}

export function decodeArgs(file: string, span: DecodeSpan = {}): string[] {
  const start = span.start && span.start > 0 ? span.start : 0;
  const args = ['-nostdin', '-hide_banner', '-loglevel', 'info'];
  /* Input seeking: fast, and exact for decoded audio. */
  if (start > 0) args.push('-ss', start.toFixed(3));
  if (span.end !== undefined && span.end > start) args.push('-t', (span.end - start).toFixed(3));
  args.push('-i', file, '-map', '0:a:0', '-vn', '-sn', '-dn', '-ac', '1', '-ar', '16000', '-f', 'f32le', 'pipe:1');
  return args;
}

/** The input duration from ffmpeg's banner ("Duration: 00:01:02.03"), or null. */
export function parseDuration(stderr: string): number | null {
  const match = /Duration:\s*(\d+):(\d{2}):(\d{2}(?:\.\d+)?)/.exec(stderr);
  if (!match) return null;
  return Number(match[1]) * 3600 + Number(match[2]) * 60 + Number(match[3]);
}

/** A short, human reason from ffmpeg's error output. */
export function decodeError(stderr: string): string {
  if (/does not contain any stream|matches no streams|Output file #0 does not contain/i.test(stderr)) return 'This file has no audio to transcribe.';
  if (/No such file or directory/i.test(stderr)) return 'The media file could not be found.';
  if (/Invalid data found|could not find codec|Unknown format/i.test(stderr)) return 'This file’s audio could not be read.';
  return 'The audio could not be decoded.';
}

/** Where the bundled ffmpeg lives in a packaged app and in development. */
export function bundledFfmpeg(packaged: boolean, resourcesPath: string, appPath: string): string {
  return packaged ? path.join(resourcesPath, 'encoder', 'ffmpeg') : path.join(appPath, 'node_modules', 'ffmpeg-static', 'ffmpeg');
}
