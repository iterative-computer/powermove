import { expect, test } from './helpers/app';

test('Quit exits the process after saving the editing session', async ({ session }) => {
  await session.openEditor();
  const process = session.app.process();
  const closed = session.app.waitForEvent('close');
  await session.app.evaluate(({ app }) => { setTimeout(() => app.quit(), 0); });
  await closed;
  expect(process.exitCode).toBe(0);
});

test('Closing the last window exits the process', async ({ session }) => {
  const process = session.app.process();
  const closed = session.app.waitForEvent('close', { timeout: 10_000 });
  await session.app.evaluate(({ BrowserWindow }) => { BrowserWindow.getAllWindows()[0]!.close(); });
  await closed;
  expect(process.exitCode).toBe(0);
});

test('Quit exits after discarding unsaved edits', async ({ session }) => {
  await session.openEditor();
  await session.page.evaluate(() => { const PM = (window as any).PM; PM.proj.name = 'Quit with edits'; PM.autosave(); });
  const process = session.app.process();
  const closed = session.app.waitForEvent('close');
  await session.app.evaluate(({ app }) => { setTimeout(() => app.quit(), 0); });
  await closed;
  expect(process.exitCode).toBe(0);
});

test('Closing one of two windows keeps the other window open', async ({ session }) => {
  const secondWindow = session.app.waitForEvent('window');
  await session.app.evaluate(({ Menu }) => {
    const item = Menu.getApplicationMenu()!.getMenuItemById('newWindow')!;
    item.click(item, undefined, {} as any);
  });
  const page = await secondWindow;
  await page.waitForFunction(() => Boolean((window as any).PM?.app));
  await session.app.evaluate(({ BrowserWindow }) => { BrowserWindow.getAllWindows()[0]!.close(); });
  await expect.poll(() => session.app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().length)).toBe(1);
  expect(session.app.process().exitCode).toBeNull();
  expect(await session.app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.isDestroyed())).toBe(false);
});

test('Repeated Quit can be cancelled while preparation waits for imports', async ({ session }) => {
  await session.openEditor();
  const process = session.app.process();
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.app.importQueue = new Promise<void>(resolve => { PM.releaseCloseTestImport = resolve; });
  });
  for (let i = 0; i < 2; i++) {
    await session.app.evaluate(({ app }) => { app.quit(); app.quit(); });
    await session.page.getByRole('button', { name: 'Cancel Close', exact: true }).click();
    await expect.poll(() => session.page.evaluate(() => (window as any).PM.commitClosePreparation())).toBe(false);
    expect(process.exitCode).toBeNull();
  }
  await session.page.evaluate(() => (window as any).PM.releaseCloseTestImport());
  const closed = session.app.waitForEvent('close');
  await session.app.evaluate(({ app }) => { setTimeout(() => app.quit(), 0); });
  await closed;
  expect(process.exitCode).toBe(0);
});

test('A failed recovery flush keeps edits open and allows Quit to be retried', async ({ session }) => {
  await session.openEditor();
  const process = session.app.process();
  const id = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.proj.name = 'Retain these edits'; PM.autosave();
    PM.originalCloseTestFlush = PM.store.flush;
    PM.store.flush = async () => { throw new Error('Synthetic recovery failure'); };
    return PM.proj.id;
  });
  await session.app.evaluate(({ app }) => { setTimeout(() => app.quit(), 0); });
  await expect(session.page.getByText('Could not finish closing. Your projects are still open.', { exact: true })).toBeVisible();
  expect(process.exitCode).toBeNull();
  expect(await session.page.evaluate(id => {
    const PM = (window as any).PM;
    return PM.proj.id === id && PM.proj.name === 'Retain these edits' && !!PM.Projects.get(id);
  }, id)).toBe(true);
  await session.page.evaluate(() => { const PM = (window as any).PM; PM.store.flush = PM.originalCloseTestFlush; });
  const closed = session.app.waitForEvent('close');
  await session.app.evaluate(({ app }) => { setTimeout(() => app.quit(), 0); });
  await closed;
  expect(process.exitCode).toBe(0);
});

test('Cancelling a later window preserves an earlier approved unsaved project', async ({ session }) => {
  await session.openEditor();
  const first = session.page;
  const firstId = await first.evaluate(() => { const PM = (window as any).PM; PM.proj.name = 'First edited project'; PM.autosave(); return PM.proj.id; });
  const next = session.app.waitForEvent('window');
  await session.app.evaluate(({ Menu }) => {
    const item = Menu.getApplicationMenu()!.getMenuItemById('newWindow')!;
    item.click(item, undefined, {} as any);
  });
  const second = await next;
  await second.waitForFunction(() => Boolean((window as any).PM?.app));
  await second.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.projectReady;
    await PM.Kernel.loader.builtinsReady;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Second project' }) }));
    PM.proj.name = 'Second edited project'; PM.autosave();
  });
  await session.app.evaluate(({ dialog }) => {
    let calls = 0;
    (globalThis as any).closeTestDecisions = 0;
    dialog.showMessageBox = async () => {
      (globalThis as any).closeTestDecisions++;
      return { response: calls++ ? 1 : 2, checkboxChecked: false };
    };
  });
  await session.app.evaluate(({ app }) => { setTimeout(() => app.quit(), 0); });
  await expect.poll(() => session.app.evaluate(() => (globalThis as any).closeTestDecisions)).toBe(2);
  await expect.poll(() => first.evaluate(() => (window as any).PM.commitClosePreparation())).toBe(false);
  expect(await first.evaluate(id => { const PM = (window as any).PM; return PM.proj.id === id && !!PM.Projects.get(id); }, firstId)).toBe(true);
  expect(await session.app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().length)).toBe(2);
  expect(session.app.process().exitCode).toBeNull();
});
