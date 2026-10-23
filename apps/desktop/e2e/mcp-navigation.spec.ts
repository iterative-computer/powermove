import { expect, test } from './helpers/app';
import { connectMcpHost } from '../src/main/agent-tools/mcp-client.mjs';

test('the same MCP connection starts a fresh session after a normal editor reload', async ({ session }) => {
  await session.openEditor();
  const projectId = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.flushProject();
    return PM.proj.id;
  });
  const client = await connectMcpHost(session.userData);
  try {
    await expect(client.callTool('start_session', { prompt: 'Inspect before reload', access: 'editor' })).resolves.toMatchObject({ isError: false });
    const window = await session.app.browserWindow(session.page);
    const contentsId = await window.evaluate(window => window.webContents.id);
    await window.evaluate(window => new Promise<void>(resolve => {
      window.webContents.once('did-finish-load', () => resolve());
      window.webContents.reload();
    }));
    await expect.poll(() => window.evaluate(async window => {
      if (window.webContents.isLoading()) return false;
      return window.webContents.executeJavaScript('Boolean(window.PM?.app)').catch(() => false);
    })).toBe(true);
    const recoveredId = await window.evaluate(window => window.webContents.executeJavaScript('window.PM.projectReady.then(() => window.PM.proj.id)'));
    expect(recoveredId).toBe(projectId);
    expect(await window.evaluate(window => window.webContents.id)).toBe(contentsId);

    await expect(client.callTool('get_project_state', {})).rejects.toThrow('start_session');
    await expect(client.callTool('start_session', { prompt: 'Inspect after reload', access: 'editor' })).resolves.toMatchObject({ isError: false });
    await expect(client.callTool('get_project_state', {})).resolves.toMatchObject({ isError: false });
    await expect(client.callTool('finish_session', { commit: false })).resolves.toMatchObject({ isError: false });
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await client.close(); }
});
