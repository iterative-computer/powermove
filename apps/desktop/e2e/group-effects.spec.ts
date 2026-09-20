import { expect, test } from './helpers/app';

test('groups composite their contents through visual layer features', async ({ session }) => {
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));

  const proof = await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Group effects', w: 320, h: 180, dur: 4, fps: 30, bg: '#000000' });
    const left = PM.mkLayer('shape', {
      name: 'Left', d: { w: 60, h: 60, color: '#FF0000' },
      p: { 'position.x': 100, 'position.y': 90 },
    }, project);
    const right = PM.mkLayer('shape', {
      name: 'Right', d: { w: 60, h: 60, color: '#FF0000' },
      p: { 'position.x': 220, 'position.y': 90 },
    }, project);
    project.layers = [left, right];
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
    PM.ProjectsScreen.hide();
    const group = PM.groupLayers([left.id, right.id], 'Styled cards');
    PM.selectLayers(group.id);

    const pixel = (x: number, y: number) => {
      const canvas = PM.renderFrameTo(0, 320, 180, { mblur: false });
      return Array.from(canvas.getContext('2d').getImageData(x, y, 1, 1).data) as number[];
    };
    const outsideBefore = pixel(64, 90);
    const add = PM.Edit.apply({ type: 'add_effect', target: group.id, effect: 'blur', parameters: { amount: 24 } }, { origin: 'agent' });
    const outsideAfter = pixel(64, 90);

    const mask = PM.mkMask('rect', project);
    mask.p.x.v = 100; mask.p.y.v = 90; mask.p.w.v = 80; mask.p.h.v = 80; mask.p.feather.v = 0;
    group.masks.push(mask); PM.touch();
    const maskedRight = pixel(220, 90);

    const transition = PM.Edit.apply({
      type: 'set_transition', layer: group.id, edge: 'in', transition: { type: 'crossfade', dur: .5 },
    }, { origin: 'agent' });
    return {
      group: group.id,
      add,
      transition,
      outsideBefore,
      outsideAfter,
      maskedRight,
      saved: JSON.parse(PM.serialize()).proj.layers.find((layer: any) => layer.id === group.id),
    };
  });

  expect(proof.add.ok).toBe(true);
  expect(proof.transition.ok).toBe(true);
  expect(proof.outsideBefore[0]).toBeLessThan(3);
  expect(proof.outsideAfter[0]).toBeGreaterThan(3);
  expect(proof.maskedRight[0]).toBeLessThan(3);
  expect(proof.saved.fx).toHaveLength(1);
  expect(proof.saved.masks).toHaveLength(1);
  expect(proof.saved.transitionIn?.type).toBe('crossfade');

  const inspector = page.locator('#panel-inspector');
  await expect(inspector.locator(`[data-inspector-layer="${proof.group}"]`)).toBeVisible();
  await expect(inspector.getByRole('button', { name: 'Add effect', exact: true })).toBeVisible();
  await expect(inspector.getByText('Gaussian Blur', { exact: true })).toBeVisible();
  await expect(inspector.getByText('Masks', { exact: true })).toBeVisible();
  await expect(inspector.getByRole('combobox', { name: 'Blend mode', exact: true })).toBeVisible();
  await expect(inspector.getByRole('radiogroup', { name: 'Motion blur', exact: true })).toBeVisible();
  await expect(inspector.getByRole('combobox', { name: 'Track matte', exact: true })).toBeVisible();
  expect(session.diagnostics.pageErrors).toEqual([]);
});
