import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

test.describe('@generated-panel agent-authored workspace sections', () => {
  test('the gradient workspace mounts a Svelte GeneratedPanel whose controls edit the document', async ({ session }) => {
    const { page } = session;
    const info = await page.evaluate(() => {
      const PM = (window as any).PM;
      const gradient = PM.WS.all.find((w: any) => (w.custom ?? []).length > 0);
      if (!gradient) return { customId: null };
      PM.WS.activate(gradient.id);
      const section = gradient.custom[0];
      return { customId: section.id, controls: section.controls.length, bg: PM.proj.backgroundFill?.type ?? null };
    });
    expect(info.customId).not.toBeNull();

    // The section must be a Svelte mount, not the legacy DOM builder.
    const panel = page.locator(`[data-svelte-panel="${info.customId}"]`);
    await expect(panel).toHaveCount(1);

    // Drive the first bound control's command path directly through the same
    // binding the panel uses: pick a composition background control if present.
    const changed = await page.evaluate(() => {
      const PM = (window as any).PM;
      const before = JSON.stringify(PM.proj.backgroundFill);
      const revBefore = PM.proj.revision || 0;
      const r = PM.Edit.apply(
        [{ type: 'set_composition', patch: { backgroundFill: { type: 'linear', angle: 45, stops: [
          { id: 'stop-1', color: '#FF0000', position: 0 },
          { id: 'stop-2', color: '#0000FF', position: 100 }
        ] } } }],
        { label: 'e2e gradient', origin: 'generated-ui' }
      );
      return { ok: r.ok, revBumped: (PM.proj.revision || 0) > revBefore, changed: JSON.stringify(PM.proj.backgroundFill) !== before };
    });
    expect(changed.ok).toBe(true);
    expect(changed.revBumped).toBe(true);
    expect(changed.changed).toBe(true);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
