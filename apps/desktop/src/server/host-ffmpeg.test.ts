import path from 'node:path';

import { describe, expect, it } from 'vitest';

import { SHARED_FFMPEG_DIR } from '../main/ffmpeg-binary';
import { hostFfmpeg } from './cli';

describe('the CLI host FFmpeg', () => {
  it('uses POWERMOVE_FFMPEG, then a Powermove FFmpeg build, then ffmpeg from PATH', () => {
    const none = () => false;
    expect(hostFfmpeg({ POWERMOVE_FFMPEG: 'tools/ffmpeg', PM_FFMPEG_DIR: '/b' }, () => true)).toBe(path.resolve('tools/ffmpeg'));
    expect(hostFfmpeg({ PM_FFMPEG_DIR: '/b' }, (file) => file === '/b/bin/ffmpeg')).toBe('/b/bin/ffmpeg');
    expect(hostFfmpeg({ PM_FFMPEG_DIR: '/b' }, (file) => file === path.join(SHARED_FFMPEG_DIR, 'bin/ffmpeg'))).toBe(path.join(SHARED_FFMPEG_DIR, 'bin/ffmpeg'));
    expect(hostFfmpeg({}, none)).toBe('ffmpeg');
  });
});
