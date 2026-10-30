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

test('Live reuses the previous frame while the camera moves', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-temporal');
    api.scene3d.edit({ operation: 'add_camera', id: 'cam', camera: { p: { x: 0, y: 1, z: 5, targetX: 0, targetY: 0, targetZ: 0 } } });
    api.scene3d.edit({ operation: 'add_object', object: { id: 'ball', source: { primitive: 'sphere' }, material: { p: { color: '#C0C4CA', metalness: 1, roughness: 0.35 } } } });
    api.scene3d.edit({ operation: 'add_light', light: { id: 'key', type: 'area', p: { x: -2, y: 3, z: 3, intensity: 6, width: 2, height: 1.2 } } });
    PM.invalidate();
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.Studio?.status().ready), { timeout: 30_000 }).toBe(true);
  const history = await page.evaluate(async () => {
    const PM = (window as any).PM, cam = PM.L('cam');
    const stats: any[] = [];
    for (let i = 0; i < 12; i++) {
      // A drag: the camera moves a hair each frame, so every frame is Live.
      cam.p['position.x'].v = i * 0.002; PM.touch();
      PM.GL.render(PM.time, { mblur: false });
      stats.push(await PM.Studio.liveHistory());
    }
    // A jump to the far side: the surface now in view was never seen, so its history restarts.
    cam.p['position.x'].v = 0; cam.p['position.z'].v = -5; PM.touch();
    PM.GL.render(PM.time, { mblur: false });
    return { first: stats[0], last: stats[11], jump: await PM.Studio.liveHistory() };
  });
  // While dragging, nearly every surface pixel reuses 8+ earlier frames.
  expect(history.first.reused).toBe(0);
  expect(history.last.reused / history.last.pixels).toBeGreaterThan(0.9);
  expect(history.last.meanFrames).toBeGreaterThan(8);
  // After a jump the history restarts where it no longer fits.
  expect(history.jump.meanFrames).toBeLessThan(history.last.meanFrames / 2);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('glass focuses light onto the studio floor (caustics)', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const { page } = session;
  const edits = await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-caustics');
    const results = [
      api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 2.6, z: 4.2, targetX: 0, targetY: 0.2, targetZ: 0.4 } } }),
      api.scene3d.edit({ operation: 'add_object', object: { id: 'ball', source: { primitive: 'sphere' }, p: { y: 0.5 } } }),
      api.scene3d.edit({ operation: 'apply_material', target: 'ball', preset: 'glass' }),
      // A small, hard light behind the ball: its focused light lands in front.
      api.scene3d.edit({ operation: 'add_light', light: { id: 'key', type: 'area', p: { x: -1.2, y: 2.2, z: -2.4, targetX: 0, targetY: 0.5, targetZ: 0, intensity: 400, width: 0.15, height: 0.15 } } }),
      api.scene3d.edit({ operation: 'set_studio', on: true }),
    ];
    PM.invalidate();
    return results.map((r: any) => r.ok);
  });
  expect(edits.every(Boolean)).toBe(true);
  await expect.poll(() => page.evaluate(() => (window as any).PM.Studio?.status().ready), { timeout: 30_000 }).toBe(true);
  const result = await page.evaluate(() => {
    const PM = (window as any).PM;
    const frame = () => PM.GL.renderToPixels(0, 240, 160, { transparent: false, mblur: false });
    PM.agentFrameCapture = true;
    const on = frame();
    PM.Studio.setCaustics(false);
    const off = frame();
    PM.Studio.setCaustics(true);
    PM.agentFrameCapture = false;
    let brighter = 0, sumOn = 0, sumOff = 0;
    for (let i = 0; i < on.length; i += 4) {
      const a = on[i]! + on[i + 1]! + on[i + 2]!, b = off[i]! + off[i + 1]! + off[i + 2]!;
      sumOn += a; sumOff += b;
      if (a > b + 45) brighter++;
    }
    return { brighter, sumOn, sumOff };
  });
  // A pool of focused light the plain trace never finds.
  expect(result.brighter).toBeGreaterThan(40);
  expect(result.sumOn).toBeGreaterThan(result.sumOff);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
