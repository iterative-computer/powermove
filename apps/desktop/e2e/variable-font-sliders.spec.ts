import { expect } from '@playwright/test';
import { test } from './helpers/app';

test('variable font numeric fields scrub, animate, undo, and persist', async ({ session }) => {
  test.setTimeout(60000);
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return !!timeline?.cv; });
  const hasFont = await page.evaluate(async () => (await (window as any).queryLocalFonts()).some((font: any) => font.family === 'Geist Width'));
  test.skip(!hasFont, 'Requires installed Geist Width');
  const id = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Variable font sliders', dur: 5 }));
    const layer = PM.mkLayer('text', { d: { text: 'Variable', font: 'Geist Width', size: 100,
      'fontAxis.wght': PM.P(400), 'fontAxis.wdth': PM.P(100) } });
    PM.proj.layers.push(layer); PM.selectLayers(layer.id); PM.bus.emit('layers'); PM.invalidate();
    return layer.id;
  });
  for (const [tag, name] of [['wght', 'Weight'], ['wdth', 'Width']]) {
    const slider = page.locator(`[data-font-axis="${tag}"] [role="spinbutton"]`);
    await expect(slider).toBeVisible({ timeout: 30000 });
    await expect(page.locator('[data-font-axis] input[type=range]')).toHaveCount(0);
    const before = await slider.inputValue();
    const box = (await slider.boundingBox())!;
    await page.mouse.move(box.x + box.width * .5, box.y + box.height / 2);
    await page.mouse.down(); await page.mouse.move(box.x + box.width * .8, box.y + box.height / 2, { steps: 8 }); await page.mouse.up();
    expect(await slider.inputValue()).not.toBe(before);
    await page.evaluate(() => (window as any).PM.hist.undo());
    await expect(slider).toHaveValue(before);
    await page.mouse.move(box.x + box.width * .5, box.y + box.height / 2);
    await page.mouse.down(); await page.mouse.move(box.x + box.width * .7, box.y + box.height / 2, { steps: 3 });
    await page.keyboard.press('Escape'); await page.mouse.up();
    expect(await page.evaluate(({ id, tag }) => (window as any).PM.L(id).d['fontAxis.' + tag].v, { id, tag })).toBe(Number(before));
    await expect(slider).toHaveValue(before);
    await page.getByRole('button', { name: `Animate ${name} · ${tag}`, exact: true }).click();
    await page.evaluate(() => (window as any).PM.setTime(1));
    await slider.focus(); await slider.press('ArrowUp'); await slider.press('Enter'); await slider.press('Tab');
    const saved = await page.evaluate(({ id, tag }) => {
      const PM = (window as any).PM;
      const restored = PM.hydrateProject(JSON.parse(PM.serialize()).proj);
      return restored.layers.find((layer: any) => layer.id === id).d['fontAxis.' + tag];
    }, { id, tag });
    expect(saved.kf).toHaveLength(2);
    await page.evaluate(() => (window as any).PM.setTime(0));
  }
  await page.screenshot({ path: '/tmp/powermove-variable-font-sliders.png' });
  expect(session.diagnostics.pageErrors).toEqual([]);
});


