import { execFileSync } from 'node:child_process';
import { mkdtemp, readdir, rm, stat } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { expect, test } from '@playwright/test';

import { launchApp } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

/* With Use New Engine on, solid and text MP4s and ProRes render in the job
   engine and land at the chosen file. The job engine's MP4 is muxed by
   AVAssetWriter; the current exporter's FFmpeg encoder tag starts with Lavf. */

const ffprobe = ffmpegBinary.replace(/ffmpeg$/, 'ffprobe');
const probe = (file: string) => JSON.parse(execFileSync(ffprobe, ['-v', 'error', '-show_entries', 'format_tags=encoder:stream=codec_name,pix_fmt,nb_frames,color_transfer', '-of', 'json', file], { encoding: 'utf8' }));

test('the export sheet renders solid, text, and ProRes outputs in the job engine', async () => {
  test.setTimeout(120_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-engine-export-'));
  let session: Awaited<ReturnType<typeof launchApp>> | undefined;
  try {
    const active = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
    session = active;
    // The job engine starts on demand: the first export finds it stopped
    // and must still render in it, not route to the current exporter.
    await expect.poll(() => active.app.evaluate(() => {
      const status = (globalThis as { __powermoveNativeEngine?: { engine: { status(): Record<string, { state: string }> } | null } }).__powermoveNativeEngine?.engine?.status();
      return status ? `${status['interactive']!.state}/${status['job']!.state}` : 'missing';
    }), { timeout: 20_000 }).toBe('ready/stopped');
    await active.page.waitForFunction(() => Boolean((window as any).PM?.ProjectsScreen && (window as any).PM?.mkProject));
    await active.page.evaluate(async () => { await (window as any).PM.Kernel.loader.builtinsReady; });
    await active.page.evaluate(() => {
      const PM = (window as any).PM;
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'E2E composition' }) }));
      PM.ProjectsScreen.hide();
    });
    await active.page.waitForSelector('#body .dock', { state: 'visible' });
    await active.page.waitForFunction(() => Boolean((window as any).PM.Kernel.services.get('viewer')?.stage?.isConnected));
    const compatibility = await active.page.evaluate(() => {
      const gl = (window as any).PM.GL;
      const deferred = gl.peekContext() === null;
      const context = gl.gl;
      return { deferred, ready: !!context, same: context === gl.gl, firstUseMs: gl.compatFirstUseMs };
    });
    expect(compatibility.deferred, 'the native viewer leaves the legacy WebGL context unallocated').toBe(true);
    expect(compatibility).toMatchObject({ ready: true, same: true });
    expect(compatibility.firstUseMs, 'first compatibility WebGL use').toBeLessThan(50);
    test.info().annotations.push({ type: 'compat-first-use-ms', description: String(compatibility.firstUseMs) });
    const exportTo = async (file: string, options: Record<string, unknown>): Promise<unknown> => {
      let asked = 0;
      await active.app.evaluate(({ dialog }, filePath) => {
        (globalThis as { __saveDialogs?: number }).__saveDialogs = 0;
        dialog.showSaveDialog = async () => {
          (globalThis as { __saveDialogs?: number }).__saveDialogs! += 1;
          return { canceled: false, filePath };
        };
      }, file);
      const result = await active.page.evaluate(async (options) => (window as any).PM.Export.run(options), options);
      asked = await active.app.evaluate(() => (globalThis as { __saveDialogs?: number }).__saveDialogs ?? 0);
      return { result, asked };
    };
    await active.page.evaluate(() => {
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

    // Text is now drawn by the job engine too, with one save panel.
    await active.page.evaluate(() => {
      const PM = (window as any).PM;
      PM.Edit.apply([{ type: 'add_layer', layerType: 'text', name: 'Title', content: { text: 'Native title' }, duration: 1 }], { label: 'Title', origin: 'user' });
    });
    const textFile = path.join(root, 'text.mp4');
    expect(await exportTo(textFile, { format: 'mp4', scale: 1, fps: 24, range: 'all', quality: 'high', mblur: false, audio: false })).toEqual({ result: { cancelled: false }, asked: 1 });
    const textMp4 = probe(textFile);
    expect(textMp4.format.tags?.encoder ?? '').not.toMatch(/^Lavf/);
    expect(textMp4.streams[0]).toMatchObject({ codec_name: 'h264', nb_frames: '24' });
    expect((await stat(textFile)).size).toBeGreaterThan(1000);
    // No temporary or work files remain beside the outputs.
    expect((await readdir(root)).sort()).toEqual(['engine.mov', 'engine.mp4', 'text.mp4']);
  } finally {
    await session?.close();
    await rm(root, { recursive: true, force: true });
  }
});
