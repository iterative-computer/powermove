import { expect, test } from './helpers/app';

test('Properties remembers the scroll position of each layer', async ({ session }) => {
  const { page } = session;
  await session.openEditor();
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Inspector scroll' }));
    PM.proj.layers.push(PM.mkLayer('text'), PM.mkLayer('text'));
    PM.selectLayers([PM.proj.layers[0].id]);
    PM.bus.emit('layers');
  });
  const body = page.locator('#panel-inspector > .body');
  const position = () => body.evaluate((element) => element.scrollTop);
  const select = async (index: number | null) => {
    await page.evaluate((index) => {
      const PM = (window as any).PM;
      PM.selectLayers(index === null ? [] : [PM.proj.layers[index].id]);
    }, index);
  };
  await expect(page.locator('#panel-inspector [data-inspector-layer]')).toBeVisible();
  const savedPosition = await body.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
    return element.scrollTop;
  });
  expect(savedPosition).toBeGreaterThan(250);
  await select(null);
  await expect(page.locator('#panel-inspector [data-inspector-layer]')).toHaveCount(0);
  await expect.poll(position).toBeLessThan(savedPosition);
  await select(0);
  await expect.poll(position).toBe(savedPosition);

  await select(1);
  await expect.poll(position).toBe(0);
  await body.evaluate((element) => { element.scrollTop = 120; });
  await expect.poll(position).toBe(120);
  await select(0);
  await expect.poll(position).toBe(savedPosition);
  await select(1);
  await expect.poll(position).toBe(120);

  // Routine inspector refreshes must leave the current position alone.
  await page.evaluate(() => (window as any).PM.Kernel.services.get('inspector').refresh());
  await expect.poll(position).toBe(120);

  // A different project must not inherit positions, even if layer IDs match.
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const layers = structuredClone(PM.proj.layers);
    const project = PM.mkProject({ name: 'Another project' });
    project.layers = layers;
    PM.replaceProject(project);
    PM.selectLayers([layers[1].id]);
    PM.bus.emit('layers');
  });
  await expect.poll(position).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
