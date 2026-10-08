import { expect, test } from './helpers/app';

test('text color follows the pointer throughout a drag', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Text color drag', dur: 5 }));
    const layer = PM.mkLayer('text', { name: 'Text', dur: 5, d: { text: 'Color', color: '#F2F2F2' } });
    PM.proj.layers.push(layer);
    PM.selectLayers(layer.id);
    PM.bus.emit('layers');
    PM.invalidate();
  });
  const row = page.locator('#panel-inspector .row').filter({ has: page.locator('[data-property-path="c.color"]') });
  await row.locator('.color-field').click();
  const picker = page.locator('.color-picker');
  await picker.evaluate(element => Promise.all(element.getAnimations().map(animation => animation.finished)));
  const plane = (await picker.getByRole('slider', { name: 'Saturation and brightness' }).boundingBox())!;
  const color = () => page.evaluate(() => (window as any).PM.proj.layers[0].d.color);
  await page.mouse.move(plane.x + plane.width * .25, plane.y + plane.height * .25);
  await page.mouse.down();
  await expect.poll(color).not.toBe('#F2F2F2');
  const first = await color();
  await page.mouse.move(plane.x + plane.width * .75, plane.y + plane.height * .5, { steps: 5 });
  await expect.poll(color).not.toBe(first);
  await page.mouse.up();
  for (const name of ['Saturation and brightness', 'Hue', 'Opacity']) {
    const slider = picker.getByRole('slider', { name, exact: true });
    const box = (await slider.boundingBox())!;
    await page.mouse.move(box.x + box.width * .5, box.y + box.height * .3);
    await page.mouse.down();
    const start = await color();
    await page.mouse.move(box.x + box.width * .7, box.y + box.height * .7, { steps: 8 });
    await expect.poll(color).not.toBe(start);
    await page.mouse.up();
  }
  await picker.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(picker).toHaveCount(0);
  await page.keyboard.press('ControlOrMeta+z');
  await expect.poll(color).toBe('#F2F2F2');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
