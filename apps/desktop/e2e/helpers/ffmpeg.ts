import path from 'node:path';

import { resolveFfmpeg } from '../../src/main/ffmpeg-binary';

/**
 * Powermove's own FFmpeg build: the one the app under test runs
 * (src/main/ffmpeg-binary.ts). It has no libx264 or prores_ks; fixtures
 * encode H.264 and ProRes through VideoToolbox.
 */
export const ffmpegBinary = resolveFfmpeg({ packaged: false, resourcesPath: '', appPath: path.resolve(__dirname, '../..') }).ffmpeg;
