import { createServer } from 'node:http';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, test, launchApp } from './helpers/app';

test('onboarding workspace handoff creates, loads and arranges an original extension without editing the project', async () => {
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
        ['set_panel_layout', { name: 'After Effects import', docks: [
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
    await session.page.evaluate(async () => {
      const PM = (window as any).PM;
      PM.AgentUI.openGlobal();
      PM.AgentUI.setProvider('compatible');
      await PM.AgentUI.importWorkspace('after-effects');
    });
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
    expect(tools).toContain('set_panel_layout');
    expect(tools).not.toContain('apply_commands');
    expect(seen.at(-1).messages.some((message: any) => message.role === 'tool' && message.content.includes('workspace-proof.frame'))).toBe(true);
    await session.page.evaluate(() => { const PM = (window as any).PM; PM.newProject(); });
    await session.page.getByRole('button', { name: 'Create', exact: true }).click();
    expect(await session.page.evaluate(() => (window as any).PM.WS.current.name)).toBe('After Effects import');
    await session.relaunch();
    expect(await session.page.evaluate(() => (window as any).PM.WS.current.name)).toBe('After Effects import');
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session?.close(); server.closeAllConnections();
    await new Promise<void>(resolve => server.close(() => resolve()));
    await rm(home, { recursive: true, force: true });
  }
});
