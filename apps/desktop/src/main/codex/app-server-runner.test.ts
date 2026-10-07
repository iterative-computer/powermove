import { ORCHESTRATION_TOOL_NAMES } from '../../shared/agent-orchestration';
// Keep runner tests independent of the developer's configured external services.
vi.mock('../agent-tools/user-mcp', async importOriginal => ({
  ...await importOriginal<typeof import('../agent-tools/user-mcp')>(),
  loadUserMcpServers: vi.fn(async () => ({}))
}));

import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import type { ChildProcessWithoutNullStreams } from 'node:child_process';

import { describe, expect, it, vi } from 'vitest';

import type { CodexRunRequest } from '../../shared/ipc';
import { CodexAppServerRunner } from './app-server-runner';

class FakeAppServer extends EventEmitter {
  readonly stdin = new PassThrough();
  readonly stdout = new PassThrough();
  readonly stderr = new PassThrough();
  readonly stdio = [this.stdin, this.stdout, this.stderr];
  readonly channel = null;
  readonly connected = false;
  readonly pid = 5252;
  readonly signalCode = null;
  readonly spawnargs: string[] = [];
  readonly spawnfile = '/fake/codex';
  exitCode: number | null = null;
  killed = false;
  readonly messages: Array<Record<string, any>> = [];
  private buffered = '';

  constructor() {
    super();
    this.stdin.setEncoding('utf8');
    this.stdin.on('data', (chunk: string) => this.receive(chunk));
  }

  kill(): boolean { this.killed = true; return true; }
  notify(method: string, params: Record<string, unknown>): void {
    this.stdout.write(`${JSON.stringify({ method, params })}\n`);
  }

  /** A request initiated by App Server, which the client must answer. */
  ask(id: number | string, method: string, params: Record<string, unknown>): void {
    this.stdout.write(`${JSON.stringify({ id, method, params })}\n`);
  }

  private receive(chunk: string): void {
    this.buffered += chunk;
    const lines = this.buffered.split('\n');
    this.buffered = lines.pop() || '';
    for (const line of lines) {
      if (!line) continue;
      const message = JSON.parse(line) as Record<string, any>;
      this.messages.push(message);
      if (typeof message.id !== 'number') continue;
      if (message.method === 'initialize') this.respond(message.id, {});
      else if (message.method === 'thread/start') this.respond(message.id, { thread: { id: 'thr_123' } });
      else if (message.method === 'turn/start') this.respond(message.id, { turn: { id: 'turn_456', status: 'inProgress' } });
      else if (message.method === 'turn/steer') this.respond(message.id, { turnId: 'turn_456' });
      else if (message.method === 'turn/interrupt') this.respond(message.id, {});
    }
  }

  private respond(id: number, result: unknown): void {
    queueMicrotask(() => this.stdout.write(`${JSON.stringify({ id, result })}\n`));
  }
}

function request(): CodexRunRequest {
  return {
    id: 'steer-run-1234',
    threadId: 'powermove-thread',
    mode: 'editor',
    prompt: 'Make a progressive blur effect',
    schema: { type: 'object', properties: { kind: { type: 'string' } }, required: ['kind'] },
    images: [],
    model: 'gpt-5.6-sol',
    reasoningEffort: 'high',
    access: 'editor',
    projectId: 'steer-project',
    projectName: 'Steer Project',
    projectJSON: null,
    attachments: [],
    consentToken: null
  };
}

