import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

/** Project with a group, overlapping video and a separate audio clip. */
async function seed(page: import('playwright').Page) {
  return page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ w: 640, h: 360, dur: 10 });
    const mk = (type: string, name: string, from: number, dur: number) => PM.mkLayer(type, { name, from, dur }, project);
    const title = mk('text', 'Title', 1, 2);
    const lower = mk('text', 'Lower third', 5, 2);
    const broll = mk('video', 'B-roll', 6, 3);
    const base = mk('video', 'Interview', 0, 10);
    const music = mk('audio', 'Music', 0, 10);
    project.layers = [title, lower, broll, base, music];
    PM.replaceProject(project);
    const group = PM.groupLayers([title.id, lower.id], 'Titles');
    PM.UIState.setGroupCollapsed(group, false);
    PM.selectLayers([]);
    const timeline = PM.Kernel.services.get('timeline');
    timeline.frameView();
    return { title: title.id, lower: lower.id, broll: broll.id, base: base.id, music: music.id, group: group.id };
  });
}

const lanes = (page: import('playwright').Page) => page.evaluate(() => {
  const timeline = (window as any).PM.Kernel.services.get('timeline');
  return Object.fromEntries(timeline.trackView.layout().items.map((item: any) => [item.id, `${item.area[0]}${item.lane}`]));
});

/** Client point at a composition time on a track lane. */
const point = (page: import('playwright').Page, time: number, area: 'video' | 'audio', lane: number) => page.evaluate(({ time, area, lane }) => {
  const timeline = (window as any).PM.Kernel.services.get('timeline');
  const rect = timeline.cv.getBoundingClientRect();
  return {
    x: rect.left + timeline.gut + (time - timeline.scrollT) * timeline.pps,
    y: rect.top + timeline.trackView.laneY(area, lane) + timeline.trackView.rowHeight() / 2,
  };
}, { time, area, lane });

test('the track timeline packs layers onto Premiere-style tracks and moves clips between them', async ({ session }) => {
  const { page } = session;
  const ids = await seed(page);
  await page.locator('#tl-mode [data-mode="tracks"]').click();
  await expect(page.locator('#tl-mode [data-mode="tracks"]')).toHaveAttribute('aria-checked', 'true');

  // Interview on V1; B-roll overlaps it and composites above, so V2; the
  // Titles block (members on V3, bar on V4) sits above B-roll; music on A1.
  await expect.poll(() => lanes(page)).toEqual({
    [ids.base]: 'v0', [ids.broll]: 'v1', [ids.group]: 'v3', [ids.title]: 'v2', [ids.lower]: 'v2', [ids.music]: 'a0',
  });

  // Drag B-roll up past the titles: it leaves V2 and becomes the top track,
  // which also moves it to the top of the layer stack.
  const from = await point(page, 7, 'video', 1);
  const to = await point(page, 7, 'video', 4);
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(from.x, from.y - 10, { steps: 2 });
  await page.mouse.move(to.x, to.y, { steps: 6 });
  await page.mouse.up();
  await expect.poll(() => lanes(page)).toMatchObject({ [ids.broll]: 'v4', [ids.base]: 'v0' });
  const stack = await page.evaluate(() => (window as any).PM.proj.layers.map((layer: any) => layer.id));
  expect(stack[0]).toBe(ids.broll);
  expect(await page.evaluate((id) => (window as any).PM.L(id).from, ids.broll)).toBeCloseTo(6, 5);

  // One undo restores both the stack order and the lanes.
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(() => lanes(page)).toMatchObject({ [ids.broll]: 'v1' });

  // Clicking the group bar's disclosure collapses it to a single nest clip.
  const bar = await point(page, 1, 'video', 3);
  await page.mouse.click(bar.x + 6, bar.y);
  await expect.poll(() => lanes(page)).toEqual({
    [ids.base]: 'v0', [ids.broll]: 'v1', [ids.group]: 'v2', [ids.music]: 'a0',
  });

  // The choice persists, and the layer timeline is one click away.
  await page.locator('#tl-mode [data-mode="layers"]').click();
  await expect(page.locator('button[title="Graph editor (Shift+F3)"]')).toBeVisible();
  expect(session.diagnostics.pageErrors).toEqual([]);
});
