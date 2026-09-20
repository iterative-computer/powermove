import { exportStillPixels } from './helpers/media';
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
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ w: 2048, h: 2048, fps: 30, dur: 2, name: 'Sequence test' }) }));
    PM.ProjectsScreen.hide(); PM.setTime(0, { raw: true, force: true });
    return ['red', 'lime', 'blue'].map(color => {
      const canvas = document.createElement('canvas'); canvas.width = canvas.height = 2048;
      const ctx = canvas.getContext('2d')!;
      ctx.fillStyle = color; ctx.fillRect(512, 512, 1024, 1024);
      ctx.globalAlpha = .5; ctx.fillRect(64, 64, 256, 256);
      return canvas.toDataURL('image/png').split(',')[1]!;
    });
  });
  const paths = ['frame_009.png', 'frame_10.png', fps === 12 ? 'frame_12.png' : 'frame_11.png'].map(name => path.join(session.userData, name));
  for (let i = 0; i < paths.length; i++) await writeFile(paths[i]!, Buffer.from(images[i]!, 'base64'));
  const chooser = page.waitForEvent('filechooser');
  await page.evaluate(() => (window as any).PM.pickFiles());
  await (await chooser).setFiles([paths[2]!, paths[0]!, paths[1]!]);
  const dialog = page.getByRole('dialog', { name: 'Import image sequence', exact: true });
  await expect(dialog).toBeVisible();
  if (fps === 12) await expect(dialog).toContainText('Missing frame number: 11');
  await dialog.getByRole('spinbutton', { name: 'Sequence frame rate' }).fill(String(fps));
  await expect(dialog).toContainText(`${(3 / fps).toFixed(3)} s`);
  await page.evaluate(() => {
    const original = window.createImageBitmap.bind(window);
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    window.createImageBitmap = (async (...args: any[]) => { await gate; return (original as any)(...args); }) as typeof createImageBitmap;
    (window as any).releaseSequence = () => { window.createImageBitmap = original; release(); };
    (window as any).sequenceProgress = [];
    const observer = new MutationObserver(() => {
      const card = document.querySelector('.toast[aria-label="Importing image sequence"]');
      if (card) (window as any).sequenceProgress.push(Number(card.querySelector('[role="progressbar"]')?.getAttribute('aria-valuenow') || 0));
    });
    observer.observe(document.body, { childList: true, subtree: true, characterData: true, attributes: true });
    (window as any).sequenceObserver = observer;
  });
  await dialog.getByRole('button', { name: 'Import sequence', exact: true }).click();
  const progress = page.getByRole('status', { name: 'Importing image sequence', exact: true });
  try {
    await expect(progress).toContainText('Importing image sequence');
    await expect(progress.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '0');
    await progress.screenshot({ path: test.info().outputPath('sequence-progress.png') });
  } finally { await page.evaluate(() => (window as any).releaseSequence()); }
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.name === 'frame sequence.webm'));
  await expect(progress).toHaveCount(0);
  const statuses = await page.evaluate(() => {
    (window as any).sequenceObserver.disconnect();
    return (window as any).sequenceProgress as number[];
  });
  expect(statuses.some(value => value > 0 && value <= 100)).toBe(true);
  const imported = await page.evaluate(() => {
    const PM = (window as any).PM, layer = PM.proj.layers[0];
    return { count: PM.proj.layers.length, type: layer.type, dur: layer.dur, from: layer.from, assets: Object.keys(PM.proj.assets).length, sourceDur: PM.assets.get(layer.d.asset).dur };
  });
  expect(imported).toMatchObject({ count: 1, type: 'video', from: 0, assets: 1 });
  expect(imported.sourceDur).toBeCloseTo(3 / fps, 3);
  expect(imported.dur).toBeCloseTo(Math.max(1 / 30, 3 / fps), 3);

  // Large image sequences must also retain alpha in Auto preview proxies.
  const previewAlpha = await page.evaluate(async () => {
    const PM = (window as any).PM, asset = PM.assets.get(PM.proj.layers[0].d.asset);
    await asset.previewReady;
    if (!asset.preview) throw new Error('Expected an editing preview for this 2K sequence');
    const cv = document.createElement('canvas'); cv.width = cv.height = 64;
    const ctx = cv.getContext('2d')!; ctx.drawImage(asset.preview.el, 0, 0, 64, 64);
    return [ctx.getImageData(0, 0, 1, 1).data[3], ctx.getImageData(4, 4, 1, 1).data[3], ctx.getImageData(32, 32, 1, 1).data[3]];
  });
  expect(previewAlpha[0]).toBe(0);
  expect(previewAlpha[1]).toBeGreaterThanOrEqual(125);
  expect(previewAlpha[1]).toBeLessThanOrEqual(131);
  expect(previewAlpha[2]).toBe(255);

  const exportPixels = (time: number) => exportStillPixels(session, time, 64, 64, [[32, 32], [0, 0], [4, 4]]);
  for (let frame = 0; frame < 3; frame++) {
    const pixels = await exportPixels(frame / fps);
    expect(pixels[0]![frame]).toBeGreaterThan(240);
    expect(pixels[0]![(frame + 1) % 3]).toBeLessThan(15);
    expect(pixels[1]![3]).toBe(0);
    expect(pixels[2]![3]).toBeGreaterThanOrEqual(125);
    expect(pixels[2]![3]).toBeLessThanOrEqual(131);
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
  expect(restored[2]![3]).toBeGreaterThanOrEqual(125);
  expect(restored[2]![3]).toBeLessThanOrEqual(131);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('sequence import warns about gaps and permits importing individual stills or cancelling', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const canvas = document.createElement('canvas'); canvas.width = canvas.height = 16;
    canvas.toBlob(blob => { void PM.importFiles([new File([blob!], 'f1.png'), new File([blob!], 'f3.png')]); });
  });
  const dialog = page.getByRole('dialog', { name: 'Import image sequence', exact: true });
  await expect(dialog).toContainText('Missing frame number: 2');
  await dialog.getByRole('button', { name: 'Individual images', exact: true }).click();
  await page.waitForFunction(() => (window as any).PM.proj.layers.filter((l: any) => l.type === 'image').length === 2);
  await page.evaluate(() => { void (window as any).PM.importFiles([new File([], 'a1.png'), new File([], 'a2.png')]); });
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await page.evaluate(() => (window as any).PM.flushProject());
  expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(2);
});

