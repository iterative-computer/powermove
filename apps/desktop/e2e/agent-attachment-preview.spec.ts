import { test, expect, type LaunchedApp } from './helpers/app';

async function openAttachment(session: LaunchedApp) {
  await session.openEditor(); await session.openAgent();
  await session.page.evaluate(() => (window as any).PM.SpatialAssistant.open());
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
    // Load a real saved thread so delayed account/status updates cannot replace
    // a conversation injected only into the rendered view state.
    const project = PM.mkProject({ name: 'Attachment preview' });
    PM.store.set(`agentThreads.${project.id}`, {
      version: 1, activeId: 'preview-thread', threads: [{
        id: 'preview-thread', title: 'Use this reference', updatedAt: Date.now(),
        composerDraft: '', attachments: [], scope: 'workspace',
        conversation: [{ role: 'user', text: 'Use this reference', attachments: [
          { id: 'preview-reference', name: 'reference.png', type: 'image/png', dataUrl: canvas.toDataURL() }
        ] }]
      }]
    });
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
  });
  return {
    page,
    trigger: page.getByRole('button', { name: 'View reference.png', exact: true }),
    preview: page.getByRole('dialog', { name: 'reference.png', exact: true })
  };
}

test('agent image attachments open a fitted preview and Escape returns focus', async ({ session }, testInfo) => {
  const { page, trigger, preview } = await openAttachment(session);
  // Keyboard activation: focus returns to the chip that opened the preview.
  await trigger.focus();
  await page.keyboard.press('Enter');
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

test('a pointer-opened attachment preview closes on Escape without re-arming its chip', async ({ session }) => {
  const { page, trigger, preview } = await openAttachment(session);
  await trigger.click();
  await expect(preview).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(preview).toHaveCount(0);
  // Pointer clicks never leave a button focused (pointer-focus contract), but focus still lands somewhere real.
  await expect(trigger).not.toBeFocused();
  expect(await page.evaluate(() => document.activeElement !== null && document.activeElement !== document.body)).toBe(true);
  await page.keyboard.press('Space');
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  await expect(preview).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
