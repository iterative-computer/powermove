import { mkdir } from 'node:fs/promises';
import { chooseNativeMenu, expect, test } from './helpers/app';

/* After Effects' 1 View / 2 Views / 4 Views: each pane chooses what it looks
   through, the selected pane is outlined and is the interactive viewer, and a
   click in another pane selects that pane and the layer under the pointer.
   Set PM_SHOT_DIR to also write the review screenshots. */

const shots = process.env.PM_SHOT_DIR;
async function shot(page: any, name: string, wait = 2500) {
  if (!shots) return;
  await mkdir(shots, { recursive: true });
  await page.waitForTimeout(wait);
  await page.screenshot({ path: `${shots}/after-${name}.png` });
}
const views = (page: any) => page.evaluate(() => (window as any).PM.Kernel.api('multi-view-e2e').scene3d.views.get());

test('1, 2 and 4 views: choose each pane’s view, select and move in another pane', async ({ session }) => {
  test.setTimeout(150_000);
  await session.openEditor();
  const page = session.page;
  // A fresh profile's storage notice sits over the viewer's top edge.
  await page.addStyleTag({ content: '#toasts{display:none!important}' });
  const box = await page.evaluate(() => {
    const PM = (window as any).PM; PM.pause(); PM.setTime(0);
    const api = PM.Kernel.api('multi-view-e2e');
    api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 4, y: 3, z: 6, targetX: 0, targetY: 0, targetZ: 0, fov: 45 } } });
    const id = api.scene3d.edit({ operation: 'add_object', object: { name: 'Box', source: { primitive: 'box' } } }).data.result.id;
    PM.selectLayers([]); PM.invalidate();
    return id;
  });
  // The layout menu sits in the viewer's bottom strip, beside the 3D view.
  const layout = page.getByRole('combobox', { name: 'View layout', exact: true });
  await expect(layout).toBeVisible();
  await expect(page.locator('.viewer-pane')).toHaveCount(0);
  await shot(page, '20-one-view');

  await layout.click();
  await page.locator('.pm-menu [role="option"], .pm-menu .di').filter({ hasText: '2 Views' }).first().click();
  await expect(page.locator('.viewer-pane')).toHaveCount(1);
  await expect(page.locator('#viewer-pane-frame')).toBeVisible();
  // Top on the left, the view you were looking through (selected) on the right.
  await expect(page.locator('.viewer-pane[data-pane="0"] .viewer-pane-label')).toHaveText('Top');
  await expect(page.locator('#stage > .viewer-pane-label')).toHaveText('Active Camera');
  expect((await views(page)).active).toBe(1);
  const stage = (await page.locator('#stage').boundingBox())!, side = (await page.locator('.viewer-pane[data-pane="0"]').boundingBox())!;
  expect(side.x + side.width).toBeLessThanOrEqual(stage.x + 1);
  expect(Math.abs(side.width - stage.width)).toBeLessThan(2);
  // The side pane gets its own Studio picture.
  await expect.poll(() => page.locator('.viewer-pane[data-pane="0"] canvas').evaluate((canvas: HTMLCanvasElement) => {
    const c = canvas.getContext('2d')!, { data } = c.getImageData(0, 0, canvas.width, canvas.height);
    let lit = 0; for (let i = 0; i < data.length; i += 4 * 17) if (data[i]! + data[i + 1]! + data[i + 2]! > 30) lit++;
    return lit;
  }), { timeout: 20_000 }).toBeGreaterThan(20);
  await shot(page, '21-two-views');

  await layout.click();
  await page.locator('.pm-menu [role="option"], .pm-menu .di').filter({ hasText: '4 Views' }).first().click();
  await expect(page.locator('.viewer-pane')).toHaveCount(3);
  expect((await views(page)).panes.map((p: any) => p.view)).toEqual(['top', 'front', 'right', 'camera']);
  expect((await views(page)).active).toBe(3);
  // The selection reads in every pane: an outline around the box in the Top view.
  await page.evaluate((id: string) => (window as any).PM.selectLayers([id]), box);
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.api('multi-view-e2e').scene3d.views.selection(0, 400, 225).length)).toBe(12);
  await shot(page, '22-four-views', 4000);
  await page.evaluate(() => (window as any).PM.selectLayers([]));

  // Each pane chooses its view from its corner label.
  const frontLabel = page.locator('.viewer-pane[data-pane="1"] .viewer-pane-label');
  const shown = await chooseNativeMenu(session, ['Left'], () => frontLabel.click());
  expect(shown).toEqual(['Active Camera', 'Front', 'Left', 'Top', 'Back', 'Right', 'Bottom', 'Custom View']);
  await expect(frontLabel).toHaveText('Left');
  expect((await views(page)).panes[1].view).toBe('left');
  await shot(page, '23-pane-view-left', 3000);

  // One click in the Top pane selects that pane and the box under the pointer.
  const top = (await page.locator('.viewer-pane[data-pane="0"] canvas').boundingBox())!;
  await page.mouse.click(top.x + top.width / 2, top.y + top.height / 2);
  await expect.poll(async () => (await views(page)).active).toBe(0);
  await expect.poll(() => page.evaluate(() => (window as any).PM.sel.layers)).toEqual([box]);
  expect((await views(page)).panes[0].view).toBe('top');
  await expect(page.locator('#stage > .viewer-pane-label')).toHaveText('Top');
  await expect(page.locator('.viewer-pane[data-pane="3"] .viewer-pane-label')).toHaveText('Active Camera');
  await shot(page, '24-top-pane-selected');

  // The gizmo works in the pane that was just a picture: drag the X arrow.
  const handle = await page.evaluate(() => {
    const viewer = (window as any).PM.Kernel.services.get('viewer'), rect = document.querySelector('#stage')!.getBoundingClientRect();
    const canvas = document.querySelector('#gl')!.getBoundingClientRect(), cx = canvas.left + canvas.width / 2, cy = canvas.top + canvas.height / 2;
    for (let y = cy - 30; y < Math.min(cy + 60, rect.bottom - 5); y += 2) for (let x = cx + 10; x < Math.min(cx + 160, rect.right - 5); x += 2)
      if (viewer.sceneGizmo.hover(x, y) && (document.querySelector('[data-scene-gizmo]') as HTMLElement)?.dataset.gizmoAxis === 'X') return { x, y };
    throw new Error('No X handle in the Top pane');
  });
  const x = () => page.evaluate((id: string) => (window as any).PM.L(id).p['position.x'].v, box);
  const before = await x();
  await page.mouse.move(handle.x, handle.y); await page.mouse.down();
  await page.mouse.move(handle.x + 50, handle.y, { steps: 8 }); await page.mouse.up();
  await expect.poll(x).toBeGreaterThan(before);
  await shot(page, '25-moved-in-top-pane');
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await x()).toBe(before);

  // Back to the Active Camera pane, then to 1 View: the camera stays the view.
  const cameraPane = (await page.locator('.viewer-pane[data-pane="3"]').boundingBox())!;
  await page.mouse.click(cameraPane.x + 12, cameraPane.y + cameraPane.height - 12);
  await expect.poll(async () => (await views(page)).active).toBe(3);
  expect(await page.evaluate(() => (window as any).PM.Kernel.api('multi-view-e2e').scene3d.getView())).toBe('camera');
  await layout.click();
  await page.locator('.pm-menu [role="option"], .pm-menu .di').filter({ hasText: '1 View' }).first().click();
  await expect(page.locator('.viewer-pane')).toHaveCount(0);
  await expect(page.locator('#viewer-pane-frame')).toHaveCount(0);
  expect(await page.locator('#stage').evaluate((el: HTMLElement) => el.style.marginLeft + el.style.marginRight)).toBe('');
});
