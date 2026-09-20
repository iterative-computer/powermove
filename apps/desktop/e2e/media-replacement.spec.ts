import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test, chooseNativeMenu } from './helpers/app';
import { importFixture } from './helpers/media';
import { MEDIA_ACCEPT } from '../src/shared/media-formats';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

test('replacement uses the import picker, progress, and image sequence flow', async ({ session }) => {
  const { page } = session;
  await importFixture(page, 'h264-aac.mp4');
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.name === 'h264-aac.mp4'));
  const before = await page.evaluate(() => {
    const PM = (window as any).PM;
    return { id: PM.proj.layers.find((l: any) => l.name === 'h264-aac.mp4').d.asset, layers: JSON.stringify(PM.proj.layers), count: Object.keys(PM.proj.assets).length };
  });
  const frames = [await imageFile(page, session.userData, 'frame_01.png', 'red'), await imageFile(page, session.userData, 'frame_02.png', 'blue')];
  const chooser = page.waitForEvent('filechooser');
  await chooseNativeMenu(session, 'Replace File…', () => page.locator('.asset-card').filter({ hasText: 'h264-aac.mp4' }).click({ button: 'right' }));
  const picker = await chooser;
  expect(picker.isMultiple()).toBe(true);
  expect(await picker.element().getAttribute('accept')).toBe(MEDIA_ACCEPT);
  expect(MEDIA_ACCEPT).toContain('.gif');
  await picker.setFiles(frames);
  const dialog = page.getByRole('dialog', { name: 'Import image sequence', exact: true });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('spinbutton', { name: 'Sequence frame rate' }).fill('12');
  await page.evaluate(() => {
    const PM = (window as any).PM, original = PM.MediaStore.put;
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    PM.MediaStore.put = async (...args: any[]) => { await gate; return original(...args); };
    (window as any).releaseReplacement = () => { PM.MediaStore.put = original; release(); };
  });
  await dialog.getByRole('button', { name: 'Import sequence', exact: true }).click();
  const progress = page.getByRole('status', { name: 'Importing image sequence', exact: true });
  try {
    await expect(progress).toContainText('Importing image sequence');
    await progress.screenshot({ path: test.info().outputPath('replacement-progress.png') });
    expect(await page.evaluate(id => (window as any).PM.proj.assets[id].name, before.id)).toBe('h264-aac.mp4');
  } finally { await page.evaluate(async () => { (window as any).releaseReplacement(); await (window as any).PM.app.importQueue; }); }
  await expect(progress).toHaveCount(0);
  expect(await page.evaluate(id => {
    const PM = (window as any).PM;
    return { name: PM.proj.assets[id].name, sequence: PM.proj.assets[id].imageSequence, layers: JSON.stringify(PM.proj.layers), count: Object.keys(PM.proj.assets).length };
  }, before.id)).toEqual({ name: 'frame sequence.webm', sequence: { fps: 12, frames: 2 }, layers: before.layers, count: before.count });
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await page.evaluate(id => (window as any).PM.proj.assets[id].name, before.id)).toBe('h264-aac.mp4');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('replacement picker remains bound to its original project', async ({ session }) => {
  const { page } = session;
  await importFixture(page, 'tone.wav');
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.name === 'tone.wav'));
  const chooser = page.waitForEvent('filechooser');
  await chooseNativeMenu(session, 'Replace File…', () => page.locator('.asset-card').filter({ hasText: 'tone.wav' }).click({ button: 'right' }));
  const picker = await chooser;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Another project' }) }));
  });
  await picker.setFiles(path.join(__dirname, './fixtures/tone.mp3'));
  await page.evaluate(() => (window as any).PM.app.importQueue);
  expect(await page.evaluate(() => Object.keys((window as any).PM.proj.assets))).toEqual([]);
  await expect(page.locator('.import-progress')).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
  // Project-close behavior is covered separately; finish this isolated picker session.
  await session.app.evaluate(({ app }) => app.exit(0)).catch(() => undefined);
});

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
  const replaceChooser = page.waitForEvent('filechooser');
  await chooseNativeMenu(session, 'Replace File…', () => page.locator('.asset-card').filter({ hasText: 'original-red.png' }).click({ button: 'right' }));
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
  const chooser = page.waitForEvent('filechooser');
  await chooseNativeMenu(session, 'Replace File…', () => page.locator('.asset-card').filter({ hasText: 'tone.wav' }).click({ button: 'right' }));
  await (await chooser).setFiles([]);
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.proj))).toBe(before.project);
  const outcomes = await page.evaluate(async id => {
    const PM = (window as any).PM;
    const attempts = [new File(['invalid'], 'broken.wav', { type: 'audio/wav' }), new File(['invalid'], 'image.png', { type: 'image/png' })];
    for (const file of attempts) { try { await PM.assets.replace(id, file); } catch {} }
    return { project: JSON.stringify(PM.proj), runtimeName: PM.assets.get(id)?.name };
  }, before.id);
  expect(outcomes).toEqual({ project: before.project, runtimeName: 'tone.wav' });
  await page.evaluate(id => (window as any).PM.importFiles([new File(['invalid'], 'broken.wav', { type: 'audio/wav' })], { replaceAssetId: id }), before.id);
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.proj))).toBe(before.project);
  await expect(page.locator('.import-progress')).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
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
    const chooser = page.waitForEvent('filechooser');
    await chooseNativeMenu(session, 'Replace File…', () => page.locator('.asset-card').filter({ hasText: media.original }).click({ button: 'right' }));
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
