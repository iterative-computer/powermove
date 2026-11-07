import { expect, test } from './helpers/app';

test('consecutive empty Fill and Stroke headings stack without an extra margin', async ({ session }, testInfo) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Empty section spacing' });
    const layer = PM.mkLayer('shape', {}, project);
    layer.d.paths = [{ id: PM.uid('path'), name: 'Path', parent: null, vertices: [],
      p: Object.fromEntries(Object.entries({ x: 0, y: 0, rotation: 0, scaleX: 100, scaleY: 100,
        closed: false, fill: '#ffffff', fillEnabled: false, fillOpacity: 100, stroke: '#ffffff', strokeWidth: 0,
        strokeOpacity: 100, trimStart: 0, trimEnd: 100, trimOffset: 0, copies: 1, repeatX: 0, repeatY: 0, repeatRotation: 0 })
        .map(([key, value]) => [key, PM.P(value)])) }];
    project.layers = [layer];
    PM.replaceProject(project); PM.selectLayers(layer.id); PM.touch(); PM.invalidate();
  });
  const measurements = [];
  for (const scheme of ['light', 'dark']) {
    await page.evaluate(scheme => (window as any).PM.theme.apply(scheme), scheme);
    const fill = page.locator('[data-empty-section="Fill"]');
    const stroke = page.locator('[data-empty-section="Stroke"]');
    await expect(fill).toBeVisible(); await stroke.scrollIntoViewIfNeeded();
    measurements.push(await stroke.evaluate(element => ({
      height: element.getBoundingClientRect().height,
      margin: getComputedStyle(element.querySelector('.sec')!).marginTop,
    })));
    await page.locator('#panel-inspector').screenshot({ path: testInfo.outputPath(`empty-sections-${scheme}.png`) });
  }
  expect(measurements).toEqual([{ height: 33, margin: '0px' }, { height: 33, margin: '0px' }]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
