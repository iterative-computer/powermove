import { expect } from '@playwright/test';
import { test } from './helpers/app';

for (const surface of ['list', 'clip']) {
  test(`Shift-click selects anchored layer ranges in the ${surface}`, async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return Boolean(timeline?.cv); });
    await page.evaluate(() => {
      const PM = (window as any).PM;
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Range selection', dur: 10 }) })); PM.ProjectsScreen.hide();
      PM.proj.layers = Array.from({ length: 5 }, (_, i) => PM.mkLayer('solid', { name: `Layer ${i}`, from: 0, dur: 10 }));
      PM.selectLayers([]);
      PM.invalidate();
    });
    await expect.poll(() => page.evaluate(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return timeline.rows.filter((r: any) => r.kind === 'layer').length; })).toBe(5);
    const rows = await page.evaluate((surface) => {
      const PM = (window as any).PM;
      const timeline = PM.Kernel.services.get('timeline');
      const box = timeline.cv.getBoundingClientRect();
      return timeline.rows.flatMap((r: any, i: number) => r.kind === 'layer' ? [{ id: r.L.id,
        x: box.x + (surface === 'list' ? 110 : timeline.gut + 60),
        y: box.y + timeline.ruler + i * timeline.row - timeline.scrollY + 8 }] : []);
    }, surface);
    const click = async (index: number, modifier?: string) => {
      if (modifier) await page.keyboard.down(modifier);
      await page.mouse.click(rows[index].x, rows[index].y);
      if (modifier) await page.keyboard.up(modifier);
    };
    const selected = () => page.evaluate(() => (window as any).PM.sel.layers);
    await click(1);
    await click(4, 'Shift');
    expect(await selected()).toEqual(rows.slice(1).map((r: any) => r.id));
    await click(2, 'Shift');
    expect(await selected()).toEqual(rows.slice(1, 3).map((r: any) => r.id));
    await click(0, 'Shift');
    expect(await selected()).toEqual(rows.slice(0, 2).map((r: any) => r.id));
    await click(4, 'Meta');
    expect(await selected()).toEqual([rows[0].id, rows[1].id, rows[4].id]);
    await click(4, 'Meta');
    expect(await selected()).toEqual([rows[0].id, rows[1].id]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
