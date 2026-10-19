import { expect, test } from '@playwright/test';
import { launchApp } from './helpers/app';

/* The scene3d public preview controls must follow the native viewer's
 * per-session Blender job and the frame it actually presented. Run with a
 * built addon and Blender 4.5+: --workers=1. */
test('native scene3d preview API renders, reports, and cancels the engine viewer job', async () => {
  test.setTimeout(180_000);
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await session.openEditor();
    const { page, app } = session;
    const blender = await page.evaluate(() => (window as any).PM.Kernel.api('native-preview-api').scene3d.blenderStatus());
    test.skip(!blender.available, 'Install Blender 4.5 or newer for the native preview API check');
    await app.evaluate(({ ipcMain }) => {
      const ipc = ipcMain as any, original = ipc._invokeHandlers.get('blender:run');
      (globalThis as any).__legacyPreviewJobs = 0;
      ipc.removeHandler('blender:run');
      ipc.handle('blender:run', async (event: unknown, job: any) => {
        if (job.operation === 'render') (globalThis as any).__legacyPreviewJobs++;
        return original(event, job);
      });
    });
    await page.evaluate(() => {
      const PM = (window as any).PM, api = PM.Kernel.api('native-preview-api');
      PM.replaceProject(PM.mkProject({ name: 'Native preview API', w: 192, h: 108, fps: 24, dur: 1 }));
      const camera = api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 0.6, z: 5 } } });
      const object = api.scene3d.edit({ operation: 'add_object', object: { id: 'preview_box', source: { primitive: 'box' }, material: { p: { color: '#FF7733' } } } });
      if (!camera.ok || !object.ok) throw Error(camera.message || object.message);
      const settings = api.scene3d.setRendering({ enabled: true, engine: 'eevee', samples: 2, previewSamples: 2, previewScale: 1, device: 'cpu' });
      if (!settings.ok) throw Error(settings.message);
      PM.Edit.apply({ type: 'replace_keyframes', target: 'preview_box', path: 'rotation.y', keyframes: [{ time: 0, value: 0 }, { time: 0.5, value: 45 }] });
      PM.setTime(0);
      PM.Projects?.put?.(PM.proj);
      (window as any).__previewStates = [];
      (window as any).__offPreview = api.scene3d.onPreviewChange((state: unknown) => (window as any).__previewStates.push(state));
    });
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection?.status().route), { timeout: 45_000 }).toBe('engine');
    await page.evaluate(() => (window as any).PM.Kernel.api('native-preview-api').scene3d.setPreviewMode('rendered'));
    await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.api('native-preview-api').scene3d.previewState().busy), { timeout: 30_000 }).toBe(true);
    const result = await page.evaluate(async () => {
      const PM = (window as any).PM, api = PM.Kernel.api('native-preview-api');
      const before = PM.EngineConnection.session()?.lastFrame?.seq ?? 0;
      await api.scene3d.renderFrame();
      const frame = PM.EngineConnection.session()?.lastFrame;
      return { before, after: frame?.seq ?? 0, pts: frame?.pts, state: api.scene3d.previewState(),
        transitions: (window as any).__previewStates.map((s: any) => ({ busy: s.busy, error: s.error })) };
    });
    expect(result.after).toBeGreaterThan(result.before);
    expect(result.pts).toBe(0);
    expect(result.state).toMatchObject({ mode: 'rendered', busy: false, error: null });
    expect(result.transitions.some((s: any) => s.busy)).toBe(true);
    expect(result.transitions.at(-1)).toMatchObject({ busy: false, error: null });

    const cancelled = await page.evaluate(async () => {
      const PM = (window as any).PM, api = PM.Kernel.api('native-preview-api');
      PM.setTime(0.5);
      const render = api.scene3d.renderFrame().then(() => 'ready', (error: Error) => error.name);
      await api.scene3d.cancelRender();
      return { result: await render, state: api.scene3d.previewState() };
    });
    expect(cancelled.result).toBe('EnginePreviewCancelled');
    expect(cancelled.state).toMatchObject({ mode: 'draft', busy: false, error: null });

    const beforeOrbit = await page.evaluate(() => {
      const PM = (window as any).PM;
      const presentation = PM.Kernel.services.get('viewer').enginePresentation;
      return { seq: presentation.latest?.seq ?? 0, viewSeq: presentation.state?.viewSeq ?? 0 };
    });
    const jobsBeforeOrbit = await app.evaluate(() =>
      (globalThis as any).__powermoveNativeEngine.engine.blender.completedRenders().length,
    );
    const orbit = await page.evaluate(() => {
      const PM = (window as any).PM;
      const viewport = PM.Kernel.api('native-preview-api').scene3d.viewport;
      PM.setTime(0);
      if (!viewport.run('view.orbit', 55, -20)) throw Error('Editor orbit was rejected');
      const state = viewport.state();
      return { mode: state.view.mode, camera: state.engineCamera };
    });
    expect(orbit.mode).toBe('editor');
    expect(orbit.camera).not.toBeNull();
    await expect.poll(() => page.evaluate(() =>
      (window as any).PM.Kernel.services.get('viewer').enginePresentation?.state?.viewSeq ?? 0,
    ), { timeout: 30_000 }).toBeGreaterThan(beforeOrbit.viewSeq);

    const restarted = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('native-preview-api');
      api.scene3d.setPreviewMode('rendered');
      await api.scene3d.renderFrame();
      const presentation = PM.Kernel.services.get('viewer').enginePresentation;
      const frame = presentation.latest;
      const snapshot = PM.EngineRender.snapshotSync({
        time: 0, width: 192, height: 108, pixels: 'premultiplied',
        background: false, motionBlurSamples: 0, draft3d: false,
      });
      return {
        frame: frame && { seq: frame.seq, viewSeq: frame.viewSeq, revision: frame.revision, pts: frame.pts },
        state: presentation.state && { viewSeq: presentation.state.viewSeq, camera: presentation.state.view?.camera },
        revision: PM.EngineDoc.barrier.revision,
        preview: api.scene3d.previewState(),
        pixels: snapshot.data.some((v: number, i: number) => i % 4 !== 3 && v > 8 && snapshot.data[i + (3 - i % 4)] > 0),
      };
    });
    expect(restarted.frame?.seq).toBeGreaterThan(beforeOrbit.seq);
    expect(restarted.frame?.pts).toBe(0);
    expect(restarted.frame?.viewSeq).toBe(restarted.state?.viewSeq);
    expect(restarted.frame?.revision).toBeGreaterThanOrEqual(restarted.revision);
    expect(restarted.state?.camera).toEqual(orbit.camera);
    expect(restarted.preview).toMatchObject({ mode: 'rendered', busy: false, error: null });
    expect(restarted.pixels).toBe(true);
    expect(await app.evaluate(() =>
      (globalThis as any).__powermoveNativeEngine.engine.blender.completedRenders().length,
    )).toBeGreaterThan(jobsBeforeOrbit);
    expect(await app.evaluate(() => (globalThis as any).__legacyPreviewJobs)).toBe(0);
    await page.evaluate(() => (window as any).__offPreview?.dispose());
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});
