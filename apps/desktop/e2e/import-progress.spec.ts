import { expect, test } from './helpers/app';

test('single-file imports show persistent progress through storage and clear it on success or failure', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const original = PM.MediaStore.put;
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    PM.MediaStore.put = async (...args: any[]) => { await gate; return original(...args); };
    (window as any).releaseImport = () => { PM.MediaStore.put = original; release(); };
    const canvas = document.createElement('canvas'); canvas.width = canvas.height = 16;
    canvas.toBlob(blob => { (window as any).pendingImport = PM.importFiles([new File([blob!], 'Artwork.png')]); });
  });
  const card = page.getByRole('status', { name: 'Importing file', exact: true });
  await expect(card).toBeVisible();
  await expect(card).toContainText('Importing file');
  await expect(card.getByRole('progressbar')).not.toHaveAttribute('aria-valuenow');
  for (const theme of ['light', 'dark']) {
    await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
    await card.screenshot({ path: test.info().outputPath(`import-progress-${theme}.png`) });
  }
  await page.evaluate(async () => { (window as any).releaseImport(); await (window as any).pendingImport; });
  await expect(card).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).PM.proj.layers.some((layer: any) => layer.name === 'Artwork.png'))).toBe(true);
  await page.evaluate(() => (window as any).PM.importFiles([new File(['invalid'], 'broken.png', { type: 'image/png' })]));
  await expect(card).toHaveCount(0);
  await expect(page.locator('[data-toast-error]')).toBeVisible();
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('switching projects during import removes progress without adding media to the new project', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const original = PM.MediaImport.fingerprint;
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    PM.MediaImport.fingerprint = async (...args: any[]) => { await gate; return original(...args); };
    (window as any).releaseImport = () => { PM.MediaImport.fingerprint = original; release(); };
    (window as any).pendingImport = PM.importFiles([new File(['image'], 'Waiting.png', { type: 'image/png' })]);
  });
  const card = page.getByRole('status', { name: 'Importing file', exact: true });
  await expect(card).toContainText('Importing file');
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Another project' }) }));
    (window as any).releaseImport();
    await (window as any).pendingImport;
  });
  await expect(card).toHaveCount(0);
  expect(await page.evaluate(() => Object.keys((window as any).PM.proj.assets))).toEqual([]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a successful import reads as success even when the file is named like a failure', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    const canvas = document.createElement('canvas'); canvas.width = canvas.height = 16;
    const blob = await new Promise<Blob>(resolve => canvas.toBlob(value => resolve(value!)));
    await PM.importFiles([new File([blob], 'error.png', { type: 'image/png' })]);
  });
  const toast = page.locator('.toast');
  await expect(toast).toContainText('Imported error.png');
  await expect(page.locator('.toast[data-toast-error]')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Dismiss notification' })).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).PM.proj.layers.some((layer: any) => layer.name === 'error.png'))).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
