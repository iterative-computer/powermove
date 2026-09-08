import { test, expect } from './helpers/app';

test('agent image attachments open a fitted preview and Escape returns focus', async ({ session }, testInfo) => {
  const { page } = session;
  await page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).waitFor();
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const canvas = document.createElement('canvas');
    canvas.width = 1600;
    canvas.height = 900;
    const ctx = canvas.getContext('2d')!;
    ctx.fillStyle = '#284f9e';
    ctx.fillRect(0, 0, 1600, 900);
    ctx.fillStyle = '#ffffff';
    ctx.font = '80px sans-serif';
    ctx.fillText('Attachment preview', 120, 470);
    Object.assign(PM.AgentUI.state, {
      phase: 'idle', legacyPhase: 'idle', run: null, panelRun: null,
      conversation: [{ role: 'user', text: 'Use this reference', attachments: [
        { id: 'preview-reference', name: 'reference.png', type: 'image/png', dataUrl: canvas.toDataURL() }
      ] }]
    });
  });
  const trigger = page.getByRole('button', { name: 'View reference.png', exact: true });
  await trigger.click();
  const preview = page.getByRole('dialog');
  await expect(preview).toBeVisible();
  const image = preview.getByRole('img', { name: 'reference.png', exact: true });
  await expect(image).toBeVisible();
  const bounds = await image.evaluate((node: HTMLImageElement) => {
    const box = node.getBoundingClientRect();
    return { loaded: node.complete && node.naturalWidth === 1600, width: box.width, height: box.height, fits: box.left >= 0 && box.top >= 0 && box.right <= innerWidth && box.bottom <= innerHeight };
  });
  expect(bounds.loaded).toBe(true);
  expect(bounds.width).toBeGreaterThan(300);
  expect(bounds.fits).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('attachment-preview.png') });
  await page.keyboard.press('Escape');
  await expect(preview).toHaveCount(0);
  await expect(trigger).toBeFocused();
  expect(session.diagnostics.pageErrors).toEqual([]);
});
