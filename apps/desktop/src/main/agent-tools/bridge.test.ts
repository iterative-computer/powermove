import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { EventEmitter } from 'node:events';
import * as fs from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { IPC, type AgentToolRequestEvent } from '../../shared/ipc';
import { PowermoveAgentToolBridge } from './bridge';

class FakeIpcMain extends EventEmitter {}

class FakeWebContents extends EventEmitter {
  destroyed = false;
  readonly requests: AgentToolRequestEvent[] = [];

  constructor(private readonly ipc: FakeIpcMain) { super(); }

  isDestroyed(): boolean { return this.destroyed; }

  send(channel: string, request: AgentToolRequestEvent): void {
    expect(channel).toBe(IPC.agentToolRequest);
    this.requests.push(request);
    const finish = request.tool === '__finish_run';
    queueMicrotask(() => this.ipc.emit(IPC.agentToolResponse, { sender: this }, {
      runId: request.runId,
      callId: request.callId,
      ok: true,
      content: [{ type: 'text', text: JSON.stringify({ tool: request.tool }) }],
      changed: finish ? request.arguments.commit === true : request.tool === 'apply_commands',
      revision: request.tool === 'apply_commands' ? 2 : 1,
      ...(finish && request.arguments.commit === true ? { historyId: 'agent-history-1' } : {})
    }));
  }
}

function rpc(child: ChildProcessWithoutNullStreams, message: Record<string, unknown>): Promise<any> {
  return new Promise((resolve, reject) => {
    let output = '';
    const onData = (chunk: Buffer): void => {
      output += chunk.toString('utf8');
      const newline = output.indexOf('\n');
      if (newline < 0) return;
      child.stdout.off('data', onData);
      try { resolve(JSON.parse(output.slice(0, newline))); }
      catch (error) { reject(error); }
    };
    child.stdout.on('data', onData);
    child.stdin.write(`${JSON.stringify(message)}\n`);
  });
}

