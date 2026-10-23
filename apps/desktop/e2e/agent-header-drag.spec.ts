import { expect, test } from './helpers/app';

for (const theme of ['dark', 'light']) {
  test(`drags the conversation header to float, cancel, and dock in ${theme}`, async ({ session }) => {
    await session.openEditor();
    await session.page.evaluate(theme => (window as any).PM.theme.apply(theme), theme);
    await session.openAgent();
    const { page } = session;
    const surface = page.locator('#agent-popover');
    const header = surface.locator('.thread-trigger');
    const composer = surface.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
    await composer.fill('Keep this draft while moving');
    const original = (await surface.boundingBox())!;
    const body = (await page.locator('#dock-center').boundingBox())!;
    const from = (await header.boundingBox())!;
    const x = body.x + body.width / 2;
    const y = body.y + body.height / 2;
    await page.mouse.move(from.x + 30, from.y + from.height / 2);
    await page.mouse.down();
    await page.mouse.move(x, y, { steps: 12 });
    await page.mouse.up();
    await expect(surface).toBeVisible();
    await expect(page.locator('.thread-popup')).toHaveCount(0);
    const floated = (await surface.boundingBox())!;
    expect(Math.abs(floated.x - original.x) + Math.abs(floated.y - original.y)).toBeGreaterThan(5);
    await expect(composer).toHaveText('Keep this draft while moving');
    await page.locator('#dock-center').click({ position: { x: 20, y: 80 } });
    await expect(surface).toBeVisible();

    const handle = (await header.boundingBox())!;
    await page.mouse.move(handle.x + 30, handle.y + handle.height / 2);
    await page.mouse.down();
    await page.mouse.move(handle.x + 90, handle.y + 80, { steps: 6 });
    await page.keyboard.press('Escape');
    await page.mouse.up();
    await expect(surface).toBeVisible();
    await expect.poll(async () => Math.round((await surface.boundingBox())!.x)).toBe(Math.round(floated.x));

    await header.click();
    await expect(page.locator('.thread-popup')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator('.thread-popup')).toHaveCount(0);
    const start = (await header.boundingBox())!;
    const right = (await page.locator('#dock-right').boundingBox())!;
    await page.mouse.move(start.x + 30, start.y + start.height / 2);
    await page.mouse.down();
    await page.mouse.move(right.x + right.width / 2, right.y + 8, { steps: 12 });
    await expect(page.locator('.panel-drop-preview')).toHaveClass(/on/);
    await page.mouse.up();
    await expect(page.locator('#dock-right #panel-agent')).toBeVisible();
    await expect(surface).toBeHidden();
    await expect(page.locator('#dock-right #panel-agent').getByRole('textbox', { name: 'Message Powermove agent', exact: true })).toHaveText('Keep this draft while moving');
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
