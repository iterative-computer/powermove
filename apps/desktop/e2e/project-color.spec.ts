import { expect, launchApp, test } from './helpers/app';

/* Settings › Project: Color depth belongs to Composition in either engine;
   linear blending belongs to Color with the new engine. Each change is one
   Undo step, and the native-only blend setting controls the engine stamp. */

test('Settings › Project edits linear blending and Color depth with the new engine', async ({}, info) => {
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
    const depth = panel.locator('select[aria-label="Color depth"]');
    await expect(linear).toHaveAttribute('aria-pressed', 'false');
    await expect(depth).toHaveValue('16');
    await linear.scrollIntoViewIfNeeded();
    await settings.screenshot({ path: info.outputPath('project-color.png') });

    await linear.click();
    await expect(linear).toHaveAttribute('aria-pressed', 'true');
    expect(await page.evaluate(() => (window as any).PM.proj.color))
      .toEqual({ mode: 'standard', linearBlending: true, blendDomain: 'hdrSafe' });
    await expect.poll(() => page.evaluate(() => (window as any).PM.proj.requiresEngine ?? null)).toBe('native');
    await depth.selectOption('32');
    expect(await page.evaluate(() => (window as any).PM.proj.colorDepth)).toBe(32);
    await settings.screenshot({ path: info.outputPath('project-color-on.png') });

    // One Undo step per change.
    expect(await page.evaluate(() => (window as any).PM.hist.label())).toBe('Project settings');
    await page.evaluate(() => (window as any).PM.hist.undo());
    expect(await page.evaluate(() => (window as any).PM.proj.colorDepth)).toBe(16);
    await expect(depth).toHaveValue('16');
    expect(await page.evaluate(() => (window as any).PM.proj.color.linearBlending)).toBe(true);
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
    await expect(panel.locator('select[aria-label="Color depth"]')).toHaveValue('16');
    await expect(panel.getByLabel('Blend in linear light', { exact: true })).toHaveCount(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});
