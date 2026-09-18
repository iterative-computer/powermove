import path from 'node:path';
import { expect, test, repoRoot } from './helpers/app';

test.use({ desktopLaunchOptions: { env: { CODEX_BINARY: path.join(repoRoot, 'src/main/codex/__fixtures__/fake-codex-app-server.sh') } } });

test('native dropdowns reopen and custom pickers toggle on repeated clicks', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
  await page.evaluate(() => (window as any).PM.Kernel.loader.builtinsReady);
  await session.app.evaluate(({ Menu }) => {
    (globalThis as any).__menuRequests = 0;
    Menu.prototype.popup = function (options) {
      (globalThis as any).__menuRequests++;
      options?.callback?.();
    };
  });
  let menuRequests = 0;
  const triggers = page.locator('button[aria-haspopup="menu"], button.panel-options');
  expect(await triggers.count()).toBeGreaterThan(5);
  const checked: string[] = [];
  for (let index = 0; index < await triggers.count(); index++) {
    const trigger = triggers.nth(index);
    if (!await trigger.isVisible() || !await trigger.isEnabled()) continue;
    const label = await trigger.getAttribute('aria-label');
    await test.step(label || `Dropdown ${index}`, async () => {
      for (let click = 0; click < 3; click++) {
        await trigger.click();
        await expect.poll(() => session.app.evaluate(() => (globalThis as any).__menuRequests)).toBe(++menuRequests);
      }
    });
    checked.push(label || String(index));
  }
  await test.info().attach('checked-dropdowns', { body: JSON.stringify(checked, null, 2), contentType: 'application/json' });

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
  // Older installed viewer extensions still emit inline background shorthand.
  // Verify the shell CSS also works without remounting those controls.
  await page.evaluate(() => {
    document.documentElement.dataset.theme = 'dark';
    for (const el of document.querySelectorAll<HTMLElement>('#composition-zoom,#preview-controls select')) {
      el.style.background = 'color-mix(in srgb,var(--tx) 5%,var(--bg-panel))';
      el.style.padding = '0 10px';
    }
  });
  for (const selector of ['#preview-controls select', '#composition-zoom']) {
    const control = page.locator(selector);
    expect(await control.evaluate(el => getComputedStyle(el).backgroundImage)).toContain('data:image/svg+xml');
    expect(await control.evaluate(el => ({ appearance: getComputedStyle(el).appearance, right: getComputedStyle(el).paddingRight, arrow: getComputedStyle(el).backgroundPosition }))).toEqual({ appearance: 'none', right: '26px', arrow: 'calc(100% - 10px) 50%' });
  }
  await page.locator('#preview-controls select').selectOption('0.25');
  await expect(page.locator('#preview-controls select')).toHaveValue('0.25');
  await page.locator('#composition-zoom').selectOption('1');
  await expect(page.locator('#composition-zoom')).toHaveValue('1');
  await page.locator('#preview-controls').screenshot({ path: '/private/tmp/pm-dropdown-padding-after.png' });
  await page.locator('#composition-zoom').screenshot({ path: '/private/tmp/pm-dropdown-zoom-after.png' });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
