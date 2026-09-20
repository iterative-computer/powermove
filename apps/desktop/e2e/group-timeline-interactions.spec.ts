import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

test.describe('@groups timeline selection and strip editing', () => {
  test('selecting grouped layers reveals ancestors without hiding siblings', async ({ session }) => {
    const { page } = session;
    const ids = await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ w: 640, h: 360, dur: 8 });
      const first = PM.mkLayer('shape', { name: 'First card', from: 1, dur: 4 }, project);
      const second = PM.mkLayer('text', { name: 'Second title', from: 2, dur: 3 }, project);
      project.layers = [first, second];
      PM.replaceProject(project);
      const group = PM.groupLayers([first.id, second.id], 'Titles');
      PM.UIState.setGroupCollapsed(group, true);
      PM.selectLayers(first.id);
      return { group: group.id, first: first.id, second: second.id };
    });

    await page.waitForFunction(({ group, first, second }) => {
      const PM = (window as any).PM;
      const timeline = PM.Kernel.services.get('timeline');
      return PM.UIState.getGroupCollapsed(PM.L(group)) === false
        && timeline.rows.filter((row: any) => row.kind === 'layer').map((row: any) => row.L.id).join(',') === `${group},${first},${second}`;
    }, ids);

    for (const selected of [ids.second, ids.group, ids.first]) {
      await page.evaluate(id => (window as any).PM.selectLayers(id), selected);
      await expect.poll(() => page.evaluate(() => {
        const PM = (window as any).PM;
        const timeline = PM.Kernel.services.get('timeline');
        return timeline.rows.filter((row: any) => row.kind === 'layer').map((row: any) => row.L.id);
      })).toEqual([ids.group, ids.first, ids.second]);
    }

    const clickDisclosure = async () => {
      // Row state can update before the next canvas/layout frame clamps scroll.
      // Click the presented disclosure at its current position each time.
      await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
      const twirl = await page.evaluate(() => {
        const PM = (window as any).PM;
        const timeline = PM.Kernel.services.get('timeline');
        const rect = timeline.cv.getBoundingClientRect();
        return { x: rect.left + 64, y: rect.top + timeline.ruler + timeline.row / 2 - timeline.scrollY };
      });
      await page.mouse.click(twirl.x, twirl.y);
    };
    await clickDisclosure();
    await expect.poll(() => page.evaluate(() => {
      const PM = (window as any).PM;
      const timeline = PM.Kernel.services.get('timeline');
      return timeline.rows.filter((row: any) => row.kind === 'layer').map((row: any) => row.L.id);
    })).toEqual([ids.group]);
    await clickDisclosure();
    await page.waitForFunction(({ group, first, second }) => {
      const PM = (window as any).PM;
      const timeline = PM.Kernel.services.get('timeline');
      const rows = timeline.rows.filter((row: any) => row.kind === 'layer').map((row: any) => row.L.id);
      return rows.join(',') === `${group},${first},${second}`;
    }, ids);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('both edges of a selected group strip freely trim its members', async ({ session }) => {
    const { page } = session;
    await page.waitForFunction(() => { const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline'); return Boolean(timeline?.cv); });
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ w: 640, h: 360, fps: 30, dur: 8 });
      const first = PM.mkLayer('shape', { name: 'First', from: 1, dur: 4 }, project);
      const second = PM.mkLayer('text', { name: 'Second', from: 2, dur: 3 }, project);
      project.layers = [first, second];
      PM.replaceProject(project);
      const group = PM.groupLayers([first.id, second.id], 'Titles');
      PM.selectLayers(group.id);
      const timeline = PM.Kernel.services.get('timeline');
      timeline.scrollT = 0;
      PM.invalidate('timeline');
    });

    const strip = await page.evaluate(() => {
      const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline'), rect = timeline.cv.getBoundingClientRect(), group = PM.firstSel(), span = PM.groupSpan(group);
      return { left: rect.left + timeline.gut + (span.from - timeline.scrollT) * timeline.pps, right: rect.left + timeline.gut + (span.from + span.dur - timeline.scrollT) * timeline.pps, y: rect.top + timeline.ruler + timeline.row / 2 - timeline.scrollY, pps: timeline.pps };
    });
    await page.mouse.move(strip.right, strip.y);
    await page.mouse.down();
    await page.mouse.move(strip.right + strip.pps, strip.y, { steps: 8 });
    await page.mouse.up();
    await page.mouse.move(strip.left, strip.y);
    await page.mouse.down();
    await page.mouse.move(strip.left + strip.pps / 2, strip.y, { steps: 8 });
    await page.mouse.up();

    expect(await page.evaluate(() => {
      const PM = (window as any).PM, group = PM.firstSel();
      return {
        span: PM.groupSpan(group),
        members: PM.proj.layers.filter((layer: any) => layer.group === group.id).map((layer: any) => [layer.from, layer.dur]),
        history: PM.hist.list().slice(-2),
      };
    })).toEqual({ span: { from: 1.5, dur: 4.5 }, members: [[1.5, 4.5], [2.5, 3.5]], history: ['Trim clip', 'Trim clip'] });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
