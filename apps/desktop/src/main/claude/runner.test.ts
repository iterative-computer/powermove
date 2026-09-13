import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { agentWorkspaceRoot, sessionPathFor } from '../codex/workspace';

import { describe, expect, it, vi } from 'vitest';

import type { CodexRunRequest } from '../../shared/ipc';
import { ClaudeRunner } from './runner';

function request(overrides: Partial<CodexRunRequest> = {}): CodexRunRequest {
  return {
    id: 'claude-run-1234',
    provider: 'claude',
    mode: 'editor',
    prompt: 'Return a greeting',
    schema: { type: 'object', required: ['message'], properties: { message: { type: 'string' } } },
    images: [],
    model: 'sonnet',
    reasoningEffort: 'high',
    access: 'editor',
    projectId: 'editor',
    projectName: 'Editor',
    projectJSON: null,
    attachments: [],
    consentToken: null,
    ...overrides
  };
}

function successfulChild(): EventEmitter & { stdout: PassThrough; stderr: PassThrough; pid: number; killed: boolean; kill: ReturnType<typeof vi.fn> } {
  const child = new EventEmitter() as EventEmitter & {
    stdout: PassThrough; stderr: PassThrough; pid: number; killed: boolean; kill: ReturnType<typeof vi.fn>;
  };
  child.stdout = new PassThrough();
  child.stderr = new PassThrough();
  child.pid = 919191;
  child.killed = false;
  child.kill = vi.fn(() => { child.killed = true; return true; });
  queueMicrotask(() => {
    child.stdout.write(`${JSON.stringify({
      type: 'result', subtype: 'success', is_error: false,
      result: '{"message":"hello"}', structured_output: { message: 'hello' }
    })}\n`);
    child.stdout.end();
    child.emit('close', 0, null);
  });
  return child;
}

describe('Claude runner', () => {
  it('retains the native session and workspace checkpoint when cancelled', async () => {
    const userData = await mkdtemp(path.join(tmpdir(), 'claude-cancel-test-'));
    try {
      const runner = new ClaudeRunner();
      const req = request({ mode: 'autonomous', access: 'project', projectJSON: '{}', threadId: 'thread-cancel' });
      const sessionPath = sessionPathFor(agentWorkspaceRoot(userData, req.projectId), 'project', req.threadId, 'claude');
      const pending = runner.run(req, {
        userData, extensionsDir: path.join(userData, 'extensions'), apiPackFiles: async () => [],
        binary: path.join(__dirname, '__fixtures__', 'fake-claude.sh'), timeoutMs: 5000,
        spawnProcess: (cmd, args, options) => spawn(cmd, args, { ...options, env: { ...options.env, FAKE_CLAUDE_MODE: 'hang' } }),
      });
      await vi.waitFor(async () => expect((await readFile(sessionPath, 'utf8')).trim()).toBe('11111111-1111-4111-8111-111111111111'));
      await runner.cancel(req.id);
      expect(await pending).toMatchObject({ ok: false, cancelled: true });
      expect((await readFile(sessionPath, 'utf8')).trim()).toBe('11111111-1111-4111-8111-111111111111');
      expect(JSON.parse(await readFile(`${sessionPath}.checkpoint.json`, 'utf8')).projectId).toBe(req.projectId);
    } finally { await rm(userData, { recursive: true, force: true }); }
  });
  it('runs the official CLI in structured stream mode and returns schema output', async () => {
    const spawnProcess = vi.fn(() => successfulChild() as never);
    const runner = new ClaudeRunner();
    const nativeTools = {
      command: '/test/electron',
      args: ['/test/mcp-server.mjs'],
      env: { POWERMOVE_AGENT_TOOL_TOKEN: 'secret' }
    };
    const result = await runner.run(request(), {
      userData: '/tmp/powermove-claude-runner',
      extensionsDir: '/tmp/powermove-extensions',
      apiPackFiles: async () => [],
      binary: '/bin/claude',
      spawnProcess,
      nativeTools
    });

    expect(result).toEqual({ ok: true, text: '{"message":"hello"}', access: 'editor' });
    expect(spawnProcess).toHaveBeenCalledWith(
      '/bin/claude',
      expect.arrayContaining(['--print', '--output-format', 'stream-json', '--safe-mode']),
      expect.objectContaining({ detached: true, stdio: ['ignore', 'pipe', 'pipe'] })
    );
    const argv = (spawnProcess.mock.calls[0] as unknown as [string, string[]])[1];
    expect(JSON.parse(argv[argv.indexOf('--mcp-config') + 1]!)).toEqual({
      mcpServers: { powermove: { type: 'stdio', ...nativeTools } }
    });
    expect(argv[argv.indexOf('--allowedTools') + 1]).toContain('mcp__powermove__capture_panel');
    const options = (spawnProcess.mock.calls[0] as unknown as [string, string[], { env: NodeJS.ProcessEnv }])[2];
    expect(options.env.CLAUDE_CONFIG_DIR).toBe('/tmp/powermove-claude-runner/claude-runtime');
  });

  it('completes against the executable CLI fixture through real child-process pipes', async () => {
    const runner = new ClaudeRunner();
    const activity: Array<{ type: 'trace' | 'progress'; value: unknown }> = [];
    const result = await runner.run(request({ id: 'claude-real-pipes' }), {
      userData: '/tmp/powermove-claude-real-pipes',
      extensionsDir: '/tmp/powermove-extensions',
      apiPackFiles: async () => [],
      binary: path.join(__dirname, '__fixtures__', 'fake-claude.sh'),
      timeoutMs: 5_000,
      onTrace: (step) => activity.push({ type: 'trace', value: step }),
      onProgress: (text) => activity.push({ type: 'progress', value: text })
    });
    expect(result).toEqual({ ok: true, text: '{"message":"claude editor done"}', access: 'editor' });
    const traces = activity.filter((event) => event.type === 'trace').map((event) => event.value);
    expect(traces).toContainEqual({ kind: 'tool-start', itemId: 'tool-read', toolName: 'read', label: 'Read' });
    expect(activity.findIndex((event) => event.type === 'trace' && (event.value as { kind?: string }).kind === 'tool-start'))
      .toBeLessThan(activity.findIndex((event) => event.type === 'progress' && event.value === 'Preparing the Claude result'));
    expect(traces.filter((event) => (event as { kind?: string }).kind === 'answer')).toEqual([
      { kind: 'answer', text: 'Preparing ' },
      { kind: 'answer', text: 'the Claude result' }
    ]);
  });
});
