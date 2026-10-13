import { execFile } from 'node:child_process';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';

import { describe, expect, it } from 'vitest';

import { developmentFfmpegDirs, ffmpegMissingMessage, resolveFfmpeg, SHARED_FFMPEG_DIR } from './ffmpeg-binary';

describe('Powermove FFmpeg resolution', () => {
  it('a packaged app only runs its bundled copy, whatever the environment says', () => {
    const tools = resolveFfmpeg({ packaged: true, resourcesPath: '/Applications/Powermove.app/Contents/Resources', appPath: '/x', env: { PM_FFMPEG_DIR: '/tmp/elsewhere' } });
    expect(tools).toEqual({
      root: '/Applications/Powermove.app/Contents/Resources/ffmpeg',
      ffmpeg: '/Applications/Powermove.app/Contents/Resources/ffmpeg/bin/ffmpeg',
      ffprobe: '/Applications/Powermove.app/Contents/Resources/ffmpeg/bin/ffprobe'
    });
  });

  it('development prefers PM_FFMPEG_DIR, then the staged copy, then the shared build', async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), 'pm-ffmpeg-resolve-'));
    try {
      const app = path.join(root, 'desktop'), env = path.join(root, 'env');
      expect(developmentFfmpegDirs(app, { PM_FFMPEG_DIR: env })).toEqual([env, path.join(app, 'vendor/ffmpeg'), SHARED_FFMPEG_DIR]);
      expect(developmentFfmpegDirs(app, {})).toEqual([path.join(app, 'vendor/ffmpeg'), SHARED_FFMPEG_DIR]);
      // Only directories that hold bin/ffmpeg count.
      await mkdir(path.join(app, 'vendor/ffmpeg/bin'), { recursive: true });
      await writeFile(path.join(app, 'vendor/ffmpeg/bin/ffmpeg'), '');
      expect(resolveFfmpeg({ packaged: false, resourcesPath: '', appPath: app, env: { PM_FFMPEG_DIR: env } }).ffmpeg).toBe(path.join(app, 'vendor/ffmpeg/bin/ffmpeg'));
      await mkdir(path.join(env, 'bin'), { recursive: true });
      await writeFile(path.join(env, 'bin/ffmpeg'), '');
      expect(resolveFfmpeg({ packaged: false, resourcesPath: '', appPath: app, env: { PM_FFMPEG_DIR: env } }).ffprobe).toBe(path.join(env, 'bin/ffprobe'));
    } finally { await rm(root, { recursive: true, force: true }); }
  });

  it('names the missing path and how to build it', () => {
    const tools = resolveFfmpeg({ packaged: false, resourcesPath: '', appPath: '/nowhere', env: { PM_FFMPEG_DIR: '/nowhere/ffmpeg' } });
    expect(ffmpegMissingMessage(tools)).toContain('engine/third_party/ffmpeg/build.sh');
  });

  it('the development build is the LGPL one, with VideoToolbox encoders and without libx264 or prores_ks', async () => {
    const tools = resolveFfmpeg({ packaged: false, resourcesPath: '', appPath: path.resolve(__dirname, '../..') });
    const run = promisify(execFile);
    const license = await run(tools.ffmpeg, ['-hide_banner', '-L']);
    expect(license.stdout + license.stderr).toContain('GNU Lesser General Public');
    const { stdout } = await run(tools.ffmpeg, ['-hide_banner', '-encoders']);
    const encoders = new Set(stdout.split('\n').map((line) => /^ [A-Z.]{6} (\S+)/.exec(line)?.[1]).filter(Boolean));
    for (const name of ['h264_videotoolbox', 'prores_videotoolbox', 'hevc_videotoolbox', 'aac', 'pcm_s24le', 'libvpx-vp9']) expect(encoders).toContain(name);
    for (const name of ['libx264', 'prores_ks', 'prores_aw', 'libmp3lame']) expect(encoders).not.toContain(name);
    await run(tools.ffprobe, ['-hide_banner', '-version']);
  });
});
