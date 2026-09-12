import { expect, test } from './helpers/app';

test('floating prompt attaches images and binary files, removes them, and sends their contents', async ({ session }) => {
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.WindowCapture.request = async () => null;
    PM.SpatialAssistant.activate(40, 70);
  });
  const composer = page.locator('.spatial-compose');
  await expect(composer).toBeVisible();
  await expect(composer.locator('.spatial-focus-host')).toBeHidden();
  await composer.locator('textarea').fill('Use these references');
  const picker = page.waitForEvent('filechooser');
  await composer.getByRole('button', { name: 'Add attachments' }).click();
  await (await picker).setFiles([
    { name: 'reference.png', mimeType: 'image/png', buffer: Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==', 'base64') },
    { name: 'brief.pdf', mimeType: 'application/pdf', buffer: Buffer.from([37, 80, 68, 70, 45, 0, 255, 128]) },
    { name: 'remove.zip', mimeType: 'application/zip', buffer: Buffer.from([80, 75, 0, 255]) }
  ]);
  await expect(composer.locator('.agent-attachment')).toHaveCount(3);
  await expect(composer.locator('.agent-attachment img')).toBeVisible();
  await composer.getByRole('button', { name: 'Remove remove.zip' }).click();
  await expect(composer.locator('.agent-attachment')).toHaveCount(2);
  await expect(composer.locator('textarea')).toHaveValue('Use these references');
  await expect(composer.locator('textarea')).toBeFocused();
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.AgentUI.setAccess('project');
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    PM.CodexBridge.request = (_prompt: string, _schema: any, images: string[], options: any) => {
      (window as any).__attachmentRequest = { images, files: options.attachments };
      return new Promise(() => {});
    };
  });
  await composer.locator('textarea').press('Enter');
  await page.waitForFunction(() => Boolean((window as any).__attachmentRequest));
  const request = await page.evaluate(() => (window as any).__attachmentRequest);
  expect(request.images).toHaveLength(1);
  expect(request.files.map((file: any) => file.name)).toEqual(['brief.pdf']);
  expect([...Buffer.from(request.files[0].dataBase64, 'base64')]).toEqual([37, 80, 68, 70, 45, 0, 255, 128]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