test('reimport replaces missing frames, preserves the frame rate and insertion time, and allows cancellation', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const png = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.setTime(1, { raw: true, force: true });
    const canvas = document.createElement('canvas'); canvas.width = canvas.height = 16;
    return canvas.toDataURL('image/png').split(',')[1]!;
  });
  const paths = ['f1.png', 'f2.png', 'f3.png'].map(name => path.join(session.userData, name));
  for (const file of paths) await writeFile(file, Buffer.from(png, 'base64'));
  const chooser = page.waitForEvent('filechooser');
  await page.evaluate(() => (window as any).PM.pickFiles(true));
  await (await chooser).setFiles([paths[0]!, paths[2]!]);
  const dialog = page.getByRole('dialog', { name: 'Import image sequence', exact: true });
  await expect(dialog).toContainText('Missing frame number: 2');
  await dialog.getByRole('spinbutton', { name: 'Sequence frame rate' }).fill('24');
  await dialog.screenshot({ path: test.info().outputPath('missing-frames.png') });
  await page.evaluate(() => { document.documentElement.dataset.theme = 'dark'; });
  await dialog.screenshot({ path: test.info().outputPath('missing-frames-dark.png') });
  await page.evaluate(() => { document.documentElement.dataset.theme = 'light'; });
  const cancelled = page.waitForEvent('filechooser');
  await dialog.getByRole('button', { name: 'Reimport…', exact: true }).click();
  await (await cancelled).setFiles([]);
  await expect(dialog).toContainText('Missing frame number: 2');
  await expect(dialog).toContainText('0.083 s');
  const replacement = page.waitForEvent('filechooser');
  await dialog.getByRole('button', { name: 'Reimport…', exact: true }).click();
  await (await replacement).setFiles([paths[2]!, paths[0]!, paths[1]!]);
  await expect(dialog).not.toContainText('Missing frame');
  await expect(dialog.getByRole('spinbutton', { name: 'Sequence frame rate' })).toHaveValue('24');
  await expect(dialog).toContainText('0.125 s');
  await dialog.getByRole('button', { name: 'Import sequence', exact: true }).click();
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.name === 'f sequence.webm'));
  expect(await page.evaluate(() => {
    const PM = (window as any).PM, layer = PM.proj.layers.find((l: any) => l.name === 'f sequence.webm');
    return { from: layer.from, sequence: PM.assets.get(layer.d.asset).imageSequence };
  })).toEqual({ from: 1, sequence: { fps: 24, frames: 3 } });
  const again = page.waitForEvent('filechooser');
  await page.evaluate(() => (window as any).PM.pickFiles(true));
  await (await again).setFiles([paths[0]!, paths[2]!]);
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await page.evaluate(() => (window as any).PM.flushProject());
  expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
