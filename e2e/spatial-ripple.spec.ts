import { expect, test } from './helpers/app';

async function capturePixel(page: any, x: number, y: number): Promise<number[]> {
  return page.evaluate(async ({ x, y }: { x: number; y: number }) => {
    const bitmap = await (window as any).PM.WindowCapture.request();
    if (!bitmap) throw new Error('Window capture did not return an image');
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
    const context = canvas.getContext('2d', { willReadFrequently: true });
    if (!context) throw new Error('Could not read the captured window');
    context.drawImage(bitmap, 0, 0);
    const sampleX = Math.min(bitmap.width - 1, Math.max(0, Math.round(x * bitmap.width / window.innerWidth)));
    const sampleY = Math.min(bitmap.height - 1, Math.max(0, Math.round(y * bitmap.height / window.innerHeight)));
    const pixel = Array.from(context.getImageData(sampleX, sampleY, 1, 1).data);
    bitmap.close?.();
    return pixel;
  }, { x, y });
}

test('spatial Ripple renders through Motion GPU and cleans up', async ({ session }) => {
  const page = session.page;
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.activate(window.innerWidth / 2, window.innerHeight / 2);
  });

  const host = page.locator('#spatial-assistant .spatial-ripple-host');
  await expect(host).toBeVisible();
  await expect(host).toHaveAttribute('data-renderer', /motion-gpu(?:-settled)?/, { timeout: 10_000 });

  const canvas = host.locator('canvas[data-ripple-canvas]');
  await expect(canvas).toHaveCount(1);
  const dimensions = await canvas.evaluate((element: HTMLCanvasElement) => ({
    width: element.width,
    height: element.height,
    cssWidth: element.getBoundingClientRect().width,
    cssHeight: element.getBoundingClientRect().height,
  }));
  expect(dimensions.width).toBeGreaterThan(1);
  expect(dimensions.height).toBeGreaterThan(1);
  expect(dimensions.cssWidth).toBeGreaterThan(1);
  expect(dimensions.cssHeight).toBeGreaterThan(1);

  await page.evaluate(() => (window as any).PM.SpatialAssistant.cancel());
  await expect(page.locator('#spatial-assistant')).toHaveCount(0);
});

test('change mode preserves captured colors beneath the intentional dim wash', async ({ session }) => {
  const page = session.page;
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.evaluate(() => {
    const swatch = document.createElement('div');
    swatch.dataset.rippleColorSwatch = 'true';
    Object.assign(swatch.style, {
      position: 'fixed', inset: '0', zIndex: '450', pointerEvents: 'none',
      background: 'rgb(245, 174, 40)',
    });
    document.body.appendChild(swatch);
  });
  await page.evaluate(() => new Promise<void>((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  }));

  const samplePoint = { x: 96, y: 180 };
  const before = await capturePixel(page, samplePoint.x, samplePoint.y);
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.activate(window.innerWidth / 2, window.innerHeight / 2, {
      capture: PM.WindowCapture.request(),
    });
  });
  const host = page.locator('#spatial-assistant .spatial-ripple-host');
  await expect(host).toHaveAttribute('data-renderer', 'motion-gpu-settled', { timeout: 10_000 });
  await page.evaluate(() => new Promise<void>((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  }));
  const after = await capturePixel(page, samplePoint.x, samplePoint.y);

  const wash = [8, 8, 12];
  for (let channel = 0; channel < 3; channel += 1) {
    const expected = Math.round(before[channel] * 0.88 + wash[channel] * 0.12);
    expect(
      Math.abs(after[channel] - expected),
      `channel ${channel}: before=${before.join(',')} after=${after.join(',')} expected=${expected}`,
    ).toBeLessThanOrEqual(5);
  }

  await page.evaluate(() => (window as any).PM.SpatialAssistant.cancel());
});
