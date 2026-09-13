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
  it('configures the editor thread with required live-inspection tools only', async () => {
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
      nativeTools
    });
    await vi.waitFor(() => expect(child.messages.some((message) => message.method === 'thread/start')).toBe(true));

    const started = child.messages.find((message) => message.method === 'thread/start');
    expect(started?.params.config).toEqual({
      mcp_servers: {
        powermove: {
          ...nativeTools,
          startup_timeout_sec: 10,
          tool_timeout_sec: 120,
          required: true,
          enabled_tools: [
            'get_project_state', 'get_panel_layout', 'open_panel', 'get_panel_state',
            'capture_panel', 'get_workspace_state', 'render_frames'
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
