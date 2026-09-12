import { expect, launchApp, test } from './helpers/app';

test('Settings › Project edits the live composition and its export settings', async () => {
  const session = await launchApp();
  try {
    const { page } = session;
    await page.getByRole('button', { name: 'Open settings', exact: true }).click();
    const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
    await expect(settings).toBeVisible();

    const projectTab = settings.getByRole('tab', { name: 'Project', exact: true });
    await projectTab.click();
    await expect(projectTab).toHaveAttribute('aria-selected', 'true');

    const panel = settings.getByRole('tabpanel', { name: 'Project', exact: true });
    await expect(panel).toContainText('Composition');
    await expect(panel).toContainText('Export');

    await panel.getByLabel('Resolution preset').selectOption('1080x1920');
    await expect(panel.getByLabel('Width in pixels', { exact: true })).toHaveValue('1080');
    expect(await page.evaluate(() => [(window as any).PM.proj.w, (window as any).PM.proj.h])).toEqual([1080, 1920]);

    await panel.getByLabel('Height in pixels', { exact: true }).fill('1900');
    await panel.getByLabel('Height in pixels', { exact: true }).blur();
    expect(await page.evaluate(() => (window as any).PM.proj.h)).toBe(1900);

    await panel.getByLabel('Frame rate', { exact: true }).fill('48');
    await panel.getByLabel('Frame rate', { exact: true }).blur();
    expect(await page.evaluate(() => (window as any).PM.proj.fps)).toBe(48);

    await panel.getByLabel('Export quality').selectOption('max');
    // Transparent background only applies to image formats, so it unlocks
    // once the default format is switched away from video.
    await expect(panel.getByLabel('Transparent background')).toBeDisabled();
    await panel.getByLabel('Export format').selectOption('png');
    await panel.getByLabel('Transparent background').check();
    expect(await page.evaluate(() => (window as any).PM.proj.exportDefaults))
      .toMatchObject({ format: 'png', quality: 'max', alpha: true });

    // The stored project keeps the export settings, and the Export dialog
    // opens with them rather than with the built-in defaults.
    await settings.getByRole('button', { name: 'Done', exact: true }).click();
    expect(await page.evaluate(() => (window as any).PM.Export.defaults()))
      .toMatchObject({ quality: 'max', alpha: true });

    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});

test('New composition asks for the settings that matter before creating', async () => {
  const session = await launchApp();
  try {
    const { page } = session;
    await page.evaluate(() => (window as any).PM.newProject());
    const dialog = page.getByRole('dialog', { name: 'New composition', exact: true });
    await expect(dialog).toBeVisible();

    await dialog.getByLabel('Project name').fill('Square spot');
    await dialog.getByLabel('Resolution preset').selectOption('1080x1080');
    await dialog.getByLabel('Frame rate', { exact: true }).fill('48');
    await dialog.getByLabel('Duration in seconds').fill('6');

    const background = dialog.getByRole('button', { name: /^Background · / });
    await background.click();
    const colorPicker = page.getByRole('dialog', { name: 'Background', exact: true });
    await expect(colorPicker).toBeVisible();

    // Dismissing the picker must not send focus back through a native colour
    // input and reopen it. The New composition dialog remains in place.
    await page.mouse.click(8, 8);
    await expect(colorPicker).toHaveCount(0);
    await expect(dialog).toBeVisible();
    await page.waitForTimeout(100);
    await expect(colorPicker).toHaveCount(0);

    await background.click();
    await colorPicker.getByLabel('Background hex value').fill('#AABBCC');
    await colorPicker.getByRole('button', { name: 'OK', exact: true }).click();
    await dialog.getByRole('button', { name: 'Create', exact: true }).click();

    await expect.poll(() => page.evaluate(() => ({
      name: (window as any).PM.proj.name,
      w: (window as any).PM.proj.w,
      h: (window as any).PM.proj.h,
      fps: (window as any).PM.proj.fps,
      dur: (window as any).PM.proj.dur,
      bg: (window as any).PM.proj.bg
    }))).toEqual({ name: 'Square spot', w: 1080, h: 1080, fps: 48, dur: 6, bg: '#AABBCC' });

    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});

test('Settings opens once, whichever entry point asks for it', async () => {
  const session = await launchApp();
  try {
    const { page } = session;
    const settings = page.getByRole('dialog', { name: 'Settings', exact: true });

    await page.getByRole('button', { name: 'Open settings', exact: true }).click();
    await expect(settings).toBeVisible();

    // The scrim covers the titlebar button, but the ⌘, and ⌘⇧, shortcuts still
    // reach Settings while it is open. They lead to the one dialog: a second
    // request re-uses it and moves it to the asked-for tab instead of stacking
    // another copy.
    await page.evaluate(() => (window as any).PM.SettingsUI.open());
    await page.evaluate(() => (window as any).PM.SettingsUI.open('project'));
    await expect(settings).toHaveCount(1);
    await expect(settings.getByRole('tab', { name: 'Project', exact: true }))
      .toHaveAttribute('aria-selected', 'true');

    // Closing it releases the singleton, so Settings still opens afterwards.
    await settings.getByRole('button', { name: 'Done', exact: true }).click();
    await expect(settings).toHaveCount(0);
    await page.getByRole('button', { name: 'Open settings', exact: true }).click();
    await expect(settings).toHaveCount(1);
    await expect(settings.getByRole('tab', { name: 'General', exact: true }))
      .toHaveAttribute('aria-selected', 'true');

    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});
