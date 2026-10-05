import { EventEmitter } from 'node:events';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { IPC, type AgentToolRequestEvent, type CodexRunRequest } from '../../shared/ipc';
import type { AgentTask } from '../../shared/agent-orchestration';

const fake = vi.hoisted(() => ({ native: vi.fn(), compatible: vi.fn(), cancel: vi.fn(async (_id: string) => true), quit: [] as Array<() => void> }));
vi.mock('electron', () => ({ app: { once: (_event: string, callback: () => void) => fake.quit.push(callback) } }));
vi.mock('./runner', async original => ({ ...await original<typeof import('./runner')>(), CodexRunner: class { run = fake.native; cancel = fake.cancel; cancelAll = async () => {}; } }));
vi.mock('../claude', () => ({ ClaudeRunner: class { run = fake.native; cancel = fake.cancel; cancelAll = async () => {}; }, ClaudeAccountClient: class {} }));
vi.mock('./app-server-runner', () => ({ CodexAppServerRunner: class { run = fake.native; cancel = fake.cancel; steer = async () => false; shutdown = async () => {}; } }));
vi.mock('../compatible-provider', () => ({ CompatibleProvider: class {
  status = async () => ({ model: 'local-model', models: ['local-model'], vision: false, hasKey: false, baseUrl: 'http://localhost:11434/v1' });
  run = fake.compatible; cancel = () => {}; cancelAll = () => {};
} }));
import { registerCodexIpc } from './index';

class Main extends EventEmitter {
  readonly handlers = new Map<string, Function>();
  handle(channel: string, handler: Function) { this.handlers.set(channel, handler); }
}
class Owner extends EventEmitter {
  readonly events: any[] = [];
  readonly requests: AgentToolRequestEvent[] = [];
  control?: (args: Record<string, any>) => Promise<unknown>;
  constructor(private readonly main: Main) { super(); }
  isDestroyed = () => false;
  send(channel: string, value: any) {
    if (channel === IPC.codexEvent) { this.events.push(value); return; }
    this.requests.push(value);
    queueMicrotask(async () => this.main.emit(IPC.agentToolResponse, { sender: this }, {
      runId: value.runId, callId: value.callId, ok: true,
      content: [{ type: 'text', text: JSON.stringify(value.tool === '__agent_thread_control' && this.control ? await this.control(value.arguments) : { revision: 2 }) }],
      changed: value.tool === 'apply_commands' || value.tool === '__finish_run', revision: 2,
      ...(value.tool === '__finish_run' ? { historyId: 'one-parent-history' } : {})
    }));
  }
}
function tool(config: { env: Record<string, string> }, name: string, args: Record<string, unknown>): Promise<any> {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host: '127.0.0.1', port: Number(config.env.POWERMOVE_AGENT_TOOL_PORT) });
    let output = '';
    socket.setEncoding('utf8'); socket.once('error', reject);
    socket.once('connect', () => socket.write(JSON.stringify({ token: config.env.POWERMOVE_AGENT_TOOL_TOKEN, runId: config.env.POWERMOVE_AGENT_RUN_ID, id: 1, tool: name, arguments: args, workspace: os.tmpdir() }) + '\n'));
    socket.on('data', chunk => { output += chunk; });
    socket.once('end', () => {
      const response = JSON.parse(output);
      if (!response.ok) reject(new Error(response.error));
      else resolve(JSON.parse(response.content[0].text));
    });
  });
}
const directories: string[] = [];
afterEach(async () => { fake.quit.splice(0).forEach(callback => callback()); await Promise.all(directories.splice(0).map(directory => rm(directory, { recursive: true, force: true }))); vi.clearAllMocks(); });