describe('CodexAppServerRunner steering', () => {
  it('passes GPT-6.1 Sol and Ultra to the native thread and turn', async () => {
    const child = new FakeAppServer();
    const runner = new CodexAppServerRunner({
      discoverBinary: async () => '/fake/codex',
      prepareHome: async () => '/tmp/powermove-app-server-test',
      spawnProcess: () => child as unknown as ChildProcessWithoutNullStreams,
      requestTimeoutMs: 500,
      turnTimeoutMs: 5_000
    });
    const run = runner.run({ ...request(), model: 'gpt-6.1-sol', reasoningEffort: 'ultra' }, {
      userData: '/tmp/powermove-app-server-test'
    });
    await vi.waitFor(() => expect(child.messages.some(message => message.method === 'turn/start')).toBe(true));
    expect(child.messages.find(message => message.method === 'thread/start')?.params.model).toBe('gpt-6.1-sol');
    expect(child.messages.find(message => message.method === 'turn/start')?.params).toMatchObject({
      model: 'gpt-6.1-sol', effort: 'ultra'
    });
    child.notify('item/completed', {
      threadId: 'thr_123', turnId: 'turn_456', item: { type: 'agentMessage', text: '{"kind":"scene"}' }
    });
    child.notify('turn/completed', {
      threadId: 'thr_123', turn: { id: 'turn_456', status: 'completed', error: null }
    });
    await expect(run).resolves.toMatchObject({ ok: true, text: '{"kind":"scene"}' });
    await runner.shutdown();
  });

  it('configures editor threads with user MCP tools alongside live-inspection tools', async () => {
    const child = new FakeAppServer();
    const runner = new CodexAppServerRunner({
      discoverBinary: async () => '/fake/codex',
      prepareHome: async () => '/tmp/powermove-app-server-test',
      spawnProcess: () => child as unknown as ChildProcessWithoutNullStreams,
      requestTimeoutMs: 500,
      turnTimeoutMs: 5_000
    });
    const nativeTools = {
      command: '/test/electron',
      args: ['/test/mcp-server.mjs'],
      env: { POWERMOVE_AGENT_TOOL_TOKEN: 'secret' }
    };
    const run = runner.run(request(), {
      userData: '/tmp/powermove-app-server-test',
      nativeTools,
      externalMcpServers: { studio: { command: 'fixture-server' }, powermove: { command: 'wrong-server' } }
    });
    await vi.waitFor(() => expect(child.messages.some((message) => message.method === 'thread/start')).toBe(true));

    const started = child.messages.find((message) => message.method === 'thread/start');
    expect(started?.params.sandbox).toBe('read-only');
    expect(started?.params.config).toEqual({
      mcp_servers: {
        studio: { command: 'fixture-server' },
        powermove: {
          ...nativeTools,
          startup_timeout_sec: 30,
          tool_timeout_sec: 120,
          required: true,
          enabled_tools: [
            ...ORCHESTRATION_TOOL_NAMES,
            'get_project_state', 'list_media', 'get_3d_scene', 'get_panel_layout', 'open_panel', 'get_panel_state',
            'capture_panel', 'get_workspace_state', 'validate_effect', 'render_frames',
            'probe_media', 'sample_media_frames', 'media_contact_sheet', 'media_waveform',
            'transcribe_media', 'check_project', 'export_captions',
            'store_search', 'store_extension', 'store_source', 'store_library'
          ],
          default_tools_approval_mode: 'approve'
        }
      }
    });

    await runner.cancel(request().id);
    await expect(run).resolves.toMatchObject({ cancelled: true });
    await runner.shutdown();
  });

  it('appends steering to the same active turn and returns that turn result', async () => {
    const child = new FakeAppServer();
    const runner = new CodexAppServerRunner({
      discoverBinary: async () => '/fake/codex',
      prepareHome: async () => '/tmp/powermove-app-server-test',
      spawnProcess: () => child as unknown as ChildProcessWithoutNullStreams,
      requestTimeoutMs: 500,
      turnTimeoutMs: 5_000
    });
    const run = runner.run(request(), { userData: '/tmp/powermove-app-server-test' });
    await vi.waitFor(() => expect(child.messages.some((message) => message.method === 'turn/start')).toBe(true));

    await expect(runner.steer({
      id: 'steer-run-1234',
      prompt: 'Continue, but make the edge softer',
      images: []
    })).resolves.toBe(true);
    const steering = child.messages.find((message) => message.method === 'turn/steer');
    expect(steering?.params).toEqual({
      threadId: 'thr_123',
      expectedTurnId: 'turn_456',
      input: [{ type: 'text', text: 'Continue, but make the edge softer' }]
    });

    child.notify('item/completed', {
      threadId: 'thr_123', turnId: 'turn_456',
      item: { type: 'agentMessage', text: '{"kind":"scene"}' }
    });
    child.notify('turn/completed', {
      threadId: 'thr_123', turn: { id: 'turn_456', status: 'completed', error: null }
    });
    await expect(run).resolves.toEqual({ ok: true, text: '{"kind":"scene"}', access: 'editor' });
    await runner.shutdown();
  });

  it('streams matched tool and text notifications, preserving fragments and suppressing completed duplicates', async () => {
    const child = new FakeAppServer();
    const trace: unknown[] = [];
    const progress: string[] = [];
    const runner = new CodexAppServerRunner({
      discoverBinary: async () => '/fake/codex',
      prepareHome: async () => '/tmp/powermove-app-server-test',
      spawnProcess: () => child as unknown as ChildProcessWithoutNullStreams,
      requestTimeoutMs: 500,
      turnTimeoutMs: 5_000
    });
    const run = runner.run(request(), {
      userData: '/tmp/powermove-app-server-test',
      onTrace: (event) => trace.push(event),
      onProgress: (text) => progress.push(text)
    });
    await vi.waitFor(() => expect(child.messages.some((message) => message.method === 'turn/start')).toBe(true));

    child.notify('item/started', {
      threadId: 'other-thread', turnId: 'turn_456',
      item: { id: 'ignored-thread', type: 'commandExecution', command: 'nope' }
    });
    child.notify('item/started', {
      threadId: 'thr_123', turnId: 'other-turn',
      item: { id: 'ignored-turn', type: 'commandExecution', command: 'nope' }
    });
    child.notify('codex/event/item/started', {
      threadId: 'thr_123', turnId: 'turn_456',
      item: { id: 'cmd-1', type: 'commandExecution', command: 'bun run typecheck' }
    });
    child.notify('item/started', {
      threadId: 'thr_123', turnId: 'turn_456',
      item: { id: 'files-1', type: 'file_change', changes: [{ path: '/tmp/a.ts' }, { path: 'src/b.ts' }] }
    });
    child.notify('item/started', {
      threadId: 'thr_123', turnId: 'turn_456',
      item: { id: 'mcp-1', type: 'mcpToolCall', server: 'powermove', tool: 'get_panel_state' }
    });
    child.notify('item/started', {
      threadId: 'thr_123', turnId: 'turn_456',
      item: { id: 'mcp-2', type: 'mcp_tool_call', server: 'github', tool: 'create_issue' }
    });
    child.notify('item/started', {
      threadId: 'thr_123', turnId: 'turn_456',
      item: { id: 'web-1', type: 'webSearch', query: 'Codex protocol' }
    });
    child.notify('item/started', {
      threadId: 'thr_123', turnId: 'turn_456', item: { id: 'image-1', type: 'image_generation' }
    });
    child.notify('item/started', {
      threadId: 'thr_123', turnId: 'turn_456', item: { id: 'computer-1', type: 'computerUse' }
    });
    child.notify('item/completed', {
      threadId: 'thr_123', turnId: 'turn_456',
      item: { id: 'cmd-1', type: 'command_execution', status: 'completed', aggregatedOutput: 'ok\n2 tests passed' }
    });
    child.notify('item/completed', {
      threadId: 'thr_123', turnId: 'turn_456',
      item: { id: 'mcp-1', type: 'mcp_tool_call', status: 'failed', error: { message: 'Panel unavailable' } }
    });
    child.notify('item/completed', {
      threadId: 'thr_123', turnId: 'turn_456',
      item: { id: 'mcp-2', type: 'mcpToolCall', status: 'completed', result: { content: [{ type: 'text', text: 'Issue #42' }] } }
    });
    child.notify('item/completed', {
      threadId: 'thr_123', turnId: 'turn_456',
      item: { id: 'files-1', type: 'fileChange', status: 'completed', changes: [{ path: '/tmp/a.ts' }, { path: 'src/b.ts' }] }
    });
    child.notify('item/completed', {
      threadId: 'thr_123', turnId: 'turn_456',
      item: { id: 'web-1', type: 'web_search', status: 'completed', query: 'Codex protocol' }
    });
    child.notify('item/completed', {
      threadId: 'thr_123', turnId: 'turn_456', item: { id: 'image-1', type: 'imageGeneration', status: 'completed' }
    });
    child.notify('item/completed', {
      threadId: 'thr_123', turnId: 'turn_456', item: { id: 'computer-1', type: 'computer_use', status: 'declined', error: 'Approval declined' }
    });
    child.notify('item/commandExecution/outputDelta', {
      threadId: 'thr_123', turnId: 'turn_456', itemId: 'cmd-1', delta: 'do not flood'
    });
    child.notify('item/reasoning/textDelta', {
      threadId: 'thr_123', turnId: 'turn_456', itemId: 'reason-1', delta: '  thinking\n'
    });
    child.notify('item/reasoning/summaryTextDelta', {
      threadId: 'thr_123', turnId: 'turn_456', itemId: 'reason-1', delta: ' summary  update '
    });
    child.notify('item/agentMessage/delta', {
      threadId: 'thr_123', turnId: 'turn_456', itemId: 'msg-streamed', delta: 'Hello'
    });
    child.notify('rpc/item/agentMessage/delta', {
      threadId: 'thr_123', turnId: 'turn_456', itemId: 'msg-streamed', delta: ' world'
    });
    child.notify('item/completed', {
      threadId: 'thr_123', turnId: 'turn_456',
      item: { id: 'msg-other', type: 'agentMessage', text: 'Separate message' }
    });
    child.notify('item/completed', {
      threadId: 'thr_123', turnId: 'turn_456',
      item: { id: 'msg-streamed', type: 'agentMessage', text: 'Hello world' }
    });
    child.notify('turn/completed', {
      threadId: 'thr_123', turn: { id: 'turn_456', status: 'completed' }
    });

    await expect(run).resolves.toEqual({ ok: true, text: 'Hello world', access: 'editor' });
    expect(trace).toEqual([
      { kind: 'tool-start', itemId: 'cmd-1', toolName: 'bash', label: 'Run', detail: 'bun run typecheck' },
      { kind: 'tool-start', itemId: 'files-1', toolName: 'edit', label: 'Edit', detail: 'a.ts, b.ts' },
      { kind: 'tool-start', itemId: 'mcp-1', toolName: 'get_panel_state', label: 'Get panel state', detail: 'get_panel_state' },
      { kind: 'tool-start', itemId: 'mcp-2', toolName: 'create_issue', label: 'Create issue', detail: 'create_issue' },
      { kind: 'tool-start', itemId: 'web-1', toolName: 'search', label: 'Search', detail: 'Codex protocol' },
      { kind: 'tool-start', itemId: 'image-1', toolName: 'image', label: 'Image' },
      { kind: 'tool-start', itemId: 'computer-1', toolName: 'computer', label: 'Computer' },
      { kind: 'tool-end', itemId: 'cmd-1', isError: false, output: 'ok\n2 tests passed' },
      { kind: 'tool-end', itemId: 'mcp-1', isError: true, output: 'Panel unavailable' },
      { kind: 'tool-end', itemId: 'mcp-2', isError: false, output: 'Issue #42' },
      { kind: 'tool-end', itemId: 'files-1', isError: false, output: 'a.ts, b.ts' },
      { kind: 'tool-end', itemId: 'web-1', isError: false, output: 'Codex protocol' },
      { kind: 'tool-end', itemId: 'image-1', isError: false },
      { kind: 'tool-end', itemId: 'computer-1', isError: true, output: 'Approval declined' },
      { kind: 'thought', text: '  thinking\n' },
      { kind: 'thought', text: ' summary  update ' },
      { kind: 'answer', text: 'Hello' },
      { kind: 'answer', text: ' world' },
      { kind: 'answer', text: 'Separate message' }
    ]);
    expect(progress).toEqual(['summary update']);
    await runner.shutdown();
  });

  it('rejects steering when no matching turn is active', async () => {
    const runner = new CodexAppServerRunner();
    await expect(runner.steer({ id: 'missing-run', prompt: 'continue', images: [] })).resolves.toBe(false);
  });
});

