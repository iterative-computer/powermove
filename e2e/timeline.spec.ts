import { expect } from '@playwright/test';
import { test } from './helpers/app';

test('clicking the ruler moves the playhead', async ({ session }) => {
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.TL?.cv));
  await page.waitForTimeout(800);
  const box = await page.locator('#tl-canvas').boundingBox();
  if (!box) throw new Error('no canvas');
  const gut = await page.evaluate(() => (window as any).PM.TL.gut);
  const ruler = await page.evaluate(() => (window as any).PM.TL.ruler);
  const before = await page.evaluate(() => (window as any).PM.time);
  // Click mid-ruler (below the thin work bar) well into the track area.
  await page.mouse.click(box.x + gut + 300, box.y + ruler - 6);
  const after = await page.evaluate(() => (window as any).PM.time);
  expect(after).not.toBe(before);
  expect(after).toBeGreaterThan(0);
  // Work area must be untouched by the scrub click.
  const work = await page.evaluate(() => (window as any).PM.proj.work);
  expect(work?.[0] ?? 0).toBe(0);
});
