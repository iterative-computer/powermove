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

test('animated clips unfold their keyframes inside the track timeline', async ({ session }) => {
  const { page } = session;
  const ids = await seed(page);
  await page.evaluate((id) => {
    const PM = (window as any).PM;
    const layer = PM.L(id);
    layer.p.opacity.kf = [PM.KF(.5, 0, 'linear'), PM.KF(1.5, 100, 'linear')];
    PM.setKeyOn(layer.p['position.x'], 0, 100, 'ease', 30);
    PM.setKeyOn(layer.p['position.x'], 2, 400, 'ease', 30);
    PM.touch(); PM.invalidate();
  }, ids.broll);
  await page.locator('#tl-mode [data-mode="tracks"]').click();
  const height = () => page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').trackView.contentHeight());
  const folded = await height();

  // The clip's disclosure arrow, like a group bar's, unfolds one lane per
  // animated property beneath it.
  const clip = await point(page, 6, 'video', 1);
  await page.mouse.click(clip.x + 10, clip.y);
  await expect.poll(height).toBe(folded + 2 * 24);
  expect(await page.evaluate((id) => (window as any).PM.UIState.getLayerCollapsed((window as any).PM.L(id)), ids.broll)).toBe(false);

  // The Opacity key at 6.5s sits on its lane, under the clip.
  const opacityKey = () => page.evaluate(() => {
    const timeline = (window as any).PM.Kernel.services.get('timeline');
    const view = timeline.trackView, rect = timeline.cv.getBoundingClientRect();
    const x = timeline.gut + (6.5 - timeline.scrollT) * timeline.pps;
    for (let index = 0; index < 2; index++) {
      const y = view.laneY('video', 1) + view.rowHeight() + index * 24 + 12;
      if (view.rowAt(x, y)?.key === 'opacity') return { x: rect.left + x, y: rect.top + y, pps: timeline.pps };
    }
    return null;
  });
  expect(await opacityKey()).not.toBeNull();

  // One press selects the key (and its clip) and drags it, retiming it as in
  // the layer timeline. A drag cancels, restoring the key, if another test
  // window takes focus mid-gesture; repeat it until one runs uninterrupted.
  await expect(async () => {
    const key = (await opacityKey())!;
    await page.mouse.move(key.x, key.y);
    await page.mouse.down();
    await page.mouse.move(key.x + key.pps / 2, key.y, { steps: 5 });
    await page.evaluate(() => (window as any).PM.sel.keys.length);
    await page.mouse.up();
    expect(await page.evaluate((id) => (window as any).PM.L(id).p.opacity.kf[0].t, ids.broll)).toBeCloseTo(1, 1);
  }).toPass({ timeout: 15_000 });
  expect(await page.evaluate(() => (window as any).PM.sel.keys.length)).toBe(1);
  expect(await page.evaluate(() => (window as any).PM.sel.layers)).toEqual([ids.broll]);

  // Double-clicking the clip folds it again.
  await page.mouse.dblclick(clip.x + 60, clip.y);
  await expect.poll(height).toBe(folded);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