describe('native Powermove agent tool bridge', () => {
  const bridges: PowermoveAgentToolBridge[] = [];
  const children: ChildProcessWithoutNullStreams[] = [];
  const temporaryDirectories: string[] = [];

  afterEach(async () => {
    for (const child of children.splice(0)) child.kill();
    for (const bridge of bridges.splice(0)) await bridge.shutdown();
    await Promise.all(temporaryDirectories.splice(0).map((directory) =>
      fs.rm(directory, { recursive: true, force: true })));
  });

  it('serves the shared tools through the standalone MCP shim and finalizes one history entry', async () => {
    const ipc = new FakeIpcMain();
    const owner = new FakeWebContents(ipc);
    const bridge = new PowermoveAgentToolBridge(ipc as never, {
      command: process.execPath,
      mcpServerPath: path.join(__dirname, 'mcp-server.mjs'),
      timeoutMs: 2_000
    });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'native-run-1234', owner: owner as never, baseRevision: 1 });
    const child = spawn(session.mcpConfig.command, session.mcpConfig.args, {
      env: { ...process.env, ...session.mcpConfig.env },
      stdio: ['pipe', 'pipe', 'pipe']
    });
    children.push(child);

    await expect(rpc(child, {
      jsonrpc: '2.0', id: 1, method: 'initialize',
      params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'test', version: '1' } }
    })).resolves.toMatchObject({ id: 1, result: { serverInfo: { name: 'powermove' } } });

    const listed = await rpc(child, { jsonrpc: '2.0', id: 2, method: 'tools/list', params: {} });
    expect(listed.result.tools.map((tool: any) => tool.name)).toEqual([
      'fork_builtin_extension', 'get_project_state', 'get_panel_layout', 'open_panel', 'get_panel_state', 'interact_panel', 'capture_panel', 'computer_use_panel', 'get_workspace_state', 'render_frames', 'apply_commands', 'edit_video', 'rollback_changes'
    ]);

    const state = await rpc(child, {
      jsonrpc: '2.0', id: 25, method: 'tools/call', params: { name: 'get_project_state' }
    });
    expect(state).toMatchObject({ id: 25, result: { isError: false } });

    const applied = await rpc(child, {
      jsonrpc: '2.0', id: 3, method: 'tools/call',
      params: { name: 'apply_commands', arguments: { commands: ['{"type":"set_property"}'] } }
    });
    expect(applied).toMatchObject({ id: 3, result: { isError: false } });
    expect(owner.requests.at(-1)).toMatchObject({
      runId: 'native-run-1234', tool: 'apply_commands', baseRevision: 1
    });

    const finish = await session.finish(true);
    expect(finish).toEqual({ changed: true, revision: 1, historyId: 'agent-history-1' });
    expect(owner.requests.at(-1)?.tool).toBe('__finish_run');
  });

  it('forks a built-in into the current run staging directory without calling the renderer', async () => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-agent-fork-'));
    temporaryDirectories.push(root);
    const workspace = path.join(root, 'workspace');
    const stagingRoot = path.join(workspace, '.powermove', 'extension-runs');
    const resourcesDir = path.join(root, 'builtin-extensions');
    await fs.mkdir(path.join(resourcesDir, 'timeline'), { recursive: true });
    await fs.writeFile(path.join(resourcesDir, 'timeline', 'manifest.json'), JSON.stringify({
      id: 'timeline', name: 'Timeline', version: '1.0.0', apiVersion: 1, author: 'powermove'
    }));
    await fs.writeFile(path.join(resourcesDir, 'timeline', 'index.ts'), 'export default () => undefined');

    const ipc = new FakeIpcMain();
    const owner = new FakeWebContents(ipc);
    const bridge = new PowermoveAgentToolBridge(ipc as never, {
      command: process.execPath,
      mcpServerPath: path.join(__dirname, 'mcp-server.mjs'),
      resourcesDir,
      timeoutMs: 2_000
    });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'native-run-fork', owner: owner as never, baseRevision: 0 });
    const stagingDirectory = path.join(stagingRoot, 'stage-1');
    await fs.mkdir(stagingDirectory, { recursive: true });
    const child = spawn(session.mcpConfig.command, session.mcpConfig.args, {
      cwd: workspace,
      env: { ...process.env, ...session.mcpConfig.env },
      stdio: ['pipe', 'pipe', 'pipe']
    });
    children.push(child);

    const response = await rpc(child, {
      jsonrpc: '2.0', id: 7, method: 'tools/call',
      params: { name: 'fork_builtin_extension', arguments: { id: 'timeline', forkId: 'custom-timeline' } }
    });
    expect(response).toMatchObject({ id: 7, result: { isError: false } });
    const result = JSON.parse(response.result.content[0].text);
    expect(result).toMatchObject({
      dir: path.join(await fs.realpath(stagingDirectory), 'custom-timeline'),
      forkId: 'custom-timeline',
      files: ['index.ts', 'manifest.json'],
      forkedFrom: 'timeline@1.0.0'
    });
    expect(result.reminder).toContain('extensions array with action created');
    expect(owner.requests).toEqual([]);
    await expect(fs.readFile(path.join(result.dir, '.forked-from', 'manifest.json'), 'utf8'))
      .resolves.toContain('"id":"timeline"');
  });

  it('survives a tool client that resets the connection mid-request', async () => {
    const ipc = new FakeIpcMain();
    const owner = new FakeWebContents(ipc);
    owner.send = () => {}; // never answers, so the request is still pending when the client resets
    const bridge = new PowermoveAgentToolBridge(ipc as never, {
      command: process.execPath,
      mcpServerPath: path.join(__dirname, 'mcp-server.mjs'),
      timeoutMs: 500
    });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'native-run-reset', owner: owner as never, baseRevision: 1 });
    const port = Number(session.mcpConfig.env.POWERMOVE_AGENT_TOOL_PORT);

    const uncaught: unknown[] = [];
    const onUncaught = (error: unknown): void => { uncaught.push(error); };
    process.on('uncaughtException', onUncaught);
    try {
      const client = net.createConnection({ host: '127.0.0.1', port });
      await new Promise<void>((resolve) => client.once('connect', () => resolve()));
      client.write(`${JSON.stringify({
        token: session.mcpConfig.env.POWERMOVE_AGENT_TOOL_TOKEN, runId: 'native-run-reset', tool: 'get_project_state', arguments: {}
      })}\n`);
      // Reset (RST) instead of a clean FIN so the server-side socket errors.
      client.resetAndDestroy();
      await new Promise((resolve) => setTimeout(resolve, 700));
    } finally {
      process.off('uncaughtException', onUncaught);
    }
    expect(uncaught).toEqual([]);

    // The bridge is still healthy for subsequent clients.
    const child = spawn(session.mcpConfig.command, session.mcpConfig.args, {
      env: { ...process.env, ...session.mcpConfig.env },
      stdio: ['pipe', 'pipe', 'pipe']
    });
    children.push(child);
    await expect(rpc(child, {
      jsonrpc: '2.0', id: 1, method: 'initialize',
      params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'test', version: '1' } }
    })).resolves.toMatchObject({ id: 1, result: { serverInfo: { name: 'powermove' } } });
  });

  it('deduplicates concurrent finish requests from runner cleanup and window cleanup', async () => {
    const ipc = new FakeIpcMain();
    const owner = new FakeWebContents(ipc);
    const bridge = new PowermoveAgentToolBridge(ipc as never, {
      command: process.execPath,
      mcpServerPath: path.join(__dirname, 'mcp-server.mjs'),
      timeoutMs: 2_000
    });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'native-run-concurrent', owner: owner as never, baseRevision: 0 });

    const [first, second] = await Promise.all([session.finish(false), session.finish(false)]);

    expect(first).toEqual(second);
    expect(owner.requests.filter((request) => request.tool === '__finish_run')).toHaveLength(1);
  });

  it('rejects oversized owning-renderer responses immediately instead of timing out', async () => {
    const ipc = new FakeIpcMain();
    const owner = new FakeWebContents(ipc);
    owner.send = (_channel, request) => {
      queueMicrotask(() => ipc.emit(IPC.agentToolResponse, { sender: owner }, {
        runId: request.runId, callId: request.callId, ok: true,
        content: [{ type: 'text', text: 'x'.repeat(2_000_001) }]
      }));
    };
    const bridge = new PowermoveAgentToolBridge(ipc as never, { mcpServerPath: '', timeoutMs: 60_000 });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'native-run-large', owner: owner as never, baseRevision: 0 });
    await expect(bridge.callRenderer(session, 'get_project_state', {})).rejects.toThrow('oversized response');
    owner.destroyed = true;
  });

  it('ends an in-flight request immediately if the renderer crashes', async () => {
    const ipc = new FakeIpcMain(), owner = new FakeWebContents(ipc);
    owner.send = () => { queueMicrotask(() => owner.emit('render-process-gone')); };
    const bridge = new PowermoveAgentToolBridge(ipc as never, { mcpServerPath: '', timeoutMs: 60_000 });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'native-run-crash', owner: owner as never, baseRevision: 0 });
    await expect(bridge.callRenderer(session, 'interact_panel', {})).rejects.toThrow('outcome is unknown');
    expect(owner.listenerCount('render-process-gone')).toBe(0);
    owner.destroyed = true;
  });

  it('rejects unknown tools before they reach the renderer', async () => {
    const ipc = new FakeIpcMain();
    const owner = new FakeWebContents(ipc);
    const bridge = new PowermoveAgentToolBridge(ipc as never, {
      command: process.execPath,
      mcpServerPath: path.join(__dirname, 'mcp-server.mjs'),
      timeoutMs: 2_000
    });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'native-run-5678', owner: owner as never, baseRevision: 0 });
    const child = spawn(session.mcpConfig.command, session.mcpConfig.args, {
      env: { ...process.env, ...session.mcpConfig.env },
      stdio: ['pipe', 'pipe', 'pipe']
    });
    children.push(child);
    const response = await rpc(child, {
      jsonrpc: '2.0', id: 1, method: 'tools/call',
      params: { name: 'delete_everything', arguments: {} }
    });
    expect(response.result).toMatchObject({ isError: true });
    expect(response.result.content[0].text).toContain('Unknown Powermove tool');
    expect(owner.requests).toHaveLength(0);
    await session.finish(false);
  });
});
