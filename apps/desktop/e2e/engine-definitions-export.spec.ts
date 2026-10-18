import { execFileSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { expect, test } from '@playwright/test';

import { launchApp } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

/* Contract 10 end to end with Use New Engine on: an effect registered a
   moment before an export is drawn by the job engine's export (no routing,
   no false "missing"), an override by id draws the overriding GLSL, and
   disposing the override restores the earlier registration. The export
   starts in the same task as the registration: the export sheet settles the
   kernel's definitions (kernel/engine-definitions.ts `settle`) and main
   pushes the window's set to the job's snapshot session and waits for the
   job engine's acknowledgment before submitting (main/engine/agent-jobs.ts).
   The job engine's MP4 is muxed by AVAssetWriter, the current exporter's by
   FFmpeg (encoder tag Lavf), which tells a routed export apart. */

const ffprobe = ffmpegBinary.replace(/ffmpeg$/, 'ffprobe');
const encoder = (file: string): string => JSON.parse(execFileSync(ffprobe, ['-v', 'error', '-show_entries', 'format_tags=encoder', '-of', 'json', file], { encoding: 'utf8' })).format.tags?.encoder ?? '';
/** The first frame's pixel at (10, 10) as RGB. */
const pixel = (file: string): number[] => [...execFileSync(ffmpegBinary, ['-v', 'error', '-i', file, '-frames:v', '1', '-vf', 'crop=1:1:10:10', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'])];

test('an effect registered just before an export is drawn by the job engine, with overrides by id', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-engine-definitions-'));
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await expect.poll(() => session.app.evaluate(() => {
      const status = (globalThis as { __powermoveNativeEngine?: { engine: { status(): Record<string, { state: string }> } | null } }).__powermoveNativeEngine?.engine?.status();
      return status ? `${status['interactive']!.state}/${status['job']!.state}` : 'missing';
    }), { timeout: 20_000 }).toBe('ready/ready');
    await session.openEditor();
    await expect.poll(() => session.page.evaluate(() => !!(window as any).PM.EngineDefinitions), { timeout: 20_000 }).toBe(true);
    await session.page.evaluate(() => {
      const PM = (window as any).PM;
      PM.proj = PM.mkProject({ name: 'Definitions export', w: 320, h: 180, fps: 24, dur: 1 });
      PM.proj.work = [0, 1];
      PM.bus.emit('project');
      PM.Edit.apply([{ type: 'add_layer', layerType: 'solid', name: 'Red', content: { color: '#e83a5e', w: 320, h: 180 }, duration: 1 }], { label: 'Red', origin: 'user' });
    });
    const exportTo = async (file: string, register: 'publish' | 'override' | 'dispose'): Promise<unknown> => {
      await session.app.evaluate(({ dialog }, filePath) => {
        dialog.showSaveDialog = async () => ({ canceled: false, filePath });
      }, file);
      // The registration and the export start in one task: nothing waits
      // for the registry's change to reach main before the export does.
      return session.page.evaluate(async (step) => {
        const PM = (window as any).PM;
        const w = window as any;
        const effect = (frag: string) => ({ id: 'e2e.tint', label: 'E2E tint', group: 'Color', params: [], frag });
        if (step === 'publish') {
          w.__e2eTint = PM.Kernel.registerEffect('e2e.publisher', effect('vec4 c = src(v_st); o = vec4(0.0, 0.0, c.a, c.a);'));
          const layer = PM.proj.layers[0];
          PM.Edit.apply({ type: 'add_effect', target: layer.id, effect: 'e2e.tint' }, { origin: 'user', label: 'Add effect' });
        } else if (step === 'override') {
          w.__e2eOverride = PM.Kernel.registerEffect('e2e.override', effect('vec4 c = src(v_st); o = vec4(0.0, c.a, 0.0, c.a);'));
        } else {
          w.__e2eOverride.dispose();
        }
        return PM.Export.run({ format: 'mp4', scale: 1, fps: 24, range: 'all', quality: 'high', mblur: false, audio: false });
      }, register);
    };

    const published = path.join(root, 'published.mp4');
    expect(await exportTo(published, 'publish')).toEqual({ cancelled: false });
    expect(encoder(published), 'drawn by the job engine, not routed').not.toMatch(/^Lavf/);
    const [r1, g1, b1] = pixel(published);
    expect(b1, 'the published effect is drawn').toBeGreaterThan(200);
    expect(r1! + g1!).toBeLessThan(80);

    const overridden = path.join(root, 'overridden.mp4');
    expect(await exportTo(overridden, 'override')).toEqual({ cancelled: false });
    expect(encoder(overridden)).not.toMatch(/^Lavf/);
    const [r2, g2, b2] = pixel(overridden);
    expect(g2, 'the override by id wins').toBeGreaterThan(200);
    expect(r2! + b2!).toBeLessThan(80);

    const restored = path.join(root, 'restored.mp4');
    expect(await exportTo(restored, 'dispose')).toEqual({ cancelled: false });
    expect(encoder(restored)).not.toMatch(/^Lavf/);
    const [r3, g3, b3] = pixel(restored);
    expect(b3, 'disposing the override restores the earlier registration').toBeGreaterThan(200);
    expect(r3! + g3!).toBeLessThan(80);
  } finally {
    await session.close();
    await rm(root, { recursive: true, force: true });
  }
});
