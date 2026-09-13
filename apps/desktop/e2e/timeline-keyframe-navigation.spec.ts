import { expect } from '@playwright/test';
import { test } from './helpers/app';

test('timeline shortcuts navigate real keyframes with a compact transport', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return Boolean(timeline?.cv); });
  const before = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Keyframe navigation', dur: 8 }));
    const L = PM.mkLayer('solid', { name: 'Animated', dur: 6 });
    L.from = 1;
    PM.proj.layers.push(L);
    PM.setKey(L, 'position.y', 2, 100);
    PM.setKey(L, 'position.y', 4, 200);
    PM.setKey(L, 'opacity', 3, 50);
    PM.UIState.setLayerCollapsed(L, false);
    PM.selectLayers(L.id);
    PM.sel.chan = null;
    PM.bus.emit('layers');
    PM.setTime(0);
    return JSON.stringify(L.p);
  });
  await expect(page.locator('#tl-head .tl-transport button')).toHaveCount(1);
  await expect(page.locator('#tl-time')).toBeVisible();
  const time = () => page.evaluate(() => (window as any).PM.time);
  await page.keyboard.press('Shift+J'); await expect.poll(time).toBe(0);
  await page.keyboard.press('Shift+K'); await expect.poll(time).toBe(2);
  await page.keyboard.press('Shift+K'); await expect.poll(time).toBe(3);
  await page.keyboard.press('Shift+K'); await expect.poll(time).toBe(4);
  await page.keyboard.press('Shift+K'); await expect.poll(time).toBe(4);
  await page.keyboard.press('Shift+J'); await expect.poll(time).toBe(3);
  const point = await page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    const i = timeline.rows.findIndex((r: any) => r.key === 'position.y');
    return { x: box.x + 28, y: box.y + timeline.ruler + i * timeline.row - timeline.scrollY + timeline.row / 2 };
  });
  await page.mouse.click(point.x, point.y);
  await expect.poll(time).toBe(2);
  // Property selection scopes navigation to Position Y, skipping Opacity.
  await page.keyboard.press('Shift+K'); await expect.poll(time).toBe(4);
  await page.keyboard.press('Shift+J'); await expect.poll(time).toBe(2);
  await page.keyboard.press('Shift+K'); await expect.poll(time).toBe(4);
  await expect(page.locator('#panel-inspector').getByLabel('Start', { exact: true })).toHaveCount(0);
  await expect(page.locator('#panel-inspector').getByLabel('Duration', { exact: true })).toHaveCount(0);
  await page.evaluate(() => { const input = document.createElement('input'); input.id = 'navigation-text-check'; document.body.append(input); input.focus(); });
  await page.keyboard.press('Shift+J');
  await expect.poll(time).toBe(4);
  await expect(page.locator('#navigation-text-check')).toHaveValue('J');
  await page.evaluate(() => document.querySelector('#navigation-text-check')?.remove());
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.proj.layers[0].p))).toBe(before);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('clicking away clears row selection and deleting final keys preserves value with Undo', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return Boolean(timeline?.cv); });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Keyframe deselection', dur: 8 }));
    const L = PM.mkLayer('solid', { name: 'Animated', dur: 8 });
    PM.proj.layers.push(L);
    PM.setKey(L, 'position.y', 1, 100);
    PM.setKey(L, 'position.y', 3, 300);
    PM.UIState.setLayerCollapsed(L, false);
    PM.selectLayers(L.id);
    PM.bus.emit('layers');
    PM.setTime(2);
  });
  const coords = await page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    const i = timeline.rows.findIndex((r: any) => r.key === 'position.y');
    return { x: box.x, y: box.y + timeline.ruler + i * timeline.row - timeline.scrollY + timeline.row / 2, emptyY: box.y + timeline.hgt - 8, gut: timeline.gut };
  });
  await page.mouse.click(coords.x + 115, coords.y);
  await expect.poll(() => page.evaluate(() => (window as any).PM.sel.chan)).toBe('position.y');
  await page.mouse.click(coords.x + 115, coords.emptyY);
  await expect.poll(() => page.evaluate(() => (window as any).PM.sel.chan)).toBeNull();
  await page.mouse.click(coords.x + 115, coords.y);
  await page.mouse.click(coords.x + coords.gut + 400, coords.y);
  await expect.poll(() => page.evaluate(() => (window as any).PM.sel.chan)).toBeNull();
  await expect.poll(() => page.evaluate(() => (window as any).PM.time)).toBe(2);
  await page.evaluate(() => {
    const PM = (window as any).PM, L = PM.proj.layers[0];
    PM.selectLayers(L.id); PM.sel.chan = 'position.y';
    PM.sel.keys = L.p['position.y'].kf.map((k: any) => k.i);
    PM.cmd('delete');
  });
  await expect.poll(() => page.evaluate(() => {
    const p = (window as any).PM.proj.layers[0].p['position.y'];
    return { count: p.kf.length, value: p.v };
  })).toEqual({ count: 0, value: 200 });
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers[0].p['position.y'].kf.length)).toBe(2);
  await page.evaluate(() => {
    const PM = (window as any).PM, L = PM.proj.layers[0];
    PM.Edit.apply({ type: 'replace_keyframes', target: L.id, path: 'position.y', keyframes: [], preserveHandEdits: false }, { label: 'Clear animation' });
  });
  await expect.poll(() => page.evaluate(() => {
    const p = (window as any).PM.proj.layers[0].p['position.y'];
    return { count: p.kf.length, value: p.v };
  })).toEqual({ count: 0, value: 200 });
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers[0].p['position.y'].kf.length)).toBe(2);
  expect(session.diagnostics.pageErrors).toEqual([]);
});


