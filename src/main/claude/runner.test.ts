import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import path from 'node:path';

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
  it('runs the official CLI in structured stream mode and returns schema output', async () => {
    const spawnProcess = vi.fn(() => successfulChild() as never);
    const runner = new ClaudeRunner();
    const result = await runner.run(request(), {
      userData: '/tmp/powermove-claude-runner',
      extensionsDir: '/tmp/powermove-extensions',
      apiPackFiles: async () => [],
      binary: '/bin/claude',
      spawnProcess
    });

    expect(result).toEqual({ ok: true, text: '{"message":"hello"}', access: 'editor' });
    expect(spawnProcess).toHaveBeenCalledWith(
      '/bin/claude',
      expect.arrayContaining(['--print', '--output-format', 'stream-json', '--safe-mode']),
      expect.objectContaining({ detached: true, stdio: ['ignore', 'pipe', 'pipe'] })
    );
    const options = (spawnProcess.mock.calls[0] as unknown as [string, string[], { env: NodeJS.ProcessEnv }])[2];
    expect(options.env.CLAUDE_CONFIG_DIR).toBe('/tmp/powermove-claude-runner/claude-runtime');
  });

  it('completes against the executable CLI fixture through real child-process pipes', async () => {
    const runner = new ClaudeRunner();
    const result = await runner.run(request({ id: 'claude-real-pipes' }), {
      userData: '/tmp/powermove-claude-real-pipes',
      extensionsDir: '/tmp/powermove-extensions',
      apiPackFiles: async () => [],
      binary: path.join(__dirname, '__fixtures__', 'fake-claude.sh'),
      timeoutMs: 5_000
    });
    expect(result).toEqual({ ok: true, text: '{"message":"claude editor done"}', access: 'editor' });
  });
});
