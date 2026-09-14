import { expect, test } from './helpers/app';

test('numeric scrubbing handles relative motion beyond display bounds and undoes as one edit', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    // Hidden, unfocusable test windows cannot acquire native pointer lock.
    // Emulate only that browser boundary; main-process permission policy is
    // covered in permissions.test.ts, and the actual control/edit path runs here.
    let locked: Element | null = null;
    Object.defineProperty(document, 'pointerLockElement', { get: () => locked });
    document.body.requestPointerLock = async () => {
      locked = document.body;
      document.dispatchEvent(new Event('pointerlockchange'));
    };
    document.exitPointerLock = () => {
      locked = null;
      document.dispatchEvent(new Event('pointerlockchange'));
    };
    const PM = (window as any).PM;
    PM.Edit.apply({ type: 'add_layer', layerType: 'solid', name: 'Scrub target', content: { w: 100, h: 100 } });
  });
  const field = page.getByRole('spinbutton', { name: 'Position X', exact: true });
  await expect(field).toBeVisible();
  const before = await field.inputValue();
  const box = (await field.boundingBox())!;
  const x = box.x + box.width / 2, y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + 10, y);
  await expect.poll(() => page.evaluate(() => document.pointerLockElement === document.body)).toBe(true);
  const initial = Number(await field.getAttribute('aria-valuenow'));
  // Relative motion keeps working while the screen coordinates stay fixed.
  await page.evaluate(() => {
    window.dispatchEvent(new MouseEvent('mousemove', { movementX: 10000, movementY: 0, buttons: 1 }));
  });
  await expect.poll(async () => Number(await field.getAttribute('aria-valuenow'))).toBeGreaterThan(initial + 1000);
  await page.evaluate(() => {
    window.dispatchEvent(new MouseEvent('mousemove', { movementX: -20000, movementY: 0, buttons: 1 }));
  });
  await expect.poll(async () => Number(await field.getAttribute('aria-valuenow'))).toBeLessThan(initial - 1000);
  await page.mouse.up();
  await expect.poll(() => page.evaluate(() => document.pointerLockElement === null)).toBe(true);
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect(field).toHaveValue(before);

  // Losing lock (including Escape in Chromium) cancels the entire edit.
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + 10, y);
  await page.evaluate(() => {
    window.dispatchEvent(new MouseEvent('mousemove', { movementX: 5000, buttons: 1 }));
    document.exitPointerLock();
  });
  await page.mouse.up();
  await expect(field).toHaveValue(before);
  expect(await page.evaluate(() => document.pointerLockElement)).toBeNull();

  // A click still enters text editing without locking the cursor.
  await field.click();
  await expect(field).toBeEditable();
  expect(await page.evaluate(() => document.pointerLockElement)).toBeNull();
  await field.press('Escape');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
