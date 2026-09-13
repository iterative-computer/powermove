import path from 'node:path';
import { expect, test } from './helpers/app';

test('agent history follows a saved project across file opens, project switches, and relaunch', async ({ session }) => {
  await session.openEditor();
  const filePath = path.join(session.userData, 'Agent history.pmv');
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
  }, filePath);
  const original = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.open();
    PM.AgentUI.setDraft('First project draft');
    PM.AgentUI.newThread();
    PM.AgentUI.setDraft('Active project draft');
    await PM.saveProject();
    return PM.AgentUI.state.threads.map((thread: any) => thread.id);
  });
  await session.page.evaluate(() => (window as any).PM.openProject());
  const snapshot = () => session.page.evaluate(() => {
    const PM = (window as any).PM;
    return { ids: PM.AgentUI.state.threads.map((thread: any) => thread.id), draft: PM.AgentUI.state.composerDraft };
  });
  await expect.poll(snapshot).toEqual({ ids: original, draft: 'Active project draft' });
  const reopenedId = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const id = PM.proj.id;
    PM.AgentUI.setDraft('Updated after reopening');
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Independent project' }) }));
    return id;
  });
  expect((await snapshot()).ids).not.toEqual(original);
  expect((await snapshot()).draft).toBe('');
  await session.page.evaluate(id => {
    const PM = (window as any).PM;
    PM.AgentUI.setDraft('Other project draft');
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.Projects.get(id) }));
  }, reopenedId);
  await expect.poll(snapshot).toEqual({ ids: original, draft: 'Updated after reopening' });
  await session.relaunch();
  await session.openEditor();
  await expect.poll(snapshot).toEqual({ ids: original, draft: 'Updated after reopening' });
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
  }, filePath);
  await session.page.evaluate(() => (window as any).PM.openProject());
  await expect.poll(snapshot).toEqual({ ids: original, draft: 'Updated after reopening' });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
