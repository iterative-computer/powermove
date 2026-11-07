import path from 'node:path';
import { expect, test, repoRoot } from './helpers/app';

test.use({ desktopLaunchOptions: { env: { CODEX_BINARY: path.join(repoRoot, 'src/main/codex/__fixtures__/fake-codex-app-server.sh') } } });

test('native dropdowns reopen and custom pickers toggle on repeated clicks', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
  await page.evaluate(() => (window as any).PM.Kernel.loader.builtinsReady);
  // 06d25cf64 moved menus into the shared overlay. Exercise real visible
  // triggers; pooled hidden panels are not controls the user can press.
  const menu = page.locator('[data-svelte-overlay-menu] .menu-panel[data-level="0"]');
  const triggers = page.locator('#body button[aria-haspopup="menu"], #body button.panel-options');
  expect(await triggers.count()).toBeGreaterThan(5);
  const checked: string[] = [];
  for (let index = 0; index < await triggers.count(); index++) {
    const trigger = triggers.nth(index);
    if (!await trigger.isVisible() || !await trigger.isEnabled()) continue;
    await trigger.scrollIntoViewIfNeeded();
    for (let open = 0; open < 3; open++) {
      await trigger.click();
      await expect(menu).toBeVisible();
      await page.keyboard.press('Escape');
      await expect(menu).toHaveCount(0);
    }
    checked.push(await trigger.getAttribute('aria-label') || String(index));
  }
  await test.info().attach('checked-dropdowns', { body: JSON.stringify(checked, null, 2), contentType: 'application/json' });

  await session.openAgent();
  for (const selector of ['.thread-trigger']) {
    const trigger = page.locator(selector);
    await expect(trigger).toBeVisible();
    await trigger.click();
    await expect(trigger).toHaveAttribute('aria-expanded', 'true');
    await trigger.click();
    await expect(trigger).toHaveAttribute('aria-expanded', 'false');
    await trigger.click();
    await expect(trigger).toHaveAttribute('aria-expanded', 'true');
    await page.keyboard.press('Escape');
    await expect(trigger).toHaveAttribute('aria-expanded', 'false');
  }
  await page.keyboard.press('Escape');
  await expect(page.locator('#agent-popover')).toBeHidden();
  await page.evaluate(() => (window as any).PM.cmd('newText'));
  const font = page.locator('button.font-select').first();
  await expect(font).toBeVisible();
  await font.click();
  await expect(page.locator('.font-menu')).toBeVisible();
  await font.click();
  await expect(page.locator('.font-menu')).toHaveCount(0);
  await font.click();
  await expect(page.locator('.font-menu')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.locator('.font-menu')).toHaveCount(0);
  // 21dd50aca removes the legacy preview wrapper from the modern viewer.
  // Exercise its still-supported CSS with an explicit old-extension fixture,
  // then choose values through the real current viewer dropdowns below.
  const legacyStyles = await page.evaluate(() => {
    document.documentElement.dataset.theme = 'dark';
    const preview = document.createElement('div'); preview.id = 'preview-controls';
    const quality = document.createElement('select'); quality.dataset.native = '';
    preview.append(quality); document.body.append(preview);
    try {
      return [quality, document.querySelector<HTMLElement>('#composition-zoom')!].map(control => {
        control.style.background = 'color-mix(in srgb,var(--tx) 5%,var(--bg-panel))';
        control.style.padding = '0 10px';
        const style = getComputedStyle(control);
        return { image: style.backgroundImage, appearance: style.appearance, right: style.paddingRight, arrow: style.backgroundPosition };
      });
    } finally { preview.remove(); }
  });
  for (const { image, ...style } of legacyStyles) {
    expect(image).toContain('data:image/svg+xml');
    expect(style).toEqual({ appearance: 'none', right: '26px', arrow: 'calc(100% - 10px) 50%' });
  }
  const quality = page.getByRole('combobox', { name: 'Preview resolution', exact: true });
  await quality.click(); await page.getByRole('option', { name: 'Quarter', exact: true }).click();
  await expect(page.locator('#preview-resolution')).toHaveValue('0.25');
  const zoom = page.getByRole('combobox', { name: 'Composition zoom', exact: true });
  await zoom.click(); await page.getByRole('option', { name: '100%', exact: true }).click();
  await expect(page.locator('#composition-zoom')).toHaveValue('1');
  await quality.screenshot({ path: test.info().outputPath('dropdown-preview.png') });
  await zoom.screenshot({ path: test.info().outputPath('dropdown-zoom.png') });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
