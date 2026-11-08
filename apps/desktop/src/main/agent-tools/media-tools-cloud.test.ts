import { mkdtemp, rename, rm, utimes, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import { AgentMediaTools, type MediaToolContext } from './media-tools';

const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });
async function fixture() {
  const root = await mkdtemp(path.join(tmpdir(), 'pm-analysis-cloud-')); roots.push(root);
  const file = path.join(root, 'synthetic.mov'), time = new Date('2026-01-01');
  await writeFile(file, 'original video'); await utimes(file, time, time);
  const media = new AgentMediaTools({ ffmpegPath: '/unused' });
  const run = vi.spyOn(media as any, 'run').mockResolvedValue({ code: 1, stdout: Buffer.alloc(0), stderr:
    "Input #0, mov, from 'synthetic.mov':\n  Duration: 00:00:01.00, start: 0.000000\n  Stream #0:0: Video: h264, yuv420p, 640x360, 30 fps\n" });
  const context: MediaToolContext = {
    resolve: async () => ({ path: file, origin: 'source', asset: { id: 'clip', name: 'clip', kind: 'video', duration: 1, hasAudio: false, proxy: false } }),
    renderFrames: async () => [],
    composition: async () => ({ duration: 1, fps: 30, width: 640, height: 360 })
  };
  const probe = () => media.call('probe_media', { assetId: 'clip' }, context);
  return { file, time, run, probe };
}

it('re-probes changed bytes even when sync preserves the file size and mtime', async () => {
  const { file, time, probe, run } = await fixture();
  await probe(); await probe(); expect(run).toHaveBeenCalledTimes(1);
  await writeFile(file + '.incoming', 'modified video'); await utimes(file + '.incoming', time, time);
  await rename(file + '.incoming', file);
  await probe(); expect(run).toHaveBeenCalledTimes(2);
});

it('keeps media analysis after metadata churn and identical file replacement', async () => {
  const { file, probe, run } = await fixture();
  await probe();
  await utimes(file, new Date(), new Date()); await probe();
  await writeFile(file + '.incoming', 'original video'); await rename(file + '.incoming', file); await probe();
  expect(run).toHaveBeenCalledTimes(1);
});
