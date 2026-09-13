import { expect, test } from './helpers/app';

test('moves the Preview canvas through the shared panel drag affordance', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));

  const handle = page.getByRole('button', { name: 'Move Composition panel', exact: true });
  await expect(handle).toBeVisible();
  expect(await handle.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      left: style.left,
      top: style.top,
      transform: style.transform,
      background: style.backgroundColor,
      border: style.borderTopWidth,
      shadow: style.boxShadow
    };
  })).toEqual({
    left: '8px',
    top: '7px',
    transform: 'none',
    background: 'rgba(0, 0, 0, 0)',
    border: '0px',
    shadow: 'none'
  });
  await handle.hover();
  expect(await handle.evaluate((element) => {
    const style = getComputedStyle(element);
    return [style.backgroundColor, style.borderTopWidth, style.boxShadow];
  })).toEqual(['rgba(0, 0, 0, 0)', '0px', 'none']);
  const start = await handle.boundingBox();
  const target = await page.locator('#dock-right').boundingBox();
  expect(start).not.toBeNull();
  expect(target).not.toBeNull();

  await page.mouse.move(start!.x + start!.width / 2, start!.y + start!.height / 2);
  await page.mouse.down();
  await page.mouse.move(target!.x + target!.width / 2, target!.y + 12, { steps: 12 });
  await page.mouse.up();

  await expect(page.locator('#dock-right #panel-viewer')).toBeVisible();
  expect(session.diagnostics.pageErrors).toEqual([]);
});
