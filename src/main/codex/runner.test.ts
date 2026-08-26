import { spawn } from 'node:child_process';
import { chmod, mkdtemp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { CodexRunRequest } from '../../shared/ipc';
import {
  CodexRunner,
  isCodexRunRequest,
  parseAgentExtensionChanges,
  type CodexRunOptions
} from './runner';
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
    spawnProcess: (command, args, options) => spawn(command, args, {
      ...options,
      env: { ...process.env, ...environment }
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
});
