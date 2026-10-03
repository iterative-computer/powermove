import { expect, test } from './helpers/app';

test('the agent overlay leaves the titlebar grab area draggable', async ({ session }) => {
  const { page } = session;
  await session.openEditor();
  await expect(page.locator('#agent-popover-root')).toHaveCount(1);
  await expect(page.locator('#agent-launcher')).toBeVisible();

  // Native app regions use rectangles, even for pointer-events:none overlays.
  // elementsFromPoint() misses those overlays, so inspect their bounds too.
  const blockingRegions = await page.evaluate(() => {
    const grab = document.querySelector('#titlebar > .titlebar-drag:not(.titlebar-drag-traffic)')!;
    const box = grab.getBoundingClientRect();
    const x = box.x + box.width / 2, y = box.y + box.height / 2;
    return [...document.querySelectorAll('body *')].filter(element => {
      const style = getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      return style.getPropertyValue('-webkit-app-region') === 'no-drag'
        && style.display !== 'none' && style.visibility !== 'hidden'
        && rect.left <= x && rect.right > x && rect.top <= y && rect.bottom > y;
    }).map(element => element.id || element.className);
  });
  expect(blockingRegions).toEqual([]);
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Settings', exact: true })).toBeVisible();
});
