import { expect, test } from '@playwright/test';
import { launchApp } from './helpers/app';

/* scene3d.cancelRender keeps today's contract with the engine on: it stops
 * the window's Blender worker, so the Model Inspector's Cancel ends a
 * running Regenerate or .blend import, and it resolves. Run with a built
 * addon and Blender 4.5+: --workers=1. */
test('native scene3d cancelRender stops a running model generation and resolves', async () => {
  test.setTimeout(180_000);
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await session.openEditor();
    const { page, app } = session;
    const blender = await page.evaluate(() => (window as any).PM.Kernel.api('native-cancel-model').scene3d.blenderStatus());
    test.skip(!blender.available, 'Install Blender 4.5 or newer for the model cancel check');
    await app.evaluate(({ ipcMain }) => {
      const ipc = ipcMain as any, original = ipc._invokeHandlers.get('blender:run');
      const g = globalThis as any;
      g.__modelJobs = { started: 0, settled: [] as string[] };
      ipc.removeHandler('blender:run');
      ipc.handle('blender:run', async (event: unknown, job: any) => {
        if (job.operation !== 'generate') return original(event, job);
        g.__modelJobs.started++;
        try { const out = await original(event, job); g.__modelJobs.settled.push('done'); return out; }
        catch (error) { g.__modelJobs.settled.push('failed'); throw error; }
      });
    });
    await page.evaluate(() => {
      const PM = (window as any).PM, api = PM.Kernel.api('native-cancel-model');
      PM.replaceProject(PM.mkProject({ name: 'Native model cancel', w: 192, h: 108, fps: 24, dur: 1 }));
      const camera = api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 0.6, z: 5 } } });
      if (!camera.ok) throw Error(camera.message);
    });
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection?.status().route), { timeout: 45_000 }).toBe('engine');
    const before = await page.evaluate(() => JSON.stringify((window as any).PM.proj.comps));
    await page.evaluate(() => {
      const api = (window as any).PM.Kernel.api('native-cancel-model');
      const w = window as any;
      w.__model = api.scene3d.model({ operation: 'create_model', recipe: { kind: 'rocket', parameters: {}, modifiers: [] } });
      w.__model.then((r: unknown) => { w.__modelSettled = r; }, (e: Error) => { w.__modelSettled = { thrown: String(e) }; });
    });
    // The job reaches main's worker (or, if it never does, say why).
    await expect.poll(async () => {
      const started = await app.evaluate(() => (globalThis as any).__modelJobs.started);
      const settled = await page.evaluate(() => (window as any).__modelSettled ?? null);
      return started === 1 ? 'started' : settled ? JSON.stringify(settled) : 'waiting';
    }, { timeout: 30_000 }).toBe('started');
    const cancelled = await page.evaluate(async () => {
      const PM = (window as any).PM, api = PM.Kernel.api('native-cancel-model');
      const cancel = await api.scene3d.cancelRender().then(() => 'resolved', (error: Error) => `${error.name}: ${error.message}`);
      return { cancel, model: await (window as any).__model, comps: JSON.stringify(PM.proj.comps) };
    });
    expect(cancelled.cancel).toBe('resolved');
    expect(cancelled.model.ok).toBe(false);
    expect(cancelled.comps).toBe(before);
    expect(await app.evaluate(() => (globalThis as any).__modelJobs.settled)).toEqual(['failed']);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});
