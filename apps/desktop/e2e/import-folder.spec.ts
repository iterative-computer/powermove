import { copyFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { fixturePath } from './helpers/media';

for (const cloud of [false, true]) test(`a native ${cloud ? 'cloud' : 'local'} folder drop imports nested media without a file read warning`, async ({ session }) => {
  await session.openEditor();
  const folder = path.join(session.userData, 'Dropped media');
  await mkdir(path.join(folder, 'Nested'), { recursive: true });
  await copyFile(fixturePath('still-red.png'), path.join(folder, 'Red.png'));
  await copyFile(fixturePath('still-alpha.png'), path.join(folder, 'Nested', 'Nested.png'));
  if (cloud) await session.app.evaluate(({ ipcMain }, folder) => {
    (globalThis as any).folderDownloads = [];
    const downloaded = new Set<string>();
    const download = (ipcMain as any)._invokeHandlers.get('cloud:download');
    ipcMain.removeHandler('cloud:status');
    ipcMain.handle('cloud:status', (_event, paths: string[]) => Object.fromEntries(paths.map(source => [source, source.startsWith(folder) && !downloaded.has(source) ? 'icloud' : 'local'])));
    ipcMain.removeHandler('cloud:download');
    ipcMain.handle('cloud:download', async (event, source: string) => {
      (globalThis as any).folderDownloads.push(source);
      // bf26d9b3e reads downloaded originals too. Keep real source bytes/handles.
      const result = await download(event, source);
      downloaded.add(source);
      return result;
    });
  }, folder);
  // 4f5cf623d renamed Media to Project and put it in a shared tab stack.
  await session.page.evaluate(() => (window as any).PM.Layout.activateTab('assets'));
  const panel = session.page.locator('#panel-assets');
  const box = await panel.boundingBox();
  expect(box).toBeTruthy();
  const cdp = await session.page.context().newCDPSession(session.page);
  try {
    for (const type of ['dragEnter', 'dragOver', 'drop']) {
      await cdp.send('Input.dispatchDragEvent', {
        type, x: box!.x + box!.width / 2, y: box!.y + box!.height / 2,
        data: { items: [], files: [folder], dragOperationsMask: 1 },
      });
    }
    await session.page.evaluate(async () => { await (window as any).PM.app.importQueue; });
    const names = await session.page.evaluate(() => Object.values((window as any).PM.proj.assets).map((a: any) => a.name).sort());
    expect(names).toEqual(['Nested.png', 'Red.png']);
    if (cloud) expect(await session.app.evaluate(() => (globalThis as any).folderDownloads)).toEqual([
      path.join(folder, 'Nested', 'Nested.png'), path.join(folder, 'Red.png'),
    ]);
    await expect(session.page.locator('[data-toast-error="true"]')).toHaveCount(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await cdp.detach(); }
});
