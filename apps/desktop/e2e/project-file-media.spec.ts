import { expect, test } from './helpers/app';
import { fixturePath, importFixture } from './helpers/media';
import { readFile } from 'node:fs/promises';
import { decodeProjectContainer } from '../src/shared/project-container';
import path from 'node:path';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

test('saved project files restore imported video without the original session media', async ({ session }) => {
  await importFixture(session.page, 'h264-aac.mp4');
  await session.page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.name === 'h264-aac.mp4'));
  // bf26d9b3e links selected originals. This persistence/reuse fixture
  // deliberately owns its media, as a portable project import does.
  await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    for (const meta of Object.values<any>(PM.proj.assets)) {
      const blob = await PM.MediaStore.get(meta.storageKey);
      if (!blob || !await PM.MediaStore.put(meta.storageKey, blob)) throw new Error('Could not retain fixture media');
      delete meta.sourcePath;
    }
  });
  const destination = path.join(session.userData, 'Media.pmv');
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, destination);
  expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  const saved = await readFile(destination);
  const document = decodeProjectContainer(saved);
  expect(document.binary).toBe(true);
  expect(document.media).toHaveLength(1);
  await session.page.evaluate(async () => {
    const PM = (window as any).PM, assets = Object.values(PM.proj.assets);
    // bf26d9b3e focuses an already-open file instead of reloading it. Close
    // the document before clearing its cache so Open really restores the PMV.
    if (!await PM.Tabs.close(PM.proj.id)) throw new Error('Could not close saved media fixture');
    for (const asset of assets) await PM.MediaStore.remove(asset);
  });
  await session.relaunch();
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
  }, destination);
  await session.page.evaluate(() => (window as any).PM.openProject());
  await session.page.waitForFunction(() => [...(window as any).PM.assets.map.values()].some((a: any) => a.name === 'h264-aac.mp4' && a.el?.readyState >= 2));
  const restored = await session.page.evaluate(async () => {
    const PM = (window as any).PM, asset = Object.values(PM.proj.assets)[0];
    const blob = await PM.MediaStore.get(asset);
    const layer = PM.proj.layers.find((item: any) => item.name === 'h264-aac.mp4');
    PM.Edit.apply({ type: 'set_layer', target: layer.id, patch: { from: 0 } });
    PM.setTime(1.5, { raw: true, force: true });
    // This verifies persisted source pixels. Await the real capture decoder;
    // a background preview may use another element or supersede its own seek.
    const capturing = PM.agentFrameCapture;
    PM.agentFrameCapture = true;
    try {
      await PM.prepareFrame(1.5, { mblur: false });
      const prepared = PM.preparedVideoFrames?.size === 1;
      const frame = PM.renderFrameTo(1.5, PM.proj.w, PM.proj.h);
      return { prepared, bytes: blob?.size, pixel: [...frame.getContext('2d').getImageData(frame.width / 2, frame.height / 2, 1, 1).data] };
    } finally { PM.agentFrameCapture = capturing; PM.preparedVideoFrames = null; }
  });
  expect(restored.prepared).toBe(true);
  expect(restored.bytes).toBeGreaterThan(0);
  expect(restored.pixel[1]).toBeGreaterThan(225);
  expect(restored.pixel[0]).toBeLessThan(30);
  // Reopening retains the persisted media revision, so the next metadata-only
  // save must not read/upload that video again.
  const resaved = await session.page.evaluate(async () => {
    const PM = (window as any).PM, original = PM.MediaStore.getForSave;
    const records: any[] = [], uploaded: string[] = [];
    PM.MediaStore.getForSave = async (asset: any) => {
      const saved = await original(asset);
      if (saved) {
        const source = saved.source ?? saved.blob;
        records.push({ id: asset.id, storageKey: asset.storageKey, revision: saved.revision, size: source.size, type: source.type });
        Object.defineProperty(source, saved.source ? 'read' : 'slice', { value: () => { uploaded.push(asset.id); throw new Error('Reopened media was uploaded again'); } });
      }
      return saved;
    };
    try {
      PM.Projects.rename(PM.proj.id, 'Reopened and saved');
      const file = { ...PM.projectFileState() };
      const saved = await PM.saveProject();
      return { saved, projectId: PM.proj.id, file: { path: file.path, dirty: file.dirty }, records, uploaded };
    }
    finally { PM.MediaStore.getForSave = original; }
  });
  expect(resaved.saved, JSON.stringify({ ...resaved, previousMedia: document.media.map(source => ({
    id: source.id, revision: (source as { revision?: string }).revision, type: source.type, size: source.data.length,
  })) })).toBe(true);
});

