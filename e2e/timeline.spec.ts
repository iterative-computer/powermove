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

test('clicking the transport button pauses playback', async ({ session }) => {
  const { page } = session;
  const transport = page.getByRole('button', { name: 'Play / Pause (Space)' });

  await transport.click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.playing)).toBe(true);
  await expect.poll(() => page.evaluate(() => (window as any).PM.time)).toBeGreaterThan(0);

  const box = await transport.boundingBox();
  if (!box) throw new Error('no transport button');
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.waitForTimeout(100);
  await page.mouse.up();
  await expect.poll(() => page.evaluate(() => (window as any).PM.playing)).toBe(false);
  const pausedAt = await page.evaluate(() => (window as any).PM.time);
  await page.waitForTimeout(150);
  expect(await page.evaluate(() => (window as any).PM.time)).toBe(pausedAt);
});
