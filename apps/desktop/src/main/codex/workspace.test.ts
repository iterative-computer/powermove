import { lstat, mkdir, mkdtemp, readFile, readdir, rm, stat, symlink, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';
import type { CodexRunRequest } from '../../shared/ipc';
import { agentResultSchema } from './instructions';
import {
  agentStateRoot,
  agentWorkspaceRoot,
  clearSession,
  discardExtensionStage,
  prepareAgentWorkspace,
  preserveCancelledRun,
  readSession,
  safeAgentComponent,
  sessionPathFor,
  writeSession,
  type PrepareAgentWorkspaceOptions
} from './workspace';

const temporaryDirectories: string[] = [];

it('preserves a failed run before the provider has written its first thread session', async () => {
  const userData = await temporaryDirectory();
  const req = request({ threadId: 'never-started' });
  const options = { ...workspaceOptions(), extensionsDir: path.join(userData, 'extensions') };
  const original = await prepareAgentWorkspace(req, userData, 'project', agentResultSchema(), options);
  await writeFile(path.join(original.stagingDirectory, 'draft.txt'), 'partial work');
  expect(await readSession(original.sessionPath)).toBeNull();
  await preserveCancelledRun(original);
  const resumed = await prepareAgentWorkspace(req, userData, 'project', agentResultSchema(), options);
  expect(resumed.runId).toBe(original.runId);
  expect(await readFile(path.join(resumed.stagingDirectory, 'draft.txt'), 'utf8')).toBe('partial work');
});

async function temporaryDirectory(): Promise<string> {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'powermove-workspace-test-'));
  temporaryDirectories.push(directory);
  return directory;
}

