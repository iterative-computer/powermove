import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { EventEmitter } from 'node:events';
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

  afterEach(async () => {
    for (const child of children.splice(0)) child.kill();
    for (const bridge of bridges.splice(0)) await bridge.shutdown();
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
      'get_project_state', 'get_panel_layout', 'render_frames', 'apply_commands', 'edit_video', 'rollback_changes'
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
