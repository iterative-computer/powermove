import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => {
  await session.page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
});

test('library lists every registered panel, opens hidden panels, and offers local refinement and creation', async ({ session }, info) => {
  const { page } = session;
  await page.getByRole('button', { name: 'Open panel library', exact: true }).click();
  const library = page.getByRole('dialog', { name: 'Panel library', exact: true });
  await expect(library).toBeVisible();
  expect(await library.locator('.panel-library-row').count()).toBe(await page.evaluate(() => Object.keys((window as any).PM.PANELS).length));
  await expect(library).not.toContainText('All projects');
  await expect(library).not.toContainText('This project');
  await expect(page.locator('.titlebar-contribution')).toHaveCount(0);
  await page.screenshot({ path: info.outputPath('panel-library.png') });
  await library.getByRole('searchbox', { name: 'Search panels' }).fill('Notes');
  await expect(library.locator('.panel-library-row')).toHaveCount(1);
  await library.getByRole('button', { name: 'Refine Notes', exact: true }).click();
  const refiner = page.getByRole('region', { name: 'Refine Notes', exact: true });
  await expect(refiner).toBeVisible();
  await expect(refiner.getByRole('button', { name: 'Choose focused panels' })).toHaveText('Notes');
  await expect(refiner.getByRole('combobox', { name: 'Model', exact: true })).toBeVisible();
  await refiner.getByRole('textbox', { name: 'Message Powermove agent' }).fill('Make these controls easier to scan');
  await refiner.getByRole('button', { name: 'Choose focused panels' }).click();
  const popup = page.getByRole('dialog', { name: 'Panel focus', exact: true });
  await popup.getByRole('checkbox', { name: 'Timeline', exact: true }).check();
  await popup.getByRole('button', { name: 'Done' }).click();
  await expect(refiner.getByRole('button', { name: 'Choose focused panels' })).toHaveText('2 panels');
  await expect(refiner.getByRole('textbox', { name: 'Message Powermove agent' })).toHaveValue('Make these controls easier to scan');
  await page.screenshot({ path: info.outputPath('panel-composer.png') });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.AgentUI.setAccess('project');
    PM.CodexBridge.request = (prompt: string, _schema: any, _images: any, options: any) => new Promise((resolve) => {
      (window as any).__panelRun = { prompt, resolve, options };
    });
  });
  await refiner.getByRole('textbox', { name: 'Message Powermove agent' }).press('Enter');
  await page.waitForFunction(() => Boolean((window as any).__panelRun));
  const prompt = await page.evaluate(() => (window as any).__panelRun.prompt);
  expect(prompt).toContain('USER-SELECTED PANEL FOCUS');
  expect(prompt).toContain('panels:notes,timeline');
  expect(prompt).toContain('PANEL DESIGN DEFAULT');
  expect(prompt).toContain('POWERMOVE_UI_TARGET');
  await expect(refiner).toBeVisible();
  await page.evaluate(() => (window as any).__panelRun.resolve({ text: JSON.stringify({ summary: 'Ready', commands: [], artifacts: [], externalActions: [], notes: [] }) }));
  await expect(refiner).toContainText('Ready');
  await refiner.getByRole('button', { name: 'Close panel composer' }).click();
  await page.getByRole('button', { name: 'Open panel library', exact: true }).click();
  await library.getByRole('button', { name: 'New panel', exact: true }).click();
  await expect(page.locator('#agent-composer-agent')).toHaveValue('Create a new panel that ');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('drawing a box notes every touched panel and sends that focus with the prompt', async ({ session }) => {
  const { page } = session;
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const region = await page.evaluate(() => {
    const PM = (window as any).PM;
    const viewer = document.getElementById('panel-viewer')!.getBoundingClientRect();
    const timeline = document.getElementById('panel-timeline')!.getBoundingClientRect();
    PM.WindowCapture.request = async () => null;
    PM.SpatialAssistant.activate(40, 70);
    return { x: viewer.left + 40, y: viewer.bottom - 35, toX: viewer.right - 40, toY: timeline.top + 60 };
  });
  await expect(page.locator('.spatial-compose')).toBeVisible();
  await page.waitForTimeout(360); // The deliberate arming interval protects a shake from becoming a selection.
  await page.mouse.move(region.x, region.y); await page.mouse.down();
  await page.mouse.move(region.toX, region.toY, { steps: 12 }); await page.mouse.up();
  await expect(page.locator('.spatial-target')).toContainText('Composition');
  await expect(page.locator('.spatial-target')).toContainText('Timeline');
  await expect(page.locator('.spatial-focus-host')).toContainText('2 panels');
  await page.keyboard.press('Escape');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
