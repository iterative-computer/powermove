import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';

import { describe, expect, it, vi } from 'vitest';

import { ClaudeAccountClient, claudeAccountStatusFromJson } from './account';

function childThatCloses(stdoutText: string, code: number): EventEmitter & {
  stdout: PassThrough; stderr: PassThrough; killed: boolean; kill: ReturnType<typeof vi.fn>;
} {
  const child = new EventEmitter() as EventEmitter & {
    stdout: PassThrough; stderr: PassThrough; killed: boolean; kill: ReturnType<typeof vi.fn>;
  };
  child.stdout = new PassThrough();
  child.stderr = new PassThrough();
  child.killed = false;
  child.kill = vi.fn(() => { child.killed = true; return true; });
  queueMicrotask(() => {
    child.stdout.end(stdoutText);
    child.emit('close', code, null);
  });
  return child;
}

function waitingChild(): EventEmitter & {
  stdout: PassThrough; stderr: PassThrough; killed: boolean; kill: ReturnType<typeof vi.fn>;
} {
  const child = new EventEmitter() as EventEmitter & {
    stdout: PassThrough; stderr: PassThrough; killed: boolean; kill: ReturnType<typeof vi.fn>;
  };
  child.stdout = new PassThrough();
  child.stderr = new PassThrough();
  child.killed = false;
  child.kill = vi.fn(() => { child.killed = true; return true; });
  return child;
}

describe('Claude account client', () => {
  it('maps only Claude Code status metadata into the renderer contract', () => {
    expect(claudeAccountStatusFromJson({
      loggedIn: true,
      email: 'editor@example.com',
      subscriptionType: 'pro',
      authMethod: 'claude.ai'
    })).toEqual({
      state: 'connected', email: 'editor@example.com', planType: 'pro', detail: null
    });
    expect(claudeAccountStatusFromJson({ loggedIn: false, token: 'must-not-cross' })).toEqual({
      state: 'disconnected', email: null, planType: null, detail: null
    });
  });

  it('starts the official subscription login in Powermove private config and can stop it', async () => {
    const login = waitingChild();
    const spawnProcess = vi.fn((_binary: string, args: readonly string[]) => {
      if (args[0] === 'auth' && args[1] === 'status') {
        return childThatCloses('{"loggedIn":false,"authMethod":"none"}\n', 1) as never;
      }
      return login as never;
    });
    const client = new ClaudeAccountClient({
      userData: '/tmp/powermove-claude-account',
      claudeBinaryPref: () => null
    }, {
      discoverBinary: async () => '/bin/claude',
      prepareHome: async () => '/tmp/powermove-claude-account/claude-runtime',
      spawnProcess,
      loginTimeoutMs: 60_000
    });

    await expect(client.connect()).resolves.toMatchObject({ state: 'connecting' });
    expect(spawnProcess).toHaveBeenLastCalledWith(
      '/bin/claude',
      ['auth', 'login', '--claudeai'],
      expect.objectContaining({
        env: expect.objectContaining({ CLAUDE_CONFIG_DIR: '/tmp/powermove-claude-account/claude-runtime' }),
        stdio: ['ignore', 'pipe', 'pipe']
      })
    );
    await client.shutdown();
    expect(login.kill).toHaveBeenCalledWith('SIGTERM');
  });
});
