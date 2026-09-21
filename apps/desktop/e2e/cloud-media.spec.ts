import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

test.describe.configure({ timeout: 120_000 });

test('cloud media can be deferred, downloaded in place, and automatically restored', async ({ session }) => {
  await session.openEditor();
  const { page, app } = session;
  await importFixture(page, 'tone.wav');
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((layer: any) => layer.name === 'tone.wav'));
  // Simulate only provider metadata and the user's native-sheet choice. The
  // download, chunk transport, decoding and durable restoration are real.
  await app.evaluate(({ ipcMain, dialog }) => {
    (globalThis as any).cloudPrompts = [];
    ipcMain.removeHandler('cloud:status');
    ipcMain.handle('cloud:status', (_event, paths: string[]) => Object.fromEntries(paths.map(source => [source, 'icloud'])));
    ipcMain.removeHandler('media:open-local-source');
    ipcMain.handle('media:open-local-source', () => null);
    dialog.showMessageBox = (async (_window: any, options: any) => {
      (globalThis as any).cloudPrompts.push(options);
      return { response: 1, checkboxChecked: true };
    }) as any;
  });
  const before = await page.evaluate(async () => {
    const PM = (window as any).PM;
    const meta = Object.values(PM.proj.assets).find((asset: any) => asset.name === 'tone.wav') as any;
    await PM.MediaStore.remove(meta);
    PM.assets.clear();
    await PM.assets.restoreProject(PM.proj);
    return { id: meta.id, layers: JSON.stringify(PM.proj.layers) };
  });
  const card = page.locator(`[data-asset-id="${before.id}"]`);
  await expect(card).toHaveClass(/is-cloud/);
  await expect(card).toContainText('In cloud');
  await expect(card).toContainText('Stored in iCloud');
  await expect(card.getByRole('button', { name: 'Download tone.wav from cloud' })).toBeVisible();
  await card.screenshot({ path: test.info().outputPath('cloud-media-card.png') });
  expect(await app.evaluate(() => (globalThis as any).cloudPrompts)).toMatchObject([
    { buttons: ['Download files', 'Not now'], checkboxChecked: false, checkboxLabel: 'Automatically download cloud media when opening projects' },
  ]);
  await card.getByRole('button', { name: 'Download tone.wav from cloud' }).click();
  await expect(card).not.toHaveClass(/is-offline/);
  expect(await page.evaluate(async id => {
    const PM = (window as any).PM;
    return { layers: JSON.stringify(PM.proj.layers), cached: !!await PM.MediaStore.get(PM.proj.assets[id]), automatic: PM.store.get('autoDownloadCloudMedia') };
  }, before.id)).toEqual({ layers: before.layers, cached: true, automatic: true });
  await page.evaluate(async id => {
    const PM = (window as any).PM;
    await PM.MediaStore.remove(PM.proj.assets[id]);
    PM.assets.clear();
    await PM.assets.restoreProject(PM.proj);
  }, before.id);
  await expect(card).not.toHaveClass(/is-offline/);
  expect(await app.evaluate(() => (globalThis as any).cloudPrompts.length)).toBe(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
