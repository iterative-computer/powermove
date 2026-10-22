import { expect, test } from './helpers/app';

test('numeric keyframe times extend beyond the layer strip, Undo, and reject invalid times', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM.Kernel.services.get('timeline')?.cv));
  const ids = await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Numeric keyframe times', dur: 10 });
    const layer = PM.mkLayer('solid', { name: 'Late layer', from: 2, dur: 3 }, project);
    project.layers = [layer];
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
    PM.ProjectsScreen.hide();
    const current = PM.L(layer.id);
    PM.setKey(current, 'opacity', 3, 50);
    PM.setKey(current, 'opacity', 4, 100);
    PM.selectLayers(current.id);
    const timeline = PM.Kernel.services.get('timeline');
    timeline.setMode('layers');
    timeline.graph = false;
    timeline.pps = 45;
    timeline.scrollT = 0;
    timeline.scrollY = 0;
    timeline.reveal(current, ['opacity']);
    PM.hist.clear();
    PM.touch(); PM.invalidate();
    return { layer: current.id, key: current.p.opacity.kf[0].i };
  });
  const state = () => page.evaluate(ids => {
    const PM = (window as any).PM;
    const layer = PM.L(ids.layer);
    const key = layer.p.opacity.kf.find((key: any) => key.i === ids.key);
    return { time: key.t, value: key.v, from: layer.from, duration: layer.dur };
  }, ids);
  const openKey = async () => {
    await expect.poll(() => page.evaluate(ids => {
      const timeline = (window as any).PM.Kernel.services.get('timeline');
      return timeline.rows.some((row: any) => row.L.id === ids.layer && row.key === 'opacity');
    }, ids)).toBe(true);
    const point = await page.evaluate(ids => {
      const PM = (window as any).PM;
      const timeline = PM.Kernel.services.get('timeline');
      const layer = PM.L(ids.layer);
      const key = layer.p.opacity.kf.find((key: any) => key.i === ids.key);
      const index = timeline.rows.findIndex((row: any) => row.L.id === ids.layer && row.key === 'opacity');
      const box = timeline.cv.getBoundingClientRect();
      return {
        x: box.x + timeline.gut + (layer.from + key.t - timeline.scrollT) * timeline.pps,
        y: box.y + timeline.ruler + index * timeline.row - timeline.scrollY + timeline.row / 2,
      };
    }, ids);
    await page.mouse.dblclick(point.x, point.y);
    const dialog = page.getByRole('dialog', { name: 'Edit keyframe', exact: true });
    await expect(dialog).toBeVisible();
    return dialog;
  };

  for (const time of [1, 6]) {
    const dialog = await openKey();
    await dialog.getByRole('spinbutton', { name: 'Time (s)', exact: true }).fill(String(time));
    await dialog.getByRole('spinbutton', { name: 'Value', exact: true }).fill('75');
    await dialog.getByRole('button', { name: 'Apply', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect.poll(state).toEqual({ time: time - 2, value: 75, from: 2, duration: 3 });
    await page.keyboard.press('Meta+z');
    await expect.poll(state).toEqual({ time: 1, value: 50, from: 2, duration: 3 });
  }

  const dialog = await openKey();
  await dialog.getByRole('spinbutton', { name: 'Value', exact: true }).fill('75');
  for (const [time, error] of [
    [10 + 1 / 30, 'That time falls outside the composition.'],
    [4, 'Another keyframe already sits on that frame.'],
  ] as const) {
    await dialog.getByRole('spinbutton', { name: 'Time (s)', exact: true }).fill(String(time));
    await dialog.getByRole('button', { name: 'Apply', exact: true }).click();
    await expect(dialog.getByRole('status')).toHaveText(error);
    expect(await state()).toEqual({ time: 1, value: 50, from: 2, duration: 3 });
  }
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
