import { spawn } from 'node:child_process';
import { chmod, mkdtemp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { CodexRunRequest } from '../../shared/ipc';
import {
  codexErrorFromStdout,
  CodexRunner,
  isCodexRunRequest,
  parseAgentExtensionChanges,
  type CodexRunOptions
} from './runner';
import { isolatedCodexHome } from './isolation';
import { agentWorkspaceRoot, sessionPathFor } from './workspace';

const fakeCodex = path.join(__dirname, '__fixtures__', 'fake-codex.sh');
const temporaryDirectories: string[] = [];

beforeAll(async () => chmod(fakeCodex, 0o755));

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((directory) =>
    rm(directory, { recursive: true, force: true })));
  vi.restoreAllMocks();
});

async function temporaryDirectory(label: string): Promise<string> {
  const directory = await mkdtemp(path.join(tmpdir(), `${label}-`));
  temporaryDirectories.push(directory);
  return directory;
}

function request(overrides: Partial<CodexRunRequest> = {}): CodexRunRequest {
  return {
    id: 'runner-request-1234',
    mode: 'autonomous',
    prompt: 'Create the requested deliverable.',
    schema: null,
    images: [],
    model: 'gpt-5.6-sol',
    reasoningEffort: 'high',
    access: 'project',
    projectId: 'runner-project',
    projectName: 'Runner Project',
    projectJSON: '{"layers":[]}',
    attachments: [],
    consentToken: null,
    ...overrides
  };
}

it('validates optional thread ids without accepting paths or unbounded input', () => {
  expect(isCodexRunRequest(request())).toBe(true);
  expect(isCodexRunRequest(request({threadId:'thread-123'}))).toBe(true);
  for (const threadId of ['', '../outside', 'a/b', 'x'.repeat(121), 12]) {
    expect(isCodexRunRequest({...request(),threadId})).toBe(false);
  }
});

function fakeOptions(userData: string, environment: Record<string, string>): CodexRunOptions {
  return {
    userData,
    extensionsDir: path.join(userData, 'extensions'),
    apiPackFiles: async () => [
      { name: 'EXTENSIONS.md', text: '# Extensions' },
      { name: 'api.ts', text: 'export interface PowermoveAPI {}' },
      { name: 'extensions.ts', text: 'export const EXTENSION_ID = /x/;' },
      { name: 'project.ts', text: 'export interface Project {}' },
      { name: 'commands.ts', text: 'export type EditCommand = never;' }
    ],
    binary: fakeCodex,
    timeoutMs: 10_000,
    discoverDisabledSkillPaths: async () => [],
    spawnProcess: (command, args, options) => spawn(command, args, {
      ...options,
      env: { ...options.env, ...environment }
    })
  };
}

