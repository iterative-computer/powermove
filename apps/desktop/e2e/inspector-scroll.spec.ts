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
  // Properties shows every row in a side column; whichever of the panel body
  // and the column overflows is the one that scrolls.
  const body = page.locator('#panel-inspector > .body');
  const scroller = (element: Element): HTMLElement => {
    const own = element as HTMLElement;
    if (own.scrollHeight > own.clientHeight + 1) return own;
    const dock = own.closest<HTMLElement>('.dock');
    return dock && dock.scrollHeight > dock.clientHeight + 1 ? dock : own;
  };
  const position = () => body.evaluate((element, pick) => (new Function('return ' + pick)() as (e: Element) => HTMLElement)(element).scrollTop, scroller.toString());
  const scrollTo = (top: number | 'end') => body.evaluate((element, [pick, value]) => {
    const target = (new Function('return ' + pick)() as (e: Element) => HTMLElement)(element);
    target.scrollTop = value === 'end' ? target.scrollHeight : Number(value);
    return target.scrollTop;
  }, [scroller.toString(), top] as const);
  const select = async (index: number | null) => {
    await page.evaluate((index) => {
      const PM = (window as any).PM;
      PM.selectLayers(index === null ? [] : [PM.proj.layers[index].id]);
    }, index);
  };
  await expect(page.locator('#panel-inspector [data-inspector-layer]')).toBeVisible();
  const savedPosition = await scrollTo('end');
  expect(savedPosition).toBeGreaterThan(250);
  await select(null);
  await expect(page.locator('#panel-inspector [data-inspector-layer]')).toHaveCount(0);
  await expect.poll(position).toBeLessThan(savedPosition);
  await select(0);
  await expect.poll(position).toBe(savedPosition);

  await select(1);
  await expect.poll(position).toBe(0);
  await scrollTo(120);
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
