import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { decodeProjectContainer } from '../src/shared/project-container';
import { chooseNativeMenu, expect, test } from './helpers/app';

test('right-click menus create a composition and preserve the current project on cancel', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => (window as any).PM.WS.activate('design', true));
  await expect(page.locator('#tl-canvas')).toBeVisible();
  const original = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.hist.do('Add original layer', () => PM.addLayer(PM.mkLayer('solid', { name: 'Original layer' }, PM.proj)));
    return { projectId: PM.proj.id, projectName: PM.proj.name, compId: PM.proj.compId };
  });
  const destination = path.join(session.userData, 'Multiple compositions.pmv');
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => {
      (globalThis as any).__compositionSaveDialogs = ((globalThis as any).__compositionSaveDialogs || 0) + 1;
      return { canceled: false, filePath };
    };
  }, destination);
  expect(await page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  const dialog = page.getByRole('dialog', { name: 'New Composition', exact: true });
  // Empty timeline space, below the layer rows: a row would open that layer's menu.
  const emptyTimeline = { x: 20, y: (await page.locator('#tl-canvas').boundingBox())!.height - 16 };

  /* The empty timeline offers it under New ▸. (The canvas here is covered by
     the original layer, and a layer's menu keeps to that layer.) */
  for (const selector of ['.tl-comp-tab[aria-selected="true"]', '#tl-canvas']) {
    await chooseNativeMenu(session, selector === '#tl-canvas' ? ['New', 'Composition…'] : 'New Composition…', () =>
      page.locator(selector).click({ button: 'right', position: selector === '#tl-canvas' ? emptyTimeline : { x: 20, y: 20 } }));
    await expect(dialog).toBeVisible();
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    expect(await page.evaluate(() => ({ projectId: (window as any).PM.proj.id, compId: (window as any).PM.proj.compId })))
      .toEqual({ projectId: original.projectId, compId: original.compId });
  }

  await chooseNativeMenu(session, ['New', 'Composition…'], () =>
    page.locator('#tl-canvas').click({ button: 'right', position: emptyTimeline }));
  await dialog.getByLabel('Composition Name', { exact: true }).fill('Composition from context menu');
  await dialog.locator('select[aria-label="Resolution preset"]').selectOption('1080x1080');
  await dialog.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.compName))
    .toBe('Composition from context menu');
  expect(await page.evaluate(() => ({ id: (window as any).PM.proj.id, name: (window as any).PM.proj.name })))
    .toEqual({ id: original.projectId, name: original.projectName });
  expect(await page.evaluate(() => (window as any).PM.Tabs.list().length)).toBe(1);
  await expect(page.locator('.tl-comp-tab')).toHaveCount(2);
  expect(await page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  expect(await session.app.evaluate(() => (globalThis as any).__compositionSaveDialogs)).toBe(1);
  const file = decodeProjectContainer(await readFile(destination)).document.proj;
  expect(file.id).toBe(original.projectId);
  expect(file.compName).toBe('Composition from context menu');
  expect(file.comps[original.compId].layers[0].name).toBe('Original layer');
  const saved = await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.hydrateProject(JSON.parse(PM.serialize()).proj);
    return { id: project.id, name: project.compName, width: project.w, layers: project.layers.length,
      originalLayers: Object.values(project.comps).flatMap((comp: any) => comp.layers.map((layer: any) => layer.name)) };
  });
  expect(saved).toEqual({ id: original.projectId, name: 'Composition from context menu', width: 1080, layers: 0, originalLayers: ['Original layer'] });
  await page.locator(`.tl-comp-tab[data-comp-id="${original.compId}"]`).click();
  expect(await page.evaluate(() => (window as any).PM.proj.layers.map((layer: any) => layer.name))).toEqual(['Original layer']);
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
  }, destination);
  await page.evaluate(() => (window as any).PM.openProject());
  expect(await page.evaluate(() => (window as any).PM.Comps.list().length)).toBe(2);
  const undone = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.hist.undo();
    return { count: PM.Comps.list().length, layers: PM.proj.layers.map((layer: any) => layer.name) };
  });
  expect(undone).toEqual({ count: 1, layers: ['Original layer'] });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
