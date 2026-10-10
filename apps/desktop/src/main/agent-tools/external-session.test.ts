import { EventEmitter } from 'node:events';
import { mkdtemp, rm, stat, writeFile, mkdir } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { IPC, type AgentToolRequestEvent } from '../../shared/ipc';
import { PowermoveAgentToolBridge } from './bridge';
import { ExternalAgentSession, mcpToolResult } from './external-session';
import { startExternalMcp } from './external-server';
import { connectMcpHost } from './mcp-client.mjs';
import { POWERMOVE_AGENT_TOOLS } from './spec';
import { AgentOrchestrator } from '../agent-orchestrator';

describe('external Powermove MCP sessions', () => {
  const cleanup: Array<() => Promise<unknown>> = [];
  afterEach(async () => { for (const close of cleanup.splice(0).reverse()) await close(); });
  async function setup() {
    const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-mcp-'));
    cleanup.push(() => rm(root, { recursive: true, force: true }));
    const ipc = new EventEmitter();
    const owner = Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false,
      send: (_channel: string, request: AgentToolRequestEvent) => queueMicrotask(() => ipc.emit(IPC.agentToolResponse, { sender: owner }, {
        runId: request.runId, callId: request.callId, ok: true,
        content: [{ type: 'text', text: JSON.stringify(request.tool === '__mcp_context'
          ? { projectId: 'project-one', projectName: 'One', projectJSON: '{"id":"project-one","revision":2,"layers":[]}', home: false }
          : { tool: request.tool, arguments: request.arguments }) }], changed: false, revision: 2
      })) });
    const bridge = new PowermoveAgentToolBridge(ipc as any, { mcpServerPath: path.join(__dirname, 'mcp-server.mjs'), timeoutMs: 1000 });
    cleanup.push(() => bridge.shutdown());
    const detach = vi.fn(async () => {}), attach = vi.fn(async (_request: unknown, _session: unknown) => detach);
    const session = new ExternalAgentSession({ bridge, getOwner: () => owner as any, userData: root,
      extensionsDir: path.join(root, 'extensions'), apiPackFiles: async () => [{ name: 'EXTENSIONS.md', text: 'API documentation' }], attach });
    cleanup.push(() => session.close());
    return { root, owner, bridge, session, attach, detach };
  }

  it('exposes every shared harness tool and retains staged validation and commit lifecycle', async () => {
    const { root, session, attach, detach } = await setup();
    expect(session.tools()).toEqual(expect.arrayContaining([...POWERMOVE_AGENT_TOOLS]));
    await expect(session.call('apply_commands', {})).rejects.toThrow('start_session');
    const opened: any = await session.call('start_session', { prompt: 'Create a video' });
    expect(opened).toMatchObject({ projectId: 'project-one', access: 'project' });
    expect(attach.mock.calls[0]?.[0]).toMatchObject({ access: 'project', mode: 'autonomous' });
    expect((await stat(opened.stagingDirectory)).isDirectory()).toBe(true);
    await mkdir(path.join(opened.stagingDirectory, 'bad-effect'));
    await writeFile(path.join(opened.stagingDirectory, 'bad-effect/manifest.json'), '{}');
    await expect(session.call('publish_extensions', { extensions: [{ id: 'bad-effect', action: 'created', summary: 'Broken' }] })).rejects.toThrow();
    expect((await stat(opened.stagingDirectory)).isDirectory()).toBe(true);
    await writeFile(path.join(opened.artifactsDirectory, 'notes.txt'), 'deliverable');
    await session.call('finish_session', { commit: true });
    expect(detach).toHaveBeenCalledOnce();
    expect((await stat(path.join(opened.artifactsDirectory, 'notes.txt'))).isFile()).toBe(true);
    await expect(stat(opened.stagingDirectory)).rejects.toMatchObject({ code: 'ENOENT' });
    expect(root).toContain('powermove-mcp');
  });

  it('enforces planning access and blocks project switching during an active session', async () => {
    const { session } = await setup();
    await session.call('start_session', { prompt: 'Inspect', access: 'editor' });
    await expect(session.call('create_project', { name: 'Other' })).rejects.toThrow('Finish');
    await expect(session.call('apply_commands', { commands: [] })).rejects.toThrow('only inspect');
    await expect(session.call('export_video', {})).rejects.toThrow('project access');
    await expect(session.call('publish_extensions', { extensions: [] })).rejects.toThrow('Planning');
    expect(mcpToolResult(await session.call('get_project_state', {}))).toMatchObject({ isError: false });
  });

  it('authenticates local discovery and closes the owning session on disconnect', async () => {
    const { root, session, detach } = await setup();
    const host = await startExternalMcp({ userData: root, createSession: () => session });
    cleanup.push(() => host.close());
    if (process.platform !== 'win32') expect((await stat(host.file)).mode & 0o777).toBe(0o600);
    const client = await connectMcpHost(root);
    expect(await client.listTools()).toEqual(session.tools());
    await client.callTool('start_session', { prompt: 'Build' });
    const response: any = await client.callTool('get_project_state', {});
    expect(response).toMatchObject({ isError: false });
    await client.close(); await host.close();
    expect(detach).toHaveBeenCalledOnce();
    await expect(stat(host.file)).rejects.toMatchObject({ code: 'ENOENT' });
  });

  it('attaches an external root without running a provider and delegates through the existing orchestrator', async () => {
    const { root } = await setup();
    const execute = vi.fn(async () => ({ ok: true as const, text: 'Child finished', access: 'project' as const }));
    const orchestrator = new AgentOrchestrator({ directory: path.join(root, 'tasks'),
      providers: async () => [{ providerInstanceId: 'chatgpt', displayName: 'ChatGPT', driverKind: 'codex', connected: true, models: [{ id: 'test-model', label: 'Test', options: [] }], canRunChildTask: true, canRunCrossProviderChildTask: true, constraints: [] }] as any,
      execute, cancel: async () => {} });
    const request: any = { id: 'external-root', projectId: 'project-one', threadId: 'external-thread', provider: 'chatgpt', model: null, access: 'project', mode: 'autonomous', prompt: 'Build', projectJSON: '{}', projectName: 'One', images: [], attachments: [], schema: null, consentToken: null, reasoningEffort: null };
    const detach = await orchestrator.attachExternal(request);
    expect(execute).not.toHaveBeenCalled();
    expect(await orchestrator.call(request.id, 'orchestrator_capabilities', {})).toMatchObject({ features: { automaticResultDelivery: false } });
    const task: any = await orchestrator.call(request.id, 'delegate_task', { task: 'Review', mode: 'wait', target: { model: 'test-model' } });
    expect(task).toMatchObject({ status: 'completed', summary: 'Child finished' });
    expect(execute).toHaveBeenCalledOnce();
    await detach();
    await expect(orchestrator.call(request.id, 'task_status', { taskId: task.taskId })).rejects.toThrow('no longer active');
  });
});
