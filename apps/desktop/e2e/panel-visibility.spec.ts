import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

test('keeps a closed panel hidden across relaunch and workspace changes', async ({ session }) => {
  const before = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.registerPanel('api-browser', { title: 'API Browser', build(body: HTMLElement) { body.textContent = 'API'; } });
    PM.WS.mutate((workspace: any) => PM.Layout.addPanel(workspace, 'api-browser', 'right'));
    PM.WS.mutate((workspace: any) => PM.Layout.closePanel(workspace, 'api-browser'));
    return {
      visible: PM.Layout.hasPanel(PM.WS.current, 'api-browser'),
      preference: PM.store.get('panelVisibility', {})['api-browser']
    };
  });
  expect(before).toEqual({ visible: false, preference: false });

  await session.relaunch();
  await session.openEditor();
  const after = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.registerPanel('api-browser', { title: 'API Browser', build(body: HTMLElement) { body.textContent = 'API'; } });
    const workspace = PM.WS.snapshot();
    workspace.id = 'other-workspace';
    workspace.hiddenPanels = (workspace.hiddenPanels ?? []).filter((panel: any) => panel.id !== 'api-browser');
    for (const dock of workspace.layout.docks) dock.panels = dock.panels.filter((panel: any) => panel.id !== 'api-browser');
    workspace.layout.docks.find((dock: any) => dock.id === 'right').panels.push({ id: 'api-browser', size: 275 });

    PM.WS.restoreSnapshot(workspace);
    const hidden = PM.WS.current.hiddenPanels.find((panel: any) => panel.id === 'api-browser');
    return {
      visible: PM.Layout.hasPanel(PM.WS.current, 'api-browser'),
      hiddenSize: hidden?.spec?.size,
      preference: PM.store.get('panelVisibility', {})['api-browser']
    };
  });

  expect(after).toEqual({ visible: false, hiddenSize: 275, preference: false });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
