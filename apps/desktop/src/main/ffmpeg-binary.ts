/*
 * Where Powermove's own FFmpeg lives.
 *
 * The app ships an LGPL-only FFmpeg built by engine/third_party/ffmpeg/build.sh
 * (plan section 24 item 1): the libav* dylibs plus `ffmpeg` and `ffprobe`
 * from the same configuration. Packaged, it sits in
 * `Contents/Resources/ffmpeg/{bin,lib,share/licenses}`; the executables find
 * their dylibs through `@executable_path/../lib`. The build has no libx264,
 * no prores_ks and no other GPL or nonfree component: H.264 and ProRes are
 * encoded through VideoToolbox (`h264_videotoolbox`, `prores_videotoolbox`).
 *
 * Development and tests use, in order: `$PM_FFMPEG_DIR` (set by
 * engine/env.sh), the copy staged for packaging in `apps/desktop/vendor/ffmpeg`
 * (engine/third_party/ffmpeg/package.sh), and the shared build cache that
 * build.sh writes by default.
 */
import { existsSync } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';

export interface FfmpegBuild {
  /** The build's root: bin/, lib/, share/licenses/. */
  root: string;
  ffmpeg: string;
  ffprobe: string;
}

/** build.sh's default output, shared by every worktree (engine/env.sh). */
export const SHARED_FFMPEG_DIR = path.join(homedir(), 'Library', 'Caches', 'powermove-engine', 'ffmpeg', 'out');

/** The directory, relative to apps/desktop, that package.sh stages for electron-builder. */
export const STAGED_FFMPEG_DIR = path.join('vendor', 'ffmpeg');

/** The directory under the packaged app's Resources. */
export const PACKAGED_FFMPEG_DIR = 'ffmpeg';

export function ffmpegBuild(root: string): FfmpegBuild {
  return { root, ffmpeg: path.join(root, 'bin', 'ffmpeg'), ffprobe: path.join(root, 'bin', 'ffprobe') };
}

/** Development candidates in priority order (see the header). */
export function developmentFfmpegDirs(appPath: string, env: NodeJS.ProcessEnv = process.env): string[] {
  const dirs = [env['PM_FFMPEG_DIR'], path.join(appPath, STAGED_FFMPEG_DIR), SHARED_FFMPEG_DIR];
  return dirs.filter((dir): dir is string => !!dir).map((dir) => path.resolve(dir));
}

/**
 * The FFmpeg build this process should run. A packaged app only ever uses its
 * own bundled copy. In development the first candidate whose `bin/ffmpeg`
 * exists wins; when none does, the first candidate is returned so the spawn
 * error names a concrete path (and `ffmpegMissingMessage` explains it).
 */
export function resolveFfmpeg(options: { packaged: boolean; resourcesPath: string; appPath: string; env?: NodeJS.ProcessEnv }): FfmpegBuild {
  if (options.packaged) return ffmpegBuild(path.join(options.resourcesPath, PACKAGED_FFMPEG_DIR));
  const dirs = developmentFfmpegDirs(options.appPath, options.env);
  const found = dirs.find((dir) => existsSync(path.join(dir, 'bin', 'ffmpeg')));
  return ffmpegBuild(found ?? dirs[0]!);
}

/** Why a development FFmpeg is missing and how to get one. */
export function ffmpegMissingMessage(build: FfmpegBuild): string {
  return `Powermove's FFmpeg build was not found at ${build.ffmpeg}. Build it with engine/third_party/ffmpeg/build.sh (or set PM_FFMPEG_DIR to its output).`;
}
