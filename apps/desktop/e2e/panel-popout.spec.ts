import { chooseNativeMenu, expect, test } from './helpers/app';

test('a panel opens in a native window, keeps edits, and returns to the layout', async ({ session }) => {
  await session.openEditor();
  const { page, app } = session;
  await page.getByRole('button', { name: 'Open panel library', exact: true }).click();
  const library = page.getByRole('dialog', { name: 'Panel library', exact: true });
  await library.getByRole('searchbox', { name: 'Search panels' }).fill('Notes');
  await library.getByRole('button', { name: 'Add Notes to workspace', exact: true }).click();
  const notes = page.getByRole('textbox', { name: 'Project notes', exact: true });
  await notes.fill('Before pop-out');
  await chooseNativeMenu(session, 'Pop out to window', () => page.locator('#panel-notes > header').click({ button: 'right' }));
  await expect.poll(() => app.windows().length).toBe(2);
  const child = app.windows().find(window => window !== page)!;
  await expect(child).toHaveTitle('Notes — Powermove');
  await expect(child.locator('body')).toHaveCSS('display', 'flex');
  await expect(child.getByRole('textbox', { name: 'Project notes', exact: true })).toHaveValue('Before pop-out');
  await child.getByRole('textbox', { name: 'Project notes', exact: true }).fill('Edited in the window');
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.notes)).toBe('Edited in the window');
  await child.getByRole('button', { name: 'Return Notes to the Powermove layout', exact: true }).click();
  await expect(notes).toBeVisible();
  await expect(notes).toHaveValue('Edited in the window');
  await expect.poll(() => app.windows().length).toBe(1);

  // Closing the native window also restores the live panel, and it can reopen.
  expect(await page.evaluate(() => (window as any).PM.Popout.open('notes'))).toBe(true);
  await expect.poll(() => app.windows().length).toBe(2);
  const reopened = app.windows().find(window => window !== page)!;
  await expect(reopened.getByRole('textbox', { name: 'Project notes', exact: true })).toHaveValue('Edited in the window');
  expect(await page.evaluate(() => (window as any).PM.Popout.open('notes'))).toBe(true);
  expect(app.windows()).toHaveLength(2);
  await app.evaluate(({ BrowserWindow }) => {
    BrowserWindow.getAllWindows().find(window => window.webContents.getURL() === 'about:blank')!.close();
  });
  await expect(notes).toBeVisible();
  await expect(notes).toHaveValue('Edited in the window');
  await expect.poll(() => app.windows().length).toBe(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('only named panel windows from the editor are allowed', async ({ session }) => {
  await session.openEditor();
  const { page, app } = session;
  expect(await page.evaluate(() => ({
    unnamed: window.open('about:blank') === null,
    malformed: window.open('about:blank', 'pm-panel-notes<script>') === null,
    appPage: window.open(location.href, 'pm-panel-notes') === null
  }))).toEqual({ unnamed: true, malformed: true, appPage: true });
  expect(await page.evaluate(() => (window as any).PM.Popout.open('assets'))).toBe(true);
  await expect.poll(() => app.windows().length).toBe(2);
  const child = app.windows().find(window => window !== page)!;
  await expect(child.locator('#panel-assets')).toBeVisible();
  expect(await child.evaluate(() => ({
    nested: window.open('about:blank', 'pm-panel-notes') === null,
    node: typeof (window as any).require,
    process: typeof (window as any).process
  }))).toEqual({ nested: true, node: 'undefined', process: 'undefined' });
});
