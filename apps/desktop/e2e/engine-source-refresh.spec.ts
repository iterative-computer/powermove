import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });

test('an open paused source monitor repaints interpretation edits without a seek', async ({ session }) => {
  test.setTimeout(90_000);
  await session.openEditor();
  const { page } = session;
  await importFixture(page, 'h264-aac.mp4');
  await page.evaluate(async () => {
    const canvas = document.createElement('canvas'); canvas.width = 64; canvas.height = 64;
    const context = canvas.getContext('2d')!; context.fillStyle = 'rgb(90, 130, 170)'; context.fillRect(0, 0, 64, 64);
    const blob = await new Promise<Blob>(resolve => canvas.toBlob(blob => resolve(blob!), 'image/png'));
    await (window as any).PM.importFiles([new File([blob], 'midtone.png', { type: 'image/png' })], { placement: null });
  });
  const pixels = () => page.evaluate(() => {
    const monitor = (window as any).PM.EngineSourceMedia.monitorValue;
    const canvas = document.createElement('canvas'); canvas.width = 32; canvas.height = 32;
    const surface = monitor.surface;
    const width = surface instanceof HTMLImageElement ? surface.naturalWidth : surface.videoWidth;
    const height = surface instanceof HTMLImageElement ? surface.naturalHeight : surface.videoHeight;
    const context = canvas.getContext('2d')!;
    context.drawImage(surface, width * .4, height * .4, width * .2, height * .2, 0, 0, 32, 32);
    return Array.from(context.getImageData(0, 0, 32, 32).data);
  });
  for (const name of ['midtone.png', 'h264-aac.mp4']) {
    const at = name.endsWith('.mp4') ? .25 : 0;
    await page.evaluate(({ name, at }) => {
      const PM = (window as any).PM;
      const asset = Object.values<any>(PM.proj.assets).find(asset => asset.name === name);
      PM.Kernel.services.get('viewer').preview.show(asset.id);
      const monitor = PM.EngineSourceMedia.monitorValue;
      (window as any).__sourceRefresh = { asset: asset.id, paints: 0 };
      monitor.addEventListener('loadeddata', () => (window as any).__sourceRefresh.paints++);
      monitor.currentTime = at;
    }, { name, at });
    await page.waitForFunction(() => {
      const monitor = (window as any).PM.EngineSourceMedia.monitorValue;
      return (window as any).__sourceRefresh.paints > 0 && !monitor.rendering
        && monitor.paintedSize?.width === Math.round(monitor.surface.clientWidth * devicePixelRatio)
        && monitor.paintedSize?.height === Math.round(monitor.surface.clientHeight * devicePixelRatio) && (monitor.surface instanceof HTMLImageElement
        ? monitor.surface.complete && monitor.surface.naturalWidth > 0 : monitor.surface.readyState >= 2);
    });
    const before = await pixels();
    await page.evaluate(() => {
      const PM = (window as any).PM, state = (window as any).__sourceRefresh;
      state.paints = 0;
      PM.hist.do('Interpret Footage', () => { PM.proj.assets[state.asset].interpret = { transfer: 'linear' }; });
      PM.bus.emit('assets');
    });
    await expect.poll(() => page.evaluate(() => (window as any).__sourceRefresh.paints)).toBeGreaterThan(0);
    await expect.poll(async () => (await pixels()).filter((value, i) => Math.abs(value - before[i]!) > 3).length).toBeGreaterThan(100);
    expect(await page.evaluate(() => {
      const monitor = (window as any).PM.EngineSourceMedia.monitorValue;
      return { time: monitor.currentTime, paused: monitor.paused, error: monitor.error?.message ?? null };
    })).toEqual({ time: at, paused: true, error: null });

    await page.evaluate(() => {
      const PM = (window as any).PM, state = (window as any).__sourceRefresh;
      state.paints = 0;
      PM.hist.undo();
    });
    await expect.poll(() => page.evaluate(() => (window as any).__sourceRefresh.paints)).toBeGreaterThan(0);
    await expect.poll(async () => Math.max(...(await pixels()).map((value, i) => Math.abs(value - before[i]!))), { message: `${name}: undo restores source colours` }).toBeLessThanOrEqual(2);
    await page.evaluate(() => {
      const PM = (window as any).PM;
      (window as any).__sourceRefresh.paints = 0;
      PM.hist.redo();
    });
    await expect.poll(() => page.evaluate(() => (window as any).__sourceRefresh.paints)).toBeGreaterThan(0);
    await expect.poll(async () => (await pixels()).filter((value, i) => Math.abs(value - before[i]!) > 3).length).toBeGreaterThan(100);
    await page.keyboard.press('Escape');
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const enabled of [false, true]) test.describe(`source switching, engine ${enabled ? 'on' : 'off'}`, () => {
  test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: enabled ? '1' : '0' } } });

  test('switching between video and audio starts each source and reaches its end', async ({ session }) => {
    await session.openEditor();
    for (const name of ['h264-aac.mp4', 'tone.wav']) {
      await importFixture(session.page, name);
      await session.page.waitForFunction(name => {
        const PM = (window as any).PM;
        return Object.values<any>(PM.proj.assets).some(asset => asset.name === name)
          && [...PM.assets.map.values()].some((asset: any) => asset.name === name);
      }, name);
    }
    for (const name of ['h264-aac.mp4', 'tone.wav', 'h264-aac.mp4', 'tone.wav']) {
      await session.page.evaluate(({ name, enabled }) => {
        const PM = (window as any).PM, preview = PM.Kernel.services.get('viewer').preview;
        const asset = Object.values<any>(PM.proj.assets).find(asset => asset.name === name);
        preview.show(asset.id);
        const source = enabled ? PM.EngineSourceMedia.monitorValue : document.querySelector('#source-preview video, #source-preview audio');
        (window as any).__sourcePlayback = { source, heard: false, ended: false };
        source.addEventListener('timeupdate', () => {
          if (source.levels?.peak.some((value: number) => value > .001)) (window as any).__sourcePlayback.heard = true;
        });
        source.addEventListener('ended', () => { (window as any).__sourcePlayback.ended = true; });
        preview.toggle(asset.id);
      }, { name, enabled });
      await expect.poll(() => session.page.evaluate(() => {
        const source = (window as any).__sourcePlayback.source;
        return { advancing: source.currentTime > .05, error: source.error?.message ?? null };
      }), { message: `${name}: source playback advances` }).toEqual({ advancing: true, error: null });
      if (enabled) await expect.poll(() => session.page.evaluate(() => (window as any).__sourcePlayback.heard),
        { message: `${name}: source produces audio` }).toBe(true);
      if (enabled && name.endsWith('.mp4')) {
        const time = await session.page.evaluate(() => {
          const PM = (window as any).PM, source = (window as any).__sourcePlayback.source;
          const time = source.currentTime;
          PM.hist.do('Interpret playing source', () => {
            const asset = PM.proj.assets[source.asset];
            asset.interpret = { transfer: asset.interpret?.transfer === 'linear' ? 'srgb' : 'linear' };
          });
          PM.bus.emit('assets');
          return time;
        });
        await expect.poll(() => session.page.evaluate(time => {
          const source = (window as any).__sourcePlayback.source;
          return !source.paused && source.currentTime >= time + .05 && !source.error;
        }, time), { message: 'Interpret Footage preserves ongoing source playback' }).toBe(true);
      }
      await session.page.evaluate(() => {
        const source = (window as any).__sourcePlayback.source;
        source.currentTime = source.duration - .05;
      });
      await expect.poll(() => session.page.evaluate(() => (window as any).__sourcePlayback.ended)).toBe(true);
      await session.page.locator('#source-preview .sp-scrub').focus();
      await session.page.keyboard.press('Home');
      await session.page.keyboard.press('Escape');
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});


test('an original ProRes monitor survives seeks, resize and playback with a legacy rendition present', async ({ session }) => {
  await session.openEditor();
  await importFixture(session.page, 'prores422.mov');
  const { page } = session;
  await page.waitForFunction(() => [...(window as any).PM.assets.map.values()].some((a: any) => a.name === 'prores422.mov'));
  const asset = await page.evaluate(() => {
    const PM = (window as any).PM;
    const asset = Object.values<any>(PM.proj.assets).find(a => a.name === 'prores422.mov');
    PM.Kernel.services.get('viewer').preview.show(asset.id);
    return { rendition: asset.rendition, duration: asset.dur };
  });
  expect(asset.rendition).toBe(true);
  // The fixture is red for its first second and green for the next. The last
  // request uses the stored metadata's duration; the monitor clamps it to the
  // original's own endpoint (2 s) and holds the final, green picture.
  for (const [width, time, shown, red] of [[320, .25, .25, true], [240, .5, .5, true], [320, 0, 0, true], [240, asset.duration, 2, false]] as const) {
    await page.evaluate(async ({ width, time }) => {
      const PM = (window as any).PM, source = PM.EngineSourceMedia.monitorValue;
      const root = document.querySelector<HTMLElement>('#source-preview')!;
      root.style.width = `${width}px`;
      const ready = new Promise<void>((resolve, reject) => {
        const timer = setTimeout(() => { source.removeEventListener('loadeddata', done); reject(new Error('ProRes paint timed out')); }, 10_000);
        const done = () => { clearTimeout(timer); source.removeEventListener('loadeddata', done); resolve(); };
        source.addEventListener('loadeddata', done);
      });
      source.currentTime = time;
      await ready;
    }, { width: width!, time: time! });
    await expect.poll(() => page.evaluate(() => {
      const source = (window as any).PM.EngineSourceMedia.monitorValue;
      const video = source.surface as HTMLVideoElement;
      const canvas = document.createElement('canvas'); canvas.width = 1; canvas.height = 1;
      const ctx = canvas.getContext('2d')!; ctx.drawImage(video, 0, 0, 1, 1);
      const pixel = ctx.getImageData(0, 0, 1, 1).data;
      return { time: source.currentTime, error: source.error?.message ?? null,
        native: video.srcObject instanceof MediaStream && !video.currentSrc,
        red: pixel[0]! > 200 && pixel[2]! < 40, green: pixel[1]! > 200 && pixel[2]! < 40,
        sized: source.paintedSize?.width === Math.round(video.clientWidth * devicePixelRatio) };
    }), { message: 'ProRes original is visibly painted at the resized source time' })
      .toEqual({ time: shown, error: null, native: true, red, green: !red, sized: true });
  }
  await page.evaluate(() => {
    const source = (window as any).PM.EngineSourceMedia.monitorValue;
    source.currentTime = 0; source.play();
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.EngineSourceMedia.monitorValue.currentTime)).toBeGreaterThan(.05);
  await page.evaluate(() => (window as any).PM.EngineSourceMedia.monitorValue.pause());
  expect(await page.evaluate(() => (window as any).PM.EngineSourceMedia.monitorValue.paused)).toBe(true);
  await page.evaluate(() => {
    const source = (window as any).PM.EngineSourceMedia.monitorValue;
    (window as any).__proresEnded = false;
    source.addEventListener('ended', () => { (window as any).__proresEnded = true; });
    source.currentTime = source.duration - .05; source.play();
  });
  await expect.poll(() => page.evaluate(() => (window as any).__proresEnded)).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