async function waitForFile(file: string): Promise<void> {
  for (let attempt = 0; attempt < 200; attempt += 1) {
    try {
      await readFile(file);
      return;
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
  }
  throw new Error(`Timed out waiting for ${file}`);
}

it('starts each thread fresh and resumes only the selected thread', async () => {
  const userData = await temporaryDirectory('powermove-threads-runner');
  const invocationFile = path.join(userData, 'invocations.txt');
  const runner = new CodexRunner();
  const options = fakeOptions(userData, { FAKE_CODEX_INVOCATIONS: invocationFile });
  for (const threadId of ['thread-a', 'thread-b', 'thread-a']) {
    expect((await runner.run(request({threadId}), options)).ok).toBe(true);
  }
  expect((await readFile(invocationFile, 'utf8')).trim().split('\n')).toEqual(['fresh', 'fresh', 'resume']);
});

describe('CodexRunner validation and authority', () => {
  it('drops invalid extension changes and caps the typed result at 32 items', () => {
    const valid = Array.from({ length: 35 }, (_, index) => ({
      id: `extension-${index}`,
      action: index % 2 === 0 ? 'created' : 'updated',
      summary: `Change ${index}`
    }));
    expect(parseAgentExtensionChanges([
      { id: '../escape', action: 'removed' },
      { id: 'valid-extension', action: 'invalid' },
      ...valid
    ])).toEqual(valid.slice(0, 32));
    expect(parseAgentExtensionChanges(null)).toBeUndefined();
  });

  it('returns validated extension changes separately while preserving result text', async () => {
    const result = await new CodexRunner().run(request({ id: 'extensions-run-1234' }), fakeOptions(
      await temporaryDirectory('runner-extensions'),
      {
        FAKE_CODEX_RESULT: JSON.stringify({
          summary: 'done',
          commands: [],
          artifacts: [],
          externalActions: [],
          notes: [],
          extensions: [
            { id: 'valid-extension', action: 'created', summary: 'Adds a command' },
            { id: '../invalid', action: 'removed' }
          ]
        })
      }
    ));

    expect(result).toMatchObject({
      ok: true,
      extensions: [{ id: 'valid-extension', action: 'created', summary: 'Adds a command' }]
    });
    if (!result.ok) throw new Error(result.error);
    expect(JSON.parse(result.text).extensions).toHaveLength(2);
  });

  it('validates the frozen request shape and collapses autonomous editor access to project', async () => {
    const req = request({ access: 'editor' });
    expect(isCodexRunRequest(req)).toBe(true);
    const result = await new CodexRunner().run(req, fakeOptions(await temporaryDirectory('runner-collapse'), {}));
    expect(result.ok && result.access).toBe('project');
  });

  it('consumes computer consent once before running', async () => {
    const userData = await temporaryDirectory('runner-consent');
    const consume = vi.fn(() => true);
    const req = request({ access: 'computer', consentToken: 'one-use-token' });
    const result = await new CodexRunner().run(req, {
      ...fakeOptions(userData, {}),
      consumeConsentToken: consume
    });
    expect(result.ok).toBe(true);
    expect(consume).toHaveBeenCalledOnce();

    const denied = await new CodexRunner().run(req, {
      ...fakeOptions(userData, {}),
      consumeConsentToken: () => false
    });
    expect(denied).toEqual({
      ok: false,
      error: 'Computer access requires fresh approval for this run.',
      cancelled: false
    });
  });
});

describe('CodexRunner lifecycle', () => {
  it('launches Codex with the app-owned isolated home', async () => {
    const userData = await temporaryDirectory('runner-isolation');
    let launchedHome: string | undefined;
    const options = fakeOptions(userData, {});
    options.spawnProcess = (command, args, spawnOptions) => {
      launchedHome = spawnOptions.env?.CODEX_HOME;
      return spawn(command, args, spawnOptions);
    };

    const result = await new CodexRunner().run(request({
      id: 'editor-isolation-1234',
      mode: 'editor',
      access: 'editor',
      projectJSON: null
    }), options);

    expect(result.ok).toBe(true);
    expect(launchedHome).toBe(isolatedCodexHome(userData));
  });

  it('forwards structured traces from editor-mode stdout', async () => {
    const trace: unknown[] = [];
    const result = await new CodexRunner().run(request({
      id: 'editor-trace-1234',
      mode: 'editor',
      access: 'editor',
      projectJSON: null
    }), {
      ...fakeOptions(await temporaryDirectory('runner-editor-trace'), {}),
      onTrace: (step) => trace.push(step)
    });

    expect(result.ok).toBe(true);
    expect(trace).toEqual([
      { kind: 'tool-start', itemId: '0', toolName: 'bash', label: 'bash · pwd' },
      { kind: 'tool-end', itemId: '0', isError: false },
      { kind: 'thought', text: 'Reviewing the café timeline' },
      { kind: 'answer', text: 'Preparing the final animation' }
    ]);
  });

  it('guards the cancel-before-spawn race', async () => {
    const userData = await temporaryDirectory('runner-before-spawn');
    const runner = new CodexRunner();
    const spawnProcess = vi.fn(() => { throw new Error('must not spawn'); });
    const pending = runner.run(request({
      id: 'editor-cancel-1234',
      mode: 'editor',
      access: 'editor',
      projectJSON: null
    }), {
      userData,
      extensionsDir: path.join(userData, 'extensions'),
      apiPackFiles: async () => [],
      binary: fakeCodex,
      spawnProcess
    });
    await runner.cancel('editor-cancel-1234');

    await expect(pending).resolves.toEqual({
      ok: false,
      error: 'The Codex run was cancelled.',
      cancelled: true
    });
    expect(spawnProcess).not.toHaveBeenCalled();
  });

  it('kills the detached process group and clears session plus partial artifacts', async () => {
    const userData = await temporaryDirectory('runner-mid-cancel');
    const pidFile = path.join(userData, 'fake.pid');
    const root = agentWorkspaceRoot(userData, 'runner-project');
    const sessionPath = sessionPathFor(root, 'project');
    await mkdir(path.dirname(sessionPath), { recursive: true });
    await writeFile(sessionPath, 'old-session');

    const runner = new CodexRunner();
    const pending = runner.run(request({ id: 'mid-cancel-1234' }), fakeOptions(userData, {
      FAKE_CODEX_MODE: 'hang',
      FAKE_CODEX_PID_FILE: pidFile
    }));
    await waitForFile(pidFile);
    const pid = Number((await readFile(pidFile, 'utf8')).trim());
    await runner.cancel('mid-cancel-1234');
    const result = await pending;

    expect(result).toEqual({ ok: false, error: 'The Codex run was cancelled.', cancelled: true });
    await expect(readFile(sessionPath, 'utf8')).rejects.toMatchObject({ code: 'ENOENT' });
    await expect(readdir(path.join(root, 'artifacts'))).resolves.toEqual([]);
    expect(() => process.kill(pid, 0)).toThrow();
  }, 15_000);

  it('settles a timeout and clears the collapsed authority session', async () => {
    const userData = await temporaryDirectory('runner-timeout');
    const root = agentWorkspaceRoot(userData, 'runner-project');
    const sessionPath = sessionPathFor(root, 'project');
    await mkdir(path.dirname(sessionPath), { recursive: true });
    await writeFile(sessionPath, 'editor-session');

    const result = await new CodexRunner().run(request({
      id: 'timeout-run-1234',
      mode: 'editor',
      access: 'editor',
      projectJSON: null
    }), {
      ...fakeOptions(userData, { FAKE_CODEX_MODE: 'hang' }),
      timeoutMs: 25
    });

    expect(result).toEqual({ ok: false, error: 'The Codex run was cancelled.', cancelled: true });
    await expect(readFile(sessionPath, 'utf8')).rejects.toMatchObject({ code: 'ENOENT' });
  });

  it('clears an unknown resumed session and retries exactly once fresh', async () => {
    const userData = await temporaryDirectory('runner-stale');
    const invocationFile = path.join(userData, 'invocations.txt');
    const root = agentWorkspaceRoot(userData, 'runner-project');
    const sessionPath = sessionPathFor(root, 'project');
    await mkdir(path.dirname(sessionPath), { recursive: true });
    await writeFile(sessionPath, 'stale-thread');

    const progress: string[] = [];
    const result = await new CodexRunner().run(request({ id: 'stale-run-1234' }), {
      ...fakeOptions(userData, {
        FAKE_CODEX_MODE: 'stale-resume',
        FAKE_CODEX_INVOCATIONS: invocationFile
      }),
      onProgress: (text) => progress.push(text)
    });

    expect(result.ok).toBe(true);
    expect((await readFile(invocationFile, 'utf8')).trim().split('\n')).toEqual(['resume', 'fresh']);
    expect(await readFile(sessionPath, 'utf8')).toBe('thread-recorded-1');
    expect(progress).toContain('Working with project files and shell tools…');
    if (!result.ok) throw new Error(result.error);
    const parsed = JSON.parse(result.text) as Record<string, unknown>;
    expect(parsed.projectId).toBe('runner-project');
    expect(parsed.access).toBe('project');
    expect(parsed.artifacts).toEqual([
      expect.objectContaining({
        path: expect.stringMatching(/^[^/]+\/deliverable\.txt$/),
        name: 'deliverable.txt',
        mime: 'text/plain',
        importToTimeline: true
      })
    ]);
  });

  it('isolates a resumed session from broken user MCP configuration on the first attempt', async () => {
    const userData = await temporaryDirectory('runner-mcp-fallback');
    const invocationFile = path.join(userData, 'invocations.txt');
    const root = agentWorkspaceRoot(userData, 'runner-project');
    const sessionPath = sessionPathFor(root, 'project');
    await mkdir(path.dirname(sessionPath), { recursive: true });
    await writeFile(sessionPath, 'session-with-broken-mcp');

    const progress: string[] = [];
    const result = await new CodexRunner().run(request({ id: 'mcp-fallback-1234' }), {
      ...fakeOptions(userData, {
        FAKE_CODEX_MODE: 'mcp-fallback',
        FAKE_CODEX_INVOCATIONS: invocationFile
      }),
      onProgress: (text) => progress.push(text)
    });

    expect(result.ok).toBe(true);
    expect((await readFile(invocationFile, 'utf8')).trim().split('\n')).toEqual(['resume']);
    expect(await readFile(sessionPath, 'utf8')).toBe('thread-recorded-1');
    expect(progress).not.toContain('One of your Codex integrations failed to start — retrying without integrations…');
  });

  it('reports a structured CLI error instead of an unrelated stderr notice', async () => {
    const result = await new CodexRunner().run(request({ id: 'structured-failure-1234' }), fakeOptions(
      await temporaryDirectory('runner-structured-failure'),
      { FAKE_CODEX_MODE: 'structured-failure' }
    ));

    expect(result).toEqual({
      ok: false,
      error: 'The agent failed: The structured failure is the real cause',
      cancelled: false
    });
  });

  it('surfaces a JSONL API failure and clears the thread recorded by the failed run', async () => {
    const userData = await temporaryDirectory('runner-jsonl-failure');
    const root = agentWorkspaceRoot(userData, 'runner-project');
    const sessionPath = sessionPathFor(root, 'project');

    const result = await new CodexRunner().run(
      request({ id: 'jsonl-failure-1234' }),
      fakeOptions(userData, { FAKE_CODEX_MODE: 'fail-after-thread' })
    );

    expect(result).toEqual({
      ok: false,
      error: "Powermove's agent response format was rejected. Restart Powermove and retry; if it persists, update the app.",
      cancelled: false
    });
    await expect(readFile(sessionPath, 'utf8')).rejects.toMatchObject({ code: 'ENOENT' });
  });
});

describe('humanizeCodexFailure', () => {
  it('extracts completed error items without treating normal tool output as a failure', () => {
    expect(codexErrorFromStdout([
      '{"type":"item.completed","item":{"type":"error","message":"The tool could not start"}}',
      '{"type":"item.completed","item":{"type":"command_execution","aggregated_output":"ordinary output"}}'
    ].join('\n'))).toBe('The tool could not start');
  });

  it('extracts failures from Codex JSONL, including nested JSON messages', () => {
    expect(codexErrorFromStdout([
      '{"type":"thread.started","thread_id":"thread-1"}',
      '{"type":"turn.failed","error":{"message":"{\\"error\\":{\\"message\\":\\"Invalid schema for response_format codex_output_schema\\"}}"}}'
    ].join('\n'))).toBe('Invalid schema for response_format codex_output_schema');
  });

  it('maps MCP auth failures to an actionable sentence', async () => {
    const { humanizeCodexFailure } = await import('./runner');
    const raw = '2026-08-26T22:35:35.620618Z ERROR rmcp::transport::worker: worker quit with fatal: Transport channel closed, when AuthRequired(AuthRequiredError { www_authenticate_header: "Bearer realm=\\"OAuth\\"" })';
    const text = humanizeCodexFailure(raw);
    expect(text).toContain('MCP server');
    expect(text).toContain('codex');
    expect(text).not.toContain('rmcp::');
    expect(text).not.toContain('www_authenticate');
  });

  it('maps login failures and missing binary', async () => {
    const { humanizeCodexFailure } = await import('./runner');
    expect(humanizeCodexFailure('Error: not logged in. Please run codex login.')).toContain('codex login');
    expect(humanizeCodexFailure('spawn codex ENOENT')).toContain('installed');
  });

  it('does not mistake an unrelated MCP credential warning for a Codex login failure', async () => {
    const { humanizeCodexFailure } = await import('./runner');
    const raw = [
      'Error: failed to initialize in-process app-server client: Operation not permitted (os error 1)',
      'WARN codex_mcp: OAuth refresh credentials for server motioner are missing an authorization server issuer'
    ].join('\n');
    const text = humanizeCodexFailure(raw);
    expect(text).toContain('restricted development environment');
    expect(text).toContain('reopen it normally');
    expect(text).not.toContain('codex login');
  });

  it('reduces unknown stderr to its last meaningful line without log noise', async () => {
    const { humanizeCodexFailure } = await import('./runner');
    const text = humanizeCodexFailure('2026-08-26T10:00:00Z WARN codex::something: first\n2026-08-26T10:00:01Z ERROR codex::other: everything exploded badly');
    expect(text).toBe('The agent failed: everything exploded badly');
  });

  it('falls back cleanly on empty diagnostics', async () => {
    const { humanizeCodexFailure } = await import('./runner');
    expect(humanizeCodexFailure('', 'The autonomous agent failed.')).toBe('The autonomous agent failed.');
  });
});
