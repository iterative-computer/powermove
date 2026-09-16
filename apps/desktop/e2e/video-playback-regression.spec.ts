import { copyFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

test('an inactive copy cannot stop video playback', async ({ session }) => {
  await session.openEditor();
  await importFixture(session.page, 'h264-aac.mp4');
  await session.page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'video'));
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    const layer = PM.proj.layers.find((l: any) => l.type === 'video');
    layer.from = 0; layer.dur = 2;
    const copy = structuredClone(layer); copy.id = PM.uid('L'); copy.from = 2;
    PM.proj.layers.push(copy); PM.bus.emit('layers');
    const video = PM.assets.get(layer.d.asset).el;
    PM.setTime(.1, { raw: true, force: true }); PM.play();
    await new Promise(r => setTimeout(r, 1000));
    const result = { paused: video.paused, time: video.currentTime };
    PM.pause(); return result;
  });
  expect(result.paused).toBe(false);
  expect(result.time).toBeGreaterThan(.7);
});

test('profiles 4K editing playback and adjacent frame seeks', async ({ session }) => {
  test.skip(!process.env.PM_VIDEO_FIXTURE, 'Optional private 4K source');
  test.setTimeout(90000);
  await session.openEditor();
  const source = path.join(session.userData, '4k.webm');
  await copyFile(process.env.PM_VIDEO_FIXTURE!, source);
  const chooser = session.page.waitForEvent('filechooser');
  await session.page.evaluate(() => (window as any).PM.pickFiles());
  await (await chooser).setFiles(source);
  await session.page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'video'));
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM, layer = PM.proj.layers.find((l: any) => l.type === 'video');
    layer.from = 0; PM.proj.fps = 30;
    const asset = PM.assets.get(layer.d.asset);
    await asset.previewReady;
    const video = asset.preview?.el || asset.el;
    let frames = 0, active = true;
    const presented = () => { frames++; if (active) video.requestVideoFrameCallback(presented); };
    video.requestVideoFrameCallback(presented);
    PM.setTime(.5, { raw: true, force: true }); PM.play();
    await new Promise(r => setTimeout(r, 2000));
    const playbackFrames = frames; PM.pause(); active = false;
    const seeks: number[] = [];
    for (let i = 0; i < 15; i++) {
      const started = performance.now();
      await new Promise<void>((resolve, reject) => {
        const timer = setTimeout(() => reject(Error('Frame step timed out')), 10000);
        video.addEventListener('seeked', () => { clearTimeout(timer); resolve(); }, { once: true });
        PM.step(i < 10 ? 1 : -1);
      });
      seeks.push(performance.now() - started);
    }
    // The offline path must prepare the full source even while Auto uses a proxy.
    PM.agentFrameCapture = true;
    let sourceFrame;
    try {
      await PM.prepareFrame(1);
      const frame = [...PM.preparedVideoFrames.values()][0] as HTMLCanvasElement;
      sourceFrame = { w: frame.width, h: frame.height };
    } finally { PM.agentFrameCapture = false; PM.preparedVideoFrames = null; }
    const fullResolution = { w: asset.w, h: asset.h };
    const originalBytes = (await PM.MediaStore.get(PM.proj.assets[asset.id])).size;
    return { width: video.videoWidth, height: video.videoHeight, playbackFrames, seeks, fullResolution, sourceFrame, originalBytes };

  });
  console.log('4K playback/step profile', result);
  expect(result.sourceFrame).toEqual(result.fullResolution);
  expect(Math.max(result.fullResolution.w, result.fullResolution.h)).toBeGreaterThanOrEqual(3840);
  expect(result.originalBytes).toBeGreaterThan(0);
  expect(result.playbackFrames).toBeGreaterThan(40);
  expect(Math.max(...result.seeks)).toBeLessThan(100);
});
