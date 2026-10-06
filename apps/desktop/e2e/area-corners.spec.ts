import type { Page } from '@playwright/test';

import { chooseNativeMenu, expect, test } from './helpers/app';

const IDS = ['area-a', 'area-b', 'area-c'];
const SPARE = 'area-spare';

async function setup(page: Page): Promise<void> {
  await page.evaluate(({ ids, spare }) => {
    const PM = (window as any).PM;
    for (const id of [...ids, spare]) {
      if (PM.PANELS[id]) continue;
      PM.registerPanel(id, {
        title: id.replace('area-', 'Area ').toUpperCase(),
        build(body: HTMLElement) { body.textContent = id; }
      });
    }
    for (const id of [...ids, spare]) PM.Layout.rememberPanelOpen(id);
    PM.WS.mutate((workspace: any) => {
      for (const dock of workspace.layout.docks) dock.panels = dock.panels.filter((panel: any) => ![...ids, spare].includes(panel.id));
      workspace.hiddenPanels = (workspace.hiddenPanels || []).filter((item: any) => ![...ids, spare].includes(item.id));
      const right = PM.Layout.ensureDock(workspace, 'right');
      right.hidden = false;
      right.panels = [{ id: ids[0], size: 260 }, { id: ids[1], flex: true }, { id: ids[2], size: 220 }];
    });
  }, { ids: IDS, spare: SPARE });
  await expect(page.locator('#dock-right .panel')).toHaveCount(3);
}

const order = (page: Page) => page.evaluate(() => [...document.querySelectorAll<HTMLElement>('#dock-right .panel[data-panel]')].map(panel => panel.dataset.panel));
const hidden = (page: Page) => page.evaluate(() => ((window as any).PM.WS.current.hiddenPanels || []).map((item: any) => item.id));

async function cornerDrag(page: Page, id: string, corner: string, to: (rect: { x: number; y: number; width: number; height: number }) => { x: number; y: number }, target = id, shot?: string): Promise<void> {
  const handle = page.locator(`#panel-${id} > .panel-corner[data-corner="${corner}"]`);
  const box = (await handle.boundingBox())!;
  const rect = (await page.locator(`#panel-${target}`).boundingBox())!;
  const point = to(rect);
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(point.x, point.y, { steps: 8 });
  await page.waitForTimeout(120);
  if (shot) await page.screenshot({ path: shot });
  await page.mouse.up();
}

test.describe('@area-corners Blender-style area corners', () => {
  test('corners join, dock, replace and split panels', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await setup(page);
    await expect(page.locator('#panel-area-a > .panel-corner')).toHaveCount(4);

    // Join: drag A's bottom corner into its neighbour B. B closes, A grows.
    const before = (await page.locator('#panel-area-a').boundingBox())!.height;
    await expect(async () => {
      await cornerDrag(page, 'area-a', 'bottom-right', r => ({ x: r.x + r.width / 2, y: r.y + r.height / 2 }), 'area-b', '/tmp/area-join.png');
      expect(await order(page)).toEqual(['area-a', 'area-c']);
    }).toPass({ timeout: 15000 });
    expect(await hidden(page)).toContain('area-b');
    expect((await page.locator('#panel-area-a').boundingBox())!.height).toBeGreaterThan(before + 100);

    // Dock: A's corner into the top band of the center viewer moves A above it.
    await setup(page);
    await expect(async () => {
      await cornerDrag(page, 'area-a', 'top-left', r => ({ x: r.x + r.width / 2, y: r.y + 30 }), 'viewer', '/tmp/area-dock.png');
      expect(await page.evaluate(() => [...document.querySelectorAll<HTMLElement>('#dock-center .panel[data-panel]')].map(p => p.dataset.panel).slice(0, 2))).toEqual(['area-a', 'viewer']);
    }).toPass({ timeout: 15000 });

    // Replace: C's corner into the middle of A takes A's slot; A is hidden.
    await setup(page);
    await expect(async () => {
      await cornerDrag(page, 'area-c', 'top-left', r => ({ x: r.x + r.width / 2, y: r.y + r.height / 2 }), 'area-a', '/tmp/area-replace.png');
      expect(await order(page)).toEqual(['area-c', 'area-b']);
    }).toPass({ timeout: 15000 });
    expect(await hidden(page)).toContain('area-a');

    // Split: dragging inside B offers closed panels for the new part.
    await setup(page);
    await expect(async () => {
      await chooseNativeMenu(session, 'AREA SPARE', () =>
        cornerDrag(page, 'area-b', 'top-right', r => ({ x: r.x + r.width / 2, y: r.y + 130 }), 'area-b', '/tmp/area-split.png'));
      expect(await order(page)).toEqual(['area-a', SPARE, 'area-b', 'area-c']);
    }).toPass({ timeout: 15000 });
    expect((await page.locator(`#panel-${SPARE}`).boundingBox())!.height).toBeGreaterThan(100);

    // Escape cancels without changing the layout.
    await setup(page);
    const handle = (await page.locator('#panel-area-a > .panel-corner[data-corner="bottom-left"]').boundingBox())!;
    await page.mouse.move(handle.x + 4, handle.y + 4);
    await page.mouse.down();
    await page.mouse.move(handle.x + 40, handle.y + 200, { steps: 6 });
    await page.keyboard.press('Escape');
    await page.mouse.up();
    expect(await order(page)).toEqual(IDS);
    await expect(page.locator('.area-op-shade')).toHaveCount(0);
  });
});
