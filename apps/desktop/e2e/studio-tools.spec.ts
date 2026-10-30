import { expect, test } from './helpers/app';

/* Studio's product-film tools: finishes, light by pointing (operation and
   Alt+Shift+click), Shots as keyframes, the seamless set, and the refine
   indicator. The scene is an original arrangement of primitives. */
test('Studio finishes, light by pointing, Shots and the studio set', async ({ session }, info) => {
  test.setTimeout(180_000);
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-tools');
    PM.theme.apply('light');
    api.scene3d.edit({ operation: 'add_camera', id: 'cam', camera: { p: { x: 0.4, y: 1.6, z: 4.6, targetX: 0, targetY: 0.2, targetZ: 0, fov: 32 } } });
    api.scene3d.edit({ operation: 'add_object', object: { id: 'slab', source: { primitive: 'box', parameters: { width: 1.6, height: 0.08, depth: 2.2 } }, p: { y: 0.04, ry: 24 }, material: { p: { color: '#888888' } } } });
    api.scene3d.edit({ operation: 'add_object', object: { id: 'orb', source: { primitive: 'sphere', parameters: { radius: 0.32 } }, p: { x: 1.25, y: 0.32, z: 0.5 } } });
    api.scene3d.edit({ operation: 'add_light', light: { id: 'key', type: 'sun', p: { x: -3, y: 5, z: 4, intensity: 2.5 } } });
    PM.invalidate();
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.Studio?.status().ready), { timeout: 30_000 }).toBe(true);

  const result = await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-tools');
    const ok = (r: any) => { if (!r.ok) throw new Error(r.message); return r; };
    const lights = () => PM.proj.layers.filter((l: any) => l.d?.definition === 'powermove.3d.light').length;
    ok(api.scene3d.edit({ operation: 'apply_material', target: 'slab', preset: 'anodized' }));
    ok(api.scene3d.edit({ operation: 'apply_material', target: 'orb', preset: 'glass' }));
    ok(api.scene3d.edit({ operation: 'set_studio', target: 'cam', on: true }));
    const before = lights();
    ok(api.scene3d.edit({ operation: 'light_highlight', at: [0, 0.08, 0], normal: [0, 1, 0], intensity: 5 }));
    const area = PM.proj.layers.filter((l: any) => l.d?.data?.light?.type === 'area');
    ok(api.scene3d.edit({ operation: 'shot', shot: 'turntable', target: 'slab', start: 0, duration: 4 }));
    ok(api.scene3d.edit({ operation: 'shot', shot: 'orbit', start: 0, duration: 4, degrees: 60 }));
    const slab = PM.L('slab'), cam = PM.L('cam');
    return {
      metal: slab.d.data.object.material.p.metalness.v, transmission: PM.L('orb').d.data.object.material.p.transmission.v,
      catcher: cam.d.data.environment.shadowCatcher,
      addedLights: lights() - before, areaCount: area.length,
      areaAboveSlab: area[0] ? area[0].p['position.y'].v > 0.08 : false,
      turntable: slab.p['rotation.y'].kf.map((k: any) => k.v), orbitKeys: cam.p['position.x'].kf.length,
      undoable: PM.hist.list().length,
    };
  });
  expect(result.metal).toBe(1);
  expect(result.transmission).toBe(1);
  expect(result.catcher).toBe(true);
  expect(result.addedLights).toBe(1);
  expect(result.areaCount).toBe(1);
  expect(result.areaAboveSlab).toBe(true);
  expect(result.turntable[1] - result.turntable[0]).toBe(360);
  expect(result.orbitKeys).toBe(9);

  // Alt+Shift+click on the slab in the viewer places another softbox.
  const lightsBefore = await page.evaluate(() => (window as any).PM.proj.layers.filter((l: any) => l.d?.definition === 'powermove.3d.light').length);
  const box = (await page.locator('#gl').boundingBox())!;
  await page.keyboard.down('Alt'); await page.keyboard.down('Shift');
  await page.mouse.click(box.x + box.width * 0.47, box.y + box.height * 0.55);
  await page.keyboard.up('Shift'); await page.keyboard.up('Alt');
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.filter((l: any) => l.d?.definition === 'powermove.3d.light').length)).toBe(lightsBefore + 1);

  // The refine indicator appears and the view settles.
  await expect(page.locator('#studio-refine')).toBeVisible({ timeout: 15_000 });
  await page.waitForTimeout(9000);
  await page.locator('#gl').evaluate((c: HTMLCanvasElement) => { c.toDataURL(); });
  await page.locator('#gl').screenshot({ path: info.outputPath('studio-viewer.png') });
  await page.evaluate(() => { const PM = (window as any).PM; PM.selectLayers(['slab']); PM.invalidate(); });
  await expect(page.locator('#panel-inspector').getByRole('group', { name: 'Finish' })).toBeVisible();
  await page.locator('#panel-inspector').screenshot({ path: info.outputPath('studio-inspector-model.png') });
  await page.evaluate(() => { const PM = (window as any).PM; PM.selectLayers(['cam']); PM.invalidate(); });
  await expect(page.locator('#panel-inspector').getByRole('group', { name: 'Shots' })).toBeVisible();
  await page.locator('#panel-inspector').getByRole('group', { name: 'Shots' }).scrollIntoViewIfNeeded();
  await page.locator('#panel-inspector').screenshot({ path: info.outputPath('studio-inspector-camera.png') });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