it('returns startup failures as results and permits a later retry', async () => {
  const child = new FakeAppServer();
  const discoverBinary = vi.fn().mockRejectedValueOnce(new Error('Binary unavailable')).mockResolvedValue('/fake/codex');
  const runner = new CodexAppServerRunner({ discoverBinary, prepareHome: async () => '/tmp/pm-test',
    spawnProcess: () => child as unknown as ChildProcessWithoutNullStreams });
  try {
    await expect(runner.run(request(), { userData: '/tmp/pm-test' })).resolves.toMatchObject({ ok: false, error: 'Binary unavailable' });
    const run = runner.run(request(), { userData: '/tmp/pm-test' });
    await vi.waitFor(() => expect(child.messages.some(m => m.method === 'turn/start')).toBe(true));
    await runner.cancel(request().id);
    await expect(run).resolves.toMatchObject({ cancelled: true });
  } finally { await runner.shutdown(); }
});

it('honors cancellation during startup without starting a turn', async () => {
  const child = new FakeAppServer();
  let ready!: (binary: string) => void;
  const runner = new CodexAppServerRunner({
    discoverBinary: () => new Promise(resolve => { ready = resolve; }),
    prepareHome: async () => '/tmp/pm-test', spawnProcess: () => child as unknown as ChildProcessWithoutNullStreams
  });
  const run = runner.run(request(), { userData: '/tmp/pm-test' });
  const cancelled = await runner.cancel(request().id);
  ready('/fake/codex');
  await vi.waitFor(() => expect(child.messages.some(m => m.method === 'initialize')).toBe(true));
  if (!cancelled) await runner.cancel(request().id);
  try {
    expect(cancelled).toBe(true);
    await expect(run).resolves.toMatchObject({ cancelled: true });
    expect(child.messages.some(m => m.method === 'turn/start')).toBe(false);
  } finally { await runner.shutdown(); await run; }
});

