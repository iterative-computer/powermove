import { expect, test } from './helpers/app';

test.describe('@dock-layout Svelte DockLayout behind the shell switch', () => {
  test('keeps panel and viewer hosts alive across layout and workspace moves', async ({ session }) => {
    await session.page.evaluate(async () => {
      (window as any).PM.store.set('shellSvelte', true);
      await (window as any).powermove.store.flush();
    });
    await session.relaunch();

    const { page, diagnostics } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
    await expect(page.locator('#body > .dock .panel[data-panel]')).not.toHaveCount(0);
    await expect(page.locator('#pm-panel-pool')).toHaveCount(1);

    const movedPanel = await page.evaluate(() => {
      const PM = (window as any).PM;
      if (!PM.Layout.findPanel(PM.WS.current, 'perf')) {
        PM.WS.mutate((workspace: any) => PM.Layout.addPanel(workspace, 'perf', 'right'));
      }
      const panel = document.getElementById('panel-perf')!;
      (panel as any).__dockLayoutIdentity = 'perf-live';
      const from = PM.Layout.findPanel(PM.WS.current, 'perf').dock.id;
      const to = from === 'left' ? 'right' : 'left';
      PM.WS.mutate((workspace: any) => PM.Layout.movePanel(workspace, 'perf', to));
      const moved = document.getElementById('panel-perf')!;
      return { from, to, sameNode: moved === panel };
    });
    expect(movedPanel.sameNode).toBe(true);
    await expect(page.locator(`#body #dock-${movedPanel.to} #panel-perf`)).toHaveCount(1);

    const panelState = await page.evaluate(() => {
      const PM = (window as any).PM;
      const moved = document.getElementById('panel-perf')!;
      PM.WS.mutate((workspace: any) => PM.Layout.hidePanel(workspace, 'perf'));
      const hiddenInPool = Boolean(document.querySelector('#pm-panel-pool #panel-perf'));
      PM.WS.mutate((workspace: any) => PM.Layout.restorePanel(workspace, 'perf'));
      return {
        identity: (document.getElementById('panel-perf') as any)?.__dockLayoutIdentity,
        sameNode: document.getElementById('panel-perf') === moved,
        hiddenInPool,
        restored: Boolean(document.querySelector('#body > .dock #panel-perf'))
      };
    });
    expect(panelState).toEqual({ identity: 'perf-live', sameNode: true, hiddenInPool: true, restored: true });

    const viewerState = await page.evaluate(() => {
      const PM = (window as any).PM;
      const originalWorkspace = PM.WS.current.id;
      const alternate = PM.WS.all.find((workspace: any) => workspace.id !== originalWorkspace);
      const canvas = document.getElementById('gl')!;
      (canvas as any).__dockLayoutCanvasIdentity = 'viewer-live';
      if (alternate) {
        PM.WS.activate(alternate.id, true);
        PM.WS.activate(originalWorkspace, true);
      }
      const current = document.getElementById('gl');
      return {
        sameNode: current === canvas,
        identity: (current as any)?.__dockLayoutCanvasIdentity,
        glAlive: Boolean(PM.GL.gl),
        svelteLayout: Boolean(document.getElementById('pm-panel-pool'))
      };
    });
    expect(viewerState).toEqual({ sameNode: true, identity: 'viewer-live', glAlive: true, svelteLayout: true });
    await expect(page.locator('#body .dock #gl')).toHaveCount(1);
    expect(diagnostics.pageErrors).toEqual([]);
  });
});
