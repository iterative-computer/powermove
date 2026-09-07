import { expect, launchApp, test } from './helpers/app';

test('Export dialog gates fields by format and accepts a custom size', async () => {
  const session = await launchApp();
  try {
    const { page } = session;
    await page.evaluate(() => (window as any).PM.Export.dialog());
    const dialog = page.getByRole('dialog', { name: 'Export', exact: true });
    await expect(dialog).toBeVisible();

    // Video format: quality visible, transparency hidden.
    await expect(dialog.getByText('Quality', { exact: true })).toBeVisible();
    await expect(dialog.getByText('Transparent background')).toBeHidden();
    await expect(dialog.getByRole('button', { name: 'Export video', exact: true })).toBeVisible();
    await expect(dialog).toContainText('frames ·');

    // Custom resolution reveals pixel inputs and updates the summary.
    await dialog.getByText('Resolution').locator('..').getByRole('button').click();
    await page.getByRole('menuitem', { name: 'Custom size…' }).dispatchEvent('click');
    const width = dialog.getByLabel('Export width in pixels');
    await expect(width).toBeVisible();
    await width.fill('1000');
    await width.blur();
    await expect(dialog).toContainText('1000×562');
    await expect(dialog.getByLabel('Export height in pixels')).toHaveValue('562');

    // Still frame: no frame rate / range / quality, transparency appears,
    // action button renames.
    await dialog.getByRole('button', { name: 'Images', exact: true }).click();
    await expect(dialog.getByText('Quality', { exact: true })).toBeHidden();
    await expect(dialog.getByText('Frame rate', { exact: true })).toBeHidden();
    await expect(dialog.getByText('Transparent background')).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Export frame', exact: true })).toBeVisible();

    await dialog.getByRole('button', { name: 'Code', exact: true }).click();
    await expect(dialog.getByRole('button', { name: 'Export code', exact: true })).toBeVisible();
    await expect(dialog.getByText('Resolution', { exact: true })).toBeHidden();
    await expect(dialog.getByText('Frame rate', { exact: true })).toBeHidden();
    await expect(dialog.getByText('Transparent background')).toBeHidden();
    await expect(dialog).toContainText('-web.zip');

    await expect(dialog.getByText('Agent handoff', { exact: true })).toBeVisible();
    await expect(dialog.getByLabel('Render preset name')).toBeHidden();
    await page.screenshot({ path: 'e2e/test-results/export-code-dialog.png' });
    await dialog.getByRole('button', { name: 'Video', exact: true }).click();
    await expect(dialog.getByLabel('Export width in pixels')).toHaveValue('1000');
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});
