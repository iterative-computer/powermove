import { expect, test } from './helpers/app';

test('moving a group in a large timeline expands selection once and remains undoable', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ w: 640, h: 360, dur: 6 });
    const group = PM.mkLayer('group', { name: 'Moving group' }, project);
    group.from = 1; group.dur = 2;
    project.layers = [group, ...Array.from({ length: 500 }, (_, index) => {
      const layer = PM.mkLayer('solid', { name: `Layer ${index}` }, project);
      layer.from = 1; layer.dur = 2;
      if (index < 5) layer.group = group.id;
      return layer;
    })];
    PM.replaceProject(project);
    PM.selectLayers([group.id]);
    const timeline = PM.Kernel.services.get('timeline');
    timeline.scrollT = timeline.scrollY = 0;
    timeline.pps = 100;
    PM.invalidate();
  });
  await page.waitForFunction(() => {
    const PM = (window as any).PM;
    return PM.Kernel.services.get('timeline').rows[0]?.L.name === 'Moving group';
  });
  const point = await page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    const original = PM.expandGroups;
    (window as any).__gestureExpandCount = 0;
    PM.expandGroups = function (...args: any[]) {
      (window as any).__gestureExpandCount++;
      return original.apply(this, args);
    };
    return { x: box.x + timeline.gut + 200, y: box.y + timeline.ruler + timeline.row / 2 };
  });
  await page.mouse.move(point.x, point.y);
  await page.mouse.down();
  expect(await page.evaluate(() => (window as any).__gestureExpandCount)).toBe(1);
  await page.mouse.move(point.x + 100, point.y, { steps: 5 });
  await page.mouse.up();
  const positions = () => page.evaluate(() => (window as any).PM.proj.layers.map((layer: any) => layer.from));
  expect(await positions()).toEqual([...Array(6).fill(2), ...Array(495).fill(1)]);
  await page.keyboard.press('Meta+z');
  await expect.poll(positions).toEqual(Array(501).fill(1));
  expect(session.diagnostics.pageErrors).toEqual([]);
});
