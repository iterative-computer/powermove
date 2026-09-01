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
