import { expect } from '@playwright/test';
import { test } from './helpers/app';

test('color picker stays above adjacent panels and inside the viewport', async ({ session }, testInfo) => {
  await session.openEditor();
  const { app, page } = session;
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]?.setSize(1000, 600));
  await page.waitForFunction(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    return Boolean(timeline?.cv);
  });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Color picker overlay', dur: 5 }));
    const layer = PM.mkLayer('shape', {
      name: 'Orange shape',
      dur: 5,
      d: { shape: 'rect', color: '#FF6B1A', w: 180, h: 120 }
    });
    PM.proj.layers.push(layer);
    PM.selectLayers(layer.id);
    PM.bus.emit('layers');
    PM.invalidate();
  });

  const fillRow = page.locator('#panel-inspector .row').filter({ has: page.locator('[data-property-path="c.color"]') });
  await fillRow.locator('.color-field').click();
  const picker = page.locator('.color-picker');
  await expect(picker).toBeVisible();
  const eyedropper = page.getByRole('button', { name: 'Sample screen color', exact: true });
  await expect(eyedropper).toBeVisible();
  expect(await page.evaluate(() => ({ secure: window.isSecureContext, available: typeof (window as any).EyeDropper === 'function' })))
    .toEqual({ secure: true, available: true });
  await page.evaluate(() => {
    Object.defineProperty(window, 'EyeDropper', {
      configurable: true,
      value: class { async open() { return { sRGBHex: '#0a84ff' }; } }
    });
  });
  await eyedropper.click();
  await expect(page.getByRole('textbox', { name: 'Fill hex value', exact: true })).toHaveValue('#0A84FF');

  const geometry = await picker.evaluate(element => {
    const rect = element.getBoundingClientRect();
    const layer = element.parentElement!;
    const bottomPoint = document.elementFromPoint(rect.left + rect.width / 2, rect.bottom - 4);
    return {
      overlayParent: layer.parentElement?.tagName,
      bottomInsidePicker: Boolean(bottomPoint && element.contains(bottomPoint)),
      insideViewport: rect.top >= 0 && rect.bottom <= window.innerHeight,
      width: Math.round(rect.width),
      height: Math.round(rect.height)
    };
  });
  expect(geometry).toMatchObject({ overlayParent: 'BODY', bottomInsidePicker: true, insideViewport: true });
  expect(geometry.width).toBeLessThanOrEqual(360);
  expect(geometry.height).toBeLessThanOrEqual(300);
  await page.screenshot({ path: testInfo.outputPath('color-picker-overlay.png') });
  // Exercise both sides and the constrained middle without depending on inspector row order.
  for (const top of [70, 520, 290]) {
    await page.getByRole('button', { name: 'Close color picker', exact: true }).click();
    const trigger = fillRow.locator('.color-field');
    await trigger.evaluate((element, y) => {
      Object.assign((element as HTMLElement).style, { position: 'fixed', top: `${y}px`, right: '24px' });
      (element as HTMLButtonElement).click();
    }, top);
    await expect(picker).toBeVisible();
    const placement = await picker.evaluate(element => {
      const button = document.querySelector('#panel-inspector .color-field')!.getBoundingClientRect();
      const popup = element.getBoundingClientRect();
      return {
        above: popup.bottom <= button.top - 5,
        below: popup.top >= button.bottom + 5,
        inside: popup.top >= 12 && popup.bottom <= window.innerHeight - 12
      };
    });
    expect(placement.inside).toBe(true);
    expect(placement.above || placement.below).toBe(true);
    if (top === 70) expect(placement.below).toBe(true);
    if (top === 520) expect(placement.above).toBe(true);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