it('ignores a delayed exit from an older process after reconnecting', async () => {
  const children = [new FakeAppServer(), new FakeAppServer()];
  const runner = new CodexAppServerRunner({ discoverBinary: async () => '/fake/codex', prepareHome: async () => '/tmp/pm-test',
    spawnProcess: vi.fn().mockImplementationOnce(() => children[0]).mockImplementationOnce(() => children[1]) });
  const first = runner.run(request(), { userData: '/tmp/pm-test' });
  await vi.waitFor(() => expect(children[0]!.messages.some(m => m.method === 'turn/start')).toBe(true));
  children[0]!.emit('error', new Error('Connection closed'));
  await expect(first).resolves.toMatchObject({ ok: false });
  const second = runner.run(request(), { userData: '/tmp/pm-test' });
  await vi.waitFor(() => expect(children[1]!.messages.some(m => m.method === 'turn/start')).toBe(true));
  children[0]!.emit('exit', 1, null);
  children[1]!.notify('item/completed', { threadId: 'thr_123', turnId: 'turn_456', item: { type: 'agentMessage', text: '{"ok":true}' } });
  children[1]!.notify('turn/completed', { threadId: 'thr_123', turn: { id: 'turn_456', status: 'completed' } });
  try { await expect(second).resolves.toMatchObject({ ok: true }); }
  finally { await runner.shutdown(); }
});

