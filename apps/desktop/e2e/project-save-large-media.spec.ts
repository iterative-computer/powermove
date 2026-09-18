import { readdir, stat, readFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { decodeProjectContainer } from '../src/shared/project-container';

async function newestBackup(userData: string): Promise<string> {
  const root = path.join(userData, 'backups');
  const files: string[] = [];
  for (const id of await readdir(root)) for (const name of await readdir(path.join(root, id))) files.push(path.join(root, id, name));
  files.sort((a, b) => path.basename(b).localeCompare(path.basename(a)));
  if (!files[0]) throw new Error('No project backup was written');
  return files[0];
}

test('saves, backs up and reopens a portable project larger than 256 MiB', async ({ session }) => {
  test.setTimeout(120000);
  await session.openEditor();
  const destination = path.join(session.userData, 'Large media.pmv');
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
  }, destination);
  const imported = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    const cv = document.createElement('canvas'); cv.width = cv.height = 16;
    const ctx = cv.getContext('2d')!; ctx.fillStyle = '#00ff00'; ctx.fillRect(0, 0, 16, 16);
    const png = await new Promise<Blob>(r => cv.toBlob(b => r(b!)));
    // PNG permits trailing data. Use a valid small picture with a large source
    // payload to exercise real storage and IPC without committing huge fixtures.
    const file = new File([png, ...Array(257).fill(new Uint8Array(1024 * 1024))], 'large.png', { type: 'image/png' });
    await PM.importFiles([file]); await PM.app.importQueue;
    const asset = Object.values(PM.proj.assets)[0] as any;
    return { id: asset.id, bytes: file.size };
  });
  expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  expect((await stat(destination)).size).toBeGreaterThan(256 * 1024 * 1024);
  const first = decodeProjectContainer(await readFile(destination));
  expect(first.media[0]!.data.length).toBe(imported.bytes);
  await session.page.evaluate(() => {
    const PM = (window as any).PM; PM.proj.name = 'Updated large media'; PM.bus.emit('project');
  });
  expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  expect(decodeProjectContainer(await readFile(await newestBackup(session.userData))).document.proj.name).toBe(first.document.proj.name);
  await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    for (const asset of Object.values(PM.proj.assets)) await PM.MediaStore.remove(asset);
    await PM.openProject();
  });
  const reopened = await session.page.evaluate(async id => {
    const PM = (window as any).PM;
    return { name: PM.proj.name, bytes: (await PM.MediaStore.get(PM.proj.assets[id]))?.size, dirty: PM.app.dirty };
  }, imported.id);
  expect(reopened).toEqual({ name: 'Updated large media', bytes: imported.bytes, dirty: false });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