describe('provider execution with the authenticated orchestration bridge', () => {
  const connected = () => ({ status: async () => ({ state: 'connected' } as any), models: async () => [], onChanged: () => () => {}, connect: async () => ({} as any), disconnect: async () => ({} as any), shutdown: async () => {} });
  const rootRequest = { id: 'root-controls', threadId: 'controls-thread', provider: 'chatgpt', mode: 'autonomous', access: 'project', prompt: 'Control the review', schema: null, model: null, reasoningEffort: null, projectId: 'project', projectName: 'Project', projectJSON: '{"revision":1}', images: [], attachments: [], consentToken: null };

  it('routes task updates over the real authenticated MCP bridge and preserves the parent transaction', async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-task-control-integration-')); directories.push(directory);
    const main = new Main(), owner = new Owner(main), account = connected();
    let stopFirst!: (result: any) => void;
    const first = new Promise(resolve => { stopFirst = resolve; });
    const children: Array<{ request: CodexRunRequest; workspaceId: string }> = [];
    let task!: AgentTask;
    fake.cancel.mockImplementation(async id => { if (id.startsWith('subagent-')) stopFirst({ ok: false, error: 'Interrupted for model change', cancelled: true }); return true; });
    fake.native.mockImplementation(async (request: CodexRunRequest, options: any) => {
      if (request.id.startsWith('subagent-')) {
        children.push({ request, workspaceId: options.workspaceId });
        if (request.provider === 'chatgpt') return first;
        await tool(options.nativeTools, 'apply_commands', { commands: ['{"type":"set_property"}'] });
        return { ok: true, text: 'Claude completed the revised review', access: 'project' };
      }
      if (!request.prompt.includes('SUBAGENT RESULTS')) {
        task = await tool(options.nativeTools, 'delegate_task', { task: 'Review timing' });
        const catalog = await tool(options.nativeTools, 'orchestrator_capabilities', {});
        const model = catalog.providers.find((provider: any) => provider.providerInstanceId === 'claude').models[0];
        const effort = model.options[0]?.options.find((option: any) => option.id === 'medium')?.id;
        const updated = await tool(options.nativeTools, 'task_send', { taskId: task.taskId, message: 'Focus on easing', target: { providerInstanceId: 'claude', model: model.id, ...(effort ? { options: { reasoningEffort: effort } } : {}) }, clientRequestId: 'review-direction' });
        expect(updated).toMatchObject({ taskId: task.taskId, delivery: 'resumed', providerInstanceId: 'claude' });
      }
      return { ok: true, text: 'Reviewed the result', access: 'project' };
    });
    registerCodexIpc(main as any, { userData: directory, getWindow: () => null, codexBinaryPref: () => null, isTrustedSender: () => true, agentToolServerPath: '/fake/agent-tools.mjs', extensionsDir: path.join(directory, 'Mods'), apiPackFiles: async () => [], openExternal: async () => {} }, account, account);
    await expect(main.handlers.get(IPC.codexRun)!({ sender: owner }, rootRequest)).resolves.toMatchObject({ ok: true, liveEditsApplied: true });
    expect(children).toHaveLength(2);
    expect(children[1]!.request).toMatchObject({ id: children[0]!.request.id, threadId: children[0]!.request.threadId, provider: 'claude' });
    expect(children[1]!.request.prompt).toContain('Focus on easing');
    expect(children[1]!.workspaceId).toBe(children[0]!.workspaceId);
    expect(owner.requests.filter(request => request.tool === '__finish_run')).toHaveLength(1);
    expect(owner.requests.find(request => request.tool === 'apply_commands')?.runId).toBe('root-controls');
    fake.cancel.mockImplementation(async () => true);
  });

  it('creates an ordinary thread with its own transaction/workspace, and returns a watched result to the parent', async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-thread-watch-integration-')); directories.push(directory);
    const main = new Main(), owner = new Owner(main), account = connected();
    let launched: Promise<any> | undefined;
    let finishWork!: () => void;
    const work = new Promise<void>(resolve => { finishWork = resolve; });
    const childRequest = { ...rootRequest, id: 'independent-run', threadId: 'agent-thread-independent', prompt: 'Independent work' };
    owner.control = async args => {
      if (args.tool === 'thread_launch') {
        launched = main.handlers.get(IPC.codexRun)!({ sender: owner }, { ...childRequest, provider: args.provider, model: args.model, reasoningEffort: args.reasoningEffort });
        return { threadId: childRequest.threadId, runId: childRequest.id, status: 'running' };
      }
      return { thread: { threadId: childRequest.threadId, runId: childRequest.id, busy: true, provider: 'claude', access: 'project' }, messages: [] };
    };
    fake.native.mockImplementation(async (request: CodexRunRequest, options: any) => {
      if (request.id === childRequest.id) {
        expect(options.workspaceId).toBe(`thread-${childRequest.threadId}`);
        await work;
        await tool(options.nativeTools, 'apply_commands', { commands: ['{"type":"set_property"}'] });
        return { ok: true, text: 'Independent thread result', access: 'project' };
      }
      if (!request.prompt.includes('WATCHED THREAD RESULTS')) {
        const thread = await tool(options.nativeTools, 'thread_launch', { title: 'Independent work', message: 'Work in a separate thread', target: { providerInstanceId: 'claude' }, clientRequestId: 'independent-launch' });
        const watching = await tool(options.nativeTools, 'thread_watch', { threadId: thread.threadId });
        expect(watching).toMatchObject({ runId: childRequest.id, watching: true });
        finishWork();
      } else expect(request.prompt).toContain('Independent thread result');
      return { ok: true, text: 'Watched and reviewed', access: 'project' };
    });
    registerCodexIpc(main as any, { userData: directory, getWindow: () => null, codexBinaryPref: () => null, isTrustedSender: () => true, agentToolServerPath: '/fake/agent-tools.mjs', extensionsDir: path.join(directory, 'Mods'), apiPackFiles: async () => [], openExternal: async () => {} }, account, account);
    await expect(main.handlers.get(IPC.codexRun)!({ sender: owner }, rootRequest)).resolves.toMatchObject({ ok: true, text: 'Watched and reviewed' });
    await launched;
    expect(owner.requests.filter(request => request.tool === '__finish_run').map(request => request.runId).sort()).toEqual(['independent-run', 'root-controls']);
    expect(owner.requests.find(request => request.tool === 'apply_commands')?.runId).toBe('independent-run');
  });

  it.each(['chatgpt', 'claude'] as const)('lets a %s parent run children on another native provider and a compatible provider', async provider => {
    const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-orchestration-integration-')); directories.push(directory);
    const main = new Main(), owner = new Owner(main);
    const account = { status: async () => ({ state: 'connected' } as any), models: async () => [], onChanged: () => () => {}, connect: async () => ({} as any), disconnect: async () => ({} as any), shutdown: async () => {} };
    const childProvider = provider === 'chatgpt' ? 'claude' : 'chatgpt';
    const result = { summary: 'Task complete', commands: [], artifacts: [], extensions: [], notes: [], externalActions: [] };
    const nativeRequests: CodexRunRequest[] = [];
    fake.native.mockImplementation(async (request: CodexRunRequest, options: any) => {
      nativeRequests.push(request);
      expect(options.additionalInstructions).toContain('orchestrator_capabilities');
      if (request.id === 'root-integration' && !request.prompt.includes('SUBAGENT RESULTS')) {
        const catalog = await tool(options.nativeTools, 'orchestrator_capabilities', {});
        expect(catalog.providers.every((item: any) => item.canRunCrossProviderChildTask)).toBe(true);
        await tool(options.nativeTools, 'delegate_task', { task: 'Native review', target: { providerInstanceId: childProvider } });
        await tool(options.nativeTools, 'delegate_task', { task: 'Local review', target: { providerInstanceId: 'compatible', model: 'local-model' } });
      } else if (request.id.startsWith('subagent-')) {
        expect(options.workspaceId).toBe(`subagent-${request.threadId}`);
        await tool(options.nativeTools, 'apply_commands', { commands: ['{"type":"set_property"}'] });
      }
      return { ok: true, text: JSON.stringify(result), access: 'project' };
    });
    fake.compatible.mockImplementation(async (request, _trace, callTool, options) => {
      expect(request.provider).toBe('compatible');
      expect(options.workspaceId).toBe(`subagent-${request.threadId}`);
      const response = await callTool('orchestrator_capabilities', {});
      expect(JSON.parse(response.content[0].text).inheritedProviderInstanceId).toBe('compatible');
      return { ok: true, text: JSON.stringify({ ...result, summary: 'Local review complete' }), access: 'project' };
    });
    registerCodexIpc(main as any, {
      getWindow: () => null, userData: directory, extensionsDir: path.join(directory, 'extensions'), apiPackFiles: async () => [],
      isTrustedSender: () => true, codexBinaryPref: () => null, openExternal: async () => {}, agentToolServerPath: path.join(__dirname, '../agent-tools/mcp-server.mjs')
    }, account, account);
    const response = await main.handlers.get(IPC.codexRun)!({ sender: owner }, {
      id: 'root-integration', threadId: 'integration-thread', provider, mode: 'autonomous', access: 'project', prompt: 'Review with two providers',
      schema: null, model: null, reasoningEffort: null, projectId: 'project', projectName: 'Project', projectJSON: '{"revision":1}', images: [], attachments: [], consentToken: null
    });
    expect(response).toMatchObject({ ok: true, liveEditsApplied: true, liveEditHistoryId: 'one-parent-history' });
    expect(fake.compatible).toHaveBeenCalledTimes(1);
    expect(nativeRequests.filter(request => request.id === 'root-integration')).toHaveLength(2);
    expect(nativeRequests.find(request => request.id.startsWith('subagent-'))?.provider).toBe(childProvider);
    const tasks = owner.events.filter(event => event.step?.kind === 'task').map(event => event.step.task as AgentTask);
    expect(tasks.filter(task => task.status === 'completed')).toHaveLength(2);
    expect(owner.requests.filter(request => request.tool === '__finish_run')).toHaveLength(1);
    expect(owner.requests.filter(request => request.tool === 'apply_commands').every(request => request.runId === 'root-integration')).toBe(true);
  });
});