it('releases a timed-out turn and preserves the timeout result while interrupting it', async () => {
  const child = new FakeAppServer();
  const runner = new CodexAppServerRunner({ discoverBinary: async () => '/fake/codex', prepareHome: async () => '/tmp/pm-test',
    spawnProcess: () => child as unknown as ChildProcessWithoutNullStreams, turnTimeoutMs: 30 });
  try {
    await expect(runner.run(request(), { userData: '/tmp/pm-test' })).resolves.toMatchObject({
      ok: false, cancelled: false, error: expect.stringContaining('too long')
    });
    expect(child.messages.some(m => m.method === 'turn/interrupt')).toBe(true);
    await expect(runner.steer({ id: request().id, prompt: 'continue', images: [] })).resolves.toBe(false);
  } finally { await runner.shutdown(); }
});

it('terminates an unresponsive startup before launching another process', async () => {
  const stuck = new FakeAppServer();
  stuck.stdin.removeAllListeners('data');
  const next = new FakeAppServer();
  const runner = new CodexAppServerRunner({ discoverBinary: async () => '/fake/codex', prepareHome: async () => '/tmp/pm-test',
    spawnProcess: vi.fn().mockReturnValueOnce(stuck).mockReturnValueOnce(next), requestTimeoutMs: 30 });
  try {
    await expect(runner.run(request(), { userData: '/tmp/pm-test' })).resolves.toMatchObject({ ok: false });
    expect(stuck.killed).toBe(true);
    const run = runner.run(request(), { userData: '/tmp/pm-test' });
    await vi.waitFor(() => expect(next.messages.some(m => m.method === 'turn/start')).toBe(true));
    await runner.cancel(request().id);
    await expect(run).resolves.toMatchObject({ cancelled: true });
  } finally { await runner.shutdown(); }
});

