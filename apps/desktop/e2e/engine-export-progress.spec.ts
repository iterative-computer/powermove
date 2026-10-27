import { mkdtemp, rm, stat } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { expect, test, type ElectronApplication } from '@playwright/test';

import { launchApp } from './helpers/app';

/* With Use New Engine on, an export shows its progress in the native UI from
   the click on Export until it ends: the export sheet (Preparing…, then
   frames, percent and time left), then a notice with Show in Finder. A render
   queue job, which renders without the sheet, shows in the titlebar's task
   pill while it renders and ends with the same notice. */

async function stubPanels(app: ElectronApplication, files: string[]): Promise<void> {
  await app.evaluate(({ dialog, shell }, paths) => {
    const g = globalThis as { __panels?: string[]; __revealed?: string[] };
    g.__panels = [...paths];
    g.__revealed = [];
    dialog.showSaveDialog = (async () => {
      const filePath = g.__panels!.shift();
      return filePath ? { canceled: false, filePath } : { canceled: true, filePath: '' };
    }) as typeof dialog.showSaveDialog;
    shell.showItemInFolder = (file: string) => { g.__revealed!.push(file); };
  }, files);
}
const revealed = (app: ElectronApplication) => app.evaluate(() => (globalThis as { __revealed?: string[] }).__revealed ?? []);

test('an engine export shows progress from the click to Show in Finder', async () => {
  test.setTimeout(180_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-export-progress-'));
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    const { app, page } = session;
    await session.openEditor();
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.proj = PM.mkProject({ name: 'Progress', w: 640, h: 360, fps: 30, dur: 4 });
      PM.proj.work = [0, 4];
      PM.bus.emit('project');
      PM.Edit.apply([
        { type: 'add_layer', layerType: 'solid', name: 'Red', content: { color: '#e83a5e', w: 640, h: 360 }, duration: 4 },
        { type: 'add_layer', layerType: 'text', name: 'Title', content: { text: 'Progress' }, duration: 4 }
      ], { label: 'Layers', origin: 'user' });
      PM.exportDefaults.write({ format: 'h264', scale: 1, fps: 30, range: 'all', quality: 'high', mblur: false, audio: false });
    });
    const file = path.join(root, 'Progress.mp4');
    await stubPanels(app, [file]);
    // Every value the sheet's progress bar and label take, from its first appearance.
    await page.evaluate(() => {
      const seen: { values: number[]; labels: string[] } = { values: [], labels: [] };
      (window as any).__progress = seen;
      new MutationObserver(() => {
        const sheet = document.querySelector('.export-progress');
        if (!sheet) return;
        const value = Number(sheet.querySelector('[role="progressbar"]')?.getAttribute('aria-valuenow'));
        if (seen.values.at(-1) !== value) seen.values.push(value);
        const label = sheet.querySelector('.export-progress-label')?.textContent ?? '';
        if (seen.labels.at(-1) !== label) seen.labels.push(label);
      }).observe(document.body, { subtree: true, childList: true, attributes: true, characterData: true });
    });

    // File > Export…, then the sheet's Export button.
    await app.evaluate(({ BrowserWindow, Menu }) => {
      const window = BrowserWindow.getAllWindows().find((w) => !w.isDestroyed() && w.webContents.getURL().length > 0);
      Menu.getApplicationMenu()!.getMenuItemById('export')!.click(undefined, window);
    });
    const exportSheet = page.getByRole('dialog', { name: 'Export', exact: true });
    await expect(exportSheet).toBeVisible();
    await exportSheet.locator('.mf .btn.pri').click();

    const progress = page.getByRole('dialog', { name: 'Exporting Progress' });
    await expect(progress).toBeVisible({ timeout: 5_000 });
    await expect(progress.getByRole('progressbar', { name: 'Export progress' })).toBeVisible();
    await expect(page.locator('.toast', { hasText: 'Export finished' })).toBeVisible({ timeout: 120_000 });
    await expect(progress).toBeHidden();

    const seen = await page.evaluate(() => (window as any).__progress as { values: number[]; labels: string[] });
    expect(seen.labels[0], 'the sheet opens before the job is accepted').toBe('Preparing…');
    expect(seen.labels.some((label) => /^Frame \d+ \/ 120/.test(label)), seen.labels.join(' | ')).toBe(true);
    expect(seen.values.some((value) => value > 0 && value < 100), `progress advances: ${seen.values.join(',')}`).toBe(true);
    expect((await stat(file)).size).toBeGreaterThan(1000);
    expect(await page.evaluate(() => (window as any).PM.Export.busy)).toBe(false);

    await page.locator('.toast', { hasText: 'Export finished' }).getByRole('button', { name: 'Show in Finder' }).click();
    await expect.poll(() => revealed(app)).toEqual([file]);

    // A render queue job: a task in the titlebar while it renders, then the notice.
    const queued = path.join(root, 'Queued.mp4');
    await stubPanels(app, [queued]);
    expect(await page.evaluate(() => (window as any).PM.Export.addToQueue({ format: 'h264', scale: 1, fps: 30, range: 'all', quality: 'high', mblur: false, audio: false }))).toMatchObject({ status: 'added' });
    await page.evaluate(() => (window as any).__powermoveTestBridge.nativeEngine.queueAction({ kind: 'start' }));
    await expect(page.locator('.task-pill')).toBeVisible({ timeout: 30_000 });
    await expect(page.locator('.toast', { hasText: 'Exported Queued.mp4' })).toBeVisible({ timeout: 120_000 });
    await expect(page.locator('.task-pill')).toBeHidden();
    await page.locator('.toast', { hasText: 'Exported Queued.mp4' }).getByRole('button', { name: 'Show in Finder' }).click();
    await expect.poll(() => revealed(app)).toEqual([queued]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
    await rm(root, { recursive: true, force: true });
  }
});
