import { expect, test } from './helpers/app';

test('keeps the timeline usable after double-clicks and a saved collapsed state', async ({ session }) => {
  await session.openEditor();
  const { page, diagnostics } = session;
  const panel = page.locator('#panel-timeline');
  const canvas = page.locator('#tl-canvas');
  await expect(canvas).toBeVisible();
  const originalHeight = await panel.evaluate(el => el.getBoundingClientRect().height);

  // Exercise real pointer sequences on the ruler, content, and empty toolbar.
  const bounds = (await canvas.boundingBox())!;
  await canvas.dblclick({ position: { x: bounds.width - 40, y: 10 } });
  await canvas.dblclick({ position: { x: bounds.width - 40, y: Math.min(bounds.height - 10, 90) } });
  await page.locator('#tl-head').dblclick({ position: { x: 2, y: 14 } });
  await expect(canvas).toBeVisible();
  expect(await panel.evaluate(el => el.getBoundingClientRect().height)).toBeCloseTo(originalHeight, 0);

  // Older panel shells and saved workspaces could collapse the hidden header.
  await panel.locator(':scope > header').dispatchEvent('dblclick');
  await expect(canvas).toBeVisible();
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.WS.mutate((ws: any) => {
      PM.Layout.findPanel(ws, 'timeline').spec.collapsed = true;
    });
  });
  await expect(canvas).toBeVisible();
  await expect(panel).toHaveAttribute('data-collapsed', '0');
  expect(await panel.evaluate(el => el.getBoundingClientRect().height)).toBeCloseTo(originalHeight, 0);
  expect(diagnostics.pageErrors).toEqual([]);
});