describe('CodexAppServerRunner questions', () => {
  function start() {
    const child = new FakeAppServer();
    const runner = new CodexAppServerRunner({
      discoverBinary: async () => '/fake/codex',
      prepareHome: async () => '/tmp/powermove-app-server-test',
      spawnProcess: () => child as unknown as ChildProcessWithoutNullStreams,
      requestTimeoutMs: 500,
      turnTimeoutMs: 5_000
    });
    const traces: any[] = [];
    const progress: string[] = [];
    const run = runner.run(request(), {
      userData: '/tmp/powermove-app-server-test',
      onTrace: (step) => traces.push(step),
      onProgress: (text) => progress.push(text)
    });
    return { child, runner, traces, progress, run };
  }
  const scope = { threadId: 'thr_123', turnId: 'turn_456' };
  const complete = (child: FakeAppServer) => {
    child.notify('item/completed', { ...scope, item: { type: 'agentMessage', id: 'msg_final', text: '{"kind":"done"}' } });
    child.notify('turn/completed', { threadId: 'thr_123', turn: { id: 'turn_456', status: 'completed' } });
  };

  it('opts into the experimental API that carries agent questions', async () => {
    const { child, runner, run } = start();
    await vi.waitFor(() => expect(child.messages.some((message) => message.method === 'turn/start')).toBe(true));
    expect(child.messages.find((message) => message.method === 'initialize')?.params.capabilities)
      .toEqual({ experimentalApi: true, requestAttestation: false });
    await runner.cancel(request().id);
    await run;
    await runner.shutdown();
  });

  it('surfaces request_user_input and answers it with the reply', async () => {
    const { child, runner, traces, run } = start();
    await vi.waitFor(() => expect(child.messages.some((message) => message.method === 'turn/start')).toBe(true));
    // Our own requests used ids 1-3; a server request reusing one must not settle it.
    child.ask(3, 'item/tool/requestUserInput', {
      ...scope, itemId: 'call_q', isBlocking: true, autoResolutionMs: null,
      questions: [
        { id: 'layout', header: 'Layout', question: 'Stack or grid?', isOther: false, isSecret: false,
          options: [{ label: 'Stack', description: 'One column' }, { label: 'Grid', description: '' }] },
        { id: 'bad', question: '' }
      ]
    });
    await vi.waitFor(() => expect(traces.some((step) => step.kind === 'question')).toBe(true));
    expect(traces.find((step) => step.kind === 'question')).toEqual({
      kind: 'question', itemId: 'call_q', transport: 'reply', blocking: true,
      questions: [{ id: 'layout', header: 'Layout', question: 'Stack or grid?', allowOther: false, secret: false,
        options: [{ label: 'Stack', description: 'One column' }, { label: 'Grid', description: '' }] }]
    });

    expect(runner.answer({ id: request().id, itemId: 'call_q', answers: { layout: ['Grid'] } })).toBe(true);
    expect(runner.answer({ id: request().id, itemId: 'call_q', answers: { layout: ['Grid'] } })).toBe(false);
    await vi.waitFor(() => expect(child.messages).toContainEqual({ id: 3, result: { answers: { layout: { answers: ['Grid'] } } } }));

    complete(child);
    await expect(run).resolves.toMatchObject({ ok: true, text: '{"kind":"done"}' });
    await runner.shutdown();
  });

  it('releases an unanswered question when the turn ends and reports provider resolution', async () => {
    const { child, runner, traces, run } = start();
    await vi.waitFor(() => expect(child.messages.some((message) => message.method === 'turn/start')).toBe(true));
    child.ask('q-a', 'item/tool/requestUserInput', { ...scope, itemId: 'call_a', isBlocking: false, questions: [{ id: 'x', question: 'Why?' }] });
    child.ask('q-b', 'item/tool/requestUserInput', { ...scope, itemId: 'call_b', isBlocking: false, questions: [{ id: 'y', question: 'How?' }] });
    await vi.waitFor(() => expect(traces.filter((step) => step.kind === 'question')).toHaveLength(2));
    expect(traces.find((step) => step.itemId === 'call_a')).toMatchObject({ blocking: false, questions: [{ allowOther: true }] });

    child.notify('serverRequest/resolved', { threadId: 'thr_123', requestId: 'q-a' });
    await vi.waitFor(() => expect(traces).toContainEqual({ kind: 'question-closed', itemId: 'call_a' }));

    complete(child);
    await run;
    expect(child.messages).toContainEqual({ id: 'q-b', result: { answers: {} } });
    expect(child.messages).not.toContainEqual({ id: 'q-a', result: { answers: {} } });
    await runner.shutdown();
  });

  it('shows async message questions as one card, never as prose or the final answer', async () => {
    const { child, runner, traces, run } = start();
    await vi.waitFor(() => expect(child.messages.some((message) => message.method === 'turn/start')).toBe(true));
    // `request_user_input_async` (Codex 0.159): started and completed carry the same item.
    const item = {
      type: 'agentMessage', id: 'msg_q', text: 'Keep the logo?\n- Yes\n- No\n\nAny deadline?', delivery: 'async', phase: 'final_answer',
      questions: [{ title: 'Keep the logo?', options: ['Yes', 'No'] }, { title: 'Any deadline?', options: null }]
    };
    child.notify('item/started', { ...scope, item });
    child.notify('item/agentMessage/delta', { ...scope, itemId: 'msg_q', delta: 'Keep the logo?' });
    child.notify('item/completed', { ...scope, item });
    child.notify('item/completed', { ...scope, item: { type: 'agentMessage', id: 'msg_update', text: 'Still going.', delivery: 'async' } });
    await vi.waitFor(() => expect(traces).toContainEqual({ kind: 'answer', text: 'Still going.' }));
    expect(traces.filter((step) => step.kind === 'question')).toHaveLength(1);
    expect(traces.filter((step) => step.kind === 'answer')).toEqual([{ kind: 'answer', text: 'Still going.' }]);
    expect(traces.find((step) => step.kind === 'question')).toEqual({
      kind: 'question', itemId: 'msg_q', transport: 'message', blocking: false,
      questions: [
        { id: 'msg_q-0', header: '', question: 'Keep the logo?', allowOther: true, secret: false,
          options: [{ label: 'Yes', description: '' }, { label: 'No', description: '' }] },
        { id: 'msg_q-1', header: '', question: 'Any deadline?', allowOther: true, secret: false, options: [] }
      ]
    });

    child.notify('turn/completed', { threadId: 'thr_123', turn: { id: 'turn_456', status: 'completed' } });
    await expect(run).resolves.toMatchObject({ ok: false });
    await runner.shutdown();
  });

  it('answers requests it does not support instead of stalling the turn', async () => {
    const { child, runner, run } = start();
    await vi.waitFor(() => expect(child.messages.some((message) => message.method === 'turn/start')).toBe(true));
    child.ask(90, 'mcpServer/elicitation/request', { threadId: 'thr_123', turnId: 'turn_456', serverName: 'powermove', mode: 'form' });
    child.ask(91, 'item/commandExecution/requestApproval', { ...scope, itemId: 'cmd' });
    await vi.waitFor(() => expect(child.messages).toContainEqual({ id: 90, result: { action: 'decline', content: null, _meta: null } }));
    await vi.waitFor(() => expect(child.messages.find((message) => message.id === 91 && message.error)).toBeDefined());
    complete(child);
    await expect(run).resolves.toMatchObject({ ok: true });
    await runner.shutdown();
  });

  it('shows compaction, reroutes, and retries without treating them as the answer', async () => {
    const { child, runner, traces, progress, run } = start();
    await vi.waitFor(() => expect(child.messages.some((message) => message.method === 'turn/start')).toBe(true));
    child.notify('item/started', { ...scope, item: { type: 'contextCompaction', id: 'cmp' } });
    child.notify('item/completed', { ...scope, item: { type: 'contextCompaction', id: 'cmp' } });
    child.notify('error', { ...scope, willRetry: true, error: { message: 'stream disconnected', codexErrorInfo: null } });
    child.notify('model/rerouted', { ...scope, fromModel: 'gpt-6', toModel: 'gpt-6-mini', reason: 'highRiskCyberActivity' });
    child.notify('warning', { threadId: null, message: 'Usage is nearly exhausted.' });
    await vi.waitFor(() => expect(progress).toEqual([
      'Connection interrupted. Retrying…', 'Continuing on gpt-6-mini', 'Usage is nearly exhausted.'
    ]));
    expect(traces).toEqual([
      { kind: 'tool-start', itemId: 'cmp', toolName: 'compact', label: 'Compacting context' },
      { kind: 'tool-end', itemId: 'cmp', isError: false, output: 'Earlier context summarized' }
    ]);

    child.notify('turn/completed', { threadId: 'thr_123', turn: { id: 'turn_456', status: 'failed',
      error: { message: 'context_length_exceeded', codexErrorInfo: 'contextWindowExceeded' } } });
    await expect(run).resolves.toMatchObject({ ok: false, error: expect.stringContaining('even after compacting') });
    await runner.shutdown();
  });
});
