import { expect, test } from './helpers/app';

/* Reads extension-visible uiState after real presentation. A deliberate
 * legacy reference render happens only after native metadata was observed. */
test('scene warnings reach uiState and recover with the displayed scene', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('scene-warning-proof');
    PM.replaceProject(PM.mkProject({ name: 'Scene diagnostics', w: 160, h: 90 }));
    for (const command of [
      { operation: 'add_camera', camera: { p: { z: 6 } } },
      { operation: 'add_object', object: { id: 'painted', source: { primitive: 'box' }, material: { maps: { color: 'missing_texture' } } } },
      { operation: 'add_object', object: { id: 'model', source: { assetId: 'missing_model' }, p: { x: 1.5 } } },
    ]) {
      const result = api.scene3d.edit(command);
      if (!result.ok) throw Error(result.message);
    }
    PM.invalidate();
  });
  await expect.poll(() => page.evaluate(() => {
    const PM = (window as any).PM;
    return ['painted', 'model'].map(id => PM.UIState.getShaderMeta(PM.L(id)).sceneWarnings?.length ?? -1);
  })).toEqual([2, 2]);
  const comparison = await page.evaluate(() => {
    const PM = (window as any).PM;
    const read = () => ['painted', 'model'].map(id => {
      const meta = PM.UIState.getShaderMeta(PM.L(id));
      return { missing: meta.missing, warnings: [...meta.sceneWarnings] };
    });
    const native = read();
    PM.GL.renderToPixels(0, 160, 90, { transparent: true, mblur: false });
    return { native, today: read() };
  });
  expect(comparison.native).toEqual(comparison.today);
  expect([...comparison.native[0].warnings].sort()).toEqual(['Missing model: missing_model', 'Missing texture: missing_texture']);
  expect(comparison.native.every(meta => meta.missing === false)).toBe(true);
  await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('scene-warning-proof');
    for (const command of [
      { operation: 'update_object', target: 'painted', patch: { material: { maps: {} } } },
      { operation: 'update_object', target: 'model', patch: { source: { primitive: 'box' } } },
    ]) {
      const result = api.scene3d.edit(command);
      if (!result.ok) throw Error(result.message);
    }
    PM.invalidate();
  });
  await expect.poll(() => page.evaluate(() => {
    const PM = (window as any).PM;
    return ['painted', 'model'].map(id => PM.UIState.getShaderMeta(PM.L(id)).sceneWarnings);
  })).toEqual([[], []]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
