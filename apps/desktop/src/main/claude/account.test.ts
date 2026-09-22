import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';

import { describe, expect, it, vi } from 'vitest';

import { ClaudeAccountClient, claudeAccountStatusFromJson, claudeModelsFromSdk } from './account';

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
  it('includes resolved Claude model IDs from SDK aliases', () => {
    expect(claudeModelsFromSdk([
      { value: 'opus', resolvedModel: 'claude-opus-5-5', displayName: 'Opus', description: '', supportedEffortLevels: ['low', 'high', 'max'] },
      { value: 'haiku', resolvedModel: 'claude-haiku-4-5-20251001', displayName: 'Haiku', description: '' },
    ])).toEqual([
      { id: 'opus', label: 'Opus', reasoningEfforts: ['low', 'high', 'max'] },
      { id: 'claude-opus-5-5', label: 'claude-opus-5-5', reasoningEfforts: ['low', 'high', 'max'] },
      { id: 'haiku', label: 'Haiku', reasoningEfforts: [] },
      { id: 'claude-haiku-4-5-20251001', label: 'claude-haiku-4-5-20251001', reasoningEfforts: [] },
    ]);
  });
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

  it('shares one pending status check across simultaneous requests', async () => {
    const statusChild = waitingChild();
    const spawnProcess = vi.fn(() => statusChild as never);
    const client = new ClaudeAccountClient({ userData: '/tmp/claude-account', claudeBinaryPref: () => null }, {
      discoverBinary: async () => '/bin/claude', prepareHome: async () => '/tmp/claude-account', spawnProcess
    });
    const first = client.status();
    const second = client.status();
    await vi.waitFor(() => expect(spawnProcess).toHaveBeenCalled());
    const spawnCount = spawnProcess.mock.calls.length;
    statusChild.stdout.end('{"loggedIn":false}');
    statusChild.emit('close', 1, null);
    await expect(first).resolves.toMatchObject({ state: 'disconnected' });
    await expect(second).resolves.toMatchObject({ state: 'disconnected' });
    expect(spawnCount).toBe(1);
    await client.shutdown();
  });

  it('keeps the event loop available while a real status process is still running', async () => {
    const { mkdtemp, writeFile, rm } = await import('node:fs/promises');
    const { tmpdir } = await import('node:os');
    const path = await import('node:path');
    const directory = await mkdtemp(path.join(tmpdir(), 'pm-claude-slow-check-'));
    const binary = path.join(directory, 'slow claude');
    await writeFile(binary, `#!/bin/sh
sleep 0.2
printf '%s\\n' '{"loggedIn":false}'
`, { mode: 0o755 });
    const client = new ClaudeAccountClient({ userData: directory, claudeBinaryPref: () => null }, {
      discoverBinary: async () => binary, prepareHome: async () => directory
    });
    let completed = false;
    try {
      const status = client.status().then(value => { completed = true; return value; });
      await new Promise(resolve => setTimeout(resolve, 20));
      expect(completed).toBe(false);
      await expect(status).resolves.toMatchObject({ state: 'disconnected' });
    } finally { await client.shutdown(); await rm(directory, { recursive: true, force: true }); }
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