test('installed variable font exposes its axes before editing and changes real glyph pixels', async ({ session }) => {
  test.setTimeout(60000);
  await session.openEditor();
  const { page } = session;
  const hasFont = await page.evaluate(async () => typeof (window as any).queryLocalFonts === 'function'
    && (await (window as any).queryLocalFonts()).some((font: any) => font.family === 'Geist Width'));
  test.skip(!hasFont, 'Glyph rendering proof requires the installed Geist Width variable font.');
  await page.waitForFunction(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return !!timeline?.cv; });
  const id = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Detected variable axes', dur: 5 }));
    const layer = PM.mkLayer('text', { d: { text: 'VARIABLE WIDTH', font: 'Geist Width', weight: 400, size: 100 } });
    PM.proj.layers.push(layer); PM.selectLayers(layer.id); PM.bus.emit('layers'); PM.invalidate();
    return layer.id;
  });
  const width = page.locator('[data-font-axis="wdth"] [role="spinbutton"]');
  await expect(width).toBeVisible();
  expect(await page.evaluate(id => Object.keys((window as any).PM.L(id).d).filter(key => key.startsWith('fontAxis.')), id)).toEqual([]);
  for (const [tag, name] of [['wght', 'Weight'], ['wdth', 'Width']]) {
    const slider = page.locator(`[data-font-axis="${tag}"] [role="spinbutton"]`);
    const images: string[] = [];
    for (const key of ['Home', 'End']) {
      await slider.focus(); await slider.fill((await slider.getAttribute(key === 'Home' ? 'aria-valuemin' : 'aria-valuemax'))!); await slider.press('Enter');
      // FontFace loading is asynchronous; allow the rendered face to settle.
      await page.waitForTimeout(250);
      images.push(await page.evaluate(async id => {
        const PM = (window as any).PM;
        await document.fonts.ready; PM.rasterClear();
        return PM.raster(PM.L(id), 1, 0).cv.toDataURL();
      }, id));
    }
    expect(images[0]).not.toBe(images[1]);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('first-pass keyframe playback renders every width sample without delayed font swaps', async ({ session }) => {
  test.setTimeout(60000);
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return !!timeline?.cv; });
  const hasFont = await page.evaluate(async () => (await (window as any).queryLocalFonts()).some((font: any) => font.family === 'Geist Width'));
  test.skip(!hasFont, 'Requires installed Geist Width');
  const id = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Variable font playback', dur: 5 }));
    const layer = PM.mkLayer('text', { d: { text: 'VARIABLE WIDTH', font: 'Geist Width', weight: 400, size: 100, 'fontAxis.wdth': PM.P(75) } });
    PM.proj.layers.push(layer); PM.selectLayers(layer.id); PM.bus.emit('layers');
    PM.Edit.apply({ type: 'replace_keyframes', target: layer.id, path: 'c.fontAxis.wdth', keyframes: [
      { time: 0, value: 75, ease: 'linear' }, { time: 4, value: 125, ease: 'linear' }
    ] });
    PM.invalidate(); return layer.id;
  });
  await page.waitForFunction(() => [...document.fonts].some(face => face.family.startsWith('Powermove Axis') && face.status === 'loaded'));
  const result = await page.evaluate(id => {
    const PM = (window as any).PM, layer = PM.L(id);
    const sample = (frame: number) => {
      const t = frame / 30;
      const raster = PM.raster(layer, 1, t);
      return { width: raster.w, pixels: raster.cv.toDataURL(), value: PM.resolveContent(layer, t)['fontAxis.wdth'] };
    };
    // No font-loading pauses between samples, including beyond the face-cache limit.
    const first = Array.from({ length: 121 }, (_, frame) => sample(frame));
    const replay = Array.from({ length: 121 }, (_, frame) => sample(frame));
    return { stable: first.every((item, i) => item.pixels === replay[i].pixels),
      unique: new Set(first.map(item => item.pixels)).size,
      widths: [first[0].width, first[60].width, first[120].width],
      midpoint: first[60].value };
  }, id);
  expect(result.midpoint).toBe(100);
  expect(result.unique).toBeGreaterThan(100);
  expect(result.widths[0]).toBeLessThan(result.widths[1]);
  expect(result.widths[1]).toBeLessThan(result.widths[2]);
  expect(result.stable).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('SF Pro weight scrubbing reuses decoded faces without crashing the renderer', async ({ session }) => {
  test.setTimeout(60000);
  await session.openEditor();
  const { page } = session;
  const hasFont = await page.evaluate(async () => (await (window as any).queryLocalFonts()).some((font: any) => font.family === 'SF Pro'));
  test.skip(!hasFont, 'Requires installed SF Pro');
  const id = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'SF Pro scrub regression', dur: 5 }));
    const layer = PM.mkLayer('text', { d: { text: 'THE NEW', font: 'SF Pro', size: 345.45, tracking: -2, 'fontAxis.wght': PM.P(100) } });
    PM.proj.layers.push(layer); PM.selectLayers(layer.id); PM.invalidate();
    return layer.id;
  });
  await page.waitForFunction(() => [...document.fonts].some(face => face.family.startsWith('Powermove Axis')));
  const result = await page.evaluate(id => {
    const PM = (window as any).PM, layer = PM.L(id);
    const sample = (weight: number) => {
      layer.d['fontAxis.wght'].v = weight;
      PM.rasterClear();
      return PM.raster(layer, 1, 0).cv.toDataURL();
    };
    const before = sample(100);
    for (let i = 0; i < 1000; i++) sample(40 + i / 2);
    const heavy = sample(540), replay = sample(100);
    return { stable: before === replay, changed: before !== heavy,
      faces: [...document.fonts].filter(face => face.family.startsWith('Powermove Axis')).length };
  }, id);
  expect(result.stable).toBe(true);
  expect(result.changed).toBe(true);
  expect(result.faces).toBeLessThanOrEqual(8);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
