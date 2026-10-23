import { expect, test } from './helpers/app';

test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });

test('SVG material maps draw the Chromium map upload through the native 3D pass', async ({ session }) => {
  test.setTimeout(90_000);
  await session.openEditor();
  const { page } = session;
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'SVG material maps', w: 320, h: 180, fps: 30, dur: 2 }));
    const art = `<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="16" fill="#ff7733"/><rect y="16" width="32" height="16" fill="#3388ff" opacity=".55"/><circle cx="16" cy="16" r="9" fill="#fff" opacity=".4"/></svg>`;
    const asset = await PM.assets.add(new File([art], 'material.svg', { type: 'image/svg+xml' }), { silent: true });
    const api = PM.Kernel.api('svg-map-proof');
    api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 0, z: 6 } } });
    api.scene3d.edit({ operation: 'add_object', object: { id: 'mapped', source: { primitive: 'box' }, material: { p: { color: '#ffffff', emissive: '#ffffff', emissiveIntensity: 1 }, maps: { color: asset.id, emissive: asset.id } } } });
    PM.selectLayers(['mapped']);
    PM.invalidate();
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.EngineSvgRasters.stats.posted), { timeout: 20_000 }).toBeGreaterThan(0);
  const compare = () => page.evaluate(() => {
    const PM = (window as any).PM;
    const native = PM.EngineRender.snapshotSync({ time: 0, width: 320, height: 180, pixels: 'premultiplied', motionBlurSamples: 0, background: false });
    const legacy: Uint8Array = PM.GL.renderToPixels(0, 320, 180, { transparent: true, mblur: false });
    // Native pixels are top-down; the explicit legacy reference is bottom-up.
    const at = (x: number, y: number) => ((179 - y) * 320 + x) * 4;
    let nonempty = 0, interior = 0, max = 0;
    for (let y = 2; y < 178; y++) for (let x = 2; x < 318; x++) {
      const i = (y * 320 + x) * 4;
      if (native.data[i + 3] > 0) nonempty++;
      // Exclude silhouette MSAA exactly as the renderer's existing 3D bar:
      // the flat interior establishes the map's colors and orientation.
      if (legacy[at(x, y) + 3] < 255 || native.data[i + 3] < 255) continue;
      let edge = false;
      for (let dy = -2; dy <= 2; dy++) for (let dx = -2; dx <= 2; dx++) if (legacy[at(x + dx, y + dy) + 3] < 255) edge = true;
      if (edge) continue;
      interior++;
      for (let c = 0; c < 4; c++) max = Math.max(max, Math.abs(native.data[i + c] - legacy[at(x, y) + c]));
    }
    return { nonempty, interior, max, errors: PM.EngineSvgRasters.stats.failed, route: PM.EngineConnection.status().route };
  });
  const result = await compare();
  console.log(`SVG maps: ${JSON.stringify(result)}`);
  expect(result.route).toBe('engine');
  expect(result.errors).toBe(0);
  expect(result.nonempty).toBeGreaterThan(500);
  expect(result.interior).toBeGreaterThan(500);
  expect(result.max).toBeLessThanOrEqual(2);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
