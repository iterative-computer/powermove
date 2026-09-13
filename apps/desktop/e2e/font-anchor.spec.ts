import { expect } from '@playwright/test';
import { test } from './helpers/app';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

test('font changes and animation preserve a centered text anchor', async ({ session }) => {
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Font anchor', dur: 5 })); PM.setTime(0);
    const layer = PM.mkLayer('text', { d: { text: 'Anchor\nTypography', size: 80 } });
    PM.proj.layers.push(layer); PM.ProjectIndex.invalidate();
    const initial = PM.GL.bounds(layer, 0);
    layer.p['anchor.x'].v = (initial.x0 + initial.x1) / 2;
    layer.p['anchor.y'].v = (initial.y0 + initial.y1) / 2;
    const errors: number[] = [];
    const check = (time = 0) => {
      const b = PM.GL.bounds(layer, time);
      errors.push(Math.abs((b.x0 + b.x1) / 2 - PM.ev(layer, 'anchor.x', time)),
        Math.abs((b.y0 + b.y1) / 2 - PM.ev(layer, 'anchor.y', time)));
    };
    for (const [key, value] of Object.entries({ size: 150, tracking: 12, leading: 1.8, weight: 800, italic: true, align: 'right', font: 'Georgia' })) {
      const edit = PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'c.' + key, value, preserveHandEdits: false });
      if (!edit.ok) throw new Error(JSON.stringify(edit));
      check();
    }
    PM.Edit.apply({ type: 'replace_keyframes', target: layer.id, path: 'c.size', keyframes: [{ time: 0, value: 80 }, { time: 4, value: 180 }], preserveHandEdits: false });
    for (const time of [0, 1, 2, 3, 4]) check(time);
    const beforeMove = PM.GL.bounds(layer, 0);
    PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'anchor.x', value: 25, preserveHandEdits: false });
    const afterMove = PM.GL.bounds(layer, 0);
    errors.push(Math.abs(beforeMove.x0 - afterMove.x0), Math.abs(beforeMove.x1 - afterMove.x1));
    const saved = PM.hydrateProject(JSON.parse(PM.serialize()).proj);
    return { errors, persisted: !!saved.layers.find((item: any) => item.id === layer.id).d.fontAnchorBounds };
  });
  expect(Math.max(...result.errors), JSON.stringify(result)).toBeLessThan(0.001);
  expect(result.persisted).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
