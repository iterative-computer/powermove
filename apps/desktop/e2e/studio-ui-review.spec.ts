import { mkdir } from 'node:fs/promises';
import { test } from './helpers/app';

/* Review screenshots of the 3D inspector, Add menu and Export dialog.
   Runs only with PM_SHOT_DIR set; PM_SHOT_PREFIX names the set (before/after). */

const shots = process.env.PM_SHOT_DIR;
const prefix = process.env.PM_SHOT_PREFIX || 'shot';

test('3D review screenshots', async ({ session }) => {
  test.skip(!shots, 'PM_SHOT_DIR not set');
  test.setTimeout(120_000);
  await session.openEditor();
  const page = session.page;
  const snap = async (name: string) => { await mkdir(shots!, { recursive: true }); await page.waitForTimeout(400); await page.screenshot({ path: `${shots}/${prefix}-${name}.png` }); };
  const ids = await page.evaluate(() => {
    const PM = (window as any).PM; PM.pause(); PM.AutomaticPreview?.cancel?.();
    const api = PM.Kernel.api('studio-ui-review');
    const cam = api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 1.2, z: 4 } } });
    const box = api.scene3d.edit({ operation: 'add_object', object: { name: 'Cube', source: { primitive: 'box' } } });
    const light = api.scene3d.edit({ operation: 'add_light', light: { type: 'area', name: 'Softbox' } });
    return { camera: cam.data?.result?.id, box: box.data?.result?.id, light: light.data?.result?.id };
  });
  for (const theme of ['light', 'dark']) {
    await page.evaluate(theme => (window as any).PM.theme.apply(theme), theme);
    await page.evaluate(id => (window as any).PM.selectLayers([id]), ids.camera); await snap(`${theme}-camera-inspector`);
    await page.evaluate(id => (window as any).PM.selectLayers([id]), ids.box); await snap(`${theme}-model-inspector`);
    await page.evaluate(id => (window as any).PM.selectLayers([id]), ids.light); await snap(`${theme}-light-inspector`);
  }
  await page.evaluate(() => (window as any).PM.theme.apply('light'));
  await page.evaluate(() => (window as any).PM.cmd('3d.add-menu')); await snap('light-add-menu');
  await page.keyboard.press('Escape');
  await page.evaluate(() => { const PM = (window as any).PM; PM.Export.dialog(); });
  await page.waitForTimeout(300);
  const png = page.getByRole('dialog', { name: 'Export', exact: true }).getByRole('button', { name: /Image/ }).first();
  if (await png.count()) await png.click().catch(() => {});
  await snap('light-export-dialog');
});
