import { expect, test } from './helpers/app';

/* Studio, the path-traced 3D renderer: it draws 3D layers in the viewer and
   in exports, splits one trace into per-layer pixels, and falls back to the
   classic renderer when turned off. */
test('Studio renders 3D layers, keeps each layer separate and can be turned off', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-e2e');
    api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 0.8, z: 5, targetX: 0, targetY: 0, targetZ: 0 } } });
    api.scene3d.edit({ operation: 'add_object', object: { id: 'left', source: { primitive: 'sphere' }, p: { x: -0.8 }, material: { p: { color: '#D2361F', roughness: 0.2 } } } });
    api.scene3d.edit({ operation: 'add_object', object: { id: 'right', source: { primitive: 'box' }, p: { x: 0.9 }, material: { p: { color: '#C0C4CA', metalness: 1, roughness: 0.3 } } } });
    api.scene3d.edit({ operation: 'add_light', light: { id: 'key', type: 'sun', p: { x: -3, y: 5, z: 4, intensity: 3 } } });
    PM.invalidate();
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.Studio?.status().ready), { timeout: 30_000 }).toBe(true);
  const result = await page.evaluate(() => {
    const PM = (window as any).PM;
    const coverage = (px: Uint8Array | number[]) => { let n = 0; for (let i = 3; i < px.length; i += 4) if (px[i]! > 8) n++; return n; };
    const frame = () => PM.GL.renderToPixels(0, 200, 120, { transparent: true, mblur: false });
    PM.agentFrameCapture = true;
    const studio = frame(), framesAfter = PM.Studio.status().frames;
    PM.Studio.setEnabled(false);
    const classic = frame();
    PM.Studio.setEnabled(true);
    PM.agentFrameCapture = false;
    const hide = PM.L('left'), from = hide.from; hide.from = 1000; PM.touch();
    PM.agentFrameCapture = true;
    const rightOnly = frame();
    PM.agentFrameCapture = false;
    hide.from = from; PM.touch();
    return { framesAfter, studio: coverage(studio), classic: coverage(classic), rightOnly: coverage(rightOnly), differs: JSON.stringify([...studio]) !== JSON.stringify([...classic]) };
  });
  expect(result.framesAfter).toBeGreaterThan(0);
  expect(result.studio).toBeGreaterThan(1000);
  // Both renderers cover about the same pixels; Studio shades them differently.
  expect(Math.abs(result.studio - result.classic) / result.classic).toBeLessThan(0.15);
  expect(result.differs).toBe(true);
  expect(result.rightOnly).toBeGreaterThan(200);
  expect(result.rightOnly).toBeLessThan(result.studio);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
