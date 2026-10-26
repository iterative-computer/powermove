import { expect, test } from './helpers/app';
import { connectMcpHost } from '../src/main/agent-tools/mcp-client.mjs';

test('offers recovery after an editor process crashes and restores its latest saved project', async ({ session }) => {
  await session.openEditor();
  const projectId = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.proj.name = 'Renderer recovery';
    await PM.flushProject();
    return PM.proj.id;
  });
  await session.app.evaluate(({ dialog }) => {
    (globalThis as any).__recoveryDialogs = [];
    dialog.showMessageBox = async (...args: any[]) => {
      (globalThis as any).__recoveryDialogs.push(args.at(-1));
      return { response: 0, checkboxChecked: false };
    };
  });
  const window = await session.app.browserWindow(session.page);
  await window.evaluate(window => window.webContents.forcefullyCrashRenderer());
  await expect.poll(() => session.app.evaluate(() => (globalThis as any).__recoveryDialogs.length)).toBe(1);
  const message = await session.app.evaluate(() => (globalThis as any).__recoveryDialogs[0]);
  expect(message.buttons).toEqual(['Reload', 'Close Window']);
  await expect.poll(() => window.evaluate(async window => {
    if (window.webContents.isCrashed() || window.webContents.isLoading()) return false;
    return window.webContents.executeJavaScript('Boolean(window.PM?.app)').catch(() => false);
  })).toBe(true);
  // Playwright retains its crashed Page state after an Electron-initiated
  // reload. Inspect the recovered document through the surviving native host.
  const recovered = await window.evaluate(window => window.webContents.executeJavaScript(`
    window.PM.projectReady.then(() => ({ id: window.PM.proj.id, name: window.PM.proj.name,
      tabs: window.PM.Tabs.list().length }))
  `));
  expect(recovered).toEqual({ id: projectId, name: 'Renderer recovery', tabs: 1 });
  expect(session.app.windows()).toHaveLength(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a crashed editor can close without waiting for a save from the missing process', async ({ session }) => {
  await session.openEditor();
  await session.page.evaluate(async () => await (window as any).PM.flushProject());
  await session.app.evaluate(({ dialog }) => {
    dialog.showMessageBox = async () => ({ response: 1, checkboxChecked: false });
  });
  const window = await session.app.browserWindow(session.page);
  let closed = false;
  session.page.on('close', () => { closed = true; });
  await window.evaluate(window => window.webContents.forcefullyCrashRenderer());
  await expect.poll(() => closed).toBe(true);
});

test('the external MCP connection survives editor crash and reload without timing out', async ({ session }) => {
  await session.openEditor();
  await session.page.evaluate(async () => await (window as any).PM.flushProject());
  const client = await connectMcpHost(session.userData);
  try {
    await client.callTool('start_session', { prompt: 'Inspect recovery', access: 'editor' });
    // Keep the crashed window open until the test explicitly reloads it.
    await session.app.evaluate(({ dialog }) => {
      dialog.showMessageBox = () => new Promise(resolve => { (globalThis as any).__answerRecovery = resolve; });
    });
    const window = await session.app.browserWindow(session.page);
    await window.evaluate(window => window.webContents.forcefullyCrashRenderer());
    await expect.poll(() => window.evaluate(window => window.webContents.isCrashed())).toBe(true);
    await expect(client.callTool('list_projects', {})).rejects.toThrow('No Powermove editor is available');
    await expect.poll(() => session.app.evaluate(() => typeof (globalThis as any).__answerRecovery)).toBe('function');
    await window.evaluate(window => window.webContents.reload());
    await expect.poll(() => window.evaluate(async window => {
      if (window.webContents.isCrashed() || window.webContents.isLoading()) return false;
      return window.webContents.executeJavaScript('Boolean(window.PM?.app)').catch(() => false);
    })).toBe(true);
    await session.app.evaluate(() => (globalThis as any).__answerRecovery({ response: 0, checkboxChecked: false }));
    await window.evaluate(window => window.webContents.executeJavaScript('window.PM.projectReady'));
    await expect(client.callTool('start_session', { prompt: 'Inspect recovered project', access: 'editor' })).resolves.toMatchObject({ isError: false });
    await expect(client.callTool('get_project_state', {})).resolves.toMatchObject({ isError: false });
    await client.callTool('finish_session', { commit: false });
  } finally { await client.close(); }
});