afterEach(async () => {
  const { rm } = await import('node:fs/promises');
  await Promise.all(temporaryDirectories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

it('isolates parallel child inputs, result schemas, sessions and stages from the parent workspace', async () => {
  const userData = await temporaryDirectory();
  const options = { ...workspaceOptions(), extensionsDir: path.join(userData, 'extensions') };
  const [parent, first, second] = await Promise.all([
    prepareAgentWorkspace(request({ threadId: 'parent' }), userData, 'project', { title: 'parent-schema' }, options),
    prepareAgentWorkspace(request({ threadId: 'first', projectJSON: '{"child":1}' }), userData, 'project', { title: 'first-schema' }, { ...options, workspaceId: 'subagent-first' }),
    prepareAgentWorkspace(request({ threadId: 'second', projectJSON: '{"child":2}' }), userData, 'project', { title: 'second-schema' }, { ...options, workspaceId: 'subagent-second' })
  ]);
  expect(new Set([parent.root, first.root, second.root]).size).toBe(3);
  expect(await readFile(path.join(parent.inputsDirectory, 'powermove-project.json'), 'utf8')).toBe('{"layers":[]}');
  expect(await readFile(path.join(first.inputsDirectory, 'powermove-project.json'), 'utf8')).toBe('{"child":1}');
  expect(await readFile(path.join(second.inputsDirectory, 'powermove-project.json'), 'utf8')).toBe('{"child":2}');
  expect(JSON.parse(await readFile(parent.schemaPath, 'utf8')).title).toBe('parent-schema');
  expect(JSON.parse(await readFile(first.schemaPath, 'utf8')).title).toBe('first-schema');
  expect(new Set([parent.sessionPath, first.sessionPath, second.sessionPath]).size).toBe(3);
  expect(new Set([parent.stagingDirectory, first.stagingDirectory, second.stagingDirectory]).size).toBe(3);
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
    const root = agentWorkspaceRoot(await temporaryDirectory(), 'project');
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
    expect(layout.outputPath).toBe(path.join(agentStateRoot(layout.root), 'result-run-123.json'));
    expect(layout.schemaPath).toBe(path.join(userData, 'Agent State', 'Project_123', 'result-schema.json'));
    expect(layout.sessionPath).toBe(path.join(agentStateRoot(layout.root), 'session-v2-project.txt'));
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

    const nested = await prepareAgentWorkspace(
      request(),
      userData,
      'project',
      agentResultSchema(),
      workspaceOptions({ apiPackFiles: [
        { name: 'samples/media-browser/index.ts', text: 'sample' },
        { name: 'samples/../escape.ts', text: 'nope' },
        { name: '/abs.ts', text: 'nope' }
      ] })
    );
    await expect(readFile(path.join(nested.apiPackDirectory, 'samples/media-browser/index.ts'), 'utf8')).resolves.toBe('sample');
    await expect(readFile(path.join(nested.apiPackDirectory, 'escape.ts'), 'utf8')).rejects.toThrow();
    await expect(readFile(path.join(nested.apiPackDirectory, 'abs.ts'), 'utf8')).rejects.toThrow();

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

describe('links planted by Project commands', () => {
  /* Each link leads outside to a folder or file holding `keep`; main must
     leave every one untouched and write only inside the workspace. */
  async function plant(root: string, outside: string, links: Record<string, 'file' | 'folder'>) {
    const targets: Array<[string, 'file' | 'folder']> = [];
    for (const [name, kind] of Object.entries(links)) {
      const target = path.join(outside, name.replaceAll('/', '_'));
      if (kind === 'folder') { await mkdir(target); await writeFile(path.join(target, 'keep'), 'keep'); }
      else await writeFile(target, 'keep');
      await mkdir(path.dirname(path.join(root, name)), { recursive: true });
      await symlink(target, path.join(root, name));
      targets.push([target, kind]);
    }
    return async () => {
      for (const [target, kind] of targets) {
        if (kind === 'folder') expect(await readdir(target)).toEqual(['keep']);
        else expect(await readFile(target, 'utf8')).toBe('keep');
      }
    };
  }

  async function prepared(links: Record<string, 'file' | 'folder'>) {
    const userData = await temporaryDirectory();
    const outside = await temporaryDirectory();
    const extensionsDir = path.join(userData, 'extensions');
    await mkdir(path.join(extensionsDir, 'live-mod'), { recursive: true });
    await writeFile(path.join(extensionsDir, 'live-mod', 'index.ts'), 'live');
    const untouched = await plant(agentWorkspaceRoot(userData, 'Project_123'), outside, links);
    const layout = await prepareAgentWorkspace(request(), userData, 'project', agentResultSchema(),
      workspaceOptions({ extensionsDir, apiPackFiles: [{ name: 'api.ts', text: 'api' }, { name: 'samples/a/index.ts', text: 'sample' }] }), 'run-123');
    await untouched();
    await expect(readFile(path.join(layout.inputsDirectory, 'powermove-project.json'), 'utf8')).resolves.toBe('{"layers":[]}');
    await expect(readFile(path.join(layout.attachmentsDirectory, 'brief-txt'), 'utf8')).resolves.toBe('hello');
    await expect(readFile(layout.imagePaths[0]!)).resolves.toHaveLength(8);
    await expect(readFile(path.join(layout.apiPackDirectory, 'samples/a/index.ts'), 'utf8')).resolves.toBe('sample');
    await expect(readFile(path.join(layout.stagingDirectory, 'live-mod', 'index.ts'), 'utf8')).resolves.toBe('live');
    for (const folder of [layout.inputsDirectory, layout.apiPackDirectory, layout.runDirectory, layout.stagingDirectory]) {
      expect((await lstat(folder)).isDirectory()).toBe(true);
    }
    return { layout, outside };
  }

  it('replaces linked top-level folders instead of writing through them', async () => {
    await prepared({ inputs: 'folder', 'powermove-api': 'folder', artifacts: 'folder', '.powermove': 'folder' });
  });

  it('replaces linked files and folders inside the workspace', async () => {
    await prepared({
      'inputs/powermove-project.json': 'file', 'inputs/attachments': 'folder', 'inputs/references/reference-0.png': 'file',
      'powermove-api/api.ts': 'file', 'powermove-api/samples': 'folder', '.powermove/extension-runs': 'folder',
      '.powermove/result-schema.json': 'file', 'artifacts/run-123': 'folder'
    });
    await prepared({ 'inputs/attachments/brief-txt': 'file', 'inputs/references': 'folder', '.powermove/extension-runs/run-123': 'folder' });
  });

  it('removes a stage without deleting through a folder swapped for a link above it', async () => {
    const { layout, outside } = await prepared({});
    const decoy = path.join(outside, 'extension-runs', 'run-123');
    await mkdir(decoy, { recursive: true });
    await writeFile(path.join(decoy, 'keep'), 'keep');
    await rm(layout.internalDirectory, { recursive: true });
    await symlink(outside, layout.internalDirectory);
    await discardExtensionStage(layout);
    expect(await readdir(decoy)).toEqual(['keep']);
  });

  it('removes a stage and any links inside it without following them', async () => {
    const { layout, outside } = await prepared({});
    await writeFile(path.join(outside, 'keep'), 'keep');
    await symlink(outside, path.join(layout.stagingDirectory, 'linked'));
    await discardExtensionStage(layout);
    await expect(lstat(layout.stagingDirectory)).rejects.toMatchObject({ code: 'ENOENT' });
    expect(await readdir(outside)).toEqual(['keep']);
    expect((await readdir(agentStateRoot(layout.root))).filter(name => name.startsWith('.scratch-'))).toEqual([]);
  });
});

describe('state the agent could forge', () => {
  const FLAG = '--dangerously-bypass-approvals-and-sandbox';

  it('keeps sessions and checkpoints outside the workspace, ignoring copies planted inside it', async () => {
    const userData = await temporaryDirectory();
    const extensionsDir = path.join(userData, 'extensions');
    const root = agentWorkspaceRoot(userData, 'Project_123');
    const legacy = path.join(root, '.powermove', 'session-v2-project.txt');
    const forgedStage = path.join(root, '.powermove', 'extension-runs', 'forged');
    await mkdir(forgedStage, { recursive: true });
    await writeFile(legacy, FLAG);
    await writeFile(`${legacy}.checkpoint.json`, JSON.stringify({
      liveDirectory: extensionsDir, stagingDirectory: forgedStage, projectId: 'Project_123', runId: 'forged',
      historyRoot: path.join(userData, 'Agent Change History', 'Project_123'), baselineRootHash: 'x', baselineHashes: {}
    }));
    const layout = await prepareAgentWorkspace(request(), userData, 'project', agentResultSchema(), workspaceOptions({ extensionsDir }));
    expect(layout.runId).not.toBe('forged');
    expect(path.relative(root, layout.sessionPath).startsWith('..')).toBe(true);
    expect(await readSession(layout.sessionPath)).toBeNull();
    await preserveCancelledRun(layout);
    await expect(stat(`${layout.sessionPath}.checkpoint.json`)).resolves.toBeTruthy();
    expect(await readFile(legacy, 'utf8')).toBe(FLAG);
  });

  it('never returns a session id that could read as a command-line flag', async () => {
    const file = sessionPathFor(agentWorkspaceRoot(await temporaryDirectory(), 'project'), 'project');
    await mkdir(path.dirname(file), { recursive: true });
    for (const forged of [FLAG, '-r', 'id with spaces', 'a\nb']) {
      await writeFile(file, forged);
      expect(await readSession(file)).toBeNull();
    }
    await writeSession(file, FLAG);
    expect(await readSession(file)).toBeNull();
    await writeSession(file, '019a3c5e-7b2d-7c11-9e4f-2a6b8d0c1e3f');
    expect(await readSession(file)).toBe('019a3c5e-7b2d-7c11-9e4f-2a6b8d0c1e3f');
  });

  it('refuses a checkpoint whose baseline is not a map of extension hashes', async () => {
    const userData = await temporaryDirectory();
    const options = workspaceOptions({ extensionsDir: path.join(userData, 'extensions') });
    const layout = await prepareAgentWorkspace(request(), userData, 'project', agentResultSchema(), options);
    await preserveCancelledRun({ ...layout, baselineHashes: { '../escape': 'x' } });
    await expect(prepareAgentWorkspace(request(), userData, 'project', agentResultSchema(), options)).rejects.toThrow(/checkpoint is invalid/);
    await preserveCancelledRun(layout);
    await expect(prepareAgentWorkspace(request(), userData, 'project', agentResultSchema(), options)).resolves.toMatchObject({ runId: layout.runId });
  });
});
