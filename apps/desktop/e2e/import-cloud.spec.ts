import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { expect, test } from './helpers/app';

test('cloud imports show download state, use fresh bytes, and clear failed tasks', async ({ session }) => {
  await session.openEditor();
  const { page, app } = session;
  const directory = await mkdtemp(path.join(tmpdir(), 'powermove-cloud-import-'));
  const source = path.join(directory, 'Cloud artwork.png');
  await writeFile(source, 'stale placeholder');
  try {
    const bytes = await page.evaluate(async () => {
      const canvas = document.createElement('canvas'); canvas.width = canvas.height = 16;
      const blob = await new Promise<Blob>(resolve => canvas.toBlob(value => resolve(value!)));
      return Array.from(new Uint8Array(await blob.arrayBuffer()));
    });
    await app.evaluate(({ ipcMain }, bytes) => {
      for (const channel of ['cloud:status', 'cloud:download', 'cloud:read', 'cloud:release']) ipcMain.removeHandler(channel);
      let release!: () => void;
      const gate = new Promise<void>(resolve => { release = resolve; });
      (globalThis as any).releaseCloudImport = release;
      (globalThis as any).cloudImportFailure = false;
      (globalThis as any).cloudImportReleases = 0;
      ipcMain.handle('cloud:status', (_event, paths: string[]) => Object.fromEntries(paths.map(source => [source, 'icloud'])));
      ipcMain.handle('cloud:download', async () => {
        await gate;
        if ((globalThis as any).cloudImportFailure) throw new Error('Cloud download failed. Check your connection and try again.');
        return { token: 'cloud-test', size: bytes.length };
      });
      ipcMain.handle('cloud:read', (_event, { offset, length }) => new Uint8Array(bytes.slice(offset, offset + length)));
      ipcMain.handle('cloud:release', () => { (globalThis as any).cloudImportReleases++; });
    }, bytes);
    await page.evaluate(() => {
      const input = document.createElement('input'); input.type = 'file'; input.id = 'cloud-import-test';
      input.onchange = () => { (window as any).pendingCloudImport = (window as any).PM.importFiles(Array.from(input.files!)); };
      document.body.append(input);
    });
    await page.locator('#cloud-import-test').setInputFiles(source);
    const task = page.getByRole('button', { name: 'Downloading from iCloud… · Cloud artwork.png', exact: true });
    await expect(task).toBeVisible();
    await task.click();
    const tasks = page.getByRole('group', { name: 'Running tasks' });
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
      await expect(tasks).toContainText('Downloading from iCloud…');
      await tasks.screenshot({ path: test.info().outputPath(`cloud-download-${theme}.png`) });
    }
    await app.evaluate(() => (globalThis as any).releaseCloudImport());
    await page.evaluate(() => (window as any).pendingCloudImport);
    await expect(task).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).PM.proj.layers.some((layer: any) => layer.name === 'Cloud artwork.png'))).toBe(true);
    expect(await app.evaluate(() => (globalThis as any).cloudImportReleases)).toBe(1);
    await app.evaluate(() => { (globalThis as any).cloudImportFailure = true; });
    await page.locator('#cloud-import-test').setInputFiles([]);
    await page.locator('#cloud-import-test').setInputFiles(source);
    await page.evaluate(() => (window as any).pendingCloudImport);
    await expect(task).toHaveCount(0);
    await expect(page.locator('[data-toast-error]')).toContainText('Cloud download failed');
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await rm(directory, { recursive: true, force: true }); }
});
