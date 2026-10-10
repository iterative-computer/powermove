import { ORCHESTRATION_TOOL_NAMES } from '../../shared/agent-orchestration';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { EventEmitter } from 'node:events';
import * as fs from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';

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

const initializedClients = new WeakSet<ChildProcessWithoutNullStreams>();
async function rpc(child: ChildProcessWithoutNullStreams, message: Record<string, unknown>): Promise<any> {
  if (message.method !== 'initialize' && !initializedClients.has(child)) await rpc(child, { jsonrpc: '2.0', id: 'fixture-initialize', method: 'initialize', params: { protocolVersion: '2025-06-18' } });
  if (message.method === 'initialize') initializedClients.add(child);
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

  it('launches the fixed app entrypoint without requiring Electron Node mode', async () => {
    const ipc = new FakeIpcMain();
    const bridge = new PowermoveAgentToolBridge(ipc as any, {
      mcpServerPath: '/resources/agent-tools/mcp-server.mjs',
      command: '/Applications/Powermove.app/Contents/MacOS/Powermove',
      commandArgs: ['--powermove-agent-tools']
    });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'packaged-tools', owner: new FakeWebContents(ipc) as any, baseRevision: 0 });
    expect(session.mcpConfig.args).toEqual(['--powermove-agent-tools']);
    expect(session.mcpConfig.env).not.toHaveProperty('ELECTRON_RUN_AS_NODE');
    expect(session.mcpConfig.env.POWERMOVE_AGENT_RUN_ID).toBe('packaged-tools');
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
      ...ORCHESTRATION_TOOL_NAMES,
      'get_3d_scene', 'edit_3d', 'fork_builtin_extension', 'get_project_state', 'list_media', 'replace_media', 'import_media', 'manage_media', 'select_layers', 'inspect_creative_workspace', 'set_panel_layout', 'get_panel_layout', 'inspect_creative_extension', 'open_panel', 'get_panel_state', 'interact_panel', 'capture_panel', 'computer_use_panel', 'get_workspace_state', 'render_frames', 'apply_commands', 'edit_video', 'rollback_changes', 'validate_effect', 'stage_fork_rebase',
      'probe_media', 'sample_media_frames', 'media_contact_sheet', 'media_waveform', 'transcribe_media', 'check_project',
      'store_search', 'store_extension', 'store_source', 'store_library', 'store_install', 'store_update', 'store_uninstall', 'store_publish_prepare', 'store_publish',
      'generate_captions', 'export_captions'
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

  it('rejects forged composition calls in an app-only extension run', async () => {
    const ipc = new FakeIpcMain();
    const owner = new FakeWebContents(ipc);
    const bridge = new PowermoveAgentToolBridge(ipc as never, {
      command: process.execPath,
      mcpServerPath: path.join(__dirname, 'mcp-server.mjs'),
      timeoutMs: 2_000
    });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'app-extension-run', owner: owner as never, baseRevision: 0, context: 'app' });
    const child = spawn(session.mcpConfig.command, session.mcpConfig.args, {
      env: { ...process.env, ...session.mcpConfig.env },
      stdio: ['pipe', 'pipe', 'pipe']
    });
    children.push(child);
    await rpc(child, { jsonrpc: '2.0', id: 1, method: 'initialize',
      params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'test', version: '1' } } });
    const listed = await rpc(child, { jsonrpc: '2.0', id: 2, method: 'tools/list', params: {} });
    expect(listed.result.tools.map((tool: any) => tool.name)).toEqual([
      ...ORCHESTRATION_TOOL_NAMES,
      'fork_builtin_extension', 'inspect_creative_workspace', 'set_panel_layout', 'get_panel_layout', 'inspect_creative_extension', 'validate_effect', 'stage_fork_rebase',
      'store_search', 'store_extension', 'store_source', 'store_library', 'store_install', 'store_update', 'store_uninstall', 'store_publish_prepare', 'store_publish'
    ]);
    for (const name of ['get_project_state', 'apply_commands']) {
      const denied = await rpc(child, { jsonrpc: '2.0', id: name, method: 'tools/call',
        params: { name, arguments: name === 'apply_commands' ? { commands: ['{}'] } : {} } });
      expect(denied.result.isError).toBe(true);
      await expect(bridge.callTool(session, name, {})).rejects.toThrow('no project attached');
    }
    expect(owner.requests).toEqual([]);
    const allowed = await bridge.callTool(session, 'validate_effect', { definition: {} });
    expect(allowed.ok).toBe(true);
    expect(owner.requests.map(item => item.tool)).toEqual(['validate_effect']);
  });

  it('runs media tools in main on the file the renderer resolves', async () => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-agent-media-'));
    temporaryDirectories.push(root);
    const tone = path.join(root, 'tone.wav');
    const ffmpegPath = path.resolve(__dirname, '../../../node_modules/ffmpeg-static/ffmpeg');
    await new Promise<void>((resolve, reject) => spawn(ffmpegPath, ['-hide_banner', '-loglevel', 'error', '-y', '-f', 'lavfi', '-i', 'sine=f=220:d=1', tone])
      .once('close', (code) => code === 0 ? resolve() : reject(new Error(`ffmpeg ${code}`))));
    const ipc = new FakeIpcMain();
    const owner = new FakeWebContents(ipc);
    owner.send = (channel: string, request: AgentToolRequestEvent) => {
      owner.requests.push(request);
      queueMicrotask(() => ipc.emit(IPC.agentToolResponse, { sender: owner }, {
        runId: request.runId, callId: request.callId, ok: true,
        content: [{ type: 'text', text: JSON.stringify({ path: tone, origin: 'source', asset: { id: 'a1', name: 'tone.wav', kind: 'audio', duration: 1, hasAudio: true, proxy: false } }) }]
      }));
    };
    const bridge = new PowermoveAgentToolBridge(ipc as never, { mcpServerPath: '/resources/agent-tools/mcp-server.mjs', ffmpegPath });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'media-run-1', owner: owner as never, baseRevision: 0 });

    const response = await bridge.callTool(session, 'probe_media', { assetId: 'a1' });

    expect(owner.requests.map((request) => [request.tool, request.arguments])).toEqual([['__media_source', { assetId: 'a1' }]]);
    expect(response.ok).toBe(true);
    const result = JSON.parse((response.content[0] as { text: string }).text);
    expect(result).toMatchObject({ asset: { id: 'a1', name: 'tone.wav' }, format: 'wav', audio: [{ codec: 'pcm_s16le', channels: 1 }] });

    const appSession = await bridge.openSession({ runId: 'media-run-app', owner: owner as never, baseRevision: 0, context: 'app' });
    await expect(bridge.callTool(appSession, 'transcribe_media', { assetId: 'a1' })).rejects.toThrow('no project attached');
  });

  it('reads composition info for a composition contact sheet without a project-state read', async () => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-agent-media-'));
    temporaryDirectories.push(root);
    const still = path.join(root, 'frame.png');
    const ffmpegPath = path.resolve(__dirname, '../../../node_modules/ffmpeg-static/ffmpeg');
    await new Promise<void>((resolve, reject) => spawn(ffmpegPath, ['-hide_banner', '-loglevel', 'error', '-y', '-f', 'lavfi', '-i', 'color=c=blue:s=320x180', '-frames:v', '1', still])
      .once('close', (code) => code === 0 ? resolve() : reject(new Error(`ffmpeg ${code}`))));
    const png = new Uint8Array(await fs.readFile(still));
    const ipc = new FakeIpcMain();
    const owner = new FakeWebContents(ipc);
    owner.send = (channel: string, request: AgentToolRequestEvent) => {
      owner.requests.push(request);
      const times = Array.isArray(request.arguments.times) ? request.arguments.times : [];
      const content = request.tool === '__composition_info'
        ? [{ type: 'text' as const, text: JSON.stringify({ width: 1280, height: 720, fps: 30, duration: 4, workArea: [0, 4] }) }]
        : request.tool === 'render_frames'
          ? [{ type: 'text' as const, text: '{}' }, ...times.map(() => ({ type: 'image' as const, data: png, mimeType: 'image/png' as const }))]
          : [];
      queueMicrotask(() => ipc.emit(IPC.agentToolResponse, { sender: owner }, { runId: request.runId, callId: request.callId, ok: content.length > 0, content, ...(content.length ? {} : { error: 'unexpected' }) }));
    };
    const bridge = new PowermoveAgentToolBridge(ipc as never, { mcpServerPath: '/resources/agent-tools/mcp-server.mjs', ffmpegPath, mediaFontFile: null });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'media-run-comp', owner: owner as never, baseRevision: 0 });

    const response = await bridge.callTool(session, 'media_contact_sheet', { target: 'composition', count: 4 });

    expect(response.ok).toBe(true);
    expect(owner.requests.map((request) => request.tool)).toEqual(['__composition_info', 'render_frames']);
    expect(JSON.parse((response.content[0] as { text: string }).text)).toMatchObject({ target: 'composition', columns: 2, rows: 2 });
    expect(response.content[1]).toMatchObject({ type: 'image', mimeType: 'image/jpeg' });
  });

  it('stops rendering a composition sheet at the call deadline and answers with the cells that finished', async () => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-agent-media-'));
    temporaryDirectories.push(root);
    const still = path.join(root, 'frame.png');
    const ffmpegPath = path.resolve(__dirname, '../../../node_modules/ffmpeg-static/ffmpeg');
    await new Promise<void>((resolve, reject) => spawn(ffmpegPath, ['-hide_banner', '-loglevel', 'error', '-y', '-f', 'lavfi', '-i', 'color=c=blue:s=320x180', '-frames:v', '1', still])
      .once('close', (code) => code === 0 ? resolve() : reject(new Error(`ffmpeg ${code}`))));
    const png = new Uint8Array(await fs.readFile(still));
    const ipc = new FakeIpcMain();
    const owner = new FakeWebContents(ipc);
    let renders = 0;
    owner.send = (channel: string, request: AgentToolRequestEvent) => {
      owner.requests.push(request);
      const times = Array.isArray(request.arguments.times) ? request.arguments.times : [];
      if (request.tool === 'render_frames' && ++renders > 1) return; // a heavy composition: the second batch never finishes
      const content = request.tool === '__composition_info'
        ? [{ type: 'text' as const, text: JSON.stringify({ width: 1280, height: 720, fps: 30, duration: 8 }) }]
        : [{ type: 'text' as const, text: '{}' }, ...times.map(() => ({ type: 'image' as const, data: png, mimeType: 'image/png' as const }))];
      queueMicrotask(() => ipc.emit(IPC.agentToolResponse, { sender: owner }, { runId: request.runId, callId: request.callId, ok: true, content }));
    };
    // 15 s of the budget is kept for assembling the sheet, so rendering gets 1.5 s.
    const bridge = new PowermoveAgentToolBridge(ipc as never, { mcpServerPath: '/resources/agent-tools/mcp-server.mjs', ffmpegPath, mediaFontFile: null, mediaCallBudgetMs: 16_500 });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'media-run-deadline', owner: owner as never, baseRevision: 0 });

    const started = Date.now();
    const response = await bridge.callTool(session, 'media_contact_sheet', { target: 'composition', count: 8 });

    expect(Date.now() - started).toBeLessThan(10_000);
    expect(response.ok).toBe(true);
    expect(renders).toBe(2);
    const result = JSON.parse((response.content[0] as { text: string }).text);
    expect(result.cells).toHaveLength(5);
    expect(result.omitted).toHaveLength(3);
    expect(response.content[1]).toMatchObject({ type: 'image', mimeType: 'image/jpeg' });
  });

  it('reports media tools as unavailable without a bundled ffmpeg', async () => {
    const ipc = new FakeIpcMain();
    const owner = new FakeWebContents(ipc);
    const bridge = new PowermoveAgentToolBridge(ipc as never, { mcpServerPath: '/resources/agent-tools/mcp-server.mjs' });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'media-run-2', owner: owner as never, baseRevision: 0 });
    await expect(bridge.callTool(session, 'media_waveform', { assetId: 'a1' })).rejects.toThrow(/unavailable/);
    expect(owner.requests).toHaveLength(0);
  });

  it('routes store tools to the gateway in main, never the renderer', async () => {
    const ipc = new FakeIpcMain();
    const owner = new FakeWebContents(ipc);
    const search = vi.fn().mockResolvedValue({ items: [{ handle: 'ada', slug: 'glow' }], nextCursor: null });
    const bridge = new PowermoveAgentToolBridge(ipc as never, {
      mcpServerPath: '/resources/agent-tools/mcp-server.mjs',
      storeAgent: { search } as never
    });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'store-run-1', owner: owner as never, baseRevision: 0 });

    const response = await bridge.callTool(session, 'store_search', { query: 'glow' });

    expect(search).toHaveBeenCalledWith({ query: 'glow' });
    expect(response.ok).toBe(true);
    expect(owner.requests).toHaveLength(0);
    const item = response.content?.[0];
    expect(item).toMatchObject({ type: 'text' });
    expect(JSON.parse((item as { text: string }).text)).toEqual({ items: [{ handle: 'ada', slug: 'glow' }], nextCursor: null });
  });

  it('reports a clear error when the store is unavailable', async () => {
    const ipc = new FakeIpcMain();
    const owner = new FakeWebContents(ipc);
    const bridge = new PowermoveAgentToolBridge(ipc as never, {
      mcpServerPath: '/resources/agent-tools/mcp-server.mjs',
      storeAgent: null
    });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'store-run-2', owner: owner as never, baseRevision: 0 });

    await expect(bridge.callTool(session, 'store_install', { handle: 'a', slug: 'b' })).rejects.toThrow(/Store is unavailable/);
    expect(owner.requests).toHaveLength(0);
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
    const apiFork = await bridge.callTool(session, 'fork_builtin_extension', { id: 'timeline', forkId: 'api-timeline' });
    expect(apiFork.ok).toBe(true);
    expect(JSON.parse((apiFork.content[0] as { text: string }).text).forkId).toBe('api-timeline');
    expect(owner.requests).toEqual([]);
  });

  it('uses the API run’s explicit stage without racing filesystem discovery', async () => {
    const ipc = new FakeIpcMain();
    const owner = new FakeWebContents(ipc);
    const discover = vi.fn(async () => { throw new Error('The filesystem snapshot raced workspace setup'); });
    const stageForkRebase = vi.fn(async ({ stagingDirectory }) => ({ workingDir: `${stagingDirectory}/my-fork` }));
    const bridge = new PowermoveAgentToolBridge(ipc as never, {
      command: process.execPath, mcpServerPath: path.join(__dirname, 'mcp-server.mjs'), stageForkRebase
    });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'api-rebase', owner: owner as never, baseRevision: 0, resolveStagingDirectory: discover });
    session.stagingDirectory = '/private/run-stage';
    const result = await bridge.callTool(session, 'stage_fork_rebase', { id: 'my-fork' });
    expect(result.ok).toBe(true);
    expect(stageForkRebase).toHaveBeenCalledWith({ forkId: 'my-fork', stagingDirectory: '/private/run-stage' });
    expect(discover).not.toHaveBeenCalled();
  });

  it('runs fork rebase staging in main for the owning run directory', async () => {
    const ipc = new FakeIpcMain();
    const owner = new FakeWebContents(ipc);
    const stageForkRebase = vi.fn(async ({ forkId, stagingDirectory }) => ({
      forkId,
      workingDir: `${stagingDirectory}/${forkId}`,
      baseDir: `${stagingDirectory}/.rebase/${forkId}/base`,
      oursDir: `${stagingDirectory}/.rebase/${forkId}/ours`,
      changedByUser: ['index.ts'], changedUpstream: ['index.ts'], conflicts: ['index.ts']
    }));
    const bridge = new PowermoveAgentToolBridge(ipc as never, {
      command: process.execPath,
      mcpServerPath: path.join(__dirname, 'mcp-server.mjs'),
      timeoutMs: 2_000,
      stageForkRebase
    });
    bridges.push(bridge);
    const session = await bridge.openSession({
      runId: 'native-run-rebase', owner: owner as never, baseRevision: 0,
      resolveStagingDirectory: async () => '/private/stage/current-run'
    });
    const child = spawn(session.mcpConfig.command, session.mcpConfig.args, {
      env: { ...process.env, ...session.mcpConfig.env },
      stdio: ['pipe', 'pipe', 'pipe']
    });
    children.push(child);

    const response = await rpc(child, {
      jsonrpc: '2.0', id: 9, method: 'tools/call',
      params: { name: 'stage_fork_rebase', arguments: { id: 'my-fork' } }
    });
    expect(response).toMatchObject({ id: 9, result: { isError: false } });
    expect(stageForkRebase).toHaveBeenCalledExactlyOnceWith({
      forkId: 'my-fork', stagingDirectory: '/private/stage/current-run'
    });
    expect(owner.requests).toHaveLength(0);
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

  it('marks the window driven by the agent in main and the renderer for all of computer_use_panel', async () => {
    const ipc = new FakeIpcMain();
    const log: string[] = [];
    let driving = 0;
    const input = { drive: vi.fn(() => { driving += 1; log.push('drive'); return () => { driving -= 1; log.push('release'); }; }) };
    const owner = new FakeWebContents(ipc) as FakeWebContents & Record<string, unknown>;
    owner.send = (channel: string, payload: AgentToolRequestEvent | boolean) => {
      if (channel === IPC.agentToolInput) { log.push(`renderer ${payload}`); return; }
      const request = payload as AgentToolRequestEvent;
      log.push(request.tool);
      const text = request.tool === '__prepare_panel_input' ? { points: [{ x: 5, y: 6 }] }
        : request.tool === '__panel_bounds' ? { x: 0, y: 0, width: 10, height: 10 } : {};
      queueMicrotask(() => ipc.emit(IPC.agentToolResponse, { sender: owner }, {
        runId: request.runId, callId: request.callId, ok: true, revision: 1, content: [{ type: 'text', text: JSON.stringify(text) }]
      }));
    };
    owner.sendInputEvent = (event: { type: string }) => log.push(`${event.type}${driving ? ' (driven)' : ''}`);
    const image = { isEmpty: () => false, getSize: () => ({ width: 10, height: 10 }), toJPEG: () => Buffer.from('jpeg') };
    owner.capturePage = async () => ({ ...image, resize: () => image });
    const bridge = new PowermoveAgentToolBridge(ipc as never, { mcpServerPath: '', timeoutMs: 2_000, userInput: input });
    bridges.push(bridge);
    const session = await bridge.openSession({ runId: 'native-run-drive', owner: owner as never, baseRevision: 0 });
    await bridge.callTool(session, 'computer_use_panel', { panelId: 'ext.panel', action: 'click', points: [{ x: 5, y: 6 }] });
    expect(input.drive).toHaveBeenCalledExactlyOnceWith(owner);
    expect(log).toEqual([
      'drive', 'renderer true', '__prepare_panel_input',
      'mouseMove (driven)', 'mouseDown (driven)', 'mouseUp (driven)',
      'get_project_state', '__panel_bounds', 'release', 'renderer false'
    ]);
    log.length = 0;
    owner.sendInputEvent = () => { throw new Error('no input'); };
    await expect(bridge.callTool(session, 'computer_use_panel', { panelId: 'ext.panel', action: 'click', points: [{ x: 5, y: 6 }] })).rejects.toThrow('no input');
    expect(log).toEqual(['drive', 'renderer true', '__prepare_panel_input', 'release', 'renderer false']);
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

  it('lists and runs run_outside_sandbox only in a session that asks, in main', async () => {
    const ipc = new FakeIpcMain();
    const owner = new FakeWebContents(ipc);
    const bridge = new PowermoveAgentToolBridge(ipc as never, {
      command: process.execPath,
      mcpServerPath: path.join(__dirname, 'mcp-server.mjs'),
      timeoutMs: 2_000
    });
    bridges.push(bridge);
    const plain = await bridge.openSession({ runId: 'plain-run-1', owner: owner as never, baseRevision: 0 });
    expect(plain.mcpConfig.toolTimeoutSec).toBeUndefined();
    await expect(bridge.callTool(plain, 'run_outside_sandbox', { command: 'true', reason: 'x' })).rejects.toThrow(/not available/);

    const outsideSandbox = vi.fn(async ({ command }: { command: string }) => `Exit code: 0\nran ${command}`);
    const asking = await bridge.openSession({ runId: 'asking-run-1', owner: owner as never, baseRevision: 0, outsideSandbox });
    expect(asking.mcpConfig.toolTimeoutSec).toBeGreaterThan(3_600);
    const child = spawn(asking.mcpConfig.command, asking.mcpConfig.args, {
      env: { ...process.env, ...asking.mcpConfig.env },
      stdio: ['pipe', 'pipe', 'pipe']
    });
    children.push(child);
    const listed = await rpc(child, { jsonrpc: '2.0', id: 1, method: 'tools/list', params: {} });
    expect(listed.result.tools.map((tool: { name: string }) => tool.name)).toContain('run_outside_sandbox');
    const called = await rpc(child, {
      jsonrpc: '2.0', id: 2, method: 'tools/call',
      params: { name: 'run_outside_sandbox', arguments: { command: 'echo hi', reason: 'Testing.' } }
    });
    expect(called.result).toMatchObject({ isError: false, content: [{ type: 'text', text: 'Exit code: 0\nran echo hi' }] });
    expect(outsideSandbox).toHaveBeenCalledWith({ command: 'echo hi', reason: 'Testing.', timeoutMs: 120_000 });

    outsideSandbox.mockRejectedValueOnce(new Error('The user declined.'));
    const declined = await rpc(child, {
      jsonrpc: '2.0', id: 3, method: 'tools/call',
      params: { name: 'run_outside_sandbox', arguments: { command: 'echo no', reason: 'Testing.' } }
    });
    expect(declined.result).toMatchObject({ isError: true });
    expect(declined.result.content[0].text).toContain('The user declined.');
    expect(owner.requests.filter(request => request.tool === 'run_outside_sandbox')).toHaveLength(0);
    await Promise.all([plain.finish(false), asking.finish(false)]);
  });
});
