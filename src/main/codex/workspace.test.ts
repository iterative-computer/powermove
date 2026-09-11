import { mkdtemp, readFile, stat } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';
import type { CodexRunRequest } from '../../shared/ipc';
import { agentResultSchema } from './instructions';
import {
  agentWorkspaceRoot,
  clearSession,
  prepareAgentWorkspace,
  readSession,
  safeAgentComponent,
  sessionPathFor,
  writeSession,
  type PrepareAgentWorkspaceOptions
} from './workspace';

const temporaryDirectories: string[] = [];

async function temporaryDirectory(): Promise<string> {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'powermove-workspace-test-'));
  temporaryDirectories.push(directory);
  return directory;
}

afterEach(async () => {
  const { rm } = await import('node:fs/promises');
  await Promise.all(temporaryDirectories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

function request(overrides: Partial<CodexRunRequest> = {}): CodexRunRequest {
  return {
    id: 'request-1234',
    mode: 'autonomous',
    prompt: 'Make it move.',
    schema: null,
    images: [new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])],
    model: null,
    reasoningEffort: null,
    access: 'project',
    projectId: 'Project_123',
    projectName: 'Project',
    projectJSON: '{"layers":[]}',
    attachments: [{ name: 'brief.txt', data: new TextEncoder().encode('hello') }],
    consentToken: null,
    ...overrides
  };
}

function workspaceOptions(overrides: Partial<PrepareAgentWorkspaceOptions> = {}): PrepareAgentWorkspaceOptions {
  return {
    extensionsDir: path.join(os.tmpdir(), 'powermove-user-extensions'),
    apiPackFiles: [
      { name: 'EXTENSIONS.md', text: '# Extensions\n' },
      { name: 'api.ts', text: 'export interface PowermoveAPI {}\n' },
      { name: 'extensions.ts', text: 'export interface ExtensionManifest {}\n' },
      { name: 'project.ts', text: 'export interface Project {}\n' },
      { name: 'commands.ts', text: 'export type EditCommand = never;\n' }
    ],
    ...overrides
  };
}

describe('safeAgentComponent', () => {
  it('isolates thread sessions within each project and authority', async () => {
    const root = await temporaryDirectory();
    const a = sessionPathFor(root, 'project', 'thread-a');
    const b = sessionPathFor(root, 'project', 'thread-b');
    const computer = sessionPathFor(root, 'computer', 'thread-a');
    await writeSession(a, 'codex-a'); await writeSession(b, 'codex-b');
    expect(await readSession(a)).toBe('codex-a');
    expect(await readSession(b)).toBe('codex-b');
    expect(await readSession(computer)).toBeNull();
    await clearSession(a);
    expect(await readSession(b)).toBe('codex-b');
    expect(() => sessionPathFor(root, 'project', '../escape')).toThrow(/Invalid/);
    expect(() => sessionPathFor(root, 'project', '')).toThrow(/Invalid/);
  });
  it('replaces unsafe runs, trims hyphens, preserves the allowlist, and caps length', () => {
    expect(safeAgentComponent(' --hello !@# world__- ')).toBe('hello-world__');
    expect(safeAgentComponent('!!!', 'fallback')).toBe('fallback');
    expect(safeAgentComponent('x'.repeat(150))).toHaveLength(120);
  });

  it('builds a userData-relative workspace root', () => {
    expect(agentWorkspaceRoot('/user-data', 'project-id'))
      .toBe(path.join('/user-data', 'Agent Workspaces', 'project-id'));
  });
});

describe('prepareAgentWorkspace', () => {
  it('preserves a 30 MB project snapshot in the agent workspace', async () => {
    const projectJSON = JSON.stringify({ layers: [], data: 'x'.repeat(30 * 1024 * 1024) });
    const layout = await prepareAgentWorkspace(request({ projectJSON }), await temporaryDirectory(),
      'project', agentResultSchema(), workspaceOptions(), 'large-project');
    expect(await readFile(path.join(layout.inputsDirectory, 'powermove-project.json'), 'utf8')).toBe(projectJSON);
  });

  it('creates the complete per-run layout with binary inputs', async () => {
    const userData = await temporaryDirectory();
    const options = workspaceOptions();
    const layout = await prepareAgentWorkspace(request(), userData, 'project', agentResultSchema(), options, 'run-123');

    expect(layout.runId).toBe('run-123');
    expect(layout.root).toBe(path.join(userData, 'Agent Workspaces', 'Project_123'));
    expect(layout.runDirectory).toBe(path.join(layout.root, 'artifacts', 'run-123'));
    expect(layout.outputPath).toBe(path.join(layout.root, '.powermove', 'result-run-123.json'));
    expect(layout.sessionPath).toBe(path.join(layout.root, '.powermove', 'session-v2-project.txt'));
    expect(layout.apiPackDirectory).toBe(path.join(layout.root, 'powermove-api'));
    expect(layout.liveDirectory).toBe(options.extensionsDir);
    expect(layout.extensionsDir).toBe(path.join(layout.root, '.powermove', 'extension-runs', 'run-123'));
    expect(layout.stagingDirectory).toBe(layout.extensionsDir);
    expect(layout.historyRoot).toBe(path.join(userData, 'Agent Change History', 'Project_123'));
    expect(layout.imagePaths).toEqual([path.join(layout.root, 'inputs', 'references', 'reference-0.png')]);

    await expect(readFile(path.join(layout.inputsDirectory, 'powermove-project.json'), 'utf8'))
      .resolves.toBe('{"layers":[]}');
    await expect(readFile(path.join(layout.attachmentsDirectory, 'brief-txt'), 'utf8')).resolves.toBe('hello');
    await expect(readFile(path.join(layout.apiPackDirectory, 'EXTENSIONS.md'), 'utf8'))
      .resolves.toBe('# Extensions\n');
    await expect(readFile(path.join(layout.apiPackDirectory, 'commands.ts'), 'utf8'))
      .resolves.toBe('export type EditCommand = never;\n');
    await expect(readFile(layout.schemaPath, 'utf8')).resolves.toSatisfy((contents: string) =>
      JSON.stringify(JSON.parse(contents)) === JSON.stringify(agentResultSchema())
    );
    await expect(stat(layout.runDirectory)).resolves.toSatisfy((metadata) => metadata.isDirectory());
  });

  it('selects the computer session and jpg extension for non-PNG bytes', async () => {
    const layout = await prepareAgentWorkspace(
      request({ access: 'computer', images: [new Uint8Array([0xff, 0xd8, 0xff])] }),
      await temporaryDirectory(),
      'computer',
      agentResultSchema(),
      workspaceOptions(),
      'computer-run'
    );
    expect(layout.sessionPath).toBe(sessionPathFor(layout.root, 'computer'));
    expect(layout.imagePaths[0]).toMatch(/reference-0\.jpg$/);
  });

  it('rejects a missing project and preserves inputs above the old byte limits', async () => {
    const userData = await temporaryDirectory();
    await expect(prepareAgentWorkspace(
      request({ projectJSON: null }),
      userData,
      'project',
      agentResultSchema(),
      workspaceOptions()
    ))
      .rejects.toThrow(/project snapshot/i);
    const largeImage = new Uint8Array(4 * 1024 * 1024 + 1);
    largeImage.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
    const layout = await prepareAgentWorkspace(request({
      attachments: [{ name: 'large.bin', data: new Uint8Array(100 * 1024 + 1) }],
      images: [largeImage]
    }), userData, 'project', agentResultSchema(), workspaceOptions(), 'limited-run');
    const { readdir, stat } = await import('node:fs/promises');
    await expect(readdir(layout.attachmentsDirectory)).resolves.toEqual(['large-bin']);
    await expect(readdir(layout.referencesDirectory)).resolves.toEqual(['reference-0.png']);
    await expect(stat(path.join(layout.attachmentsDirectory, 'large-bin')))
      .resolves.toMatchObject({ size: 100 * 1024 + 1 });
    await expect(stat(path.join(layout.referencesDirectory, 'reference-0.png')))
      .resolves.toMatchObject({ size: 4 * 1024 * 1024 + 1 });
  });

  it('refreshes API pack files on every preparation', async () => {
    const userData = await temporaryDirectory();
    const first = await prepareAgentWorkspace(
      request(),
      userData,
      'project',
      agentResultSchema(),
      workspaceOptions({ apiPackFiles: [{ name: 'EXTENSIONS.md', text: 'old' }] }),
      'first-run'
    );
    await expect(readFile(path.join(first.apiPackDirectory, 'EXTENSIONS.md'), 'utf8')).resolves.toBe('old');
    const { writeFile } = await import('node:fs/promises');
    await writeFile(path.join(first.apiPackDirectory, 'stale.ts'), 'stale');

    const second = await prepareAgentWorkspace(
      request(),
      userData,
      'project',
      agentResultSchema(),
      workspaceOptions({ apiPackFiles: [{ name: 'EXTENSIONS.md', text: 'new' }] }),
      'second-run'
    );
    await expect(readFile(path.join(second.apiPackDirectory, 'EXTENSIONS.md'), 'utf8')).resolves.toBe('new');
    await expect(readFile(path.join(second.apiPackDirectory, 'stale.ts'), 'utf8'))
      .rejects.toMatchObject({ code: 'ENOENT' });
  });

  it('skips API pack names that can escape or collide without failing the run', async () => {
    const userData = await temporaryDirectory();
    const escaped = await prepareAgentWorkspace(
      request(),
      userData,
      'project',
      agentResultSchema(),
      workspaceOptions({ apiPackFiles: [{ name: '../outside.ts', text: 'nope' }, { name: 'ok.ts', text: 'ok' }] })
    );
    await expect(readFile(path.join(escaped.apiPackDirectory, 'ok.ts'), 'utf8')).resolves.toBe('ok');
    await expect(readFile(path.join(escaped.apiPackDirectory, '..', 'outside.ts'), 'utf8')).rejects.toThrow();

    const collided = await prepareAgentWorkspace(
      request(),
      userData,
      'project',
      agentResultSchema(),
      workspaceOptions({
        apiPackFiles: [
          { name: 'api.ts', text: 'first' },
          { name: 'api.ts', text: 'second' }
        ]
      })
    );
    await expect(readFile(path.join(collided.apiPackDirectory, 'api.ts'), 'utf8')).resolves.toBe('first');
  });

  it('requires the writable extensions directory to be absolute', async () => {
    await expect(prepareAgentWorkspace(
      request(),
      await temporaryDirectory(),
      'project',
      agentResultSchema(),
      workspaceOptions({ extensionsDir: 'relative/extensions' })
    )).rejects.toThrow(/absolute path/i);
  });
});

describe('session helpers', () => {
  it('atomically stores, reads, and clears a trimmed session id', async () => {
    const file = path.join(await temporaryDirectory(), '.powermove', 'session-v2-project.txt');
    await expect(readSession(file)).resolves.toBeNull();
    await writeSession(file, '  thread-123\n');
    await expect(readSession(file)).resolves.toBe('thread-123');
    await clearSession(file);
    await expect(readSession(file)).resolves.toBeNull();
  });
});
