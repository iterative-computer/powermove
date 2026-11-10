import { expect, test } from './helpers/app';

// Exercise the portable renderer and its own temporal history even when the
// app has a bundled native host. Native transport/denoising has separate checks.
test.use({desktopLaunchOptions:{env:{POWERMOVE_STUDIO_HOST:'/usr/bin/false'}}});

/* Studio, the path-traced 3D renderer: it draws 3D layers in the viewer and
   in exports, splits one trace into per-layer pixels, and keeps the same
   renderer for Fast and Quality previews. */
test('Studio renders 3D layers and keeps each layer separate in Fast and Quality', async ({ session }) => {
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
    const frame = (live=false) => PM.GL.renderToPixels(0, 200, 120, { transparent: true, mblur: false,editorViewport:live });
    PM.agentFrameCapture = true;
    const studio = frame(), framesAfter = PM.Studio.status().frames;
    PM.Studio.setPreview('fast');PM.agentFrameCapture=false;
    const classic = frame(true);
    PM.Studio.setPreview('quality');
    const hide = PM.L('left'), from = hide.from; hide.from = 1000; PM.touch();
    PM.agentFrameCapture = true;
    const rightOnly = frame();
    PM.agentFrameCapture = false;
    hide.from = from; PM.touch();
    return { framesAfter, studio: coverage(studio), classic: coverage(classic), rightOnly: coverage(rightOnly), differs: JSON.stringify([...studio]) !== JSON.stringify([...classic]) };
  });
  expect(result.framesAfter).toBeGreaterThan(0);
  expect(result.studio).toBeGreaterThan(1000);
  // Live and Final cover the same scene while using different sample budgets.
  expect(Math.abs(result.studio - result.classic) / result.classic).toBeLessThan(0.15);
  expect(result.differs).toBe(true);
  expect(result.rightOnly).toBeGreaterThan(200);
  expect(result.rightOnly).toBeLessThan(result.studio);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Live reuses the previous frame while the camera moves', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  // These counters belong to the portable history buffer. MetalFX owns its
  // own history, so select the fallback explicitly instead of racing it.
  await session.app.evaluate(({ ipcMain }) => {
    ipcMain.removeHandler('studio-native:request');
    ipcMain.handle('studio-native:request', () => { throw new Error('Testing WebGPU history'); });
  });
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-temporal');
    PM.pause();PM.AutomaticPreview?.cancel();PM.Studio?.setPreview('fast');
    api.scene3d.edit({ operation: 'add_camera', id: 'cam', camera: { p: { x: 0, y: 1, z: 5, targetX: 0, targetY: 0, targetZ: 0 } } });
    api.scene3d.edit({ operation: 'add_object', object: { id: 'ball', source: { primitive: 'sphere' }, material: { p: { color: '#C0C4CA', metalness: 1, roughness: 0.35 } } } });
    api.scene3d.edit({ operation: 'add_light', light: { id: 'key', type: 'area', p: { x: -2, y: 3, z: 3, intensity: 6, width: 2, height: 1.2 } } });
    PM.invalidate();
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.Studio?.status().ready), { timeout: 30_000 }).toBe(true);
  await expect.poll(() => page.evaluate(() => (window as any).PM.Studio?.status().backend)).toBe('webgpu');
  await page.evaluate(() => (window as any).PM.Studio.setPreview('fast'));
  const history = await page.evaluate(async () => {
    const PM = (window as any).PM, cam = PM.L('cam');
    const stats: any[] = [];
    await PM.Studio.idle();
    const nextFrame = async () => {
      const previous = PM.Studio.status().frames;
      // Scene evaluation and rendering follow the display clock. Reading the
      // GPU history in a tight loop can observe the same frame repeatedly.
      for (let attempt = 0; attempt < 120; attempt++) {
        await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
        PM.GL.render(PM.time, { mblur: false });
        await PM.Studio.idle();
        if (PM.Studio.status().frames > previous) return PM.Studio.liveHistory();
      }
      throw new Error('Studio did not complete the requested camera frame');
    };
    for (let i = 0; i < 12; i++) {
      // A drag: the camera moves a hair each frame, so every frame is Live.
      cam.p['position.x'].v = i * 0.002; PM.touch();
      stats.push(await nextFrame());
    }
    // A jump to the far side: the surface now in view was never seen, so its history restarts.
    cam.p['position.x'].v = 0; cam.p['position.z'].v = -5; PM.touch();
    return { first: stats[0], last: stats[11], jump: await nextFrame() };
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
    let brighter = 0, darker = 0, focusedExcess = 0;
    for (let i = 0; i < on.length; i += 4) {
      const a = on[i]! + on[i + 1]! + on[i + 2]!, b = off[i]! + off[i + 1]! + off[i + 2]!;
      if (a > b + 45) { brighter++; focusedExcess += a - b; }
      if (a < b - 45) darker++;
    }
    return { brighter, darker, focusedExcess };
  });
  // A pool of focused light the plain trace never finds, inside a shadow that
  // is now truly dark: the light the glass bends no longer also passes
  // straight through it.
  // The calibrated surface-area photon gather has a tighter footprint than
  // the old stencil: 38 high-contrast pixels at 64 spp, formerly >40. Check
  // a resolved pool and its excess as well as the complementary dark shadow;
  // the Rust suite checks analytic energy, with the Mitsuba footprint benchmark
  // recorded in docs/studio/cycles-yardstick.md.
  console.log(`caustics ${JSON.stringify(result)}`);
  expect(result.brighter).toBeGreaterThan(25);
  expect(result.focusedExcess).toBeGreaterThan(2000);
  expect(result.darker).toBeGreaterThan(40);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Final denoises by its guides, keeps edges and renders the same frame the same', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const { page } = session;
  const edits = await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-denoise');
    const results = [
      // Shallow focus on a matte sphere over the studio floor: light, lens blur and shadow are all noisy.
      api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 0.9, z: 3.2, targetX: 0, targetY: 0.5, targetZ: 0, fStop: 2 } } }),
      api.scene3d.edit({ operation: 'add_object', object: { id: 'ball', source: { primitive: 'sphere' }, p: { y: 0.5 }, material: { p: { color: '#B8BCC4', roughness: 0.6 } } } }),
      api.scene3d.edit({ operation: 'add_object', object: { id: 'far', source: { primitive: 'box' }, p: { x: 1.4, y: 0.4, z: -2.5 }, material: { p: { color: '#D2361F', roughness: 0.4 } } } }),
      api.scene3d.edit({ operation: 'auto_studio' }),
      api.scene3d.edit({ operation: 'set_environment', patch: { samples: 16 } }),
    ];
    PM.invalidate();
    return results.map((r: any) => r.ok);
  });
  expect(edits.every(Boolean)).toBe(true);
  await expect.poll(() => page.evaluate(() => (window as any).PM.Studio?.status().ready), { timeout: 30_000 }).toBe(true);
  const result = await page.evaluate(() => {
    const PM = (window as any).PM, W = 320, H = 200;
    const frame = (t: number) => PM.GL.renderToPixels(t, W, H, { transparent: false, mblur: false });
    // Grain: mean |pixel − mean of its 4 neighbours| (luma).
    const grain = (px: Uint8Array) => {
      const l = (x: number, y: number) => { const i = (y * W + x) * 4; return 0.2126 * px[i]! + 0.7152 * px[i + 1]! + 0.0722 * px[i + 2]!; };
      let sum = 0, n = 0;
      for (let y = 1; y < H - 1; y++) for (let x = 1; x < W - 1; x++) {
        sum += Math.abs(l(x, y) - (l(x - 1, y) + l(x + 1, y) + l(x, y - 1) + l(x, y + 1)) / 4); n++;
      }
      return sum / n;
    };
    PM.agentFrameCapture = true;
    PM.Studio.setDenoise(false);
    const raw = frame(0);
    PM.Studio.setDenoise(true);
    const denoised = frame(0);
    PM.Studio.setDenoise(false); PM.Studio.setDenoise(true);
    const again = frame(0);
    PM.agentFrameCapture = false;
    let changedBytes = 0, maximumDelta = 0;
    const firstDifferences: Array<{ byte: number; before: number; after: number }> = [];
    for (let i = 0; i < denoised.length; i++) if (denoised[i] !== again[i]) {
      changedBytes++; maximumDelta = Math.max(maximumDelta, Math.abs(denoised[i] - again[i]));
      if (firstDifferences.length < 8) firstDifferences.push({ byte: i, before: denoised[i], after: again[i] });
    }
    const same = changedBytes === 0;
    // The backdrop stays exactly the backdrop where nothing is near it.
    const corner = [...denoised.subarray((10 * W + 10) * 4, (10 * W + 10) * 4 + 3)];
    return { raw: grain(raw), denoised: grain(denoised), same, corner, differences: { changedBytes, maximumDelta, firstDifferences } };
  });
  expect(result.denoised).toBeLessThan(result.raw * 0.5);
  expect(result.same, JSON.stringify(result.differences)).toBe(true);
  expect(result.corner).toEqual([246, 245, 248]);
  expect(session.diagnostics.pageErrors).toEqual([]);
  console.log(`grain ${result.raw.toFixed(2)} → ${result.denoised.toFixed(2)}; corner ${result.corner}`);
});

