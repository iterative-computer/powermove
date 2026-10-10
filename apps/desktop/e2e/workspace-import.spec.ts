import { createServer } from 'node:http';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, test, launchApp } from './helpers/app';
import { WORKSPACE_IMPORT_LABEL } from '../src/shared/creative-workspace';

test('Settings workspace transfer creates, loads and arranges an original extension without editing the project', async ({}, testInfo) => {
  const home = await mkdtemp(path.join(os.tmpdir(), 'powermove-workspace-import-e2e-'));
  const seen: any[] = [];
  let run = 0;
  const server = createServer(async (req, res) => {
    try {
      let input = ''; for await (const chunk of req) input += chunk.toString();
      const body = JSON.parse(input);
      seen.push(body);
      const previous = body.messages.filter((message: any) => message.role === 'tool').length;
      if (!previous) run++;
      const stage = /Extension source belongs only in the isolated staging directory (.+); the live/.exec(body.messages[0].content)?.[1];
      const result = { summary: run === 1 ? 'Created the frame counter.' : 'Saved the imported workspace.', commands: [], artifacts: [], extensions: [], notes: [], externalActions: [] };
      const manifest = { id: 'workspace-proof', name: 'Frame counter', version: '1.0.0', apiVersion: 3, entry: 'index.ts', permissions: [] };
      const sequence: [string, unknown][] = run === 1 ? [
        ['get_panel_layout', {}],
        ['write_file', { path: `${stage}/workspace-proof/manifest.json`, text: JSON.stringify(manifest) }],
        ['write_file', { path: `${stage}/workspace-proof/index.ts`, text: "import Panel from './Panel.svelte'; export default function activate(api) { api.panels.register({id: 'workspace-proof.frame', title: 'Frame Counter', icon: 'clock', size: 160, component: Panel}); }" }],
        ['write_file', { path: `${stage}/workspace-proof/Panel.svelte`, text: '<script lang="ts">let {api}=$props(); const frame=$derived(Math.round(api.project.time() * (api.project.latest()?.fps ?? 30)));</script><div class="row"><span class="label">Current frame</span><output>{frame}</output></div><style>.row{display:flex;align-items:center;min-height:var(--row-h);gap:var(--sp-2);padding:0 var(--sp-3);font-size:var(--fs-md)}.label{flex:1;color:var(--tx-2)}output{flex:1;color:var(--tx)}</style>' }],
        ['compile_extension', { id: 'workspace-proof' }],
        ['complete_task', { ...result, extensions: [{ id: 'workspace-proof', action: 'created', summary: 'Frame counter' }] }]
      ] : [
        ['get_panel_layout', {}],
        ['set_panel_layout', { name: 'After Effects import', sourceApp: 'after-effects', docks: [
          { id: 'left', size: 240, panels: [{ id: 'assets', flex: true }] },
          { id: 'center', panels: [{ id: 'viewer', flex: true }, { id: 'timeline', size: 220 }] },
          { id: 'right', size: 300, panels: [{ id: 'workspace-proof.frame', size: 160 }, { id: 'inspector', size: 200 }] }
        ] }],
        ['get_panel_layout', {}],
        ['complete_task', result]
      ];
      const [name, args] = sequence[previous] ?? ['complete_task', result];
      res.writeHead(200, { 'Content-Type': 'text/event-stream' });
      res.end(`data: ${JSON.stringify({ choices: [{ delta: { tool_calls: [{ index: 0, id: `call-${run}-${previous}`, function: { name, arguments: JSON.stringify(args) } }] }, finish_reason: 'tool_calls' }] })}\n\n`);
    } catch (error) { res.writeHead(500); res.end(String(error)); }
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  let session: Awaited<ReturnType<typeof launchApp>> | undefined;
  try {
    const port = (server.address() as { port: number }).port;
    await writeFile(path.join(home, 'agent-provider.json'), JSON.stringify({ baseUrl: `http://127.0.0.1:${port}/v1`, model: 'workspace-test', vision: false, hasKey: false }));
    session = await launchApp({ userData: home });
    await session.openEditor();
    const before = await session.page.evaluate(() => JSON.stringify((window as any).PM.proj));
    await session.page.evaluate(() => {
      const PM = (window as any).PM;
      PM.AgentUI.setProvider('compatible');
      PM.SettingsUI.open('general');
    });
    const settings = session.page.getByRole('dialog', { name: 'Settings', exact: true });
    const transfer = settings.getByRole('button', { name: 'Bring my workspace', exact: true });
    await expect(transfer).toBeVisible({ timeout: 3000 });
    await transfer.click();
    await expect(settings).toBeHidden();
    await expect.poll(async () => {
      const state = await session!.page.evaluate(() => {
        const {phase, context, provider, composerDraft, threadId, conversation} = (window as any).PM.AgentUI.state;
        return {phase, context, provider, composerDraft, threadId, conversation};
      });
      return state.phase === 'result' ? 'result' : JSON.stringify({state, requests: seen.length, errors: session!.diagnostics.pageErrors});
    }, { timeout: 20000 }).toBe('result');
    const proof = await session.page.evaluate(() => {
      const PM = (window as any).PM;
      return { project: JSON.stringify(PM.proj), workspace: PM.WS.snapshot(), defaultWorkspace: PM.store.get('defaultWorkspace'),
        registered: !!PM.PANELS['workspace-proof.frame'], conversation: JSON.stringify(PM.AgentUI.state.conversation) };
    });
    expect(proof.project).toBe(before);
    expect(proof.registered).toBe(true);
    expect(proof.workspace.name).toBe('After Effects import');
    expect(proof.defaultWorkspace).toBe(proof.workspace.id);
    expect(proof.workspace.layout.docks.find((dock: any) => dock.id === 'right').panels.map((panel: any) => panel.id)).toEqual(['workspace-proof.frame', 'inspector']);
    expect(seen.filter(body => body.messages.some((message: any) => message.role === 'user' && message.content.includes('SANDBOX REPORT'))).map(body => body.messages.filter((message: any) => message.role === 'user'))).toEqual([]);
    expect(run).toBe(2);
    expect(proof.conversation).toContain('Saved the imported workspace.');
    const tools = seen[0].tools.map((tool: any) => tool.function.name);
    expect(tools).toContain('inspect_creative_workspace');
    expect(tools).toContain('inspect_creative_extension');
    expect(tools).toContain('set_panel_layout');
    expect(tools).not.toContain('apply_commands');
    expect(seen.at(-1).messages.some((message: any) => message.role === 'tool' && message.content.includes('workspace-proof.frame'))).toBe(true);
    await session.page.evaluate(() => { const PM = (window as any).PM; PM.newProject(); });
    await session.page.getByRole('button', { name: 'Create', exact: true }).click();
    expect(await session.page.evaluate(() => (window as any).PM.WS.current.name)).toBe('After Effects import');
    // Surface persistence failures here, before the application's quit barrier.
    await session.page.evaluate(async () => { await (window as any).PM.flushProject(); });
    await session.relaunch();
    expect(await session.page.evaluate(() => (window as any).PM.WS.current.name)).toBe('After Effects import');
    await session.page.evaluate(() => {
      const PM = (window as any).PM;
      PM.WS.create({ name: 'Unrelated workspace', sourceApp: null });
      PM.LibraryUI.open('after-effects');
    });
    const library = session.page.getByRole('dialog', { name: 'Panel library', exact: true });
    await expect(library.getByRole('button', { name: 'After Effects imports', exact: true })).toHaveAttribute('aria-current', 'page');
    await expect(library.getByRole('list', { name: 'After Effects extensions', exact: true }).locator('[data-extension-id="workspace-proof"]')).toBeVisible();
    await expect(library.getByRole('button', { name: 'Activate After Effects import', exact: true })).toBeVisible();
    await expect(library.getByRole('button', { name: 'Activate Unrelated workspace', exact: true })).toHaveCount(0);
    const search = library.getByRole('searchbox', { name: 'Search After Effects imports', exact: true });
    await search.fill('Frame counter');
    await expect(library.locator('[data-extension-id="workspace-proof"]')).toBeVisible();
    await expect(library.getByRole('button', { name: 'Activate After Effects import', exact: true })).toHaveCount(0);
    await search.fill('After Effects import');
    await expect(library.locator('[data-extension-id="workspace-proof"]')).toHaveCount(0);
    await expect(library.getByRole('button', { name: 'Activate After Effects import', exact: true })).toBeVisible();
    await search.fill('');
    for (const theme of ['dark', 'light']) {
      await session.page.evaluate(theme => (window as any).PM.theme.apply(theme), theme);
      await session.page.screenshot({ path: testInfo.outputPath(`after-effects-library-${theme}.png`) });
    }
    await library.getByRole('button', { name: 'Activate After Effects import', exact: true }).click();
    expect(await session.page.evaluate(() => (window as any).PM.WS.current.sourceApp)).toBe('after-effects');
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session?.close(); server.closeAllConnections();
    await new Promise<void>(resolve => server.close(() => resolve()));
    await rm(home, { recursive: true, force: true });
  }
});

test('Settings transfer from home keeps the request ready while the agent needs sign-in', async () => {
  const session = await launchApp({ env: { POWERMOVE_FAKE_CHATGPT_STATUS: 'disconnected' } });
  try {
    const { page } = session;
    await page.evaluate(() => (window as any).PM.SettingsUI.open('general'));
    const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
    await settings.getByRole('searchbox', { name: 'Search settings' }).fill('After Effects');
    await settings.getByRole('button', { name: 'Bring my workspace', exact: true }).click();
    await expect(settings).toBeHidden();
    await expect(page.locator('#panel-agent').getByRole('button', { name: 'Connect ChatGPT', exact: true })).toBeVisible();
    await expect.poll(() => page.evaluate(() => {
      const PM = (window as any).PM;
      return { home: PM.ProjectsScreen.isOpen, context: PM.AgentUI.state.context,
        draft: PM.AgentUI.state.composerDraft, messages: PM.AgentUI.state.conversation.length };
    })).toEqual({ home: false, context: 'app', draft: WORKSPACE_IMPORT_LABEL, messages: 0 });
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await session.close(); }
});

test('Settings transfer stays retryable when the handoff is unavailable or fails', async ({ session }) => {
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.AgentUI.importWorkspace = async () => false;
    PM.SettingsUI.open('general');
  });
  const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
  const transfer = settings.getByRole('button', { name: 'Bring my workspace', exact: true });
  for (const throws of [false, true]) {
    if (throws) await page.evaluate(() => {
      (window as any).PM.AgentUI.importWorkspace = async () => { throw new Error('Unavailable'); };
    });
    await transfer.click();
    await expect(settings).toBeVisible();
    await expect(transfer).toBeEnabled();
    await expect(page.getByText('Could not start the workspace transfer. Try again.', { exact: true })).toBeVisible();
    expect(await page.evaluate(() => (window as any).PM.ProjectsScreen.isOpen)).toBe(true);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
