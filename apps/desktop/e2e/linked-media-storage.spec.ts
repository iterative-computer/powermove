import { copyFile, mkdtemp, readFile, readdir, rename, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { fixturePath } from './helpers/media';
import { decodeProjectContainer } from '../src/shared/project-container';

test('links originals through import, save, restart, missing-source recovery and relinking; exports remain portable', async ({ session }) => {
  test.setTimeout(120_000);
  const directory = await mkdtemp(path.join(os.tmpdir(), 'powermove-linked-original-'));
  const original = path.join(directory, 'Original.mp4'), moved = path.join(directory, 'Moved.mp4');
  try {
    await copyFile(fixturePath('h264-aac.mp4'), original);
    await session.openEditor();
    const chooser = session.page.waitForEvent('filechooser');
    await session.page.evaluate(() => (window as any).PM.pickFiles());
    await (await chooser).setFiles(original);
    await session.page.waitForFunction(() => (window as any).PM.proj.layers.some((layer: any) => layer.name === 'Original.mp4'));
    const asset = await session.page.evaluate(() => {
      const PM = (window as any).PM;
      return Object.values<any>(PM.proj.assets).find(item => item.name === 'Original.mp4');
    });
    expect(asset.sourcePath).toBe(original);
    const indexRoot = path.join(session.userData, 'Media Store', 'Index');
    const indexes = await Promise.all((await readdir(indexRoot)).map(async file => JSON.parse(await readFile(path.join(indexRoot, file), 'utf8'))));
    expect(indexes.find(item => item.key === asset.storageKey)).toMatchObject({ sourcePath: original });
    expect((await readdir(path.join(session.userData, 'Media Store'))).filter(file => file.endsWith('.mp4'))).toEqual([]);

    const destination = path.join(directory, 'Linked.pmv');
    await session.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, destination);
    expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
    const saved = decodeProjectContainer(await readFile(destination));
    expect(saved.media).toEqual([]);
    expect(saved.document.proj.assets[asset.id].sourcePath).toBe(original);
    await session.page.evaluate(() => { const PM = (window as any).PM; PM.Edit.apply({ type: 'set_layer', target: PM.proj.layers[0].id, patch: { name: 'Recovered edit' } }); });
    await session.relaunch();
    await session.openEditor();
    await session.page.waitForFunction(id => Boolean((window as any).PM.assets.get(id)?.el), asset.id);
    expect(await session.page.evaluate(() => (window as any).PM.proj.layers[0].name)).toBe('Recovered edit');
    expect((await readdir(path.join(session.userData, 'Media Store'))).filter(file => file.endsWith('.mp4'))).toEqual([]);

    await rename(original, moved);
    await session.relaunch();
    await session.openEditor();
    await session.page.waitForFunction(id => !(window as any).PM.assets.loading.has(id), asset.id);
    expect(await session.page.evaluate(id => Boolean((window as any).PM.assets.get(id)), asset.id)).toBe(false);
    // Missing originals do not prevent saving unrelated edits.
    expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
    const relink = session.page.waitForEvent('filechooser');
    await session.page.evaluate(id => (window as any).PM.pickFiles(false, { replaceAssetId: id }), asset.id);
    await (await relink).setFiles(moved);
    await session.page.waitForFunction(id => (window as any).PM.proj.assets[id]?.sourcePath?.endsWith('/Moved.mp4'), asset.id);
    expect(await session.page.evaluate(id => Boolean((window as any).PM.assets.get(id)?.el), asset.id)).toBe(true);
    expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
    // bf26d9b3e adds automatic file saves and retained save history. Older
    // versions must stay metadata-only too; linked originals are never copied.
    const backupRoot = path.join(session.userData, 'backups', saved.document.proj.id);
    const versions = await Promise.all((await readdir(backupRoot)).map(async file => decodeProjectContainer(await readFile(path.join(backupRoot, file)))));
    expect(versions).toHaveLength(2);
    for (const version of versions) {
      expect(version.media).toEqual([]);
      expect(version.document.proj.assets[asset.id].sourcePath).toBe(original);
    }
    expect(versions.map(version => version.document.proj.layers[0].name).sort()).toEqual(['Original.mp4', 'Recovered edit']);

    const portable = path.join(directory, 'Portable.pmv');
    await session.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, portable);
    expect(await session.page.evaluate(() => (window as any).PM.Export.run({ format: 'json' }))).toMatchObject({ cancelled: false });
    const packed = decodeProjectContainer(await readFile(portable));
    expect(Buffer.from(packed.media.find(item => item.id === asset.id)!.data)).toEqual(await readFile(moved));
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    // Finish normal guarded saves while this linked-source fixture still exists.
    try { await session.close(); }
    finally { await rm(directory, { recursive: true, force: true }); }
  }
});
