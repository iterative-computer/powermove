import os from 'node:os';
import path from 'node:path';

import { expect, test } from '@playwright/test';
import { launchApp } from './helpers/app';

test('export settings and save panels keep an on-demand job utility warm until dismissed', async () => {
  const active = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1', POWERMOVE_ENGINE_JOB_IDLE_MS: '1000' } });
  const status = () => active.app.evaluate(() => {
    const engine = (globalThis as any).__powermoveNativeEngine.engine;
    return engine.status().job as { state: string; generation: number };
  });
  try {
    await active.openEditor();
    expect((await status()).state).toBe('stopped');
    await active.page.evaluate(() => (window as any).PM.Export.dialog());
    await expect.poll(async () => (await status()).state).toBe('ready');
    const sheetLaunch = (await status()).generation;
    await active.page.waitForTimeout(1500); // Past the diagnostic idle deadline.
    expect(await status()).toMatchObject({ state: 'ready', generation: sheetLaunch });
    await active.page.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect.poll(async () => (await status()).state).toBe('stopped');

    // Direct export callers also hold the save panel without a settings sheet,
    // then the preflight after it (the destination is asked first).
    // Substitution stays in the hidden host; no native dialog reaches the user.
    await active.app.evaluate(({ dialog }, filePath) => {
      dialog.showSaveDialog = () => new Promise(resolve => {
        (globalThis as any).__answerWarmthPanel = () => resolve({ canceled: false, filePath });
      });
    }, path.join(os.tmpdir(), 'powermove-warmth-never-written.mp4'));
    await active.page.evaluate(() => {
      const PM = (window as any).PM;
      const flush = PM.HostMedia.flushBindings.bind(PM.HostMedia);
      PM.HostMedia.flushBindings = async (project: unknown) => {
        await new Promise<void>(resolve => { (window as any).__finishExportPreflight = resolve; });
        return flush(project);
      };
      (window as any).__warmthExport = (window as any).PM.Export.run({
        format: 'h264', scale: 1, fps: 24, range: 'all', quality: 'high', audio: false, mblur: false
      });
    });
    await expect.poll(() => active.app.evaluate(() => typeof (globalThis as any).__answerWarmthPanel)).toBe('function');
    await expect.poll(async () => (await status()).state).toBe('ready');
    const panelLaunch = (await status()).generation;
    expect(panelLaunch).toBeGreaterThan(sheetLaunch);
    await active.page.waitForTimeout(1500);
    expect(await status()).toMatchObject({ state: 'ready', generation: panelLaunch });
    await active.app.evaluate(() => (globalThis as any).__answerWarmthPanel());
    await active.page.waitForFunction(() => typeof (window as any).__finishExportPreflight === 'function');
    await active.page.waitForTimeout(1500);
    expect(await status()).toMatchObject({ state: 'ready', generation: panelLaunch });
    // Cancelled from the progress sheet before the job starts.
    await active.page.getByRole('button', { name: 'Cancel', exact: true }).click();
    await active.page.evaluate(() => (window as any).__finishExportPreflight());
    expect(await active.page.evaluate(() => (window as any).__warmthExport)).toEqual({ cancelled: true });
    await expect.poll(async () => (await status()).state).toBe('stopped');
  } finally { await active.close(); }
});