test('a sun keeps its brightness between Studio Live and Final', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const { page } = session;
  const scene = (intensity: number) => page.evaluate((I: number) => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-sun');
    if (!PM.L('b')) {
      api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 1, z: 4, targetX: 0, targetY: 0, targetZ: 0 } } });
      api.scene3d.edit({ operation: 'add_object', object: { id: 'b', source: { primitive: 'box' }, material: { p: { color: '#FFFFFF', roughness: 1 } } } });
      api.scene3d.edit({ operation: 'add_light', light: { id: 'sun', type: 'sun', p: { x: 0, y: 5, z: 5, intensity: I } } });
    } else api.scene3d.edit({ operation: 'update_light', target: 'sun', patch: { p: { intensity: I } } });
  }, intensity);
  const measure = () => page.evaluate(() => {
    const PM = (window as any).PM;
    const mean = (px: Uint8Array) => { let s = 0, n = 0; for (let i = 0; i < px.length; i += 4) if (px[i + 3]! > 200) { s += px[i]! + px[i + 1]! + px[i + 2]!; n++; } return s / Math.max(1, n) / 3; };
    PM.agentFrameCapture = true;
    const studio = mean(PM.GL.renderToPixels(0, 240, 160, { transparent: true, mblur: false }));
    PM.Studio.setPreview('fast');PM.agentFrameCapture=false;
    const classic = mean(PM.GL.renderToPixels(0, 240, 160, { transparent: true, mblur: false,editorViewport:true }));
    PM.Studio.setPreview('quality');
    return { studio, classic };
  });
  await scene(3);
  await expect.poll(() => page.evaluate(() => (window as any).PM.Studio?.status().ready), { timeout: 30_000 }).toBe(true);
  const lit = await measure();
  // Both Studio tiers use the same irradiance and material model.
  expect(Math.abs(lit.studio - lit.classic) / lit.classic).toBeLessThan(0.2);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
