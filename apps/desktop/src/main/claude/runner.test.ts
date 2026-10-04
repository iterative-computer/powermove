vi.mock('../agent-tools/user-resources', () => ({ prepareUserResources: async () => undefined }));

// Keep runner tests independent of the developer's configured external services.
vi.mock('../agent-tools/user-mcp', async importOriginal => ({
  ...await importOriginal<typeof import('../agent-tools/user-mcp')>(),
  loadUserMcpServers: vi.fn(async () => ({}))
}));

import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { mkdirSync, writeFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { agentWorkspaceRoot, sessionPathFor } from '../codex/workspace';

import { describe, expect, it, vi } from 'vitest';

import type { CodexRunRequest } from '../../shared/ipc';
import { ClaudeRunner } from './runner';
import type { RequestApproval } from '../agent-approvals';

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

type FakeChild = EventEmitter & {
  stdin: PassThrough; stdout: PassThrough; stderr: PassThrough; pid: number; killed: boolean; kill: ReturnType<typeof vi.fn>;
  /** Everything the runner wrote to stdin. */
  written(): string;
};

function fakeChild(): FakeChild {
  const child = new EventEmitter() as FakeChild;
  let input = '';
  child.stdin = new PassThrough();
  child.stdin.on('data', (chunk: Buffer) => { input += chunk.toString('utf8'); });
  child.written = () => input;
  child.stdout = new PassThrough();
  child.stderr = new PassThrough();
  child.pid = 919191;
  child.killed = false;
  child.kill = vi.fn(() => { child.killed = true; return true; });
  return child;
}

function successfulChild(output: unknown = { message: 'hello' }, onChild?: (child: FakeChild) => void): FakeChild {
  const child = fakeChild();
  onChild?.(child);
  child.stdout = new PassThrough();
  child.stderr = new PassThrough();
  child.pid = 919191;
  child.killed = false;
  child.kill = vi.fn(() => { child.killed = true; return true; });
  queueMicrotask(() => {
    child.stdout.write(`${JSON.stringify({ type: 'system', subtype: 'init', session_id: '11111111-1111-4111-8111-111111111111' })}\n`);
    child.stdout.write(`${JSON.stringify({
      type: 'result', subtype: 'success', is_error: false,
      result: JSON.stringify(output), structured_output: output
    })}\n`);
    child.stdout.end();
    child.emit('close', 0, null);
  });
  return child;
}

describe('Claude runner', () => {
  it('repairs broken mod source before publishing and resumes the same staged run', async () => {
    const userData = await mkdtemp(path.join(tmpdir(), 'claude-mod-repair-'));
    try {
      const stages: string[] = [];
      const prompts: string[][] = [];
      const inputs: Array<() => string> = [];
      const result = await new ClaudeRunner().run(request({ mode: 'autonomous', access: 'project', projectJSON: '{}', threadId: 'mod-repair' }), {
        userData, extensionsDir: path.join(userData, 'extensions'), apiPackFiles: async () => [], binary: '/fake/claude',
        spawnProcess: (_binary, args) => {
          prompts.push([...args]);
          const stage = args[args.indexOf('--add-dir') + 1]!;
          stages.push(stage);
          const dir = path.join(stage, 'anchor-presets');
          mkdirSync(dir, { recursive: true });
          writeFileSync(path.join(dir, 'manifest.json'), JSON.stringify({ id: 'anchor-presets', name: 'Anchors', version: '1.0.0', apiVersion: 1, entry: 'index.ts' }));
          writeFileSync(path.join(dir, 'index.ts'), stages.length === 1 ? 'export const = ;' : 'export default function activate() {}');
          expect(existsSync(path.join(userData, 'extensions', 'anchor-presets'))).toBe(false);
          return successfulChild({ summary: 'Added anchors', commands: [], artifacts: [], externalActions: [], notes: [], extensions: [{ id: 'anchor-presets', action: 'created' }] },
            child => inputs.push(child.written)) as never;
        }
      });
      expect(result).toMatchObject({ ok: true, extensions: [{ id: 'anchor-presets', action: 'created' }] });
      expect(stages).toHaveLength(2);
      expect(stages[0]).toBe(stages[1]);
      expect(prompts[1]).toContain('--resume');
      expect(inputs[1]!()).toContain('failed compilation');
      expect(await readFile(path.join(userData, 'extensions', 'anchor-presets', 'index.ts'), 'utf8')).toContain('activate');
    } finally { await rm(userData, { recursive: true, force: true }); }
  });

  it.each(['editor', 'project'] as const)('denies no reads in %s runs: no credential or transcript paths reach the settings', async access => {
    const userData = await mkdtemp(path.join(tmpdir(), 'claude-no-denied-reads-'));
    try {
      const req = access === 'editor'
        ? request({ mode: 'editor', access: 'editor', projectJSON: null })
        : request({ mode: 'autonomous', access: 'project', projectJSON: '{}', threadId: 'no-denied-reads' });
      const argvs: string[][] = [];
      const result = await new ClaudeRunner().run(req, {
        userData, extensionsDir: path.join(userData, 'extensions'), apiPackFiles: async () => [], binary: '/fake/claude',
        spawnProcess: (_binary, args) => {
          argvs.push([...args]);
          return successfulChild(access === 'editor' ? { message: 'Done' } : { summary: 'Done', commands: [], artifacts: [], externalActions: [], notes: [] }) as never;
        }
      });
      expect(result).toMatchObject({ ok: true });
      const settings = JSON.parse(argvs[0]![argvs[0]!.indexOf('--settings') + 1]!);
      expect(settings.permissions).toBeUndefined();
      expect(settings.sandbox.filesystem).toBeUndefined();
      expect(JSON.stringify(settings)).not.toMatch(/credentials|\.ssh|history\.jsonl|denyRead/);
    } finally { await rm(userData, { recursive: true, force: true }); }
  });

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
      expect.arrayContaining(['--print', '--output-format', 'stream-json', '--tools', 'default']),
      expect.objectContaining({ detached: true, stdio: ['pipe', 'pipe', 'pipe'] })
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

  it('holds AskUserQuestion open for the person and denies every other permission prompt', async () => {
    const runner = new ClaudeRunner();
    const traces: unknown[] = [];
    const child = fakeChild();
    const line = (value: unknown) => child.stdout.write(`${JSON.stringify(value)}\n`);
    const result = runner.run(request({ id: 'claude-question' }), {
      userData: '/tmp/powermove-claude-question', extensionsDir: '/tmp/powermove-extensions', apiPackFiles: async () => [],
      binary: '/bin/claude', spawnProcess: () => child as never, onTrace: (step) => traces.push(step)
    });
    await vi.waitFor(() => expect(child.written()).toContain('Return a greeting'));
    const question = {
      question: 'Replace the Store title or add a group?', header: 'Promo', multiSelect: false,
      options: [{ label: 'Update the existing title', description: 'Edit in place' }, { label: 'Add a separate group', description: '' }]
    };
    line({ type: 'stream_event', event: { type: 'message_start', message: { id: 'm1' } } });
    line({ type: 'stream_event', event: { type: 'content_block_start', index: 0, content_block: { type: 'tool_use', id: 'toolu_q', name: 'AskUserQuestion' } } });
    line({ type: 'stream_event', event: { type: 'content_block_stop', index: 0 } });
    line({ type: 'control_request', request_id: 'perm-bash', request: { subtype: 'can_use_tool', tool_name: 'Bash', input: { command: 'ls' }, tool_use_id: 'toolu_b' } });
    line({ type: 'control_request', request_id: 'perm-q', request: {
      subtype: 'can_use_tool', tool_name: 'AskUserQuestion', tool_use_id: 'toolu_q', input: { questions: [question] }
    } });
    await vi.waitFor(() => expect(traces).toContainEqual({
      kind: 'question', itemId: 'toolu_q', transport: 'reply', blocking: true,
      questions: [{ id: 'toolu_q-0', header: 'Promo', question: question.question, allowOther: true, secret: false, options: question.options }]
    }));
    // The question is a card, never a tool row.
    expect(traces.some((step) => (step as { itemId?: string }).itemId === 'toolu_q' && (step as { kind: string }).kind !== 'question')).toBe(false);
    const responses = () => child.written().split('\n').filter(Boolean).map((text) => JSON.parse(text)).filter((message) => message.type === 'control_response');
    expect(responses()).toEqual([{ type: 'control_response', response: { subtype: 'success', request_id: 'perm-bash',
      response: { behavior: 'deny', message: 'Powermove does not allow Bash in this run.' } } }]);

    expect(runner.answer({ id: 'claude-question', itemId: 'toolu_q', answers: { 'toolu_q-0': ['Add a separate group'] } })).toBe(true);
    expect(runner.answer({ id: 'claude-question', itemId: 'toolu_q', answers: {} })).toBe(false);
    await vi.waitFor(() => expect(responses()).toHaveLength(2));
    expect(responses()[1]).toEqual({ type: 'control_response', response: { subtype: 'success', request_id: 'perm-q', response: {
      behavior: 'allow', updatedInput: { questions: [question], answers: { [question.question]: 'Add a separate group' } }
    } } });

    line({ type: 'result', subtype: 'success', is_error: false, result: '{"message":"done"}', structured_output: { message: 'done' } });
    await vi.waitFor(() => expect(child.stdin.writableEnded).toBe(true));
    child.stdout.end();
    child.emit('close', 0, null);
    await expect(result).resolves.toEqual({ ok: true, text: '{"message":"done"}', access: 'editor' });
  });

  it('asks the person before a Project run leaves its sandbox, and passes their answer to the CLI', async () => {
    const userData = await mkdtemp(path.join(tmpdir(), 'claude-approval-'));
    try {
      const runner = new ClaudeRunner();
      const child = fakeChild();
      const line = (value: unknown) => child.stdout.write(`${JSON.stringify(value)}\n`);
      const argvs: string[][] = [];
      const requestApproval = vi.fn<RequestApproval>()
        .mockResolvedValueOnce({ allowed: true })
        .mockResolvedValueOnce({ allowed: false, message: 'The user declined.' });
      const result = runner.run(request({ id: 'claude-approval', mode: 'autonomous', access: 'project', projectJSON: '{}', threadId: 'approval' }), {
        userData, extensionsDir: path.join(userData, 'extensions'), apiPackFiles: async () => [], binary: '/fake/claude',
        requestApproval, spawnProcess: (_binary, args) => { argvs.push([...args]); return child as never; }
      });
      await vi.waitFor(() => expect(argvs).toHaveLength(1));
      expect(argvs[0]![argvs[0]!.indexOf('--allowedTools') + 1]!.split(',')).not.toContain('Bash');
      const bash = { command: 'cp DMSans.ttf ~/Library/Fonts/', description: 'Install DM Sans', dangerouslyDisableSandbox: true };
      line({ type: 'control_request', request_id: 'perm-bash', request: { subtype: 'can_use_tool', tool_name: 'Bash', input: bash, tool_use_id: 'toolu_b' } });
      line({ type: 'control_request', request_id: 'perm-write', request: { subtype: 'can_use_tool', tool_name: 'Write', input: { file_path: '/Users/me/notes.txt', content: 'x' }, tool_use_id: 'toolu_w' } });
      const responses = () => child.written().split('\n').filter(Boolean).map((text) => JSON.parse(text)).filter((message) => message.type === 'control_response');
      await vi.waitFor(() => expect(responses()).toHaveLength(2));
      expect(requestApproval.mock.calls.map(([asked]) => asked)).toEqual([
        { title: 'Run a command outside the project sandbox?', detail: bash.command, reason: 'Install DM Sans' },
        { title: 'Allow Write to change a file outside the project?', detail: '/Users/me/notes.txt' }
      ]);
      expect(responses()).toEqual([
        { type: 'control_response', response: { subtype: 'success', request_id: 'perm-bash', response: { behavior: 'allow', updatedInput: bash } } },
        { type: 'control_response', response: { subtype: 'success', request_id: 'perm-write', response: { behavior: 'deny', message: 'The user declined.' } } }
      ]);
      line({ type: 'result', subtype: 'success', is_error: false, result: '{}', structured_output: { summary: 'Done', commands: [], artifacts: [], externalActions: [], notes: [] } });
      child.stdout.end();
      child.emit('close', 0, null);
      await expect(result).resolves.toMatchObject({ ok: true });
    } finally { await rm(userData, { recursive: true, force: true }); }
  });

  it('never asks from an editor run, even when approvals are available', async () => {
    const runner = new ClaudeRunner();
    const child = fakeChild();
    const requestApproval = vi.fn<RequestApproval>();
    const result = runner.run(request({ id: 'claude-editor-approval' }), {
      userData: '/tmp/powermove-claude-editor-approval', extensionsDir: '/tmp/powermove-extensions', apiPackFiles: async () => [],
      binary: '/bin/claude', spawnProcess: () => child as never, requestApproval
    });
    await vi.waitFor(() => expect(child.written()).toContain('Return a greeting'));
    child.stdout.write(`${JSON.stringify({ type: 'control_request', request_id: 'perm-bash', request: { subtype: 'can_use_tool', tool_name: 'Bash', input: { command: 'ls' }, tool_use_id: 'toolu_b' } })}\n`);
    await vi.waitFor(() => expect(child.written()).toContain('Powermove does not allow Bash in this run.'));
    expect(requestApproval).not.toHaveBeenCalled();
    child.stdout.write(`${JSON.stringify({ type: 'result', subtype: 'success', is_error: false, result: '{"message":"ok"}', structured_output: { message: 'ok' } })}\n`);
    child.stdout.end();
    child.emit('close', 0, null);
    await expect(result).resolves.toMatchObject({ ok: true });
  });

  it('declines a skipped question and closes one the CLI withdraws', async () => {
    const runner = new ClaudeRunner();
    const traces: unknown[] = [];
    const child = fakeChild();
    const line = (value: unknown) => child.stdout.write(`${JSON.stringify(value)}\n`);
    const result = runner.run(request({ id: 'claude-skip' }), {
      userData: '/tmp/powermove-claude-skip', extensionsDir: '/tmp/powermove-extensions', apiPackFiles: async () => [],
      binary: '/bin/claude', spawnProcess: () => child as never, onTrace: (step) => traces.push(step)
    });
    const ask = (id: string) => line({ type: 'control_request', request_id: `perm-${id}`, request: {
      subtype: 'can_use_tool', tool_name: 'AskUserQuestion', tool_use_id: id, input: { questions: [{ question: 'Which?', options: [] }] }
    } });
    ask('toolu_a');
    ask('toolu_b');
    await vi.waitFor(() => expect(traces.filter((step) => (step as { kind: string }).kind === 'question')).toHaveLength(2));
    expect(runner.answer({ id: 'claude-skip', itemId: 'toolu_a', answers: { 'toolu_a-0': ['  '] } })).toBe(true);
    await vi.waitFor(() => expect(child.written()).toContain('"behavior":"deny","message":"The user skipped the question.'));
    line({ type: 'control_cancel_request', request_id: 'perm-toolu_b' });
    await vi.waitFor(() => expect(traces).toContainEqual({ kind: 'question-closed', itemId: 'toolu_b' }));
    expect(runner.answer({ id: 'claude-skip', itemId: 'toolu_b', answers: { 'toolu_b-0': ['x'] } })).toBe(false);
    line({ type: 'result', subtype: 'success', is_error: false, result: '{"message":"ok"}', structured_output: { message: 'ok' } });
    child.stdout.end();
    child.emit('close', 0, null);
    await expect(result).resolves.toMatchObject({ ok: true });
  });
});
