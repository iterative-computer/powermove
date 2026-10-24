import { expect } from '@playwright/test';
import { chooseNativeMenu, test } from './helpers/app';
import path from 'node:path';

test('native timeline markers support ruler menus, Undo/Redo and save/reopen', async ({ session }, testInfo) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM.Kernel.services.get('timeline')?.cv));
  await page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Marker QA', dur: 10 }) }));
    PM.ProjectsScreen.hide();
    const layer = PM.mkLayer('solid', { name: 'Marker reference', from: 0, dur: 8 });
    PM.proj.layers = [layer]; PM.setTime(3); PM.hist.clear(); PM.invalidate();
  });
  const markers = () => page.evaluate(() => (window as any).PM.proj.markers.map((marker: any) => ({ t: marker.t, id: marker.id })));
  const point = async (time: number, pin = false) => page.evaluate(({ time, pin }) => {
    const timeline = (window as any).PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    return { x: box.x + timeline.gut + (time - timeline.scrollT) * timeline.pps,
      y: box.y + (pin ? timeline.ruler - 6 : 14) };
  }, { time, pin });
  for (const mode of ['layers', 'tracks']) {
    await page.evaluate(mode => {
      const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline');
      timeline.setMode(mode); timeline.pps = 55; timeline.scrollT = 0; PM.invalidate('timeline');
    }, mode);
    const time = mode === 'layers' ? 2 : 4;
    const at = await point(time);
    await chooseNativeMenu(session, 'Add marker here', () => page.mouse.click(at.x, at.y, { button: 'right' }));
    await expect.poll(async () => (await markers()).map((marker: any) => marker.t)).toContain(time);
    expect(await page.evaluate(() => (window as any).PM.time)).toBe(3);
    await page.keyboard.press('Meta+z');
    expect((await markers()).some((marker: any) => marker.t === time)).toBe(false);
    await page.keyboard.press('Meta+Shift+z');
    expect((await markers()).some((marker: any) => marker.t === time)).toBe(true);
    for (const scheme of ['light', 'dark']) {
      await page.evaluate(scheme => (window as any).PM.Kernel.setScheme(scheme), scheme);
      await testInfo.attach(`markers-${mode}-${scheme}`, { body: await page.locator('#tl-canvas').screenshot(), contentType: 'image/png' });
    }
  }
  const before = await markers();
  await session.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, path.join(session.userData, 'Markers.pmv'));
  expect(await page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  await session.relaunch(); await session.openEditor();
  expect(await session.page.evaluate(() => (window as any).PM.proj.markers.map((marker: any) => ({ t: marker.t, id: marker.id })))).toEqual(before);
  const reopened = session.page;
  const at = await reopened.evaluate(() => {
    const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline');
    timeline.pps = 55; timeline.scrollT = 0; PM.invalidate('timeline');
    const box = timeline.cv.getBoundingClientRect();
    return { x: box.x + timeline.gut + 2 * timeline.pps, y: box.y + timeline.ruler - 6 };
  });
  await chooseNativeMenu(session, 'Remove marker', () => reopened.mouse.click(at.x, at.y, { button: 'right' }));
  expect(await reopened.evaluate(() => (window as any).PM.proj.markers.map((marker: any) => marker.t))).toEqual([4]);
  await reopened.keyboard.press('Meta+z');
  expect(await reopened.evaluate(() => (window as any).PM.proj.markers.map((marker: any) => marker.t))).toEqual([2, 4]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
