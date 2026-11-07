import { mkdir } from 'node:fs/promises';
import path from 'node:path';
import type { Page } from '@playwright/test';
import { chooseNativeMenu, expect, inspectNativeMenu, test } from './helpers/app';

/* Workspace menu (Save Changes / Reset / Default) and My Presets (save sheet,
   Effects & Presets list, apply, rename, delete), driven through the real UI.
   POWERMOVE_UI_SHOTS=<dir> also writes the screenshots used for review. */
async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.POWERMOVE_UI_SHOTS;
  if (!dir) return;
  await mkdir(dir, { recursive: true });
  await page.waitForTimeout(220);
  await page.screenshot({ path: path.join(dir, `${name}.png`) });
}

async function theme(page: Page, value: 'dark' | 'light'): Promise<void> {
  await page.evaluate(next => (window as any).PM.theme.apply(next), value);
}

const MENU = '[data-svelte-overlay-menu] .menu-panel';

test('workspace menu saves changes, resets to them, and sets the default', async ({ session }) => {
  test.setTimeout(90_000);
  const { page } = session;
  await page.setViewportSize({ width: 1440, height: 900 });
  await session.openEditor();
  const more = page.locator('.tb-workspaces-more');

  // A user workspace whose live layout has drifted from its saved baseline.
  const id = await page.evaluate(() => {
    const PM = (window as any).PM;
    const w = PM.WS.saveCurrent('My Edit');
    PM.WS.mutate((ws: any) => { ws.layout.docks.find((d: any) => d.id === 'left').size = 377; });
    return w.id;
  });
  for (const value of ['dark', 'light'] as const) {
    await theme(page, value);
    await more.click();
    await expect(page.locator(MENU)).toBeVisible();
    await shot(page, `02-after-workspace-menu-changed-${value}`);
    await page.evaluate(() => (window as any).PM.closeMenus());
  }
  await theme(page, 'dark');
  const rows = await inspectNativeMenu(session, () => more.click());
  expect(rows.map(row => row.label)).toEqual(expect.arrayContaining([
    'Save Changes to This Workspace', 'Reset to Saved Layout', 'Set as Default Workspace',
    'New Workspace…', 'Rename Workspace…', 'Delete Workspace…', 'Panel Library…'
  ]));
  expect(rows.find(row => row.label === 'Save Changes to This Workspace')!.enabled).toBe(true);

  // Save Changes: the 377px dock becomes the baseline; the menu then says so.
  await chooseNativeMenu(session, 'Save Changes to This Workspace', () => more.click());
  await expect(page.locator('.toast, [data-toast]').filter({ hasText: 'Saved “My Edit”' }).first()).toBeVisible();
  await shot(page, '02-after-workspace-saved-toast-dark');
  const saved = await inspectNativeMenu(session, () => more.click());
  expect(saved.find(row => row.label === 'Save Changes to This Workspace')!.enabled).toBe(false);
  expect(saved.find(row => row.label === 'Reset to Saved Layout')!.enabled).toBe(false);

  // Drift again, then Reset returns to the saved 377px.
  await page.evaluate(() => (window as any).PM.WS.mutate((ws: any) => { ws.layout.docks.find((d: any) => d.id === 'left').size = 250; }));
  await chooseNativeMenu(session, 'Reset to Saved Layout', () => more.click());
  await expect.poll(() => page.evaluate(() => (window as any).PM.WS.current.layout.docks.find((d: any) => d.id === 'left').size)).toBe(377);
  await shot(page, '02-after-workspace-reset-toast-dark');

  // Set as Default: checked in the menu, marked in the list, used by new projects.
  await chooseNativeMenu(session, 'Set as Default Workspace', () => more.click());
  expect(await page.evaluate(() => (window as any).PM.WS.defaultId())).toBe(id);
  await more.click();
  const defaultRow = page.locator(`${MENU} .di`).filter({ hasText: 'Set as Default Workspace' });
  await expect(defaultRow).toHaveAttribute('aria-checked', 'true');
  await expect(page.locator(`${MENU} .di`).filter({ hasText: 'My Edit' }).locator('.kb')).toHaveText('Default');
  for (const value of ['dark', 'light'] as const) {
    await theme(page, value);
    await shot(page, `02-after-workspace-menu-default-${value}`);
  }
  await page.evaluate(() => (window as any).PM.closeMenus());
  await theme(page, 'dark');
  const opened = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.WS.activate('animate', true);
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Default project' }) }));
    return PM.WS.current.id;
  });
  expect(opened).toBe(id);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('My Presets: save from a layer, apply by double-click and drag, rename and delete', async ({ session }) => {
  test.setTimeout(120_000);
  const { page } = session;
  await page.setViewportSize({ width: 1440, height: 900 });
  await session.openEditor();
  const ids = await page.evaluate(() => {
    const PM = (window as any).PM;
    const source = PM.mkLayer('shape', { name: 'Card', p: { 'position.x': 760, 'position.y': 540 }, d: { shape: 'rect', w: 420, h: 300, color: '#f4dfb0', stroke: 0 } });
    const target = PM.mkLayer('shape', { name: 'Badge', p: { 'position.x': 1300, 'position.y': 540 }, d: { shape: 'rect', w: 260, h: 260, color: '#8fb8ff', stroke: 0 } });
    PM.proj.layers = [source, target]; PM.ProjectIndex.invalidate();
    const fx = PM.mkEffect('blur', undefined, source);
    fx.p.amount.kf = [PM.KF(0, 0, 'glide'), PM.KF(1, 24, 'glide')]; source.fx.push(fx);
    PM.animate(source, 'opacity', [{ t: 0, v: 0 }, { t: 1, v: 100 }]);
    PM.time = 0; PM.touch(); PM.selectLayers(source.id);
    PM.WS.activate('effects', true);
    return { source: source.id, target: target.id };
  });
  await page.waitForTimeout(400);

  // Empty My Presets teaches how to save one.
  const browser = page.locator('.fxb');
  await expect(browser.locator('[data-presets-empty]')).toContainText('Save Animation Preset…');
  for (const value of ['dark', 'light'] as const) { await theme(page, value); await shot(page, `02-after-my-presets-empty-${value}`); }
  await theme(page, 'dark');

  // Layer context menu ▸ Save Animation Preset…
  const openLayerMenu = () => page.evaluate(id => { const PM = (window as any).PM; PM.showLayerMenu(PM.L(id), { clientX: 640, clientY: 420 }); }, ids.source);
  await openLayerMenu();
  await expect(page.locator(`${MENU} .di`).filter({ hasText: 'Save Animation Preset…' })).toBeVisible();
  await shot(page, '02-after-layer-menu-save-preset-dark');
  await page.locator(`${MENU} .di`).filter({ hasText: 'Save Animation Preset…' }).click();

  const sheet = page.locator('[data-preset-sheet]');
  await expect(sheet).toBeVisible();
  await expect(sheet.locator('[data-preset-effect]')).toHaveAttribute('aria-checked', 'true');
  await expect(sheet.locator('[data-preset-property="Opacity"]')).toHaveAttribute('aria-checked', 'true');
  await expect(sheet.locator('[data-preset-summary]')).toHaveText('Saves Gaussian Blur and Opacity with 4 keyframes, placed from the playhead when applied.');
  for (const value of ['dark', 'light'] as const) { await theme(page, value); await shot(page, `02-after-save-sheet-${value}`); }
  await theme(page, 'dark');

  // The engine's validation shows inline; the sheet stays open.
  const name = sheet.getByLabel('Preset name');
  await name.fill('');
  await page.getByRole('button', { name: 'Save Preset' }).click();
  await expect(page.locator('.modal-action-error')).toContainText('Name the preset');
  await shot(page, '02-after-save-sheet-validation-dark');

  // Untick the effect: the preview line follows.
  await sheet.locator('[data-preset-effect]').click();
  await expect(sheet.locator('[data-preset-summary]')).toHaveText('Saves Opacity with 2 keyframes, placed from the playhead when applied.');
  await sheet.locator('[data-preset-effect]').click();
  await name.fill('Soft entrance');
  await name.press('Enter');
  await expect(sheet).toHaveCount(0);
  await expect(browser.locator('[data-kind="preset"]')).toHaveCount(1);
  await expect(browser.locator('[data-kind="preset"] .fxb-name')).toHaveText('Soft entrance');
  for (const value of ['dark', 'light'] as const) { await theme(page, value); await shot(page, `02-after-my-presets-list-${value}`); }
  await theme(page, 'dark');

  // Apply by double-click to the other layer at the playhead: one undo step.
  await page.evaluate(({ target }) => { const PM = (window as any).PM; PM.selectLayers(target); PM.time = 2; }, ids);
  await browser.locator('[data-kind="preset"] .fxb-pick').dblclick();
  await expect.poll(() => page.evaluate(({ target }) => (window as any).PM.L(target).fx.length, ids)).toBe(1);
  const applied = await page.evaluate(({ target }) => {
    const PM = (window as any).PM, layer = PM.L(target);
    return { amountKeys: layer.fx[0].p.amount.kf.map((k: any) => k.t), opacityKeys: layer.p.opacity.kf.map((k: any) => k.t) };
  }, ids);
  expect(applied.amountKeys).toEqual([2, 3]);
  expect(applied.opacityKeys).toEqual([2, 3]);
  await shot(page, '02-after-preset-applied-dark');
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(() => page.evaluate(({ target }) => {
    const layer = (window as any).PM.L(target);
    return [layer.fx.length, layer.p.opacity.kf.length];
  }, ids)).toEqual([0, 0]);

  // Drag onto the layer in the viewer.
  await page.evaluate(() => (window as any).PM.invalidate());
  {
    const point = await page.evaluate(({ target }) => {
      const PM = (window as any).PM;
      const stageEl = PM.Kernel.services.get('viewer').stage as HTMLElement;
      const rect = stageEl.getBoundingClientRect();
      const layer = PM.L(target);
      const x = rect.left + rect.width * (layer.p['position.x'].v / PM.proj.w);
      const y = rect.top + rect.height * (layer.p['position.y'].v / PM.proj.h);
      return { x, y };
    }, ids);
    const handle = await browser.locator('[data-kind="preset"] .fxb-pick').elementHandle();
    const data = await page.evaluateHandle(() => new DataTransfer());
    await handle!.dispatchEvent('dragstart', { dataTransfer: data });
    const stageHandle = await page.evaluateHandle(() => (window as any).PM.Kernel.services.get('viewer').stage);
    for (const type of ['dragenter', 'dragover', 'drop']) {
      await stageHandle.asElement()!.dispatchEvent(type, { dataTransfer: data, clientX: point.x, clientY: point.y });
    }
    await expect.poll(() => page.evaluate(({ target }) => (window as any).PM.L(target).fx.length, ids)).toBe(1);
  }

  // Rename inline from the context menu.
  await chooseNativeMenu(session, 'Rename…', () => browser.locator('[data-kind="preset"]').click({ button: 'right' }));
  const rename = browser.locator('.fxb-rename');
  await expect(rename).toBeFocused();
  await rename.fill('Gentle entrance');
  await page.waitForTimeout(400);
  await shot(page, '02-after-preset-rename-dark');
  await rename.press('Enter');
  await expect(browser.locator('[data-kind="preset"] .fxb-name')).toHaveText('Gentle entrance');
  expect(await page.evaluate(() => (window as any).PM.Presets.list()[0].name)).toBe('Gentle entrance');

  // Context menu, then Delete after confirming.
  await browser.locator('[data-kind="preset"]').click({ button: 'right' });
  await shot(page, '02-after-preset-context-menu-dark');
  await page.evaluate(() => (window as any).PM.closeMenus());
  await page.evaluate(() => { (window as any).PM.confirm = async () => true; });
  await chooseNativeMenu(session, 'Delete…', () => browser.locator('[data-kind="preset"]').click({ button: 'right' }));
  await expect(browser.locator('[data-kind="preset"]')).toHaveCount(0);
  await expect(browser.locator('[data-presets-empty]')).toBeVisible();
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Effect Controls saves the selected effect as a preset', async ({ session }) => {
  const { page } = session;
  await page.setViewportSize({ width: 1440, height: 900 });
  await session.openEditor();
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const layer = PM.mkLayer('shape', { name: 'Card', d: { shape: 'rect', w: 400, h: 300, color: '#f4dfb0', stroke: 0 } });
    PM.proj.layers = [layer]; PM.ProjectIndex.invalidate();
    layer.fx.push(PM.mkEffect('blur', undefined, layer));
    PM.touch(); PM.selectLayers(layer.id); PM.WS.activate('effects', true);
  });
  await theme(page, 'dark');
  const head = page.locator('[data-effects-layer] .fx-head').first();
  await expect(head).toBeVisible();
  await head.click({ button: 'right' });
  await expect(page.locator(`${MENU} .di`).filter({ hasText: 'Save Animation Preset…' })).toBeVisible();
  await shot(page, '02-after-effect-controls-menu-dark');
  await page.locator(`${MENU} .di`).filter({ hasText: 'Save Animation Preset…' }).click();
  const sheet = page.locator('[data-preset-sheet]');
  await expect(sheet.locator('[data-preset-effect]')).toHaveAttribute('aria-checked', 'true');
  await expect(sheet.getByLabel('Preset name')).toHaveValue('Card Gaussian Blur');
  await page.getByRole('button', { name: 'Save Preset' }).click();
  await expect(sheet).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).PM.Presets.list().map((p: any) => [p.name, p.effects.length, p.properties.length]))).toEqual([['Card Gaussian Blur', 1, 0]]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
