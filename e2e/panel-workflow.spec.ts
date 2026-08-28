import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => {
  await session.page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
});

test('library opens panels without the removed refinement feature', async ({ session }) => {
  const { page } = session;
  await expect(page.locator('.panel-refine')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Ask Powermove agent', exact: true })).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).PM.PanelRefiner)).toBeUndefined();
  await page.getByRole('button', { name: 'Open panel library', exact: true }).click();
  const library = page.getByRole('dialog', { name: 'Panel library', exact: true });
  await expect(library).toBeVisible();
  const icons = await library.locator('.panel-library-open svg').evaluateAll(nodes => nodes.map(node => (node as SVGElement).dataset.icon));
  expect(new Set(icons).size).toBe(icons.length);
  expect(icons).not.toContain('missing');
  expect(icons).not.toContain('panel');
  expect(await library.locator('.panel-library-row').count()).toBe(await page.evaluate(() => Object.keys((window as any).PM.PANELS).length));
  await expect(library.getByRole('button', { name: /^Refine / })).toHaveCount(0);
  await library.getByRole('searchbox', { name: 'Search panels' }).fill('Notes');
  await library.getByRole('button', { name: 'Open Notes', exact: true }).click();
  await expect(page.locator('#panel-notes')).toBeVisible();
  await page.locator('#panel-notes header').click({ button: 'right' });
  await expect(page.getByRole('menuitem', { name: /Refine/ })).toHaveCount(0);
  await page.keyboard.press('Escape');
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
  const options = page.locator('.spatial-focus-host');
  await expect(options).toBeHidden();
  await expect(page.locator('.spatial-compose textarea')).toHaveAttribute('placeholder', 'Full composition');
  await expect(page.locator('.spatial-send')).toBeVisible();
  await page.locator('.spatial-compose textarea').fill('Keep this draft');
  await page.waitForTimeout(360); // The deliberate arming interval protects a shake from becoming a selection.
  await page.mouse.move(region.x, region.y); await page.mouse.down();
  await page.mouse.move(region.toX, region.toY, { steps: 12 });
  await expect(options).toBeHidden();
  await page.mouse.up();
  await expect(options).toBeVisible();
  await expect(options.getByRole('button', { name: 'Choose focused panels' })).toBeVisible();
  await expect(options.getByRole('combobox', { name: 'Model', exact: true })).toBeVisible();
  await expect(options.getByRole('combobox', { name: 'Reasoning effort', exact: true })).toBeVisible();
  await expect(page.locator('.spatial-compose textarea')).toHaveValue('Keep this draft');
  await expect(page.locator('.spatial-target')).toContainText('Composition');
  await expect(page.locator('.spatial-target')).toContainText('Timeline');
  await page.keyboard.press('Escape');
  await expect(page.locator('.spatial-compose')).toHaveCount(0);
  await page.evaluate(() => (window as any).PM.SpatialAssistant.activate(40, 70));
  await expect(page.locator('.spatial-compose')).toBeVisible();
  await expect(options).toBeHidden();
  await page.waitForTimeout(360);
  // An undersized box must not reveal the options, even with remembered panel focus.
  await page.mouse.move(region.x, region.y); await page.mouse.down();
  await page.mouse.move(region.x + 20, region.y + 20); await page.mouse.up();
  await expect(page.locator('.spatial-compose')).toBeVisible();
  await expect(options).toBeHidden();
  await page.keyboard.press('Escape');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
