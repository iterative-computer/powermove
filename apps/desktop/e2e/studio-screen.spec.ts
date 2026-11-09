import { expect, test } from './helpers/app';

/* A live screen: a composition plays on a product's display in Studio, in the
   viewer and in Final frames. The tablet (a rounded slab and a screen plane)
   and the animated screen content are original. */
test('a composition plays on a product screen in Studio', async ({ session }, info) => {
  test.setTimeout(180_000);
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-screen');
    PM.theme.apply('light');
    // The screen content: a dark field with a bright bar sweeping across it.
    const comp = PM.mkProject({ name: 'Screen content', w: 1024, h: 1400, fps: 30, dur: 4, bg: '#0B0B14' });
    const bar = PM.mkLayer('solid', { from: 0, dur: 4, d: { color: '#6E7CFF', w: 220, h: 1400 } }, comp);
    PM.animate(bar, 'position.x', [{ t: 0, v: 110 }, { t: 4, v: 914 }]);
    comp.layers = [bar];
    PM.proj.comps = { ...(PM.proj.comps || {}), screen_content: { ...comp, id: 'screen_content' } };
    PM.touch();
    api.scene3d.edit({ operation: 'add_camera', id: 'cam', camera: { p: { x: 0, y: 2.9, z: 2.2, targetX: 0, targetY: 0, targetZ: 0, fov: 34 } } });
    api.scene3d.edit({ operation: 'add_object', object: { id: 'body', source: { primitive: 'roundedBox', parameters: { width: 1.35, height: 0.07, depth: 1.95, radius: 0.03 } }, p: { y: 0.035 }, material: { p: { color: '#3B3C40', metalness: 1, roughness: 0.34 } } } });
    api.scene3d.edit({ operation: 'add_object', object: { id: 'display', source: { primitive: 'plane', parameters: { width: 1.2, height: 1.8 } }, p: { y: 0.0715, rx: -90 }, material: { screen: 'screen_content', p: { color: '#050507', roughness: 0.6, clearcoat: 1, clearcoatRoughness: 0.012, specular: 0, emissiveIntensity: 1.2 } } } });
    api.scene3d.edit({ operation: 'add_light', light: { id: 'key', type: 'sun', p: { x: -3, y: 5, z: 4, intensity: 2.5 } } });
    api.scene3d.edit({ operation: 'set_studio', on: true });
    PM.invalidate();
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.Studio?.status().ready), { timeout: 30_000 }).toBe(true);
  const frames = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.agentFrameCapture = true;
    const grab = (t: number) => [...PM.GL.renderToPixels(t, 320, 180, { transparent: true, mblur: false })] as number[];
    const a = grab(0.2), b = grab(3.6);
    PM.agentFrameCapture = false;
    // The bar sweeps left to right: compare the screen's left and right halves.
    const blue = (px: number[], x0: number, x1: number) => { let s = 0; for (let y = 40; y < 140; y++) for (let x = x0; x < x1; x++) { const i = (y * 320 + x) * 4; s += px[i + 2]! - px[i]!; } return s; };
    return { leftEarly: blue(a, 110, 160), rightEarly: blue(a, 160, 210), leftLate: blue(b, 110, 160), rightLate: blue(b, 160, 210), differ: a.some((v, i) => v !== b[i]) };
  });
  expect(frames.differ).toBe(true);
  expect(frames.leftEarly).toBeGreaterThan(frames.rightEarly);
  expect(frames.rightLate).toBeGreaterThan(frames.leftLate);
  for (const t of [0.2, 2, 3.6]) {
    await page.evaluate(t => { const PM = (window as any).PM; PM.setTime(t, { raw: true, force: true }); PM.invalidate(); }, t);
    await page.waitForTimeout(7000);
    await page.locator('#gl').evaluate((c: HTMLCanvasElement) => { c.toDataURL(); });
    await page.locator('#gl').screenshot({ path: info.outputPath(`screen-${t}.png`) });
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
