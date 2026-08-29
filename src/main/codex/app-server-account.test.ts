import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import type { ChildProcessWithoutNullStreams } from 'node:child_process';

import { describe, expect, it, vi } from 'vitest';

import {
  ChatGPTAccountClient,
  chatGPTAccountStatusFromAppServer,
  isAllowedChatGPTAuthUrl
} from './app-server-account';

class FakeAppServer extends EventEmitter {
  readonly stdin = new PassThrough();
  readonly stdout = new PassThrough();
  readonly stderr = new PassThrough();
  readonly stdio = [this.stdin, this.stdout, this.stderr];
  readonly channel = null;
  readonly connected = false;
  readonly pid = 4242;
  readonly signalCode = null;
  readonly spawnargs: string[] = [];
  readonly spawnfile = '/fake/codex';
  exitCode: number | null = null;
  killed = false;
  account: Record<string, unknown> | null = null;
  readonly messages: Array<Record<string, unknown>> = [];
  private buffered = '';

  constructor(private readonly authUrl = 'https://chatgpt.com/auth/powermove') {
    super();
    this.stdin.setEncoding('utf8');
    this.stdin.on('data', (chunk: string) => this.receive(chunk));
  }

  kill(): boolean {
    this.killed = true;
    return true;
  }

  notify(method: string, params: Record<string, unknown>): void {
    this.stdout.write(`${JSON.stringify({ method, params })}\n`);
  }

  private receive(chunk: string): void {
    this.buffered += chunk;
    const lines = this.buffered.split('\n');
    this.buffered = lines.pop() || '';
    for (const line of lines) {
      if (!line) continue;
      const message = JSON.parse(line) as Record<string, unknown>;
      this.messages.push(message);
      if (typeof message.id !== 'number') continue;
      if (message.method === 'initialize') this.respond(message.id, {});
      else if (message.method === 'account/read') this.respond(message.id, {
        account: this.account,
        requiresOpenaiAuth: true
      });
      else if (message.method === 'account/login/start') this.respond(message.id, {
        type: 'chatgpt',
        loginId: 'login-123',
        authUrl: this.authUrl
      });
      else if (message.method === 'account/login/cancel') this.respond(message.id, {});
      else if (message.method === 'account/logout') {
        this.account = null;
        this.respond(message.id, {});
      }
    }
  }

  private respond(id: number, result: unknown): void {
    queueMicrotask(() => this.stdout.write(`${JSON.stringify({ id, result })}\n`));
  }
}

function harness(authUrl?: string) {
  const child = new FakeAppServer(authUrl);
  const openExternal = vi.fn(async () => undefined);
  const spawnProcess = vi.fn(() => child as unknown as ChildProcessWithoutNullStreams);
  const client = new ChatGPTAccountClient({
    userData: '/tmp/powermove-account-test',
    codexBinaryPref: () => null,
    openExternal
  }, {
    discoverBinary: async () => '/fake/codex',
    prepareHome: async () => '/tmp/powermove-account-test/codex-runtime',
    spawnProcess,
    requestTimeoutMs: 500,
    loginTimeoutMs: 5_000
  });
  return { child, client, openExternal, spawnProcess };
}

describe('ChatGPTAccountClient', () => {
  it('initializes App Server before reading the ChatGPT account', async () => {
    const { child, client, spawnProcess } = harness();
    await expect(client.status()).resolves.toMatchObject({ state: 'disconnected' });
    expect(spawnProcess).toHaveBeenCalledWith('/fake/codex', ['app-server', '--stdio'], expect.objectContaining({
      env: expect.objectContaining({ CODEX_HOME: '/tmp/powermove-account-test/codex-runtime' })
    }));
    expect(child.messages.map((message) => message.method)).toEqual([
      'initialize', 'initialized', 'account/read'
    ]);
    await client.shutdown();
  });

  it('opens the trusted browser flow and publishes the completed account', async () => {
    const { child, client, openExternal } = harness();
    const changed = vi.fn();
    client.onChanged(changed);

    await expect(client.connect()).resolves.toMatchObject({ state: 'connecting' });
    expect(openExternal).toHaveBeenCalledExactlyOnceWith('https://chatgpt.com/auth/powermove');

    child.account = { type: 'chatgpt', email: 'editor@example.com', planType: 'plus' };
    child.notify('account/login/completed', { loginId: 'login-123', success: true, error: null });
    await vi.waitFor(() => {
      expect(changed).toHaveBeenCalledWith(expect.objectContaining({
        state: 'connected', email: 'editor@example.com', planType: 'plus'
      }));
    });
    await client.shutdown();
  });

  it('never opens a non-OpenAI authentication URL', async () => {
    const { child, client, openExternal } = harness('https://example.com/phishing');
    await expect(client.connect()).resolves.toMatchObject({
      state: 'disconnected',
      detail: expect.stringContaining('unsafe')
    });
    expect(openExternal).not.toHaveBeenCalled();
    expect(child.messages.map((message) => message.method)).toContain('account/login/cancel');
    await client.shutdown();
  });

  it('disconnects only the account owned by Powermove App Server', async () => {
    const { child, client } = harness();
    child.account = { type: 'chatgpt', email: 'editor@example.com', planType: 'plus' };
    await expect(client.status()).resolves.toMatchObject({ state: 'connected' });

    await expect(client.disconnect()).resolves.toEqual({
      state: 'disconnected', email: null, planType: null, detail: null
    });
    expect(child.messages.map((message) => message.method)).toContain('account/logout');
    await expect(client.status()).resolves.toMatchObject({ state: 'disconnected' });
    await client.shutdown();
  });
});

describe('App Server account guards', () => {
  it('accepts only HTTPS OpenAI and ChatGPT login hosts', () => {
    expect(isAllowedChatGPTAuthUrl('https://auth.openai.com/codex')).toBe(true);
    expect(isAllowedChatGPTAuthUrl('https://chatgpt.com/auth')).toBe(true);
    expect(isAllowedChatGPTAuthUrl('http://chatgpt.com/auth')).toBe(false);
    expect(isAllowedChatGPTAuthUrl('https://chatgpt.com.example.org/auth')).toBe(false);
  });

  it('treats API-key and missing accounts as disconnected', () => {
    expect(chatGPTAccountStatusFromAppServer({ account: null })).toMatchObject({ state: 'disconnected' });
    expect(chatGPTAccountStatusFromAppServer({ account: { type: 'apiKey' } })).toMatchObject({ state: 'disconnected' });
  });
});
