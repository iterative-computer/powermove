import { expect, test } from './helpers/app';

test('flat controls remain consistent across home, editor, settings, library, and export in both themes', async ({ session }, testInfo) => {
  const { page } = session;
  await page.setViewportSize({ width: 1440, height: 1000 });
  async function inspect(name: string) {
    const raised = await page.locator('button:visible').evaluateAll(buttons => buttons.filter(button => {
      const style = getComputedStyle(button);
      // Preserve selection underlines and the uniform boundary that makes color swatches readable.
      const selectionCue = button.matches('.export-destination[aria-pressed="true"], .color-choice');
      return style.backgroundImage.includes('gradient') || style.boxShadow !== 'none' && !button.matches(':focus-visible') && !selectionCue;
    }).map(button => ({ label: button.getAttribute('aria-label') || button.textContent?.trim(), className: button.className, shadow: getComputedStyle(button).boxShadow, image: getComputedStyle(button).backgroundImage })));
    expect(raised, name).toEqual([]);
    await page.screenshot({ path: testInfo.outputPath(`${name}.png`) });
  }
  for (const theme of ['dark', 'light']) {
    await page.evaluate(theme => {
      const PM = (window as any).PM;
      PM.theme.apply(theme);
      PM.ProjectsScreen.show();
    }, theme);
    await inspect(`home-${theme}`);
    await session.openEditor();
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const text = PM.mkLayer('text', { name: 'Design review', text: 'Make a clear move' });
      PM.proj.layers = [text]; PM.ProjectIndex.invalidate(); PM.touch(); PM.selectLayers(text.id);
      PM.SpatialAssistant.open();
    });
    await inspect(`editor-${theme}`);
    await page.getByRole('button', { name: 'Open settings', exact: true }).click();
    const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
    for (const tab of ['General', 'Project', 'Extensions']) {
      await settings.getByRole('button', { name: tab, exact: true }).click();
      await inspect(`settings-${tab.toLowerCase()}-${theme}`);
    }
    await settings.getByRole('main').getByRole('button', { name: 'Done', exact: true }).click();
    await page.getByRole('button', { name: 'Open panel library', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'Panel library', exact: true })).toBeVisible();
    await inspect(`library-${theme}`);
    await page.keyboard.press('Escape');
    await page.evaluate(() => (window as any).PM.Export.dialog());
    const exportDialog = page.getByRole('dialog', { name: 'Export', exact: true });
    await expect(exportDialog).toBeVisible();
    await inspect(`export-${theme}`);
    await exportDialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    await page.evaluate(() => (window as any).PM.newProject());
    const composition = page.getByRole('dialog', { name: 'New composition', exact: true });
    await composition.getByRole('button', { name: /^Background · / }).click();
    const color = page.getByRole('dialog', { name: 'Background', exact: true });
    await expect(color).toBeVisible();
    await inspect(`color-${theme}`);
    await color.getByRole('button', { name: 'OK', exact: true }).click();
    await composition.getByRole('button', { name: 'Cancel', exact: true }).click();
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
