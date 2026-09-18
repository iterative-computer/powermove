import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

/* TIFF, HEIC and containers like Matroska never reach a Chromium decoder. The
   main process converts them on import, and the project stores the conversion. */

async function openFixtureSized(session: any) {
  await session.openEditor();
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', {
      detail: PM.mkProject({ w: 160, h: 90, fps: 30, dur: 3, name: 'Conversion test' }),
    }));
    PM.ProjectsScreen.hide();
    PM.setTime(0, { raw: true, force: true });
  });
}

async function centrePixel(page: any, time: number): Promise<number[]> {
  return page.evaluate(async (at: number) => {
    const PM = (window as any).PM;
    PM.setTime(at, { raw: true, force: true });
    let pixel: number[] = [];
    const original = PM.download;
    PM.download = async (blob: Blob) => {
      const image = await createImageBitmap(blob);
      const cv = document.createElement('canvas'); cv.width = 160; cv.height = 90;
      const ctx = cv.getContext('2d')!;
      ctx.clearRect(0, 0, 160, 90); ctx.drawImage(image, 0, 0); image.close();
      pixel = [...ctx.getImageData(80, 45, 1, 1).data];
    };
    try {
      const result = await PM.Export.run({ format: 'still', w: 160, h: 90, alpha: true, mblur: false });
      if (result.error) throw new Error(result.error);
      return pixel;
    } finally { PM.download = original; }
  }, time);
}

for (const file of ['still.tiff', 'still.heic']) {
  test(`a ${file.split('.').pop()!.toUpperCase()} still converts on import and renders its colour`, async ({ session }) => {
    test.setTimeout(120_000);
    await openFixtureSized(session);
    const { page } = session;

    await importFixture(page, file);
    await page.waitForFunction(name => (window as any).PM.proj.layers.some((l: any) => l.name === name), file);

    const imported = await page.evaluate(name => {
      const PM = (window as any).PM;
      const asset = PM.assets.get(PM.proj.layers.find((l: any) => l.name === name).d.asset);
      return { kind: asset.kind, format: asset.format, w: asset.w, h: asset.h };
    }, file);
    expect(imported).toEqual({ kind: 'image', format: file.split('.').pop(), w: 160, h: 90 });

    const pixel = await centrePixel(page, 0);
    expect(pixel[0]).toBeGreaterThan(200);
    expect(pixel[1]).toBeLessThan(60);
    expect(pixel[2]).toBeLessThan(60);
    expect(pixel[3]).toBe(255);

    // The PNG conversion is what the project keeps, not the original container.
    expect(await page.evaluate(() => {
      const PM = (window as any).PM, asset: any = Object.values(PM.proj.assets)[0];
      return PM.MediaStore.get(asset.storageKey).then((blob: Blob | null) => blob?.type);
    })).toBe('image/png');
  });
}

test('a Matroska video converts on import and plays both of its colours', async ({ session }) => {
  test.setTimeout(180_000);
  await openFixtureSized(session);
  const { page } = session;

  await importFixture(page, 'converted.mkv');
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.name === 'converted.mkv'), undefined, { timeout: 120_000 });

  const imported = await page.evaluate(() => {
    const PM = (window as any).PM;
    const asset = PM.assets.get(PM.proj.layers.find((l: any) => l.name === 'converted.mkv').d.asset);
    return { kind: asset.kind, w: asset.w, h: asset.h, dur: asset.dur, playbackProxy: asset.playbackProxy, hasAudio: asset.hasAudio };
  });
  expect(imported).toMatchObject({ kind: 'video', w: 160, h: 90, playbackProxy: true, hasAudio: true });
  expect(imported.dur).toBeCloseTo(2, 1);

  const red = await centrePixel(page, 0.5);
  expect(red[0]).toBeGreaterThan(180);
  expect(red[1]).toBeLessThan(80);
  const green = await centrePixel(page, 1.5);
  expect(green[1]).toBeGreaterThan(180);
  expect(green[0]).toBeLessThan(80);

  expect(await page.evaluate(() => {
    const PM = (window as any).PM, asset: any = Object.values(PM.proj.assets)[0];
    return PM.MediaStore.get(asset.storageKey).then((blob: Blob | null) => blob?.type);
  })).toBe('video/webm');
});
