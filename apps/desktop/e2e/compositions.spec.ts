import { expect, test } from './helpers/app';

test.describe('@compositions After Effects compositions', () => {
  test.beforeEach(async ({ session }) => {
    await session.openEditor();
    await session.page.evaluate(() => {
      const PM = (window as any).PM;
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Comp fixture', compName: 'Main' }) }));
      PM.ProjectsScreen.hide();
    });
    await session.page.waitForFunction(() => { const PM = (window as any).PM; return Boolean(PM?.GL?.gl && PM.Kernel.services.get('timeline')?.rows); });
  });

  test('pre-compose keeps every rendered pixel, and the new comp opens, nests and survives a save', async ({ session }) => {
    const { page } = session;
    const result = await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Precompose', compName: 'Main', w: 320, h: 180, fps: 30, dur: 6, bg: '#FFFFFF' });
      const a = PM.mkLayer('shape', { name: 'Card', from: 1, dur: 4, d: { w: 100, h: 80, color: '#EC6340' }, p: { 'position.x': 100, 'position.y': 90, rotation: 12 } }, project);
      const b = PM.mkLayer('solid', { name: 'Plate', from: 0, dur: 6, d: { w: 120, h: 60, color: '#386BDC' }, p: { 'position.x': 170, 'position.y': 60 } }, project);
      project.layers = [a, b]; PM.replaceProject(project); PM.setTime(2.5, { raw: true, force: true });
      const pixels = () => Array.from(PM.renderFrameTo(2.5, 320, 180).getContext('2d').getImageData(0, 0, 320, 180).data);
      const same = (x: number[], y: number[]) => x.every((v, i) => Math.abs(v - y[i]!) <= 2);
      const before = pixels();
      const moved = PM.Comps.precompose([a.id], { name: 'Card Comp', adjustDuration: true });
      const afterMove = pixels();
      const left = PM.Comps.precompose([b.id], { name: 'Plate Comp', mode: 'leave' });
      const afterLeave = pixels();
      return {
        moved: !!moved, left: !!left,
        moveSame: same(before, afterMove), leaveSame: same(before, afterLeave),
        layers: PM.proj.layers.map((layer: any) => [layer.name, layer.type]),
        list: PM.Comps.list().map((comp: any) => comp.name),
      };
    });
    expect(result).toEqual({
      moved: true, left: true, moveSame: true, leaveSame: true,
      layers: [['Card Comp', 'precomp'], ['Plate', 'precomp']],
      list: ['Card Comp', 'Main', 'Plate Comp'],
    });

    // The media browser lists every composition; the timeline shows one tab.
    await page.evaluate(() => (window as any).PM.WS?.activate?.('design', true));
    // Project shares its slot with Effect Controls, which is the tab shown by default.
    await page.evaluate(() => (window as any).PM.Layout.activateTab?.('assets'));
    const cards = page.locator('.asset-card.is-comp');
    await expect(cards).toHaveCount(3);
    await expect(page.locator('.asset-card.is-comp[aria-current="true"] b')).toHaveText('Main');
    await expect(page.locator('.tl-comp-tab')).toHaveCount(1);

    // Double-click a comp in the media browser opens it in a new timeline tab.
    await page.locator('.asset-card.is-comp', { hasText: 'Card Comp' }).dblclick();
    await expect(page.locator('.tl-comp-tab')).toHaveCount(2);
    await expect(page.locator('.tl-comp-tab[aria-selected="true"]')).toHaveText(/Card Comp/);
    const inside = await page.evaluate(() => { const PM = (window as any).PM; return { name: PM.proj.compName, layers: PM.proj.layers.map((l: any) => [l.name, l.from]), dur: PM.proj.dur }; });
    expect(inside).toEqual({ name: 'Card Comp', layers: [['Card', 0]], dur: 4 });

    // Nesting the parent inside its child is refused, as in AE.
    const refused = await page.evaluate(() => { const PM = (window as any).PM; const main = PM.Comps.list().find((c: any) => c.name === 'Main').id; return PM.Comps.addToTimeline(main); });
    expect(refused).toBeNull();

    // Clicking the other tab returns to Main; the saved project keeps every comp.
    await page.locator('.tl-comp-tab', { hasText: 'Main' }).click();
    const saved = await page.evaluate(() => {
      const PM = (window as any).PM;
      const proj = JSON.parse(PM.serialize()).proj;
      const reloaded = PM.hydrateProject(JSON.parse(JSON.stringify(proj)));
      return { root: reloaded.compName, comps: Object.values(reloaded.comps).map((c: any) => c.name).sort() };
    });
    expect(saved).toEqual({ root: 'Main', comps: ['Card Comp', 'Plate Comp'] });

    // Undo works across compositions without switching the open one.
    await page.evaluate(() => (window as any).PM.hist.undo());
    const undone = await page.evaluate(() => { const PM = (window as any).PM; return { open: PM.proj.compName, plate: PM.proj.layers.find((l: any) => l.name === 'Plate').type }; });
    expect(undone).toEqual({ open: 'Main', plate: 'solid' });
    await page.screenshot({ path: '/tmp/powermove-compositions-review.png' });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
