import { expect, type Page } from '@playwright/test';
import { test } from './helpers/app';

async function expectTimelineScheme(page: Page, scheme: 'light' | 'dark') {
  await expect.poll(() => page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const canvas = timeline.cv as HTMLCanvasElement;
    const context = canvas.getContext('2d')!;
    const probe = document.createElement('canvas').getContext('2d', { willReadFrequently: true })!;
    const style = getComputedStyle(document.documentElement);
    const reversed = document.documentElement.dataset.timelineSurfaces === 'reversed';
    const matches = (x: number, token: string) => {
      probe.clearRect(0, 0, 1, 1);
      probe.fillStyle = style.getPropertyValue(token).trim();
      probe.fillRect(0, 0, 1, 1);
      const expected = probe.getImageData(0, 0, 1, 1).data;
      // Sample empty lanes away from gridlines, labels, and the playhead.
      const actual = context.getImageData(Math.round(x * timeline.dpr), Math.round((timeline.ruler + 50) * timeline.dpr), 1, 1).data;
      return actual.every((value, index) => value === expected[index]);
    };
    return {
      scheme: PM.theme.current,
      gutter: matches(15, reversed ? '--bg-panel-2' : '--bg-panel'),
      tracks: matches(timeline.gut + 23, reversed ? '--bg-panel' : '--bg-panel-2'),
    };
  })).toEqual({ scheme, gutter: true, tracks: true });
}

for (const scheme of ['light', 'dark'] as const) {
  test(`theme toggle repaints the timeline in ${scheme} mode`, async ({ session }, testInfo) => {
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM.Kernel.services.get('timeline')?.cv));
    await page.evaluate((scheme) => {
      const PM = (window as any).PM;
      const opposite = scheme === 'light' ? 'dark' : 'light';
      PM.theme.apply(opposite);
      // Establish a matching initial palette even on the pre-fix build.
      PM.theme.apply(opposite);
      PM.theme.toggle();
    }, scheme);

    await expectTimelineScheme(page, scheme);
    await testInfo.attach(`timeline-${scheme}`, { body: await page.locator('#tl-canvas').screenshot(), contentType: 'image/png' });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}

test('system appearance repaints the timeline and respects an explicit preference', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM.Kernel.services.get('timeline')?.cv));
  await page.emulateMedia({ colorScheme: 'light' });
  await page.evaluate(() => (window as any).PM.Kernel.setScheme('system'));
  await expectTimelineScheme(page, 'light');
  for (const colorScheme of ['dark', 'light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectTimelineScheme(page, colorScheme);
  }
  await page.evaluate(() => (window as any).PM.Kernel.setScheme('light'));
  await expectTimelineScheme(page, 'light');
  await page.emulateMedia({ colorScheme: 'light' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await expectTimelineScheme(page, 'light');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
