import { expect, launchApp, test } from './helpers/app';

/* Settings › Project › Color (docs/rust-engine.md section 9): linear blending
   and precision, shown with the new engine only, each change one Undo step,
   and the new engine's stamp following a non-default setting. */

test('Settings › Project › Color edits linear blending and precision with the new engine', async ({}, info) => {
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await session.openEditor();
    const { page } = session;
    // A project opens in Standard with linear blending off and no color field.
    expect(await page.evaluate(() => 'color' in (window as any).PM.proj)).toBe(false);
    await page.getByRole('button', { name: 'Open settings', exact: true }).click();
    const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
    await expect(settings).toBeVisible();
    const panel = settings.getByRole('region', { name: 'Project', exact: true });
    await expect(panel).toContainText('Color');
    const linear = panel.getByLabel('Blend in linear light', { exact: true });
    const precision = panel.locator('select[aria-label="Precision"]');
    await expect(linear).toHaveAttribute('aria-pressed', 'false');
    await expect(precision).toHaveValue('float16');
    await linear.scrollIntoViewIfNeeded();
    await settings.screenshot({ path: info.outputPath('project-color.png') });

    await linear.click();
    await expect(linear).toHaveAttribute('aria-pressed', 'true');
    expect(await page.evaluate(() => (window as any).PM.proj.color))
      .toEqual({ mode: 'standard', linearBlending: true, precision: 'float16', blendDomain: 'hdrSafe' });
    await expect.poll(() => page.evaluate(() => (window as any).PM.proj.requiresEngine ?? null)).toBe('native');
    await precision.selectOption('float32');
    expect(await page.evaluate(() => (window as any).PM.proj.color.precision)).toBe('float32');
    await settings.screenshot({ path: info.outputPath('project-color-on.png') });

    // One Undo step per change.
    expect(await page.evaluate(() => (window as any).PM.hist.label())).toBe('Precision');
    await page.evaluate(() => (window as any).PM.hist.undo());
    expect(await page.evaluate(() => (window as any).PM.proj.color.precision)).toBe('float16');
    expect(await page.evaluate(() => (window as any).PM.hist.label())).toBe('Blend in linear light');
    await page.evaluate(() => (window as any).PM.hist.undo());
    expect(await page.evaluate(() => 'color' in (window as any).PM.proj)).toBe(false);
    await expect(linear).toHaveAttribute('aria-pressed', 'false');
    await expect.poll(() => page.evaluate(() => (window as any).PM.proj.requiresEngine ?? null)).toBeNull();

    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});

test('Settings › Project › Color is hidden without the new engine', async () => {
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '0' } });
  try {
    await session.openEditor();
    const { page } = session;
    await page.getByRole('button', { name: 'Open settings', exact: true }).click();
    const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
    const panel = settings.getByRole('region', { name: 'Project', exact: true });
    await expect(panel).toContainText('Composition');
    await expect(panel.getByLabel('Blend in linear light', { exact: true })).toHaveCount(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});
