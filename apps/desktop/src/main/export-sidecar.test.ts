import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { forgetExports, rememberExport, sidecarPath, writeSidecar } from './export-sidecar';

const dirs: string[] = [];
afterEach(async () => { forgetExports(7); await Promise.all(dirs.splice(0).map(dir => rm(dir, { recursive: true, force: true }))); });

describe('caption sidecars', () => {
  it('derives the sidecar name from the video', () => {
    expect(sidecarPath('/x/Film.final.mp4', 'srt')).toBe('/x/Film.final.srt');
    expect(sidecarPath('/x/Film.mov', 'vtt', 'en')).toBe('/x/Film.en.vtt');
    expect(sidecarPath('/x/Film.mov', 'vtt', '../evil')).toBe('/x/Film.vtt');
  });

  it('writes only beside a video this window exported', async () => {
    const dir = await mkdtemp(path.join(os.tmpdir(), 'pm-sidecar-'));
    dirs.push(dir);
    const video = path.join(dir, 'Film.mp4');
    expect((await writeSidecar(7, { path: video, extension: 'srt', text: 'x' })).ok).toBe(false);
    rememberExport(7, video);
    expect((await writeSidecar(8, { path: video, extension: 'srt', text: 'x' })).ok).toBe(false);
    expect((await writeSidecar(7, { path: video, extension: 'exe', text: 'x' } as any)).ok).toBe(false);
    const result = await writeSidecar(7, { path: video, extension: 'srt', suffix: 'en', text: '1\n00:00:00,000 --> 00:00:01,000\nHi\n' });
    expect(result).toEqual({ ok: true, path: path.join(dir, 'Film.en.srt') });
    expect(await readFile(path.join(dir, 'Film.en.srt'), 'utf8')).toContain('Hi');
  });
});
