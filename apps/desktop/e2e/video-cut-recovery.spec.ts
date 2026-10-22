import { copyFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { expect, test, repoRoot } from './helpers/app';

test('scrubbing across a cut presents the incoming clip despite an unfinished outgoing decode', async ({ session }) => {
  await session.openEditor();
  const sources = ['red', 'blue'].map(color => {
    const file = path.join(session.userData, `${color}.mp4`);
    execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'), [
      '-v', 'error', '-f', 'lavfi', '-i', `color=${color}:size=64x64:rate=30:duration=2`,
      '-c:v', 'libx264', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p', file,
    ]);
    return file;
  });
  const { page } = session;
  await page.evaluate(() => {
    const input = document.createElement('input'); input.type = 'file'; input.multiple = true;
    input.id = 'cut-sources'; document.body.appendChild(input);
  });
  await page.locator('#cut-sources').setInputFiles(sources);
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.importFiles([...document.querySelector<HTMLInputElement>('#cut-sources')!.files!]);
    await Promise.all([...PM.assets.map.values()].map((a: any) => a.previewReady));
    PM.pause(); PM.perf.auto = false; PM.quality = 1;
    const layers = PM.proj.layers.filter((l: any) => l.type === 'video');
    for (const layer of layers) {
      layer.from = layer.name === 'red.mp4' ? 0 : 1; layer.dur = 1;
      layer.d.trim = PM.P(0); layer.d.w = PM.P(64); layer.d.h = PM.P(64);
      layer.p['position.x'].v = 32; layer.p['position.y'].v = 32;
    }
    PM.proj.w = PM.proj.h = 64; PM.proj.layers = layers;
    PM.ProjectIndex.invalidate(); PM.touch(); PM.bus.emit('layers');
    PM.GL.previewViewport = null; PM.GL.resize(64, 64);
    PM.setTime(.2, { raw: true, force: true });
    (window as any).cutFrames = [];
    (window as any).cutRequests = [];
    const render = PM.GL.render;
    PM.GL.render = (time: number, options: any) => {
      (window as any).cutRequests.push(time);
      const result = render(time, options);
      if (result !== false) (window as any).cutFrames.push(time);
      return result;
    };
  });
  await expect.poll(() => page.evaluate(() => (window as any).cutFrames.includes(.2))).toBe(true);
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const outgoing = PM.proj.layers.find((l: any) => l.name === 'red.mp4');
    // Simulate a decoder whose old seek has not completed when the user crosses the cut.
    Object.defineProperty(PM.assets.get(outgoing.d.asset).el, 'seeking', { configurable: true, value: true });
    PM.setTime(.6, { raw: true });
  });
  await expect.poll(() => page.evaluate(() => (window as any).cutRequests.includes(.6))).toBe(true);
  await page.evaluate(() => (window as any).PM.setTime(1.2, { raw: true }));
  await expect.poll(() => page.evaluate(() => (window as any).cutFrames.includes(1.2)), { timeout: 1500 }).toBe(true);
  const pixel = await page.evaluate(() => {
    const gl = (window as any).PM.GL.gl, pixel = new Uint8Array(4);
    gl.readPixels(32, 32, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
    return [...pixel];
  });
  expect(pixel[2]).toBeGreaterThan(220);
  expect(pixel[0]).toBeLessThan(30);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a copied project settles on each trimmed video after a cut', async ({ session }) => {
  test.skip(!process.env.PM_VIDEO_CUT_PROJECT, 'Optional private video project');
  test.setTimeout(120_000);
  await session.openEditor();
  const copy = path.join(session.userData, 'Video-cuts.pmv');
  await copyFile(process.env.PM_VIDEO_CUT_PROJECT!, copy);
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
  }, copy);
  await session.page.evaluate(() => (window as any).PM.openProject());
  await session.page.waitForFunction(() => !(window as any).PM.assets.loading?.size);
  await session.page.reload();
  await session.page.waitForFunction(() => {
    const PM = (window as any).PM;
    return PM?.proj?.layers?.some((l: any) => l.type === 'video') && !PM.assets.loading?.size;
  });
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.perf.auto = false; PM.quality = 1;
    const before = JSON.stringify(PM.proj.layers);
    const videos = PM.proj.layers.filter((l: any) => l.type === 'video').sort((a: any, b: any) => a.from - b.from);
    const samples = [];
    for (const layer of videos) {
      const time = layer.from + Math.min(.2, layer.dur / 2);
      PM.setTime(time, { raw: true, force: true });
      await new Promise(r => setTimeout(r, 700));
      const asset = PM.assets.get(layer.d.asset);
      const video = asset?.el;
      samples.push({ name: layer.name, time, rendered: PM.GL.render(time, { mblur: false }) !== false,
        live: !!asset, readyState: video?.readyState, seeking: video?.seeking, at: video?.currentTime,
        error: video?.error && { code: video.error.code, message: video.error.message },
        sourceSize: asset?.persistBlob?.size, proxy: asset?.playbackProxy, preview: !!asset?.preview,
        restoreError: PM.assets.errors.get(layer.d.asset) });
    }
    PM.setTime(videos.at(-2).from, { raw: true, force: true });
    await new Promise(r => setTimeout(r, 300));
    const render = PM.GL.render, presented: number[] = [];
    PM.GL.render = (time: number, options: any) => {
      const ok = render(time, options);
      if (ok !== false && PM.playing) presented.push(time);
      return ok;
    };
    try { PM.play(); await new Promise(r => setTimeout(r, 2400)); }
    finally { PM.pause(); PM.GL.render = render; }
    samples.push({ name: 'playback cut', rendered: presented.some(time => time >= videos.at(-1).from),
      live: true, error: null, presented });
    return { samples, unchanged: before === JSON.stringify(PM.proj.layers) };
  });
  console.log('VIDEO_CUTS', JSON.stringify(result));
  expect(result.unchanged).toBe(true);
  for (const sample of result.samples) {
    expect(sample.live, JSON.stringify(sample)).toBe(true);
    expect(sample.error, JSON.stringify(sample)).toBeNull();
    expect(sample.rendered, JSON.stringify(sample)).toBe(true);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
