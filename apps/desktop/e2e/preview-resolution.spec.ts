import { expect, test } from './helpers/app';

test('preview resolution controls real pixels, survives layout changes and restart, and preserves full captures', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Preview resolution', w: 1920, h: 1080, dur: 5 });
    const layer = PM.mkLayer('solid', { d: { color: '#33aa66' } }, project);
    project.layers = [layer];
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
  });
  const resolution = () => session.page.getByRole('combobox', { name: 'Preview resolution', exact: true });
  for (const [mode, label, width, height] of [['1', 'Full', 1920, 1080], ['0.5', 'Half', 960, 540], ['0.25', 'Quarter', 480, 270]] as const) {
    await resolution().click();
    await session.page.getByRole('option', { name: label, exact: true }).click();
    await expect.poll(() => page.evaluate(() => {
      const PM = (window as any).PM;
      return [PM.GL.canvas.width, PM.GL.canvas.height, PM.quality, PM.perf.auto];
    })).toEqual([width, height, Number(mode), false]);
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.WS.restoreSnapshot(PM.WS.snapshot()); PM.play();
    });
    await page.waitForTimeout(200);
    await page.evaluate(() => { const PM = (window as any).PM; PM.pause(); PM.setTime(.5); });
    await expect(resolution()).toContainText(label);
    const capture = await page.evaluate(() => {
      const PM = (window as any).PM;
      const quality = PM.quality;
      const canvas = PM.renderFrameTo(.5, 1920, 1080, { mblur: false });
      return { width: canvas.width, height: canvas.height, quality: PM.quality, before: quality, pixel: Array.from(canvas.getContext('2d').getImageData(960, 540, 1, 1).data) };
    });
    expect(capture).toMatchObject({ width: 1920, height: 1080, quality: Number(mode), before: Number(mode) });
    expect(capture.pixel[3]).toBe(255);
  }
  await page.evaluate(async () => (window as any).PM.store.flush());
  await session.relaunch();
  await session.openEditor();
  await expect(resolution()).toContainText('Quarter');
  await resolution().click();
  await session.page.getByRole('option', { name: 'Auto', exact: true }).click();
  await expect(resolution()).toContainText('Auto');
  await expect.poll(() => session.page.evaluate(() => {
    const PM = (window as any).PM;
    return [PM.previewResolution, PM.perf.auto, PM.quality];
  })).toEqual(['auto', true, 1]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
