import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { copyChildArtifacts } from './subagent-artifacts';
import { agentWorkspaceRoot } from './workspace';

let directory: string;
const source = () => path.join(agentWorkspaceRoot(directory, 'subagent-child'), 'artifacts');
const destination = () => path.join(agentWorkspaceRoot(directory, 'project'), 'artifacts');
beforeEach(async () => {
  directory = await mkdtemp(path.join(os.tmpdir(), 'pm-child-artifacts-'));
  await mkdir(path.join(source(), 'unique-run', 'images'), { recursive: true });
});
afterEach(async () => { await rm(directory, { recursive: true, force: true }); });
const result = (file = 'unique-run/images/title.svg') => JSON.stringify({ artifacts: [{ path: file }] });

describe('subagent artifact handoff', () => {
  it('makes a reported artifact available to the parent without overwriting files', async () => {
    await writeFile(path.join(source(), 'unique-run/images/title.svg'), '<svg/>');
    await copyChildArtifacts(directory, 'subagent-child', 'project', result());
    expect(await readFile(path.join(destination(), 'unique-run/images/title.svg'), 'utf8')).toBe('<svg/>');
    await expect(copyChildArtifacts(directory, 'subagent-child', 'project', result())).rejects.toMatchObject({ code: 'EEXIST' });
  });

  it('rejects source escapes and destination links', async () => {
    const outside = path.join(directory, 'outside'); await mkdir(outside); await writeFile(path.join(outside, 'private.txt'), 'isolated fixture');
    await symlink(path.join(outside, 'private.txt'), path.join(source(), 'escape.txt'));
    await expect(copyChildArtifacts(directory, 'subagent-child', 'project', result('escape.txt'))).rejects.toThrow('escapes');
    await writeFile(path.join(source(), 'unique-run/images/title.svg'), '<svg/>');
    await mkdir(destination(), { recursive: true }); await symlink(outside, path.join(destination(), 'unique-run'));
    await expect(copyChildArtifacts(directory, 'subagent-child', 'project', result())).rejects.toThrow('through a link');
  });
});
