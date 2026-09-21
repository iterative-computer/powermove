import { expect, test } from './helpers/app';
import { fixturePath, importFixture } from './helpers/media';
import { copyFile, unlink } from 'node:fs/promises';
import path from 'node:path';

for (const fixture of ['tone.wav', 'tone.mp3', 'h264-aac.mp4', 'converted.mkv', 'still.heic', 'animated.gif']) {
test(`restores ${fixture} from its local source when cached bytes are missing`, async ({ session }) => {
  test.setTimeout(60_000);
  await session.openEditor();
  const { page } = session;
  await importFixture(page, fixture);
  await page.waitForFunction(name => Object.values((window as any).PM.proj.assets).some((asset: any) => asset.name === name), fixture);
  const result = await page.evaluate(async name => {
    const PM = (window as any).PM;
    const meta = Object.values(PM.proj.assets).find((asset: any) => asset.name === name) as any;
    const layers = JSON.stringify(PM.proj.layers);
    const sourceState = await (window as any).powermove.media.cloudStatus([meta.sourcePath]);
    await PM.MediaStore.remove(meta);
    PM.assets.clear();
    const restored = await PM.assets.restoreProject(PM.proj);
    return {
      id: meta.id, local: sourceState[meta.sourcePath],
      restored: restored.restored.length, missing: restored.missing.length,
      layersUnchanged: JSON.stringify(PM.proj.layers) === layers,
      cached: !!await PM.MediaStore.get(meta),
      playable: !!PM.assets.get(meta.id),
    };
  }, fixture);
  expect(result.local).toBe('local');
  expect(result).toMatchObject({ restored: 1, missing: 0, layersUnchanged: true, cached: true, playable: true });
  const card = page.locator(`[data-asset-id="${result.id}"]`);
  await expect(card).not.toHaveClass(/is-offline/);
  await expect(card).not.toContainText('Media offline');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
}

test('distinguishes changed source files from missing files and clears errors after recovery', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const source = path.join(session.userData, 'local.wav');
  await copyFile(fixturePath('tone.wav'), source);
  const chooser = page.waitForEvent('filechooser');
  await page.evaluate(() => (window as any).PM.pickFiles());
  await (await chooser).setFiles(source);
  await page.waitForFunction(() => Object.values((window as any).PM.proj.assets).some((asset: any) => asset.name === 'local.wav'));
  const id = await page.evaluate(() => (Object.values((window as any).PM.proj.assets).find((asset: any) => asset.name === 'local.wav') as any).id);
  const restore = () => page.evaluate(async id => {
    const PM = (window as any).PM;
    await PM.MediaStore.remove(PM.proj.assets[id]);
    PM.assets.clear();
    await PM.assets.restoreProject(PM.proj);
    return { ready: !!PM.assets.get(id), error: PM.assets.errors.get(id) || null };
  }, id);
  const card = page.locator(`[data-asset-id="${id}"]`);

  await copyFile(fixturePath('tone.mp3'), source);
  expect(await restore()).toMatchObject({ ready: false, error: expect.stringContaining('source file has changed') });
  await expect(card).toContainText('Media unavailable');
  await expect(card).toContainText('The source file has changed');
  await expect(card).not.toContainText('Missing ·');

  await unlink(source);
  expect(await restore()).toEqual({ ready: false, error: null });
  await expect(card).toContainText('Media offline');
  await expect(card).toContainText('Missing ·');

  await copyFile(fixturePath('tone.wav'), source);
  expect(await restore()).toEqual({ ready: true, error: null });
  await expect(card).not.toHaveClass(/is-offline/);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