test('saved project files restore imported audio without the original session media', async ({ session }) => {
  await importFixture(session.page, 'tone.wav');
  await session.page.waitForFunction(() => (window as any).PM.proj.layers.some((layer: any) => layer.type === 'audio' && layer.name === 'tone.wav'));
  // bf26d9b3e links selected originals. This persistence/reuse fixture
  // deliberately owns its media, as a portable project import does.
  await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    for (const meta of Object.values<any>(PM.proj.assets)) {
      const blob = await PM.MediaStore.get(meta.storageKey);
      if (!blob || !await PM.MediaStore.put(meta.storageKey, blob)) throw new Error('Could not retain fixture media');
      delete meta.sourcePath;
    }
  });
  const destination = path.join(session.userData, 'Audio.pmv');
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, destination);
  expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);

  const saved = await readFile(destination);
  const document = decodeProjectContainer(saved);
  expect(document.binary).toBe(true);
  expect(document.media).toHaveLength(1);

  await session.page.evaluate(async () => {
    const PM = (window as any).PM, assets = Object.values(PM.proj.assets);
    // bf26d9b3e focuses an already-open file instead of reloading it. Close
    // the document before clearing its cache so Open really restores the PMV.
    if (!await PM.Tabs.close(PM.proj.id)) throw new Error('Could not close saved media fixture');
    for (const asset of assets) await PM.MediaStore.remove(asset);
  });
  await session.relaunch();
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
  }, destination);
  await session.page.evaluate(() => (window as any).PM.openProject());
  await session.page.waitForFunction(() => [...(window as any).PM.assets.map.values()].some((asset: any) => asset.name === 'tone.wav' && asset.audioBlob?.size > 0));
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.setTime(0, { raw: true, force: true });
    PM.play();
  });
  await session.page.waitForFunction(() => Number(document.documentElement.dataset.audioVoices || 0) > 0);
  await session.page.evaluate(() => (window as any).PM.pause());

  const restored = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    const layer = PM.proj.layers.find((item: any) => item.type === 'audio' && item.name === 'tone.wav');
    const metadata = layer && PM.proj.assets[layer.d.asset];
    const live = layer && PM.assets.get(layer.d.asset);
    const blob = metadata && await PM.MediaStore.get(metadata);
    const decoded = live && await PM.Audio.decodeAsset(live);
    return {
      layerType: layer?.type,
      assetId: layer?.d.asset,
      metadataKind: metadata?.kind,
      bytes: blob?.size || 0,
      duration: decoded?.duration || 0,
      peaks: live?.peaks?.length || 0,
    };
  });
  expect(restored).toMatchObject({ layerType: 'audio', metadataKind: 'audio' });
  expect(restored.assetId).toBeTruthy();
  expect(restored.bytes).toBeGreaterThan(0);
  expect(restored.duration).toBeGreaterThan(0);
  expect(restored.peaks).toBeGreaterThan(0);
});

test('leaving a local project for another and coming back keeps imported audio', async ({ session }) => {
  const { page } = session;
  await importFixture(page, 'tone.wav');
  await page.waitForFunction(() => [...(window as any).PM.assets.map.values()]
    .some((asset: any) => asset.name === 'tone.wav' && asset.audioBlob?.size > 0));
  const project = await page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.flushProject();
    return { id: PM.proj.id, name: PM.proj.name };
  });

  // The window swaps to a scratch composition, then back to the audio project.
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    const other = PM.mkProject({ name: 'Somewhere else', dur: 4, w: 640, h: 360, fps: 24, bg: '#09090A' });
    PM.Projects.put(other);
    await PM.openProjectHere(other.id);
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.name)).toBe('Somewhere else');

  await page.evaluate(() => (window as any).PM.ProjectsScreen.show('projects'));
  await page.locator('.ps-card').filter({ has: page.locator('.ps-name', { hasText: project.name }) }).click();

  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.id)).toBe(project.id);
  await page.waitForFunction(() => [...(window as any).PM.assets.map.values()]
    .some((asset: any) => asset.name === 'tone.wav' && asset.audioBlob?.size > 0));
  const restored = await page.evaluate(async () => {
    const PM = (window as any).PM;
    const layer = PM.proj.layers.find((item: any) => item.type === 'audio' && item.name === 'tone.wav');
    const live = layer && PM.assets.get(layer.d.asset);
    const decoded = live && await PM.Audio.decodeAsset(live);
    return { layer: !!layer, bytes: live?.audioBlob?.size || 0, duration: decoded?.duration || 0 };
  });
  expect(restored.layer).toBe(true);
  expect(restored.bytes).toBeGreaterThan(0);
  expect(restored.duration).toBeGreaterThan(0);
});

test('local recovery keeps imported audio after closing and relaunching Powermove', async ({ session }) => {
  await importFixture(session.page, 'tone.wav');
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, path.join(session.userData, 'Audio recovery.pmv'));
  expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  const projectId = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.flushProject();
    return PM.proj.id;
  });

  await session.relaunch();
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.proj.id)).toBe(projectId);
  await session.page.waitForFunction(() => [...(window as any).PM.assets.map.values()]
    .some((asset: any) => asset.name === 'tone.wav' && asset.audioBlob?.size > 0));
  const restored = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    const layer = PM.proj.layers.find((item: any) => item.type === 'audio' && item.name === 'tone.wav');
    const live = layer && PM.assets.get(layer.d.asset);
    const decoded = live && await PM.Audio.decodeAsset(live);
    return { layer: !!layer, bytes: live?.audioBlob?.size || 0, duration: decoded?.duration || 0 };
  });
  expect(restored.layer).toBe(true);
  expect(restored.bytes).toBeGreaterThan(0);
  expect(restored.duration).toBeGreaterThan(0);
});

test('media library items reveal their original file in Finder', async ({ session }) => {
  await session.app.evaluate(({ shell }) => {
    shell.showItemInFolder = (filePath: string) => { (globalThis as any).__revealedMediaPath = filePath; };
  });
  await importFixture(session.page, 'tone.wav');

  await session.page.evaluate(() => (window as any).PM.Layout.activateTab('assets'));
  const card = session.page.locator('.asset-card').filter({ hasText: 'tone.wav' });
  await card.hover();
  await card.getByRole('button', { name: 'Reveal tone.wav in Finder' }).click();

  await expect.poll(() => session.app.evaluate(() => (globalThis as any).__revealedMediaPath || ''))
    .toBe(fixturePath('tone.wav'));
});
