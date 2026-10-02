import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

test('closing an unchanged local project skips the save prompt, while edits still prompt', async ({ session }) => {
  await session.app.evaluate(({ dialog }) => {
    (globalThis as any).__closePrompts = 0;
    dialog.showMessageBox = async () => {
      (globalThis as any).__closePrompts++;
      return { response: 1, checkboxChecked: false };
    };
  });
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', {
      detail: PM.mkProject({ name: 'Untouched close test' }),
    }));
  });
  expect(await session.page.evaluate(() => (window as any).PM.prepareToClose())).toBe(true);
  expect(await session.app.evaluate(() => (globalThis as any).__closePrompts)).toBe(0);

  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', {
      detail: PM.mkProject({ name: 'Edited close test' }),
    }));
    PM.proj.bg = '#ff0000';
    PM.autosave();
  });
  expect(await session.page.evaluate(() => (window as any).PM.prepareToClose())).toBe(false);
  await expect.poll(() => session.app.evaluate(() => (globalThis as any).__closePrompts)).toBe(1);
  expect(await session.page.evaluate(() => (window as any).PM.proj.name)).toBe('Edited close test');
  await session.app.evaluate(({ dialog }) => {
    dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false });
  });
});

test('Don’t Save removes a never-saved project from Recents and recovery across restart', async ({ session }) => {
  await session.app.evaluate(({ dialog }) => {
    dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false });
  });
  const id = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.proj.name = 'Discarded new project';
    PM.autosave();
    await PM.flushProject();
    return PM.proj.id;
  });
  expect(await session.page.evaluate(async (id) => (window as any).PM.Tabs.close(id), id)).toBe(true);
  await expect(session.page.getByText('Discarded new project', { exact: true })).toHaveCount(0);
  expect(await session.page.evaluate((id) => {
    const PM = (window as any).PM;
    return { project: PM.Projects.get(id), state: PM.Projects.getState(id), listed: PM.Projects.list().some((p: any) => p.id === id) };
  }, id)).toEqual({ project: null, state: null, listed: false });
  await session.relaunch();
  await expect(session.page.getByText('Discarded new project', { exact: true })).toHaveCount(0);
  expect(await session.page.evaluate((id) => (window as any).PM.Projects.get(id), id)).toBeNull();
});

test('Don’t Save on window close survives the final recovery flush', async ({ session }) => {
  await session.app.evaluate(({ dialog }) => {
    dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false });
  });
  const id = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.proj.name = 'Discarded window project';
    PM.autosave();
    await PM.flushProject();
    const id = PM.proj.id;
    if (!await PM.prepareToClose()) throw new Error('Close was cancelled');
    await PM.flushProject();
    return id;
  });
  await session.relaunch();
  await expect(session.page.getByText('Discarded window project', { exact: true })).toHaveCount(0);
  expect(await session.page.evaluate((id) => (window as any).PM.Projects.get(id), id)).toBeNull();
});
