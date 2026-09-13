import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

test('panel picker, independent hover states, media icons, and titlebar spacing', async ({ session }, info) => {
  const { page } = session;
  await importFixture(page, 'tone.wav');
  await importFixture(page, 'h264-aac.mp4');
  await page.waitForFunction(() => Object.keys((window as any).PM.proj.assets).length >= 2);
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.open();
  });
  const model = page.locator('#panel-agent .agent-modelbar').getByRole('combobox', { name: 'Model', exact: true });
  const effort = page.locator('#panel-agent .agent-modelbar').getByRole('combobox', { name: 'Reasoning effort', exact: true });
  for (const theme of ['dark', 'light']) {
    await page.evaluate(theme => (window as any).PM.theme.apply(theme), theme);
    await model.hover();
    expect(await model.evaluate(el => getComputedStyle(el).backgroundColor)).not.toBe('rgba(0, 0, 0, 0)');
    expect(await effort.evaluate(el => getComputedStyle(el).backgroundColor)).toBe('rgba(0, 0, 0, 0)');
    expect(await page.locator('#panel-agent .agent-modelbar').evaluate(el => getComputedStyle(el).backgroundColor)).toBe('rgba(0, 0, 0, 0)');
    await effort.hover();
    expect(await effort.evaluate(el => getComputedStyle(el).backgroundColor)).not.toBe('rgba(0, 0, 0, 0)');
    expect(await model.evaluate(el => getComputedStyle(el).backgroundColor)).toBe('rgba(0, 0, 0, 0)');
    for (const preview of await page.locator('.asset-preview').all()) {
      expect(await preview.evaluate(el => getComputedStyle(el).backgroundImage)).toBe('none');
    }
    const waveformBars = await page.locator('.asset-preview.audio .asset-wave').evaluateAll(
      waveforms => waveforms.map(waveform => waveform.querySelectorAll('i').length)
    );
    expect(waveformBars.length).toBeGreaterThan(0);
    expect(waveformBars.every(count => count === 120)).toBe(true);
    await expect(page.locator('.asset-preview.video [data-icon="film"]').first()).toBeVisible();
    const titlebar = await page.locator('#titlebar').evaluate(el => ({ left: parseFloat(getComputedStyle(el).paddingLeft), height: el.getBoundingClientRect().height }));
    expect(titlebar).toEqual({ left: 96, height: 44 });
    await page.getByRole('button', { name: 'Open panel library' }).click();
    await page.screenshot({ path: info.outputPath(`panel-icons-${theme}.png`) });
    await page.keyboard.press('Escape');
  }
  await page.locator('#panel-agent').getByRole('button', { name: 'Choose focused panels' }).click();
  const picker = page.getByRole('dialog', { name: 'Panel focus', exact: true });
  await expect(picker).toBeVisible();
  await expect(picker.getByRole('button', { name: 'All panels', exact: true })).toHaveCount(0);
  await expect(picker.getByRole('button', { name: 'Done', exact: true })).toHaveCount(0);
  await picker.getByRole('checkbox', { name: 'Timeline', exact: true }).check();
  await expect(picker.getByRole('checkbox', { name: 'Timeline', exact: true })).toBeChecked();
  await page.screenshot({ path: info.outputPath('panel-focus.png') });
  await page.keyboard.press('Escape');
  await expect(picker).toBeHidden();
  expect(session.diagnostics.pageErrors).toEqual([]);
});
