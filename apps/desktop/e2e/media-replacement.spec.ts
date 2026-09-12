import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

async function imageFile(page: any, directory: string, name: string, color: string) {
  const encoded = await page.evaluate((fill: string) => {
    const canvas = document.createElement('canvas'); canvas.width = canvas.height = 64;
    const context = canvas.getContext('2d')!; context.fillStyle = fill; context.fillRect(0, 0, 64, 64);
    return canvas.toDataURL('image/png').split(',')[1];
  }, color);
  const file = path.join(directory, name);
  await writeFile(file, Buffer.from(encoded, 'base64'));
  return file;
}

async function renderedPixel(page: any) {
  return page.evaluate(() => {
    const PM = (window as any).PM;
    const frame = PM.renderFrameTo(0, PM.proj.w, PM.proj.h);
    return [...frame.getContext('2d').getImageData(frame.width / 2, frame.height / 2, 1, 1).data];
  });
}

test('right-click replacement preserves layers, renders new pixels, undoes/redoes, and survives portable save', async ({ session }) => {
  test.setTimeout(90_000);
  const { page } = session;
  const original = await imageFile(page, session.userData, 'original-red.png', '#ff0000');
  const replacement = await imageFile(page, session.userData, 'replacement-blue.png', '#0000ff');
  const chooser = page.waitForEvent('filechooser');
  await page.evaluate(() => (window as any).PM.pickFiles());
  await (await chooser).setFiles(original);
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.name === 'original-red.png'));
  const before = await page.evaluate(() => {
    const PM = (window as any).PM;
    const layer = PM.proj.layers.find((l: any) => l.name === 'original-red.png');
    PM.proj.layers = [layer];
    layer.from = 0; layer.dur = 5;
    PM.Edit.apply({ type: 'replace_keyframes', target: layer.id, path: 'rotation.z', keyframes: [{ time: 0, value: 0 }, { time: 2, value: 45 }], preserveHandEdits: false });
    PM.setTime(0, { raw: true, force: true });
    return { id: layer.d.asset, layers: JSON.stringify(PM.proj.layers) };
  });
  expect(await renderedPixel(page)).toEqual([255, 0, 0, 255]);
  await page.locator('.asset-card').filter({ hasText: 'original-red.png' }).click({ button: 'right' });
  const replaceChooser = page.waitForEvent('filechooser');
  await page.getByRole('menuitem', { name: /Replace File/i }).click();
  await (await replaceChooser).setFiles(replacement);
  await expect.poll(() => page.evaluate(id => (window as any).PM.proj.assets[id]?.name, before.id)).toBe('replacement-blue.png');
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.proj.layers))).toBe(before.layers);
  expect(await renderedPixel(page)).toEqual([0, 0, 255, 255]);
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.hist.do('Rename layer after replacement', () => { PM.proj.layers[0].name = 'Renamed'; });
    PM.hist.undo();
  });
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(() => renderedPixel(page)).toEqual([255, 0, 0, 255]);
  await page.evaluate(() => (window as any).PM.hist.redo());
  await expect.poll(() => renderedPixel(page)).toEqual([0, 0, 255, 255]);
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.proj.layers))).toBe(before.layers);

  const destination = path.join(session.userData, 'Replacement.pmv');
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, destination);
  expect(await page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    for (const asset of Object.values(PM.proj.assets)) await PM.MediaStore.remove(asset);
  });
  await session.relaunch();
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
  }, destination);
  await session.page.evaluate(() => (window as any).PM.openProject());
  await session.page.waitForFunction(id => (window as any).PM.assets.get(id)?.el, before.id);
  await expect.poll(() => renderedPixel(session.page)).toEqual([0, 0, 255, 255]);
  expect(await session.page.evaluate(id => (window as any).PM.proj.assets[id].name, before.id)).toBe('replacement-blue.png');
});

test('replacement picker cancellation and unreadable or incompatible files leave media intact', async ({ session }) => {
  const { page } = session;
  await importFixture(page, 'tone.wav');
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.name === 'tone.wav'));
  const before = await page.evaluate(() => {
    const PM = (window as any).PM;
    return { id: PM.proj.layers.find((l: any) => l.name === 'tone.wav').d.asset, project: JSON.stringify(PM.proj) };
  });
  await page.locator('.asset-card').filter({ hasText: 'tone.wav' }).click({ button: 'right' });
  const chooser = page.waitForEvent('filechooser');
  await page.getByRole('menuitem', { name: /Replace File/i }).click();
  await (await chooser).setFiles([]);
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.proj))).toBe(before.project);
  const outcomes = await page.evaluate(async id => {
    const PM = (window as any).PM;
    const attempts = [new File(['invalid'], 'broken.wav', { type: 'audio/wav' }), new File(['invalid'], 'image.png', { type: 'image/png' })];
    for (const file of attempts) { try { await PM.assets.replace(id, file); } catch {} }
    return { project: JSON.stringify(PM.proj), runtimeName: PM.assets.get(id)?.name };
  }, before.id);
  expect(outcomes).toEqual({ project: before.project, runtimeName: 'tone.wav' });
});

for (const media of [{ original: 'tone.wav', replacement: 'tone.mp3', kind: 'audio' }, { original: 'h264-aac.mp4', replacement: 'vp8-opus.webm', kind: 'video' }]) {
  test(`${media.kind} replacement keeps clip timing and restores its previous source with Undo`, async ({ session }) => {
    const { page } = session;
    await importFixture(page, media.original);
    await page.waitForFunction(name => (window as any).PM.proj.layers.some((l: any) => l.name === name), media.original);
    const before = await page.evaluate(name => {
      const PM = (window as any).PM, layer = PM.proj.layers.find((l: any) => l.name === name);
      layer.from = 2; layer.dur = 1; layer.d.trim = .25;
      return { id: layer.d.asset, layer: JSON.stringify(layer) };
    }, media.original);
    await page.locator('.asset-card').filter({ hasText: media.original }).click({ button: 'right' });
    const chooser = page.waitForEvent('filechooser');
    await page.getByRole('menuitem', { name: /Replace File/i }).click();
    await (await chooser).setFiles(path.join(__dirname, './fixtures', media.replacement));
    await expect.poll(() => page.evaluate(id => (window as any).PM.proj.assets[id]?.name, before.id)).toBe(media.replacement);
    const after = await page.evaluate(async id => {
      const PM = (window as any).PM, live = PM.assets.get(id);
      const decoded = live.kind === 'audio' ? await PM.Audio.decodeAsset(live) : null;
      return { layer: JSON.stringify(PM.proj.layers.find((l: any) => l.d?.asset === id)), duration: decoded?.duration || live.dur, bytes: (await PM.MediaStore.get(PM.proj.assets[id]))?.size };
    }, before.id);
    expect(after.layer).toBe(before.layer);
    expect(after.duration).toBeGreaterThan(0);
    expect(after.bytes).toBeGreaterThan(0);
    await page.evaluate(() => (window as any).PM.hist.undo());
    expect(await page.evaluate(id => (window as any).PM.assets.get(id).name, before.id)).toBe(media.original);
    await page.evaluate(() => (window as any).PM.hist.redo());
    expect(await page.evaluate(id => (window as any).PM.assets.get(id).name, before.id)).toBe(media.replacement);
  });
}
