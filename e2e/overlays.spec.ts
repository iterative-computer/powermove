import { expect, test } from './helpers/app';

test.describe('@overlays Svelte overlays', () => {
  test('runs a palette command and manages modal focus and toast lifetime', async ({ session }) => {
    test.setTimeout(20_000);
    const { page, diagnostics } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
    const before = await page.evaluate(() => (window as any).PM.proj.layers.length as number);
    await page.keyboard.press('Meta+K');
    const palette = page.locator('#palette[data-svelte-overlay-palette="true"]');
    await expect(palette).toHaveCount(1);
    await palette.locator('input[role="combobox"]').fill('New solid');
    await page.keyboard.press('Enter');
    await expect(palette).toHaveCount(0);
    await page.waitForFunction((count) => (window as any).PM.proj.layers.length === count + 1, before);
    expect(await page.evaluate(() => (window as any).PM.proj.layers.some((layer: any) => layer.type === 'solid'))).toBe(true);

    await page.evaluate(() => {
      const trigger = document.createElement('button');
      trigger.id = 'overlay-e2e-trigger';
      trigger.textContent = 'Open dialog';
      document.getElementById('app')?.appendChild(trigger);
      trigger.focus();
      (window as any).PM.modal({ title: 'Overlay test', body: 'Live body', actions: [{ label: 'Done', pri: true }] });
    });
    await expect(page.locator('#scrim')).toHaveClass(/\bon\b/);
    await expect(page.locator('.modal[data-svelte-overlay-modal="true"]')).toContainText('Live body');
    await page.keyboard.press('Escape');
    await expect(page.locator('.modal[data-svelte-overlay-modal="true"]')).toHaveCount(0);
    await expect(page.locator('#scrim')).not.toHaveClass(/\bon\b/);
    expect(await page.evaluate(() => document.activeElement?.id)).toBe('overlay-e2e-trigger');

    await page.evaluate(() => (window as any).PM.toast('Overlay ready', 100));
    await expect(page.locator('#toasts .toast')).toHaveText('Overlay ready');
    await expect(page.locator('#toasts .toast')).toHaveCount(0, { timeout: 2_000 });
    expect(diagnostics.pageErrors).toEqual([]);
  });
});
