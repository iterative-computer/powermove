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
  writeSession
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

describe('safeAgentComponent', () => {
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
  it('creates the complete per-run layout with binary inputs', async () => {
    const userData = await temporaryDirectory();
    const layout = await prepareAgentWorkspace(request(), userData, 'project', agentResultSchema(), 'run-123');

    expect(layout.runId).toBe('run-123');
    expect(layout.root).toBe(path.join(userData, 'Agent Workspaces', 'Project_123'));
    expect(layout.runDirectory).toBe(path.join(layout.root, 'artifacts', 'run-123'));
    expect(layout.outputPath).toBe(path.join(layout.root, '.powermove', 'result-run-123.json'));
    expect(layout.sessionPath).toBe(path.join(layout.root, '.powermove', 'session-project.txt'));
    expect(layout.imagePaths).toEqual([path.join(layout.root, 'inputs', 'references', 'reference-0.png')]);

    await expect(readFile(path.join(layout.inputsDirectory, 'powermove-project.json'), 'utf8'))
      .resolves.toBe('{"layers":[]}');
    await expect(readFile(path.join(layout.attachmentsDirectory, 'brief-txt'), 'utf8')).resolves.toBe('hello');
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
      'computer-run'
    );
    expect(layout.sessionPath).toBe(sessionPathFor(layout.root, 'computer'));
    expect(layout.imagePaths[0]).toMatch(/reference-0\.jpg$/);
  });

  it('rejects a missing project and omits over-limit inputs', async () => {
    const userData = await temporaryDirectory();
    await expect(prepareAgentWorkspace(request({ projectJSON: null }), userData, 'project', agentResultSchema()))
      .rejects.toThrow(/project snapshot/i);
    const layout = await prepareAgentWorkspace(request({
      attachments: [{ name: 'too-big', data: new Uint8Array(100 * 1024 + 1) }],
      images: [new Uint8Array(4 * 1024 * 1024 + 1)]
    }), userData, 'project', agentResultSchema(), 'limited-run');
    const { readdir } = await import('node:fs/promises');
    await expect(readdir(layout.attachmentsDirectory)).resolves.toEqual([]);
    await expect(readdir(layout.referencesDirectory)).resolves.toEqual([]);
  });
});

describe('session helpers', () => {
  it('atomically stores, reads, and clears a trimmed session id', async () => {
    const file = path.join(await temporaryDirectory(), '.powermove', 'session-project.txt');
    await expect(readSession(file)).resolves.toBeNull();
    await writeSession(file, '  thread-123\n');
    await expect(readSession(file)).resolves.toBe('thread-123');
    await clearSession(file);
    await expect(readSession(file)).resolves.toBeNull();
  });
});
