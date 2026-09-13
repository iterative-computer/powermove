import { expect, test } from './helpers/app';

test.describe('@svelte-panel Svelte panel hosted in the legacy layout', () => {
  test('the perf panel mounts once, renders, and reacts to typed edits', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    const before = await page.evaluate(() => {
      const PM = (window as any).PM;
      const ws = PM.WS.current;
      if (!PM.Layout.findPanel(ws, 'perf')) {
        const dock = ws.layout.docks.find((d: any) => d.id !== 'center') ?? ws.layout.docks[0];
        PM.WS.mutate((w: any) => PM.Layout.addPanel(w, 'perf', dock.id));
      }
      return { layers: PM.proj.layers.length };
    });
    const panel = page.locator('#panel-perf [data-svelte-panel="perf"]');
    await expect(panel).toHaveCount(1);
    await expect(panel.locator('.row.split')).toHaveCount(8);
    const layersRow = panel.locator('.row.split', { hasText: 'Layers' }).locator('.vwrap');
    await expect(layersRow).toHaveText(String(before.layers));

    // A typed edit on the legacy side must flow through the bus → tick bridge.
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const r = PM.Edit.apply([{ type: 'add_layer', layerType: 'solid', name: 'Svelte proof' }], { label: 'e2e' });
      if (!r.ok) throw new Error(r.message);
    });
    await expect(layersRow).toHaveText(String(before.layers + 1));

    // Moving the panel between docks must not remount the component.
    const identity = await page.evaluate(() => {
      const PM = (window as any).PM;
      const el = document.querySelector('[data-svelte-panel="perf"]')!;
      (el as any).__probe = 'same-instance';
      const loc = PM.Layout.findPanel(PM.WS.current, 'perf');
      const target = PM.WS.current.layout.docks.find((d: any) => d.id !== loc.dock.id && d.id !== 'center');
      if (target) PM.WS.mutate((w: any) => PM.Layout.movePanel(w, 'perf', target.id));
      return (document.querySelector('[data-svelte-panel="perf"]') as any)?.__probe ?? null;
    });
    expect(identity).toBe('same-instance');
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
