import { requestJobEngine } from './helpers/job-engine';
import { execFileSync } from 'node:child_process';
import { mkdtemp, readdir, rm, stat } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { expect, test } from '@playwright/test';

import { launchApp } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

/* The export sheet with Use New Engine on (plan section 26, Phase 2): MP4 and
   ProRes render in the job engine and land at the chosen file; a project the
   engine cannot render yet goes to the current exporter, which writes the
   same file without asking again. The job engine's MP4 is muxed by
   AVAssetWriter, the current exporter's by FFmpeg (its `encoder` tag says
   Lavf), which tells the two apart. */

const ffprobe = ffmpegBinary.replace(/ffmpeg$/, 'ffprobe');
const probe = (file: string) => JSON.parse(execFileSync(ffprobe, ['-v', 'error', '-show_entries', 'format_tags=encoder:stream=codec_name,pix_fmt,nb_frames,color_transfer', '-of', 'json', file], { encoding: 'utf8' }));

test('the export sheet renders in the job engine and routes what it cannot render', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-engine-export-'));
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await requestJobEngine(session.app);
    await expect.poll(() => session.app.evaluate(() => {
      const status = (globalThis as { __powermoveNativeEngine?: { engine: { status(): Record<string, { state: string }> } | null } }).__powermoveNativeEngine?.engine?.status();
      return status ? `${status['interactive']!.state}/${status['job']!.state}` : 'missing';
    }), { timeout: 20_000 }).toBe('ready/ready');
    await session.openEditor();
    const exportTo = async (file: string, options: Record<string, unknown>): Promise<unknown> => {
      let asked = 0;
      await session.app.evaluate(({ dialog }, filePath) => {
        (globalThis as { __saveDialogs?: number }).__saveDialogs = 0;
        dialog.showSaveDialog = async () => {
          (globalThis as { __saveDialogs?: number }).__saveDialogs! += 1;
          return { canceled: false, filePath };
        };
      }, file);
      const result = await session.page.evaluate(async (options) => (window as any).PM.Export.run(options), options);
      asked = await session.app.evaluate(() => (globalThis as { __saveDialogs?: number }).__saveDialogs ?? 0);
      return { result, asked };
    };
    await session.page.evaluate(() => {
      const PM = (window as any).PM;
      PM.proj = PM.mkProject({ name: 'Engine export', w: 320, h: 180, fps: 24, dur: 1 });
      PM.proj.work = [0, 1];
      PM.bus.emit('project');
      PM.Edit.apply([{ type: 'add_layer', layerType: 'solid', name: 'Red', content: { color: '#e83a5e', w: 320, h: 180 }, duration: 1 }], { label: 'Red', origin: 'user' });
    });

    const mp4 = path.join(root, 'engine.mp4');
    expect(await exportTo(mp4, { format: 'mp4', scale: 1, fps: 24, range: 'all', quality: 'high', mblur: false, audio: false })).toEqual({ result: { cancelled: false }, asked: 1 });
    const engineMp4 = probe(mp4);
    expect(engineMp4.format.tags?.encoder ?? '').not.toMatch(/^Lavf/);
    expect(engineMp4.streams[0]).toMatchObject({ codec_name: 'h264', pix_fmt: 'yuv420p', nb_frames: '24', color_transfer: 'iec61966-2-1' });

    const mov = path.join(root, 'engine.mov');
    expect(await exportTo(mov, { format: 'prores', alpha: true, scale: 0.5, fps: 24, range: 'all', quality: 'high', mblur: false, audio: false })).toEqual({ result: { cancelled: false }, asked: 1 });
    expect(probe(mov).streams.find((s: { codec_name: string }) => s.codec_name === 'prores')).toMatchObject({ pix_fmt: 'yuva444p12le', nb_frames: '24' });

    // Text layers are not in the Phase 2 renderer: routed, one save panel, same file.
    await session.page.evaluate(() => {
      const PM = (window as any).PM;
      PM.Edit.apply([{ type: 'add_layer', layerType: 'text', name: 'Title', content: { text: 'Routed' }, duration: 1 }], { label: 'Title', origin: 'user' });
    });
    const routed = path.join(root, 'routed.mp4');
    expect(await exportTo(routed, { format: 'mp4', scale: 1, fps: 24, range: 'all', quality: 'high', mblur: false, audio: false })).toEqual({ result: { cancelled: false }, asked: 1 });
    expect(probe(routed).format.tags?.encoder ?? '').toMatch(/^Lavf/);
    expect((await stat(routed)).size).toBeGreaterThan(1000);
    // No temporary or work files remain beside the outputs.
    expect((await readdir(root)).sort()).toEqual(['engine.mov', 'engine.mp4', 'routed.mp4']);
  } finally {
    await session.close();
    await rm(root, { recursive: true, force: true });
  }
});
