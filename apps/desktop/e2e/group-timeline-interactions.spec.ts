import { expect, test } from './helpers/app';

test.describe('@groups timeline selection and strip editing', () => {
  test('selecting a grouped layer opens a focused group, while disclosure restores all children', async ({ session }) => {
    const { page } = session;
    const ids = await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ w: 640, h: 360, dur: 8 });
      const first = PM.mkLayer('shape', { name: 'First card', from: 1, dur: 4 }, project);
      const second = PM.mkLayer('text', { name: 'Second title', from: 2, dur: 3 }, project);
      project.layers = [first, second];
      PM.replaceProject(project);
      const group = PM.groupLayers([first.id, second.id], 'Titles');
      PM.UIState.setLayerCollapsed(group, true);
      PM.selectLayers(first.id);
      return { group: group.id, first: first.id, second: second.id };
    });

    await page.waitForFunction(({ group, first }) => {
      const PM = (window as any).PM;
      return PM.UIState.getLayerCollapsed(PM.L(group)) === false
        && PM.TL.rows.filter((row: any) => row.kind === 'layer').map((row: any) => row.L.id).join(',') === `${group},${first}`;
    }, ids);

    const twirl = await page.evaluate(() => {
      const T = (window as any).PM.TL, rect = T.cv.getBoundingClientRect();
      return { x: rect.left + 64, y: rect.top + T.ruler + T.row / 2 - T.scrollY };
    });
    await page.mouse.click(twirl.x, twirl.y);
    await page.mouse.click(twirl.x, twirl.y);
    await page.waitForFunction(({ group, first, second }) => {
      const rows = (window as any).PM.TL.rows.filter((row: any) => row.kind === 'layer').map((row: any) => row.L.id);
      return rows.join(',') === `${group},${first},${second}`;
    }, ids);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('both edges of a selected group strip freely trim its members', async ({ session }) => {
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.TL?.cv));
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ w: 640, h: 360, fps: 30, dur: 8 });
      const first = PM.mkLayer('shape', { name: 'First', from: 1, dur: 4 }, project);
      const second = PM.mkLayer('text', { name: 'Second', from: 2, dur: 3 }, project);
      project.layers = [first, second];
      PM.replaceProject(project);
      const group = PM.groupLayers([first.id, second.id], 'Titles');
      PM.selectLayers(group.id);
      PM.TL.scrollT = 0;
      PM.invalidate('timeline');
    });

    const strip = await page.evaluate(() => {
      const PM = (window as any).PM, T = PM.TL, rect = T.cv.getBoundingClientRect(), group = PM.firstSel(), span = PM.groupSpan(group);
      return { left: rect.left + T.gut + (span.from - T.scrollT) * T.pps, right: rect.left + T.gut + (span.from + span.dur - T.scrollT) * T.pps, y: rect.top + T.ruler + T.row / 2 - T.scrollY, pps: T.pps };
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
