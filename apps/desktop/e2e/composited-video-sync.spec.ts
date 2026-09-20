import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { copyFile, writeFile } from 'node:fs/promises';
import { expect, test, repoRoot } from './helpers/app';

// Different codecs can start at different times even when both clips have the
// same trim. An alpha cutout over its source exposes that as doubled subjects.
test('overlapping video layers present the same composition frame despite decoder startup delay', async ({ session }) => {
  await session.openEditor();
  const sources = ['background.webm', 'foreground.webm'].map(name => path.join(session.userData, name));
  const raw = Buffer.concat(Array.from({ length: 60 }, (_, i) => Buffer.alloc(64 * 64 * 3, 20 + i * 3)));
  const cutout = Buffer.concat(Array.from({ length: 60 }, (_, i) => {
    const frame = Buffer.alloc(64 * 64 * 4, 20 + i * 3);
    for (let pixel = 0; pixel < 64 * 64; pixel++) frame[pixel * 4 + 3] = pixel % 64 < 32 ? 0 : 255;
    return frame;
  }));
  for (const [i, source] of sources.entries()) execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'), [
    '-v', 'error', '-f', 'rawvideo', '-pix_fmt', i ? 'rgba' : 'rgb24', '-s', '64x64', '-r', '30', '-i', 'pipe:0',
    '-c:v', 'libvpx-vp9', '-pix_fmt', 'yuva420p', '-lossless', '1', '-g', '1', source,
  ], { input: i ? cutout : raw });
  const { page } = session;
  await page.evaluate(() => {
    const input = document.createElement('input'); input.type = 'file'; input.multiple = true; input.id = 'sync-sources'; document.body.appendChild(input);
  });
  await page.locator('#sync-sources').setInputFiles(sources);
  const result = await page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.importFiles([...((document.querySelector('#sync-sources') as HTMLInputElement).files!)]);
    PM.pause(); PM.perf.auto = false; PM.quality = 1;
    const layers = PM.proj.layers.filter((l: any) => l.type === 'video').sort((a: any, b: any) => a.name.localeCompare(b.name));
    for (const [i, layer] of layers.entries()) {
      layer.from = 0; layer.dur = 1.5; layer.mblur = false;
      layer.d.trim = PM.P(.2); layer.d.speed = PM.P(1);
      layer.d.w = PM.P(64); layer.d.h = PM.P(64);
      layer.p['position.x'].v = 32 + i * 32; layer.p['position.y'].v = 32;
    }
    PM.proj.w = 96; PM.proj.h = 64; PM.proj.fps = 30; PM.proj.work = [0, 1.5];
    PM.proj.layers = [...layers].reverse(); PM.ProjectIndex.invalidate(); PM.touch(); PM.bus.emit('layers');
    PM.GL.previewViewport = null; PM.GL.resize(96, 64);
    const slower = PM.assets.get(layers[1].d.asset).el;
    const play = slower.play.bind(slower);
    slower.play = () => new Promise<void>((resolve, reject) => setTimeout(() => play().then(resolve, reject), 90));
    PM.setTime(.1, { force: true }); await new Promise(r => setTimeout(r, 200));
    const render = PM.GL.render;
    let seeks = 0;
    for (const layer of layers) PM.assets.get(layer.d.asset).el.addEventListener('seeking', () => seeks++);
    const samples: { time: number; levels: number[]; seeks: number }[] = [];
    PM.GL.render = (time: number, options: any) => {
      const result = render(time, options);
      if (result !== false && PM.playing) {
        const gl = PM.GL.gl;
        const levels = [16, 80, 48].map(x => {
          const pixel = new Uint8Array(4); gl.readPixels(x, 32, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel); return pixel[0]!;
        });
        samples.push({ time, levels, seeks });
      }
      return result;
    };
    try { PM.play(); await new Promise(r => setTimeout(r, 2200)); }
    finally { PM.pause(); PM.GL.render = render; slower.play = play; }
    return samples;
  });
  expect(result.length).toBeGreaterThan(20);
  expect(result.at(-1)!.seeks).toBeLessThan(12);
  for (const sample of result) {
    expect(Math.abs(sample.levels[0]! - sample.levels[1]!), JSON.stringify(sample)).toBeLessThan(2);
    expect(Math.abs(sample.levels[0]! - sample.levels[2]!), `Transparent overlap: ${JSON.stringify(sample)}`).toBeLessThan(2);
    expect(Math.abs(sample.levels[0]! - (20 + Math.floor((sample.time + .2) * 30 + 1e-6) * 3)), JSON.stringify(sample)).toBeLessThan(4);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('private cutout project keeps both video sources on their requested frame during playback', async ({ session }) => {
  test.skip(!process.env.PM_SYNC_PROJECT, 'Optional private layered cutout project');
  test.setTimeout(120000);
  await session.openEditor();
  const fixture = path.join(session.userData, 'Cutout.pmv');
  await copyFile(process.env.PM_SYNC_PROJECT!, fixture);
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
  }, fixture);
  await session.page.evaluate(() => (window as any).PM.openProject());
  await session.page.waitForFunction(() => {
    const PM = (window as any).PM, layers = PM.proj.layers.filter((l: any) => l.type === 'video');
    return layers.length === 2 && layers.every((l: any) => PM.assets.get(l.d.asset)?.el?.readyState >= 2);
  });
  // Reload the saved local session, rather than only opening a portable file.
  // Cold preparation above must commit its derivative before reload can reuse it.
  const beforeReload = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    await Promise.all([...PM.assets.map.values()].map((a: any) => a.previewReady));
    PM.pause();
    return JSON.stringify(PM.proj.layers);
  });
  await session.page.addInitScript(() => {
    (window as any).__offlineDuringReload = false;
    new MutationObserver(() => {
      if (document.querySelector('.asset-offline')) (window as any).__offlineDuringReload = true;
    }).observe(document, { childList: true, subtree: true });
  });
  const reloadStarted = Date.now();
  await session.page.reload();
  await session.page.waitForFunction(() => {
    const PM = (window as any).PM;
    if (!PM?.assets || PM.assets.loading?.size) return false;
    const assets: any[] = Object.values(PM.proj.assets);
    return assets.length === 3 && assets.every((a: any) => {
      const live = PM.assets.get(a.id);
      return live && (!(a.playbackProxy && a.playbackProxyVersion < 3) || live.preview?.el.readyState >= 2);
    });
  }, undefined, { timeout: 5000 });
  console.log('Restored all media after reload in', Date.now() - reloadStarted, 'ms');
  expect(await session.page.evaluate(() => (window as any).__offlineDuringReload)).toBe(false);
  expect(await session.page.evaluate(() => JSON.stringify((window as any).PM.proj.layers))).toBe(beforeReload);
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.setTime(0, { force: true });
    await new Promise(r => setTimeout(r, 300));
    const render = PM.GL.render, samples: any[] = [];
    let lastPresented = performance.now(), maxGap = 0;
    const started = performance.now();
    let cycle = 0, previousTransport = 0;
    let image = '';
    PM.GL.render = (time: number, options: any) => {
      const result = render(time, options);
      if (result !== false && PM.playing) {
        const now = performance.now();
        if (now - started > 1000) maxGap = Math.max(maxGap, now - lastPresented);
        lastPresented = now;
        if (PM.time < previousTransport) cycle++;
        previousTransport = PM.time;
      }
      if (result !== false && PM.playing) samples.push({ time, cycle, videos: PM.proj.layers.filter((l: any) => l.type === 'video' && PM.active(l, time)).map((l: any) => {
        const a = PM.assets.get(l.d.asset), el = a.preview?.el || a.el;
        const trim = typeof l.d.trim === 'number' ? l.d.trim : l.d.trim.v;
        const frame = PM.playbackVideoFrame?.frames.get(l.id);
        const target = PM.clamp(time - l.from + trim, 0, a.dur - .04);
        return { name: l.name, error: (frame?.time ?? el.currentTime) - target, buffered: !!frame };
      }) });
      if (result !== false && PM.playing && time >= 1.8 && !image) image = PM.GL.canvas.toDataURL('image/png');
      return result;
    };
    try { PM.play(); await new Promise(r => setTimeout(r, 12000)); }
    finally { PM.pause(); PM.GL.render = render; }
    return { samples, image, maxGap };
  });
  const { samples, image } = result;
  if (image) await writeFile(test.info().outputPath('cutout-playback.png'), Buffer.from(image.split(',')[1]!, 'base64'));
  console.log('Layered cutout frame sync', { frames: samples.length, maxGap: result.maxGap, maxError: Math.max(...samples.flatMap(s => s.videos.map((v: any) => Math.abs(v.error)))) });
  expect(samples.length).toBeGreaterThan(240);
  expect(result.maxGap).toBeLessThan(250);

  // An asset/quality update may redraw a frame; it must not inflate the
  // sustained playback count or make the composition move backward.
  expect(new Set(samples.map(s => `${s.cycle}:${s.time}`)).size).toBeGreaterThan(240);
  for (let i = 1; i < samples.length; i++) {
    if (samples[i].cycle === samples[i - 1].cycle) expect(samples[i].time).toBeGreaterThanOrEqual(samples[i - 1].time);
  }
  for (const sample of samples.filter(s => s.videos.length > 1)) for (const video of sample.videos) {
    expect(video.buffered, JSON.stringify(sample)).toBe(true);
    expect(video.error, JSON.stringify(sample)).toBeGreaterThan(-.035);
    expect(video.error, JSON.stringify(sample)).toBeLessThan(.001);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
