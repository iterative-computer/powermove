import { expect } from '@playwright/test';
import { inflateSync } from 'node:zlib';
import { test } from './helpers/app';

/** RGB of a 1×1 screenshot. One pixel has no neighbours, so every PNG row filter leaves it as stored. */
function pixel(png: Buffer): number[] {
  const data: Buffer[] = [];
  for (let offset = 8; offset < png.length;) {
    const length = png.readUInt32BE(offset);
    if (png.toString('ascii', offset + 4, offset + 8) === 'IDAT') data.push(png.subarray(offset + 8, offset + 8 + length));
    offset += length + 12;
  }
  return [...inflateSync(Buffer.concat(data)).subarray(1, 4)];
}

test('colour alpha from the picker renders see-through and survives Undo', async ({ session }, testInfo) => {
  await session.openEditor();
  const { app, page } = session;
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]?.setSize(1100, 700));
  await page.waitForFunction(() => Boolean((window as any).PM.Kernel.services.get('timeline')?.cv));
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Colour alpha', dur: 5, bg: '#000000' }));
    const layer = PM.mkLayer('shape', { name: 'Red', dur: 5, d: { shape: 'rect', color: '#FF0000', w: 1600, h: 900 } });
    PM.proj.layers.push(layer); PM.selectLayers(layer.id); PM.bus.emit('layers'); PM.invalidate();
  });

  const sample = async () => {
    // Off-centre, clear of the selection gizmo, well inside the 1600×900 rect.
    const box = (await page.locator('#viewer-stage, #stage').first().boundingBox())!;
    const png = await page.screenshot({ clip: { x: Math.round(box.x + box.width * .42), y: Math.round(box.y + box.height * .58), width: 1, height: 1 } });
    return pixel(png);
  };
  // Screenshots arrive in the display's colour profile, so compare against the opaque red.
  await expect.poll(async () => (await sample())[0]).toBeGreaterThan(200);
  const opaque = await sample();

  const fillRow = page.locator('#panel-inspector .row').filter({ has: page.locator('[data-property-path="c.color"]') });
  await fillRow.locator('.color-field').click();
  const opacity = page.getByRole('slider', { name: 'Opacity', exact: true });
  await expect(opacity).toBeVisible();
  // Measure at rest: the popover scales in from the trigger's edge first.
  await page.locator('.color-picker').evaluate(element => Promise.all(element.getAnimations().map(animation => animation.finished)));
  const track = (await opacity.boundingBox())!;
  await page.mouse.click(track.x + track.width / 2, track.y + track.height / 2);
  await expect(page.getByRole('textbox', { name: 'Fill hex value', exact: true })).toHaveValue('#FF0000 / 50%');
  await page.screenshot({ path: testInfo.outputPath('color-alpha-picker.png') });
  await page.keyboard.press('Enter');
  await expect(page.locator('.color-picker')).toHaveCount(0);

  expect(await page.evaluate(() => (window as any).PM.proj.layers[0].d.color)).toBe('#FF000080');
  await expect(fillRow.locator('.color-field')).toContainText('#FF000050%');
  await expect.poll(async () => (await sample())[0]).toBeLessThan(opaque[0]! * .8);
  const [red] = await sample();
  expect(red).toBeGreaterThan(opaque[0]! * .3);

  await page.keyboard.press('Meta+z');
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers[0].d.color)).toBe('#FF0000');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
