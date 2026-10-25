import { requestJobEngine } from './helpers/job-engine';
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
   Both exporters write the same file (EXPORT-1), so completion and routing
   are checked through the native job's export-status reports. */

/** The first frame's pixel at (10, 10) as RGB. */
const pixel = (file: string): number[] => [...execFileSync(ffmpegBinary, ['-v', 'error', '-i', file, '-frames:v', '1', '-vf', 'format=rgb24,crop=1:1:10:10', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'])];
type JobEvidence = { result: { cancelled: boolean }; routed: unknown; statuses: Array<{ state: string; routed: boolean }> };
function expectNativeJob(evidence: JobEvidence): void {
  expect(evidence.result).toEqual({ cancelled: false });
  expect(evidence.routed, 'the exporter did not hand the job to the legacy path').toBeNull();
  expect(evidence.statuses.some(status => status.state === 'done' && !status.routed), 'the native job reported completion').toBe(true);
  expect(evidence.statuses.some(status => status.routed), 'the native job never reported routing').toBe(false);
  expect(evidence.statuses.filter(status => status.state === 'failed'), 'no native job failure').toEqual([]);
}

test('an effect registered just before an export is drawn by the job engine, with overrides by id', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-engine-definitions-'));
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await requestJobEngine(session.app);
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
    const exportTo = async (file: string, register: 'publish' | 'override' | 'dispose'): Promise<JobEvidence> => {
      await session.app.evaluate(({ dialog }, filePath) => {
        dialog.showSaveDialog = async () => ({ canceled: false, filePath });
      }, file);
      // The registration and the export start in one task: nothing waits
      // for the registry's change to reach main before the export does.
      return session.page.evaluate(async (step) => {
        const PM = (window as any).PM;
        const w = window as any;
        const statuses: JobEvidence['statuses'] = [];
        const off = w.__powermoveTestBridge.nativeEngine.onExportStatus((status: any) => statuses.push({ state: status.state, routed: status.routed === true }));
        try {
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
          PM.Export.lastRouted = null;
          const result = await PM.Export.run({ format: 'mp4', scale: 1, fps: 24, range: 'all', quality: 'high', mblur: false, audio: false });
          return { result, routed: PM.Export.lastRouted, statuses };
        } finally { off(); }
      }, register);
    };

    const published = path.join(root, 'published.mp4');
    expectNativeJob(await exportTo(published, 'publish'));
    const [r1, g1, b1] = pixel(published);
    expect(b1, 'the published effect is drawn').toBeGreaterThan(200);
    expect(r1! + g1!).toBeLessThan(80);

    const overridden = path.join(root, 'overridden.mp4');
    expectNativeJob(await exportTo(overridden, 'override'));
    const [r2, g2, b2] = pixel(overridden);
    expect(g2, 'the override by id wins').toBeGreaterThan(200);
    expect(r2! + b2!).toBeLessThan(80);

    const restored = path.join(root, 'restored.mp4');
    expectNativeJob(await exportTo(restored, 'dispose'));
    const [r3, g3, b3] = pixel(restored);
    expect(b3, 'disposing the override restores the earlier registration').toBeGreaterThan(200);
    expect(r3! + g3!).toBeLessThan(80);
  } finally {
    await session.close();
    await rm(root, { recursive: true, force: true });
  }
});

/* Decision 19 for effects (amendment 21: drawn, not routed): an effect the
   native pipeline cannot translate (a sampler3D; WebGL draws it on unit 1,
   where keepOrig's `u_orig` is set and no 3D texture is bound, so it reads
   (0, 0, 0, 1)) is drawn in a job export by the window's fallback runner
   with its input texture (contract 14: the job engine asks main, main asks
   the window's runner), and composed back into the chain. */
test('an effect only the WebGL runner draws is exported through the window\'s fallback runner', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-engine-fallback-export-'));
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await requestJobEngine(session.app);
    await expect.poll(() => session.app.evaluate(() => {
      const status = (globalThis as { __powermoveNativeEngine?: { engine: { status(): Record<string, { state: string }> } | null } }).__powermoveNativeEngine?.engine?.status();
      return status ? `${status['interactive']!.state}/${status['job']!.state}` : 'missing';
    }), { timeout: 20_000 }).toBe('ready/ready');
    await session.openEditor();
    await expect.poll(() => session.page.evaluate(() => !!(window as any).PM.EngineDefinitions), { timeout: 20_000 }).toBe(true);
    const file = path.join(root, 'fallback.mp4');
    await session.app.evaluate(({ dialog }, filePath) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath });
    }, file);
    const result = await session.page.evaluate(async () => {
      const PM = (window as any).PM;
      const statuses: JobEvidence['statuses'] = [];
      const off = (window as any).__powermoveTestBridge.nativeEngine.onExportStatus((status: any) => statuses.push({ state: status.state, routed: status.routed === true }));
      try {
        PM.proj = PM.mkProject({ name: 'Fallback export', w: 320, h: 180, fps: 24, dur: 1 });
        PM.proj.work = [0, 1];
        PM.bus.emit('project');
        PM.Edit.apply([{ type: 'add_layer', layerType: 'solid', name: 'Red', content: { color: '#e83a5e', w: 320, h: 180 }, duration: 1 }], { label: 'Red', origin: 'user' });
        PM.Kernel.registerEffect('e2e.fallback', {
          id: 'e2e.volume', label: 'E2E volume', group: 'Color', params: [], rawShader: true, keepOrig: true,
          frag: [
            '#version 300 es',
            'precision highp float;',
            'in vec2 v_uv; in vec2 v_st; in vec2 v_px;',
            'uniform sampler2D u_tex;',
            'uniform highp sampler3D u_orig;',
            'out vec4 o;',
            'void main(){ vec4 c = texture(u_tex, v_st); o = vec4(c.a, c.a, 0.0, c.a) * texture(u_orig, vec3(0.5)).a; }'
          ].join('\n')
        });
        PM.Edit.apply({ type: 'add_effect', target: PM.proj.layers[0].id, effect: 'e2e.volume' }, { origin: 'user', label: 'Add effect' });
        PM.Export.lastRouted = null;
        const result = await PM.Export.run({ format: 'mp4', scale: 1, fps: 24, range: 'all', quality: 'high', mblur: false, audio: false });
        return { result, routed: PM.Export.lastRouted, statuses };
      } finally { off(); }
    });
    expectNativeJob(result);
    const [r, g, b] = pixel(file);
    // The fallback runner drew the effect over the red solid: yellow.
    expect(r, 'the fallback program is drawn').toBeGreaterThan(200);
    expect(g, 'the fallback program is drawn').toBeGreaterThan(200);
    expect(b).toBeLessThan(60);
  } finally {
    await session.close();
    await rm(root, { recursive: true, force: true });
  }
});
