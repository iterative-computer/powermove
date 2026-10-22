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
  // Panels outside the workspace stay mounted in #pm-panel-pool, an aria-hidden
  // host parked off screen (layout/PanelPool.svelte) that no one can click.
  // Its triggers are checked below once their panel is docked.
  const triggers = page.locator(':is(button[aria-haspopup="menu"], button.panel-options):not(#pm-panel-pool *)');
  const parked = page.locator('#pm-panel-pool').locator('button[aria-haspopup="menu"], button.panel-options');
  const parkedVisible: string[] = [];
  for (let index = 0; index < await parked.count(); index++) {
    if (await parked.nth(index).isVisible()) parkedVisible.push(await parked.nth(index).getAttribute('aria-label') || String(index));
  }
  expect(parkedVisible).toEqual(['Caption actions']);
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

  // Caption actions opens a popover menu, not a native one. Dock the Captions
  // panel as the panel library does, then bring its header into view.
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const ws = PM.WS.current;
    if (!PM.Layout.findPanel(ws, 'captions')) {
      const dock = ws.layout.docks.find((d: any) => d.id !== 'center') ?? ws.layout.docks[0];
      PM.WS.mutate((w: any) => PM.Layout.addPanel(w, 'captions', dock.id));
    }
  });
  const captionActions = page.locator('#panel-captions').getByRole('button', { name: 'Caption actions', exact: true });
  await captionActions.scrollIntoViewIfNeeded();
  await expect(captionActions).toBeInViewport();
  const captionMenu = page.getByRole('menu', { name: 'Caption actions', exact: true });
  await captionActions.click();
  await expect(captionActions).toHaveAttribute('aria-expanded', 'true');
  await expect(captionMenu).toBeVisible();
  await captionActions.click();
  await expect(captionActions).toHaveAttribute('aria-expanded', 'false');
  await expect(captionMenu).toHaveCount(0);
  await captionActions.click();
  await expect(captionActions).toHaveAttribute('aria-expanded', 'true');
  await expect(captionMenu).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(captionActions).toHaveAttribute('aria-expanded', 'false');
  await expect(captionMenu).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
