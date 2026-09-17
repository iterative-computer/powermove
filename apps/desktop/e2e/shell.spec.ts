import { expect, test } from './helpers/app';

test.describe('@shell Svelte shell', () => {
  test('boots the Svelte chrome while the dock panels stay live', async ({ session }) => {
    const { page, diagnostics } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
    const titlebar = page.locator('#titlebar[data-svelte-shell="true"]');
    await expect(titlebar).toHaveCount(1);
    await expect(titlebar.locator('#tabs [role="tablist"]')).toHaveCount(1);
    await expect(titlebar.locator('#tabs [role="tablist"] .project-new')).toHaveCount(0);
    await expect(titlebar.locator('#toolbar-strip[data-svelte-toolbar]')).toHaveCount(1);
    await expect(page.locator('#body .panel')).not.toHaveCount(0);

    const secondId = await page.evaluate(() => {
      const PM = (window as any).PM;
      const second = PM.mkProject({ name: 'Shell tab target', dur: 4, w: 640, h: 360, fps: 24, bg: '#09090A' });
      PM.Projects.put(second);
      PM.Projects.markOpen(second.id);
      PM.bus.emit('projects:tabs');
      return second.id as string;
    });
    const secondTab = titlebar.locator(`.project-doc[data-tab-id="${secondId}"]`);
    await expect(secondTab).toHaveCount(1);
    await secondTab.click();
    await expect(secondTab).toHaveAttribute('aria-selected', 'true');
    await page.waitForFunction((id) => (window as any).PM.proj.id === id, secondId);

    expect(diagnostics.pageErrors).toEqual([]);
  });

  test('closes the final project tab and can reopen it from Projects', async ({ session }) => {
    const { page, diagnostics } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
    await page.getByRole('button', { name: 'New project', exact: true }).click();
    await page.getByRole('dialog', { name: 'New composition', exact: true })
      .getByRole('button', { name: 'Create', exact: true }).click();
    const activeId = await page.evaluate(() => {
      const PM = (window as any).PM;
      const id = PM.proj.id as string;
      for (const tabId of PM.Projects.tabs()) if (tabId !== id) PM.Projects.markClosed(tabId);
      PM.confirmCloseProject = async () => true;
      PM.bus.emit('projects:tabs');
      return id;
    });

    await page.locator(`.project-doc[data-tab-id="${activeId}"] .project-doc-close`).click();

    await expect(page.locator('#tabs .project-doc')).toHaveCount(0);
    await expect(page.locator('#projects-screen')).toHaveClass(/\bon\b/);
    await page.keyboard.press('Escape');
    await expect(page.locator('#projects-screen')).toHaveClass(/\bon\b/);
    await page.locator('#projects-screen .ps-card').first().click();
    await expect(page.locator(`.project-doc[data-tab-id="${activeId}"]`)).toHaveCount(1);
    await expect(page.locator('#projects-screen')).not.toHaveClass(/\bon\b/);
    expect(diagnostics.pageErrors).toEqual([]);
  });
});
