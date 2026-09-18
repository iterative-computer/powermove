import { expect, launchApp, test } from './helpers/app';

test('export inherits Settings spacing and rounded cards and controls in both themes', async ({ session }, testInfo) => {
  await session.openEditor();
  const { page } = session;
  for (const theme of ['light', 'dark']) {
    await page.evaluate(theme => {
      const PM = (window as any).PM;
      PM.theme.apply(theme);
      PM.proj.w = 1080; PM.proj.h = 1920;
      PM.Export.dialog();
    }, theme);
    const dialog = page.getByRole('dialog', { name: 'Export', exact: true });
    const field = dialog.locator('.pm-select.export-select').first();
    await expect(field).toHaveAttribute('data-squircle', '');
    const geometry = await dialog.evaluate(el => {
      const card = el.querySelector<HTMLElement>('.settings-section-body')!;
      const row = card.querySelector<HTMLElement>('.settings-row')!;
      const field = row.querySelector<HTMLElement>('.pm-select')!;
      const footer = el.querySelector<HTMLElement>('.mf .pri')!;
      return {
        padding: parseFloat(getComputedStyle(row).paddingRight),
        inset: card.getBoundingClientRect().right - field.getBoundingClientRect().right,
        cardClip: getComputedStyle(card).clipPath,
        fieldClip: getComputedStyle(field).clipPath,
        footerClip: getComputedStyle(footer).clipPath,
        overflow: el.scrollWidth > el.clientWidth,
      };
    });
    expect(geometry.padding).toBeGreaterThan(0);
    expect(geometry.inset).toBeCloseTo(geometry.padding, 0);
    expect(geometry.cardClip).not.toBe('none');
    expect(geometry.fieldClip).not.toBe('none');
    expect(geometry.footerClip).not.toBe('none');
    expect(geometry.overflow).toBe(false);
    await page.screenshot({ path: testInfo.outputPath(`export-${theme}.png`) });
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Export dialog gates fields by format and accepts a custom size', async () => {
  const session = await launchApp();
  try {
    await session.openEditor();
    const { page } = session;
    await page.evaluate(() => (window as any).PM.Export.dialog());
    const dialog = page.getByRole('dialog', { name: 'Export', exact: true });
    await expect(dialog).toBeVisible();

    // Video format: quality visible, transparency hidden.
    await expect(dialog.getByText('Quality', { exact: true })).toBeVisible();
    await expect(dialog.getByText('Transparent background')).toBeHidden();
    await expect(dialog.getByRole('button', { name: 'Export video', exact: true })).toBeVisible();
    await expect(dialog).toContainText('frames ·');

    await expect(dialog.getByRole('button', { name: 'Queue', exact: true })).toHaveCount(0);
    await dialog.locator('select[aria-label="Frame rate"]').selectOption('60');
    await expect(dialog.locator('select[aria-label="Frame rate"]')).toHaveValue('60');
    const blur = dialog.getByRole('switch', { name: 'Motion blur', exact: true });
    await blur.focus();
    await page.keyboard.press('Space');
    await expect(blur).toHaveAttribute('aria-checked', 'false');

    // Custom resolution reveals pixel inputs and updates the summary.
    await dialog.locator('select[aria-label="Resolution"]').selectOption('__custom__');
    const width = dialog.getByLabel('Export width in pixels');
    await expect(width).toBeVisible();
    await width.fill('1000');
    await width.blur();
    await expect(dialog).toContainText('1000×562');
    await expect(dialog.getByLabel('Export height in pixels')).toHaveValue('562');

    // Still frame: no frame rate / range / quality, transparency appears,
    // action button renames.
    await dialog.getByRole('button', { name: 'Images', exact: true }).click();
    await expect(dialog.getByText('Quality', { exact: true })).toBeHidden();
    await expect(dialog.getByText('Frame rate', { exact: true })).toBeHidden();
    await expect(dialog.getByText('Transparent background')).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Export frame', exact: true })).toBeVisible();

    await dialog.locator('select[aria-label="Format"]').selectOption('png');
    await expect(dialog.getByRole('combobox', { name: 'Frame rate', exact: true })).toBeVisible();
    await dialog.getByRole('button', { name: 'Project', exact: true }).click();
    await expect(dialog).toContainText('Keep every layer editable');
    await expect(dialog.getByRole('button', { name: 'Save project file' })).toBeVisible();
    await dialog.getByRole('button', { name: 'Code', exact: true }).click();
    await expect(dialog.getByRole('button', { name: 'Export code', exact: true })).toBeVisible();
    await expect(dialog.getByText('Resolution', { exact: true })).toBeHidden();
    await expect(dialog.getByText('Frame rate', { exact: true })).toBeHidden();
    await expect(dialog.getByText('Transparent background')).toBeHidden();
    await expect(dialog).toContainText('-web.zip');

    await expect(dialog.getByText('Agent handoff', { exact: true })).toBeVisible();
    await expect(dialog.getByLabel('Render preset name')).toBeHidden();
    await page.screenshot({ path: 'e2e/test-results/export-code-dialog.png' });
    await dialog.getByRole('button', { name: 'Video', exact: true }).click();
    await expect(dialog.getByLabel('Export width in pixels')).toHaveValue('1000');
    await page.screenshot({path:'/tmp/powermove-export-video.png'});
    await page.evaluate(() => { (window as any).PM.theme.apply('dark'); });
    // Fields are the Settings wells: the float surface, never transparent.
    await expect.poll(() => page.evaluate(() => {
      const trigger = document.querySelector<HTMLElement>('.pm-select.export-select')!;
      const style = getComputedStyle(trigger);
      return style.backgroundColor === getComputedStyle(document.documentElement).getPropertyValue('--bg-float').trim() || style.backgroundColor !== 'rgba(0, 0, 0, 0)';
    })).toBe(true);
    await page.screenshot({path:'/tmp/powermove-export-video-dark.png'});
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});

test('export button shows rendered frames, finishes, and can export again', async () => {
  const session = await launchApp();
  try {
    await session.openEditor();
    const { page } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.pause();
      PM.proj = PM.mkProject({name:'Export lifecycle',w:64,h:64,fps:30,dur:1,bg:'#ff6633'});
      PM.proj.exportDefaults = {...PM.Export.defaults(),format:'webm',audio:false,mblur:false,range:'all'};
      PM.download = async (blob: Blob) => { (window as any).__exportBytes = blob.size; };
      PM.Export.dialog();
    });
    await page.getByRole('button',{name:'Export video',exact:true}).click();
    await page.waitForFunction(() => (window as any).__exportBytes > 0);
    await page.waitForFunction(() => !(window as any).PM.Export.busy);
    await expect(page.getByRole('dialog')).toHaveCount(0);
    const result = await page.evaluate(async () => {
      const PM = (window as any).PM;
      (window as any).__exportBytes = 0;
      const result = await PM.Export.run({format:'webm',audio:false,mblur:false,range:'all'});
      return {result,busy:PM.Export.busy,bytes:(window as any).__exportBytes};
    });
    expect(result.result.error).toBeUndefined();
    expect(result.busy).toBe(false);
    expect(result.bytes).toBeGreaterThan(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await session.close(); }
});

test('progress displays actual pixels and cancelling releases the export', async () => {
  const session = await launchApp();
  try {
    await session.openEditor();
    const {page} = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.pause();
      PM.proj = PM.mkProject({name:'Preview check',w:640,h:360,fps:60,dur:30,bg:'#ff6633'});
      PM.download = async () => {};
      (window as any).__exportResult = PM.Export.run({format:'webm',audio:false,mblur:false,range:'all'});
    });
    const progress = page.getByRole('dialog', {name:'Exporting Preview check'});
    await expect(progress).toBeVisible();
    await page.waitForFunction(() => {
      const canvas = document.querySelector<HTMLCanvasElement>('.export-preview');
      return canvas && canvas.getContext('2d')!.getImageData(0,0,1,1).data[0] > 0;
    });
    await expect(progress.getByRole('progressbar')).toHaveAttribute('aria-valuenow', /[1-9]/);
    await page.locator('#scrim').click({ position: { x: 5, y: 5 } });
    await expect(progress).toBeVisible();
    await progress.focus();
    await page.keyboard.press('Escape');
    await expect(progress).toBeVisible();
    expect(await page.evaluate(() => (window as any).PM.Export.cancel)).toBe(false);
    await page.screenshot({path:'/tmp/powermove-export-progress.png'});
    await progress.getByRole('button',{name:'Cancel',exact:true}).click();
    const result = await page.evaluate(async () => ({result:await (window as any).__exportResult,busy:(window as any).PM.Export.busy}));
    expect(result.result.cancelled).toBe(true);
    expect(result.busy).toBe(false);
    await expect(page.locator('.export-scan')).toHaveCount(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await session.close(); }
});
