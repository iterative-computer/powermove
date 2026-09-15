import { expect, test } from './helpers/app';

test('keyboard paste places copied layers at the playhead and undo restores the stack', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Paste at playhead', dur: 12 });
    project.layers = [
      PM.mkLayer('solid', { name: 'Later', from: 3, dur: 2 }, project),
      PM.mkLayer('solid', { name: 'First', from: 1, dur: 4 }, project),
    ];
    PM.replaceProject(project);
    PM.selectLayers(project.layers.map((layer: any) => layer.id));
    (document.activeElement as HTMLElement)?.blur();
  });
  await page.keyboard.press('Meta+C');
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.setTime(6.5);
    PM.hist.clear();
  });
  await page.keyboard.press('Meta+V');
  await expect.poll(() => page.evaluate(() => {
    const PM = (window as any).PM;
    return PM.selLayers().map((layer: any) => ({ from: layer.from, dur: layer.dur }));
  })).toEqual([{ from: 8.5, dur: 2 }, { from: 6.5, dur: 4 }]);
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(4);
  await page.keyboard.press('Meta+Z');
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.map((layer: any) => layer.from))).toEqual([3, 1]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
