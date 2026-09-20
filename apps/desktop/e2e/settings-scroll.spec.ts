import { expect, test } from './helpers/app';

test('Settings is a continuous page with section navigation and highlighted search', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
  // Every section participates in layout, even before navigating to it.
  await expect(settings.locator('.sg-page:not([hidden])')).toHaveCount(4);
  const navigation = settings.getByRole('navigation', { name: 'Settings sections' });
  const project = navigation.getByRole('button', { name: 'Project', exact: true });
  await project.click();
  await expect(project).toHaveAttribute('aria-current', 'location');
  await expect(settings.getByRole('heading', { name: 'Project', exact: true })).toBeInViewport();

  await settings.locator('.sg-scroll').evaluate(el => el.scrollTo({ top: 0, behavior: 'instant' }));
  await expect(project).toHaveAttribute('aria-current', 'location');
  await navigation.getByRole('button', { name: 'General', exact: true }).focus();
  await page.keyboard.press('ArrowDown');
  await expect(navigation.getByRole('button', { name: 'Accounts', exact: true })).toBeFocused();
  await expect(settings.getByRole('heading', { name: 'Accounts', exact: true })).toBeInViewport();

  const search = settings.getByRole('searchbox', { name: 'Search settings' });
  await search.fill('  fRaMe RaTe  ');
  await expect(settings.getByLabel('Frame rate', { exact: true })).toBeInViewport();
  await expect(settings.getByLabel('Width in pixels', { exact: true })).toBeHidden();
  await expect(settings.getByLabel('Export width in pixels', { exact: true })).toBeHidden();
  await expect(navigation.getByRole('button')).toHaveCount(1);
  await expect.poll(() => page.evaluate(() => {
    const highlight = CSS.highlights.get('settings-search');
    return highlight ? Array.from(highlight, range => range.toString().toLowerCase()) : [];
  })).toContain('frame rate');
  await settings.getByLabel('Frame rate', { exact: true }).fill('48');
  await settings.getByLabel('Frame rate', { exact: true }).blur();
  expect(await page.evaluate(() => (window as any).PM.proj.fps)).toBe(48);

  await search.fill('zz-no-such-setting');
  await expect(settings.getByRole('status').filter({ hasText: 'No settings' })).toContainText('zz-no-such-setting');
  await expect(settings.locator('.sg-page:visible')).toHaveCount(0);
  await search.fill('');
  await expect(settings.locator('.sg-page:visible')).toHaveCount(4);
  await expect(settings.getByLabel('Frame rate', { exact: true })).toHaveValue('48');
  expect(await page.evaluate(() => CSS.highlights.has('settings-search'))).toBe(false);

  await search.fill('theme');
  await settings.locator('select[aria-label="Appearance"]').selectOption('light');
  await expect.poll(() => page.evaluate(() => (window as any).PM.theme.current)).toBe('light');
  await page.screenshot({ path: '/tmp/powermove-settings-search.png' });
  await search.fill('');
  await page.screenshot({ path: '/tmp/powermove-settings-scroll.png' });
  await settings.locator('select[aria-label="Appearance"]').selectOption('dark');
  await search.fill('frame rate');
  await expect(settings.getByLabel('Frame rate', { exact: true })).toBeInViewport();
  await page.screenshot({ path: '/tmp/powermove-settings-search-dark.png' });
  await settings.getByRole('main').getByRole('button', { name: 'Done', exact: true }).click();
  expect(await page.evaluate(() => CSS.highlights.has('settings-search'))).toBe(false);
  expect(session.diagnostics.pageErrors).toEqual([]);
});


test('Settings search follows live extensions into their details and works without a project', async ({ session }) => {
  const { page } = session;
  await page.evaluate(() => (window as any).PM.SettingsUI.open());
  const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
  await expect(settings.getByRole('region', { name: 'Project', exact: true })).toHaveCount(0);
  const search = settings.getByRole('searchbox', { name: 'Search settings' });
  await search.fill('Search fixture');
  await expect(settings.getByRole('status').filter({ hasText: 'No settings' })).toBeVisible();
  // A provider refresh updates the imperative extension list while search is active.
  await session.app.evaluate(({ ipcMain, BrowserWindow }) => {
    const record = {
      id: 'search-fixture', scope: 'user', enabled: true, dir: '/tmp/search-fixture',
      manifest: { id: 'search-fixture', name: 'Search fixture', version: '1.0.0', description: 'Settings search fixture', contributes: ['panels'] },
      health: { state: 'ok' }
    };
    ipcMain.removeHandler('ext:list');
    ipcMain.handle('ext:list', () => [record]);
    BrowserWindow.getAllWindows().forEach(window => window.webContents.send('ext:changed', { ids: [record.id], reason: 'create' }));
  });
  await expect(settings.getByRole('button', { name: 'Open Search fixture' })).toBeVisible();
  await settings.getByRole('button', { name: 'Open Search fixture' }).click();
  await expect(settings.getByRole('heading', { name: 'Search fixture', exact: true })).toBeVisible();
  await expect(settings.locator('.settings-extension-detail')).toContainText('Identifier');
  await expect(settings.locator('.settings-extension-detail .settings-row:visible')).not.toHaveCount(0);
  await settings.locator('.settings-extension-back').click();
  await expect(settings.getByRole('button', { name: 'Open Search fixture' })).toBeVisible();
  await page.evaluate(() => (window as any).PM.SettingsUI.open('project'));
  await expect(search).toHaveValue('');
  await expect(settings.getByRole('navigation').getByRole('button', { name: 'General' })).toHaveAttribute('aria-current', 'location');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
