import { expect, test } from './helpers/app';

test('the Speed field resizes the timeline strip while preserving trim and Undo', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const id = await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ w: 640, h: 360, dur: 20, fps: 30 });
    const layer = PM.mkLayer('video', { name: 'Speed QA', from: 2, dur: 8, d: { trim: 3, speed: 1 } }, project);
    project.layers = [layer];
    PM.replaceProject(project); PM.selectLayers(layer.id); PM.setTime(2);
    PM.hist.clear(); PM.invalidate();
    return layer.id;
  });
  const speed = page.locator('[data-svelte-panel="inspector"]').getByRole('spinbutton', { name: 'Speed', exact: true });
  await page.locator('#tl-mode [data-mode="tracks"]').click();
  const stripWidth = () => page.evaluate(id => {
    const timeline = (window as any).PM.Kernel.services.get('timeline');
    const item = timeline.trackView.layout().items.find((item: any) => item.id === id);
    return item.dur * timeline.pps;
  }, id);
  const originalWidth = await stripWidth();
  const state = () => page.evaluate(id => {
    const PM = (window as any).PM, layer = PM.L(id);
    return { speed: PM.resolveContent(layer, PM.time).speed, duration: layer.dur, from: layer.from, trim: PM.resolveContent(layer, PM.time).trim };
  }, id);

  await speed.click(); await speed.fill('2'); await speed.press('Enter');
  await expect.poll(state).toEqual({ speed: 2, duration: 4, from: 2, trim: 3 });
  await expect.poll(stripWidth).toBeCloseTo(originalWidth / 2);
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(state).toEqual({ speed: 1, duration: 8, from: 2, trim: 3 });
  await page.evaluate(() => (window as any).PM.hist.redo());
  await expect.poll(state).toEqual({ speed: 2, duration: 4, from: 2, trim: 3 });

  await speed.click(); await speed.fill('0.5'); await speed.press('Enter');
  await expect.poll(state).toEqual({ speed: .5, duration: 16, from: 2, trim: 3 });
  await expect.poll(stripWidth).toBeCloseTo(originalWidth * 2);
  const box = (await speed.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 20, box.y + box.height / 2, { steps: 5 });
  await expect.poll(async () => (await state()).duration).toBeLessThan(16);
  await page.keyboard.press('Escape'); await page.mouse.up();
  await expect.poll(state).toEqual({ speed: .5, duration: 16, from: 2, trim: 3 });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
