import { writeFile, unlink, readFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { decodeProjectContainer } from '../src/shared/project-container';

for (const fps of [12, 120]) test(`numbered images at ${fps} fps become one transparent clip that exports and survives a portable save`, async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const { page } = session;
  const images = await page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ w: 64, h: 64, fps: 30, dur: 2, name: 'Sequence test' }) }));
    PM.ProjectsScreen.hide(); PM.setTime(0, { raw: true, force: true });
    return ['red', 'lime', 'blue'].map(color => {
      const canvas = document.createElement('canvas'); canvas.width = canvas.height = 64;
      const ctx = canvas.getContext('2d')!;
      ctx.fillStyle = color; ctx.fillRect(16, 16, 32, 32);
      return canvas.toDataURL('image/png').split(',')[1]!;
    });
  });
  const paths = ['frame_009.png', 'frame_10.png', 'frame_11.png'].map(name => path.join(session.userData, name));
  for (let i = 0; i < paths.length; i++) await writeFile(paths[i]!, Buffer.from(images[i]!, 'base64'));
  const chooser = page.waitForEvent('filechooser');
  await page.evaluate(() => (window as any).PM.pickFiles());
  await (await chooser).setFiles([paths[2]!, paths[0]!, paths[1]!]);
  const dialog = page.getByRole('dialog', { name: 'Import image sequence', exact: true });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('spinbutton', { name: 'Sequence frame rate' }).fill(String(fps));
  await expect(dialog).toContainText(`3 frames · ${(3 / fps).toFixed(3)} seconds`);
  await dialog.getByRole('button', { name: 'Import sequence', exact: true }).click();
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.name === 'frame sequence.webm'));
  const imported = await page.evaluate(() => {
    const PM = (window as any).PM, layer = PM.proj.layers[0];
    return { count: PM.proj.layers.length, type: layer.type, dur: layer.dur, from: layer.from, assets: Object.keys(PM.proj.assets).length, sourceDur: PM.assets.get(layer.d.asset).dur };
  });
  expect(imported).toMatchObject({ count: 1, type: 'video', from: 0, assets: 1 });
  expect(imported.sourceDur).toBeCloseTo(3 / fps, 3);
  expect(imported.dur).toBeCloseTo(Math.max(1 / 30, 3 / fps), 3);

  async function exportPixels(time: number) {
    return session.page.evaluate(async time => {
      const PM = (window as any).PM;
      PM.setTime(time, { raw: true, force: true });
      let pixels: number[][] = [];
      const original = PM.download;
      PM.download = async (blob: Blob) => {
        const image = await createImageBitmap(blob);
        const cv = document.createElement('canvas'); cv.width = cv.height = 64;
        const ctx = cv.getContext('2d')!; ctx.drawImage(image, 0, 0); image.close();
        pixels = [[...ctx.getImageData(32, 32, 1, 1).data], [...ctx.getImageData(0, 0, 1, 1).data]];
      };
      try {
        const result = await PM.Export.run({ format: 'still', w: 64, h: 64, alpha: true, mblur: false });
        if (result.error) throw new Error(result.error);
        return pixels;
      } finally { PM.download = original; }
    }, time);
  }
  for (let frame = 0; frame < 3; frame++) {
    const pixels = await exportPixels(frame / fps);
    expect(pixels[0]![frame]).toBeGreaterThan(240);
    expect(pixels[0]![(frame + 1) % 3]).toBeLessThan(15);
    expect(pixels[1]![3]).toBe(0);
  }
  await page.evaluate(() => (window as any).PM.cmd('undo'));
  expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(0);
  await page.evaluate(() => (window as any).PM.cmd('redo'));
  expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(1);
  const destination = path.join(session.userData, 'Sequence.pmv');
  await session.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, destination);
  expect(await page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  expect(decodeProjectContainer(await readFile(destination)).media).toHaveLength(1);
  for (const file of paths) await unlink(file);
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    for (const asset of Object.values(PM.proj.assets)) await PM.MediaStore.remove(asset);
  });
  await session.relaunch();
  await session.app.evaluate(({ dialog }, filePath) => { dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] }); }, destination);
  await session.page.evaluate(() => (window as any).PM.openProject());
  await session.page.waitForFunction(() => [...(window as any).PM.assets.map.values()].some((a: any) => a.name === 'frame sequence.webm'));
  const restored = await exportPixels(1 / fps);
  expect(restored[0]![1]).toBeGreaterThan(240);
  expect(restored[1]![3]).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('sequence import validates gaps and permits importing individual stills or cancelling', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const canvas = document.createElement('canvas'); canvas.width = canvas.height = 16;
    canvas.toBlob(blob => { void PM.importFiles([new File([blob!], 'f1.png'), new File([blob!], 'f3.png')]); });
  });
  const dialog = page.getByRole('dialog', { name: 'Import image sequence', exact: true });
  await dialog.getByRole('button', { name: 'Import sequence', exact: true }).click();
  await expect(dialog).toContainText('Missing frame 2');
  await dialog.getByRole('button', { name: 'Individual images', exact: true }).click();
  await page.waitForFunction(() => (window as any).PM.proj.layers.filter((l: any) => l.type === 'image').length === 2);
  await page.evaluate(() => { void (window as any).PM.importFiles([new File([], 'a1.png'), new File([], 'a2.png')]); });
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await page.evaluate(() => (window as any).PM.flushProject());
  expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(2);
});
