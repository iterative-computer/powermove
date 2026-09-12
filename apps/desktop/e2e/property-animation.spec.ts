import { expect } from '@playwright/test';
import { test } from './helpers/app';

test('properties and timeline expose editable content animation, scrubbing, and one-step Undo', async ({ session }) => {
  const { page } = session;
  await page.waitForFunction(() => !!(window as any).PM?.TL?.cv);
  const id = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Property animation', dur: 5 }));
    const layer = PM.mkLayer('shape', { name: 'Editable shape', dur: 5, d: { w: 160, h: 160 } });
    PM.proj.layers.push(layer); PM.selectLayers(layer.id); PM.bus.emit('layers'); PM.invalidate();
    return layer.id;
  });
  const stopwatch = page.getByRole('button', { name: 'Animate Width', exact: true });
  await expect(stopwatch).toBeVisible();
  const geometry = await stopwatch.evaluate(button => {
    const label = button.closest('.row')!.querySelector('.k')!;
    return { button: button.getBoundingClientRect().right, label: label.getBoundingClientRect().left };
  });
  expect(geometry.button).toBeLessThan(geometry.label);
  await stopwatch.click();
  await expect(page.getByRole('button', { name: 'Remove animation from Width', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await page.evaluate(() => (window as any).PM.setTime(1));
  const width = page.getByRole('spinbutton', { name: 'Width', exact: true });
  await width.focus(); await width.press('ArrowUp'); await width.press('Enter');
  await expect.poll(() => page.evaluate(id => (window as any).PM.L(id).d.w.kf.length, id)).toBe(2);
  const position = await page.evaluate(id => {
    const PM = (window as any).PM, layer = PM.L(id), T = PM.TL;
    PM.UIState.setLayerCollapsed(layer, false); T.reveal(layer, ['c.w']); PM.bus.emit('layers'); PM.invalidate();
    return id;
  }, id);
  await expect.poll(() => page.evaluate(id => (window as any).PM.TL.rows.some((row: any) => row.L.id === id && row.key === 'c.w'), position)).toBe(true);
  const coords = await page.evaluate(id => {
    const T = (window as any).PM.TL, rect = T.cv.getBoundingClientRect();
    const index = T.rows.findIndex((row: any) => row.L.id === id && row.key === 'c.w');
    return { x: rect.x + T.propertyValueX + 12, y: rect.y + T.ruler + index * T.row - T.scrollY + T.row / 2 };
  }, id);
  const before = await page.evaluate(id => (window as any).PM.evP((window as any).PM.L(id), (window as any).PM.L(id).d.w, 1, 'c.w'), id);
  await page.mouse.move(coords.x, coords.y); await page.mouse.down();
  await page.mouse.move(coords.x + 35, coords.y, { steps: 5 }); await page.mouse.up();
  await expect.poll(() => page.evaluate(id => (window as any).PM.evP((window as any).PM.L(id), (window as any).PM.L(id).d.w, 1, 'c.w'), id)).toBe(before + 35);
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await page.evaluate(id => (window as any).PM.evP((window as any).PM.L(id), (window as any).PM.L(id).d.w, 1, 'c.w'), id)).toBe(before);
  // Escape cancels the entire scrub, including the temporary keyframe edit.
  await page.mouse.move(coords.x, coords.y); await page.mouse.down();
  await page.mouse.move(coords.x + 20, coords.y, { steps: 3 });
  await page.keyboard.press('Escape'); await page.mouse.up();
  expect(await page.evaluate(id => (window as any).PM.evP((window as any).PM.L(id), (window as any).PM.L(id).d.w, 1, 'c.w'), id)).toBe(before);
  // A click opens direct entry in the timeline; Enter commits one undoable edit.
  await page.mouse.click(coords.x, coords.y);
  const editor = page.getByRole('textbox', { name: 'Width', exact: true });
  await editor.fill('220'); await editor.press('Enter');
  expect(await page.evaluate(id => (window as any).PM.evP((window as any).PM.L(id), (window as any).PM.L(id).d.w, 1, 'c.w'), id)).toBe(220);
  await page.evaluate(() => (window as any).PM.hist.undo());
  await page.screenshot({ path: '/tmp/powermove-properties-timeline.png' });
  const persisted = await page.evaluate(id => {
    const PM = (window as any).PM;
    const restored = PM.hydrateProject(JSON.parse(PM.serialize()).proj);
    const layer = restored.layers.find((item: any) => item.id === id);
    return { keys: layer.d.w.kf.length, width: PM.resolveContent(layer, 1).w };
  }, id);
  expect(persisted).toEqual({ keys: 2, width: before });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('text, shader, extension, blend, and mask stopwatches animate their real values', async ({ session }) => {
  const { page } = session;
  await page.waitForFunction(() => !!(window as any).PM?.TL?.cv);
  for (const type of ['text', 'shader', 'extension', 'shape']) {
    const paths = await page.evaluate(type => {
      const PM = (window as any).PM;
      PM.replaceProject(PM.mkProject({ name: 'Property types', dur: 5 }));
      let layer;
      if (type === 'extension') {
        const result = PM.Edit.apply({ type: 'add_layer', layerType: 'extension', content: { definition: 'powermove.3d.studio-cube' } });
        if (!result.ok) throw new Error(result.message);
        layer = PM.proj.layers[0];
      } else {
        layer = PM.mkLayer(type); PM.proj.layers.push(layer);
      }
      if (type === 'shader') {
        layer.d.code = 'uniform vec3 uTint; // @param #FF0000\nuniform bool uEnabled; // @param true\nuniform float uAmount; // @param 1 0 10\nvoid mainImage(out vec4 c,in vec2 p){c=vec4(uTint*uAmount,1.); }';
        PM.syncShaderUniforms(layer);
      }
      if (type === 'shape') { layer.masks.push(PM.mkMask('rect', PM.proj)); layer.fx.push(PM.mkEffect('blur')); PM.UIState.setFxOpen(layer.fx[0], true); }
      PM.selectLayers(layer.id); PM.bus.emit('layers'); PM.invalidate();
      return type === 'text' ? ['c.text', 'c.font', 'c.size', 'c.color', 'c.align']
        : type === 'shader' ? ['u.uTint', 'u.uEnabled', 'u.uAmount']
        : type === 'extension' ? ['x.objectColor', 'x.autoRotate', 'x.size']
        : ['l.on', 'l.blend', 'l.mblur', `${layer.fx[0].id}.$enabled`, `m.${layer.masks[0].id}.on`, `m.${layer.masks[0].id}.shape`, `m.${layer.masks[0].id}.mode`];
    }, type);
    for (const path of paths) {
      const button = page.locator(`[data-property-path="${path}"]`);
      // Numeric channel rows use their channel wrapper and the same leading stopwatch.
      const control = await button.count() ? button : page.locator(`[data-channel="${path}"] .property-stopwatch`);
      await control.click();
      await expect(control).toHaveAttribute('aria-pressed', 'true');
      expect(await page.evaluate(path => {
        const PM = (window as any).PM;
        return PM.findProp(PM.firstSel(), path)?.kf.length;
      }, path)).toBe(1);
    }
    const state = await page.evaluate(paths => {
      const PM = (window as any).PM, restored = PM.hydrateProject(JSON.parse(PM.serialize()).proj);
      return paths.map(path => PM.findProp(restored.layers[0], path)?.kf.length);
    }, paths);
    expect(state).toEqual(paths.map(() => 1));
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
