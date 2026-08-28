import { expect, test } from './helpers/app';

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
