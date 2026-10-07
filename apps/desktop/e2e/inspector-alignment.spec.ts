import { expect, test } from './helpers/app';

for (const theme of ['light', 'dark']) {
  test(`Properties aligns labels with and without keyframe buttons (${theme})`, async ({ session }) => {
    const { page } = session;
    await session.openEditor();
    await page.evaluate((theme) => {
      const PM = (window as any).PM;
      document.documentElement.dataset.theme = theme;
      PM.replaceProject(PM.mkProject({ name: 'Property alignment' }));
      const layer = PM.mkLayer('text');
      PM.proj.layers.push(layer);
      PM.selectLayers([layer.id]);
      PM.bus.emit('layers');
    }, theme);
    const rows = page.locator('#panel-inspector .row.split');
    await expect(rows.filter({ hasText: 'Solo' })).toBeVisible();
    const positions = await rows.evaluateAll((elements) => elements.flatMap((row) => {
      const label = row.querySelector<HTMLElement>(':scope > .k');
      const field = row.querySelector<HTMLElement>(':scope > .vwrap');
      if (!label || !field || getComputedStyle(label).position === 'absolute') return [];
      const text = document.createRange();
      text.selectNodeContents(label);
      return [{ label: label.textContent, textX: text.getBoundingClientRect().left,
        fieldX: field.getBoundingClientRect().left,
        keyed: !!row.querySelector(':scope > .stopwatch') }];
    }));
    const solo = positions.find((row) => row.label === 'Solo')!;
    const blend = positions.find((row) => row.label === 'Blend mode')!;
    expect(solo.keyed).toBe(false);
    expect(blend.keyed).toBe(true);
    expect(solo.textX).toBe(blend.textX);
    expect(solo.fieldX).toBe(blend.fieldX);
    for (const row of positions) {
      expect(row.textX, row.label ?? '').toBe(solo.textX);
      expect(row.fieldX, row.label ?? '').toBe(solo.fieldX);
    }
    await page.getByRole('button', { name: 'Add keyframe for Blend mode', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Remove keyframe for Blend mode', exact: true })).toBeVisible();
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