test('removing the last current key disables its diamond and Undo restores it', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return Boolean(timeline?.cv); });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Last key', dur: 5 }));
    const L = PM.mkLayer('solid', { dur: 5 });
    PM.proj.layers.push(L); PM.selectLayers(L.id);
    PM.setKey(L, 'opacity', 1, 35); PM.setTime(1); PM.bus.emit('layers');
  });
  const animation = page.getByRole('button', { name: 'Remove animation from Opacity', exact: true });
  await expect(animation).toHaveAttribute('aria-pressed', 'true');
  // Uses the shared removal path called by the current-key diamond and menus.
  await page.evaluate(() => {
    const PM = (window as any).PM, p = PM.proj.layers[0].p.opacity;
    PM.hist.do('Remove current key', () => PM.removeKey(p, p.kf[0]));
  });
  await expect(page.getByRole('button', { name: 'Animate Opacity', exact: true })).toHaveAttribute('aria-pressed', 'false');
  expect(await page.evaluate(() => (window as any).PM.proj.layers[0].p.opacity.v)).toBe(35);
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect(animation).toHaveAttribute('aria-pressed', 'true');
  expect(await page.evaluate(() => (window as any).PM.proj.layers[0].p.opacity.kf.length)).toBe(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const type of ['text', 'shape']) test(`${type} color diamonds animate visible color and reveal its timeline row`, async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return !!timeline?.cv; });
  await page.evaluate(type => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Color animation', dur: 5 }));
    const L = PM.mkLayer(type, { d: { color: '#ff0000' } });
    PM.proj.layers.push(L); PM.selectLayers(L.id); PM.setTime(0); PM.bus.emit('layers');
  }, type);
  await page.getByRole('button', { name: type === 'text' ? 'Animate Color' : 'Animate Fill', exact: true }).click();
  await page.evaluate(() => (window as any).PM.setTime(2));
  const label = type === 'text' ? 'Text color' : 'Fill';
  await page.locator('#panel-inspector .row').filter({ has: page.locator('[data-property-path="c.color"]') }).locator('.color-field').click();
  await page.getByRole('textbox', { name: `${label} hex value`, exact: true }).fill('#0000ff');
  await page.getByRole('button', { name: 'Close color picker', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers[0].d.color.kf.length)).toBe(2);
  expect(await page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const L = PM.proj.layers[0];
    return { midpoint: PM.resolveContent(L, 1).color, visible: timeline.rows.some((r: any) => r.key === 'c.color') };
  })).toEqual({ midpoint: '#800080', visible: true });
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers[0].d.color.kf.length)).toBe(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('composition zoom dropdown follows presets, shortcuts, pan and Fit without changing the project', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const trigger = page.getByRole('combobox', { name: 'Composition zoom', exact: true });
  const dropdown = page.locator('#composition-zoom');
  await expect(trigger).toBeVisible();
  const before = await page.evaluate(() => JSON.stringify((window as any).PM.proj));
  await dropdown.selectOption('0.5');
  expect(await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return viewer.shown; })).toBe(.5);
  await page.evaluate(() => (window as any).PM.cmd('zoomIn'));
  await expect(dropdown).toHaveValue('custom');
  await expect(dropdown.locator('option:checked')).toHaveText('62.5%');
  await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); viewer.pan=[90,50];viewer.layout();});
  await dropdown.selectOption('1');
  expect(await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return viewer.pan; })).toEqual([90, 50]);
  await dropdown.selectOption('fit');
  expect(await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return ({fit:viewer.fit,pan:viewer.pan}); })).toEqual({fit:true,pan:[0,0]});
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.proj))).toBe(before);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
