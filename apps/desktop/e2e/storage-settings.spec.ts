import { existsSync } from 'node:fs';
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';

test('Storage previews cleanup, preserves project data, persists preferences and works in both themes', async ({ session }) => {
  const { page, userData } = session;
  const backups = path.join(userData, 'backups', 'storage-fixture');
  await mkdir(backups, { recursive: true });
  for (let day = 1; day <= 4; day++) await writeFile(path.join(backups, `2026-01-0${day}T00-00-00-000Z.pmv`), 'synthetic recovery copy');
  const recovery = path.join(backups, 'failed-save.recovery.pmv');
  await writeFile(recovery, 'synthetic uncertain save');
  await page.evaluate(() => (window as any).PM.SettingsUI.open('storage'));
  const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
  const storage = settings.getByRole('region', { name: 'Storage', exact: true });
  expect(existsSync(path.join(userData, 'library-location.json'))).toBe(false);
  expect(existsSync(path.join(userData, 'Library'))).toBe(false);
  await expect(storage.getByRole('button', { name: 'Choose Folder…', exact: true })).toHaveCount(0);
  await expect(storage.getByRole('button', { name: 'Clean Up', exact: true })).toBeEnabled();
  const automatic = storage.getByRole('button', { name: 'Clean up automatically', exact: true });
  await expect(automatic).toHaveAttribute('aria-pressed', 'true');
  await automatic.click();
  await expect(automatic).toHaveAttribute('aria-pressed', 'false');
  await storage.getByRole('combobox', { name: 'Older backup budget' }).click();
  await page.getByRole('option', { name: '1 GB', exact: true }).click();
  await expect(storage.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  await expect.poll(async () => JSON.parse(await readFile(path.join(userData, 'store/storageSettings.json'), 'utf8'))).toEqual({ automatic: false, backupBudgetGiB: 1 });

  // The native Cancel path must not remove anything.
  await storage.getByRole('button', { name: 'Clean Up', exact: true }).click();
  expect(await readdir(backups)).toHaveLength(5);
  await session.app.evaluate(({ dialog }) => { dialog.showMessageBox = async () => ({ response: 0, checkboxChecked: false }); });
  await storage.getByRole('button', { name: 'Clean Up', exact: true }).click();
  await expect(storage.getByRole('status')).toContainText('Freed');
  expect(await readdir(backups)).toEqual(['2026-01-03T00-00-00-000Z.pmv', '2026-01-04T00-00-00-000Z.pmv', 'failed-save.recovery.pmv']);
  expect(await readFile(recovery, 'utf8')).toBe('synthetic uncertain save');

  // Compare with the existing General settings in each theme.
  for (const theme of ['light', 'dark']) {
    await settings.locator('select[aria-label="Appearance"]').selectOption(theme);
    await page.evaluate(() => (window as any).PM.SettingsUI.open('general'));
    await expect(settings.getByRole('heading', { name: 'General', exact: true })).toBeInViewport();
    await page.screenshot({ path: `/tmp/powermove-storage-general-${theme}.png` });
    await page.evaluate(() => (window as any).PM.SettingsUI.open('storage'));
    await expect(storage.getByRole('heading', { name: 'Storage', exact: true })).toBeInViewport();
    await page.screenshot({ path: `/tmp/powermove-storage-${theme}.png` });
  }
  await session.relaunch();
  await session.page.evaluate(() => (window as any).PM.SettingsUI.open('storage'));
  const reopened = session.page.getByRole('region', { name: 'Storage', exact: true });
  await expect(reopened.getByRole('button', { name: 'Clean up automatically', exact: true })).toHaveAttribute('aria-pressed', 'false');
  await expect(reopened.locator('select[aria-label="Older backup budget"]')).toHaveValue('1');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
