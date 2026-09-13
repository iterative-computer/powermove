import path from 'node:path';
import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

// Normalization omits false collapsed flags; both representations are expanded.
const canonical = (value: unknown) => JSON.parse(JSON.stringify(value,
  (key, value) => key === 'collapsed' && value === false ? undefined : value));

test('restores each project layout after quit and when reopening its saved file', async ({ session }) => {
  const filePath = path.join(session.userData, 'Panel layout.pmv');
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
  }, filePath);
  expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  await session.page.evaluate(() => (window as any).PM.openProject());
  const first = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.WS.mutate((ws: any) => {
      ws.layout.docks.find((dock: any) => dock.id === 'left').size = 411;
      PM.Layout.movePanel(ws, 'agent', 'right');
      PM.Layout.hidePanel(ws, 'assets');
      ws.layout.docks.find((dock: any) => dock.id === 'right').panels.find((panel: any) => panel.id === 'agent').size = 290;
      ws.layout.docks.find((dock: any) => dock.id === 'center').panels.find((panel: any) => panel.id === 'timeline').collapsed = true;
    });
    return { id: PM.proj.id, workspace: PM.WS.snapshot() };
  });
  const second = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Other layout', w: 640, h: 360, fps: 30, dur: 4 });
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
    PM.WS.activate('design', true);
    PM.WS.mutate((ws: any) => {
      ws.layout.docks.find((dock: any) => dock.id === 'left').size = 277;
      PM.Layout.movePanel(ws, 'agent', 'left');
      PM.Layout.restorePanel(ws, 'assets');
    });
    return { id: PM.proj.id, workspace: PM.WS.snapshot() };
  });
  await session.relaunch();
  await session.openEditor();
  expect(canonical(await session.page.evaluate(() => ({
    id: (window as any).PM.proj.id, workspace: (window as any).PM.WS.snapshot(),
  })))).toEqual(canonical(second));
  // Restoring an existing tab uses its own session, even with another layout active.
  await session.page.evaluate(id => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.Projects.get(id) }));
  }, first.id);
  expect(canonical(await session.page.evaluate(() => ({
    id: (window as any).PM.proj.id, workspace: (window as any).PM.WS.snapshot(),
  })))).toEqual(canonical(first));
  await session.page.evaluate(id => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.Projects.get(id) }));
  }, second.id);
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
  }, filePath);
  await session.page.evaluate(() => (window as any).PM.openProject());
  expect(canonical(await session.page.evaluate(() => ({
    workspace: (window as any).PM.WS.snapshot(),
  })))).toEqual(canonical({ workspace: first.workspace }));
  // Reopening the already-active file must also retain its current arrangement.
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.WS.mutate((ws: any) => { ws.layout.docks.find((dock: any) => dock.id === 'right').size = 433; });
  });
  await session.page.evaluate(() => (window as any).PM.openProject());
  expect(await session.page.evaluate(() => (window as any).PM.WS.current.layout.docks.find((dock: any) => dock.id === 'right').size)).toBe(433);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
