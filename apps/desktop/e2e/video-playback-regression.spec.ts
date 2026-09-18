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

test('adjacent trimmed clips do not flash old frames across repeated cuts', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await importFixture(page, 'h264-aac.mp4');
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'video'));
  const samples = await page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.perf.auto = false; PM.quality = 1;
    const original = PM.proj.layers.find((l: any) => l.type === 'video');
    const layers = [original, structuredClone(original), structuredClone(original)];
    for (const [i, layer] of layers.entries()) {
      if (i) layer.id = PM.uid('l');
      layer.from = i * .5; layer.dur = .5; layer.mblur = false;
      layer.d.trim = PM.P(i === 1 ? 1 : 0);
      layer.d.w = PM.P(160); layer.d.h = PM.P(90);
      layer.p['position.x'].v = 80; layer.p['position.y'].v = 45;
    }
    PM.proj.w = 160; PM.proj.h = 90; PM.proj.fps = 30;
    PM.proj.layers = layers; PM.proj.work = [0, 1.5];
    PM.ProjectIndex.invalidate(); PM.touch(); PM.bus.emit('layers');
    PM.GL.previewViewport = null; PM.GL.resize(160, 90);
    PM.setTime(0, { force: true });
    await new Promise(r => setTimeout(r, 200));
    const samples: { time: number; pixel: number[] }[] = [];
    PM.play();
    try {
      const end = performance.now() + 3200;
      while (performance.now() < end) {
        await new Promise(requestAnimationFrame);
        const time = Math.floor(PM.time * 30) / 30;
        // Observe every cut, including a second traversal after looping.
        if (time < .4 || time > 1.15) continue;
        PM.GL.render(time, { mblur: false });
        const gl = PM.GL.gl, pixel = new Uint8Array(4);
        gl.readPixels(Math.floor(gl.drawingBufferWidth / 2), Math.floor(gl.drawingBufferHeight / 2), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
        samples.push({ time, pixel: [...pixel] });
      }
    } finally { PM.pause(); }
    return samples;
  });
  expect(samples.length).toBeGreaterThan(20);
  for (const { time, pixel } of samples) {
    const green = time >= .5 && time < 1;
    expect(pixel[green ? 1 : 0], `frame at ${time}: ${pixel}`).toBeGreaterThan(220);
    expect(pixel[green ? 0 : 1], `frame at ${time}: ${pixel}`).toBeLessThan(30);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const mode of ['animated speed', 'time remap']) test(`seek-driven playback keeps presenting frames with ${mode}`, async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await importFixture(page, 'h264-aac.mp4');
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'video'));
  const result = await page.evaluate(async (mode) => {
    const PM = (window as any).PM;
    PM.pause(); PM.perf.auto = false; PM.quality = 1;
    const layer = PM.proj.layers.find((l: any) => l.type === 'video');
    layer.from = 0; layer.dur = 2; layer.mblur = false;
    layer.d.trim = PM.P(0); layer.d.speed = PM.P(1);
    PM.setKeyOn(layer.d.speed, 0, 1, 'linear', 30);
    PM.setKeyOn(layer.d.speed, 2, 1, 'linear', 30);
    if (mode === 'time remap') {
      layer.d.speed = PM.P(1); layer.d.timeRemap = PM.P(true); layer.d.sourceTime = PM.P(0);
      PM.setKeyOn(layer.d.sourceTime, 0, 0, 'linear', 30);
      PM.setKeyOn(layer.d.sourceTime, 2, 2, 'linear', 30);
    }
    layer.d.w = PM.P(160); layer.d.h = PM.P(90);
    layer.p['position.x'].v = 80; layer.p['position.y'].v = 45;
    PM.proj.w = 160; PM.proj.h = 90; PM.proj.fps = 30;
    PM.proj.layers = [layer]; PM.proj.work = [0, 2];
    PM.ProjectIndex.invalidate(); PM.touch(); PM.bus.emit('layers');
    PM.GL.previewViewport = null; PM.GL.resize(160, 90);
    PM.setTime(0, { force: true });
    await new Promise(r => setTimeout(r, 200));
    PM.play();
    const samples: number[][] = [];
    try {
      while (PM.time < 1.7) {
        await new Promise(requestAnimationFrame);
        if (PM.time < 1.2) continue;
        PM.GL.render(Math.floor(PM.time * 30) / 30, { mblur: false });
        const gl = PM.GL.gl, pixel = new Uint8Array(4);
        gl.readPixels(Math.floor(gl.drawingBufferWidth / 2), Math.floor(gl.drawingBufferHeight / 2), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
        samples.push([...pixel]);
      }
    } finally { PM.pause(); }
    return samples;
  }, mode);
  expect(result.length).toBeGreaterThan(5);
  expect(result.filter(pixel => pixel[1]! > 220 && pixel[0]! < 30).length).toBeGreaterThan(result.length * .8);
});
