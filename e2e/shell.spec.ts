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
});
