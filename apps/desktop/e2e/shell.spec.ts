import { expect, test } from './helpers/app';

test.describe('@shell Svelte shell', () => {
  test('boots the Svelte chrome while the dock panels stay live', async ({ session }) => {
    const { page, diagnostics } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
    const titlebar = page.locator('#titlebar[data-svelte-shell="true"]');
    await expect(titlebar).toHaveCount(1);
    // One project per window: a document chip, never a strip of tabs.
    await expect(titlebar.locator('#doc-strip')).toHaveCount(1);
    await expect(titlebar.locator('[role="tablist"]')).toHaveCount(0);
    await expect(titlebar.locator('.project-doc')).toHaveCount(1);
    await expect(titlebar.locator('#toolbar-strip[data-svelte-toolbar]')).toHaveCount(1);
    await expect(page.locator('#body .panel')).not.toHaveCount(0);

    const activeId = await page.evaluate(() => (window as any).PM.proj.id as string);
    await expect(titlebar.locator(`.project-doc[data-project-id="${activeId}"]`)).toHaveCount(1);

    expect(diagnostics.pageErrors).toEqual([]);
  });

  test('swaps the window to the project picked from the library', async ({ session }) => {
    const { page, diagnostics } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));

    const firstId = await page.evaluate(() => (window as any).PM.proj.id as string);
    const secondId = await page.evaluate(() => {
      const PM = (window as any).PM;
      const second = PM.mkProject({ name: 'Second composition', dur: 4, w: 640, h: 360, fps: 24, bg: '#09090A' });
      PM.Projects.put(second);
      PM.bus.emit('projects:open');
      return second.id as string;
    });
    expect(secondId).not.toBe(firstId);

    await page.locator('#doc-strip .project-home').click();
    await expect(page.locator('#projects-screen')).toHaveClass(/\bon\b/);
    await page.locator('#projects-screen .ps-card').filter({ hasText: 'Second composition' }).first().click();

    await page.waitForFunction((id) => (window as any).PM.proj.id === id, secondId);
    // The window carries one document, and it is the one just picked.
    await expect(page.locator('#doc-strip .project-doc')).toHaveCount(1);
    await expect(page.locator(`#doc-strip .project-doc[data-project-id="${secondId}"]`)).toHaveCount(1);
    await expect(page.locator('#projects-screen')).not.toHaveClass(/\bon\b/);

    expect(diagnostics.pageErrors).toEqual([]);
  });

  test('renames this window\'s document in place from the titlebar', async ({ session }) => {
    const { page, diagnostics } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));

    const chip = page.locator('#doc-strip .project-doc');
    await chip.dblclick();
    const input = chip.locator('.project-doc-input');
    await expect(input).toHaveCount(1);
    await input.fill('Renamed from the titlebar');
    await input.press('Enter');

    await expect(chip.locator('.project-doc-label')).toHaveText('Renamed from the titlebar');
    await page.waitForFunction(() => (window as any).PM.proj.name === 'Renamed from the titlebar');

    expect(diagnostics.pageErrors).toEqual([]);
  });
});
