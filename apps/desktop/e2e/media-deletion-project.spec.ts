import { expect, test } from './helpers/app';

test('a stale media deletion dialog cannot delete media after switching projects', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(async () => {
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = 16;
    const blob = await new Promise<Blob>(resolve => canvas.toBlob(value => resolve(value!), 'image/png'));
    await (window as any).PM.importFiles([new File([blob], 'Shared.png', { type: 'image/png' })]);
  });
  await session.app.evaluate(({ dialog }) => {
    dialog.showMessageBox = async (_window, options) => new Promise(resolve => {
      (globalThis as any).__mediaConfirmation = { options, resolve };
    });
  });
  await page.getByRole('button', { name: 'Delete Shared.png', exact: true }).click();
  await expect.poll(() => session.app.evaluate(() => (globalThis as any).__mediaConfirmation?.options.message)).toBe('Delete “Shared.png”?');
  const nextId = await page.evaluate(() => {
    const PM = (window as any).PM;
    (window as any).__originalMediaProject = PM.proj;
    const next = PM.mkProject({ name: 'Another copy' });
    next.assets = structuredClone(PM.proj.assets);
    next.layers = structuredClone(PM.proj.layers);
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: next }));
    return next.id;
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.id)).toBe(nextId);
  await session.app.evaluate(() => { (globalThis as any).__mediaConfirmation.resolve({ response: 0, checkboxChecked: false }); });
  await expect(page.getByText('Deletion stopped because you switched projects', { exact: true }).first()).toBeVisible();
  expect(await page.evaluate(() => {
    const PM = (window as any).PM;
    return { id: PM.proj.id, media: Object.values(PM.proj.assets).map((asset: any) => asset.name), layers: PM.proj.layers.map((layer: any) => layer.name) };
  })).toEqual({ id: nextId, media: ['Shared.png'], layers: ['Shared.png'] });
  expect(await page.evaluate(() => Object.values((window as any).__originalMediaProject.assets).map((asset: any) => asset.name))).toEqual(['Shared.png']);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
