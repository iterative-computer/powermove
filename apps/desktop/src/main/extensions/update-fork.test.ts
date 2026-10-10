import { access, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { restoreExtensionChangeSet } from '../codex/change-history';
import { updateFork } from './update-fork';

const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });

async function fixture() {
  const userData = await mkdtemp(path.join(os.tmpdir(), 'powermove-direct-fork-'));
  roots.push(userData);
  const userExtensionsDir = path.join(userData, 'extensions');
  const builtinExtensionsDir = path.join(userData, 'builtins');
  const forkId = 'custom-tool';
  const live = path.join(userExtensionsDir, forkId);
  const base = path.join(live, '.forked-from');
  const current = path.join(builtinExtensionsDir, 'tool');
  await Promise.all([mkdir(base, { recursive: true }), mkdir(current, { recursive: true })]);
  const manifest = { id: 'tool', name: 'Tool', version: '1.0.0', apiVersion: 1,
    entry: 'index.ts', author: 'powermove', contributes: ['commands'] };
  const original = 'export const size = 10;\n\nexport const speed = 1;\n\nexport default function activate() {}\n';
  await Promise.all([
    writeFile(path.join(base, 'manifest.json'), JSON.stringify(manifest)),
    writeFile(path.join(base, 'index.ts'), original),
    writeFile(path.join(live, 'manifest.json'), JSON.stringify({ ...manifest,
      id: forkId, name: 'My Tool', replaces: ['tool'], forkedFrom: 'tool@1.0.0' })),
    writeFile(path.join(live, 'index.ts'), original.replace('size = 10', 'size = 20')),
    writeFile(path.join(current, 'manifest.json'), JSON.stringify({ ...manifest, version: '2.0.0', description: 'Improved tool' })),
    writeFile(path.join(current, 'index.ts'), original.replace('speed = 1', 'speed = 2'))
  ]);
  return { userData, userExtensionsDir, builtinExtensionsDir, forkId, live, base, current };
}

it('updates locally, merges separate edits in one file, preserves identity, and can restore the exact previous fork', async () => {
  const setup = await fixture();
  const beforeManifest = await readFile(path.join(setup.live, 'manifest.json'), 'utf8');
  const beforeSource = await readFile(path.join(setup.live, 'index.ts'), 'utf8');
  const result = await updateFork(setup);
  expect(result.kind).toBe('updated');
  if (result.kind !== 'updated') throw new Error('Expected a local update');
  expect(result.version).toBe('2.0.0');
  expect(await readFile(path.join(setup.live, 'index.ts'), 'utf8'))
    .toBe('export const size = 20;\n\nexport const speed = 2;\n\nexport default function activate() {}\n');
  expect(JSON.parse(await readFile(path.join(setup.live, 'manifest.json'), 'utf8'))).toMatchObject({
    id: setup.forkId, name: 'My Tool', version: '1.0.0', replaces: ['tool'],
    forkedFrom: 'tool@2.0.0', description: 'Improved tool'
  });
  expect(await readFile(path.join(setup.live, '.forked-from', 'index.ts'), 'utf8'))
    .toBe('export const size = 10;\n\nexport const speed = 2;\n\nexport default function activate() {}\n');
  await restoreExtensionChangeSet({ liveDirectory: setup.userExtensionsDir,
    historyRoot: path.join(setup.userData, 'Agent Change History', result.projectId), changeSetId: result.changeSetId });
  expect(await readFile(path.join(setup.live, 'manifest.json'), 'utf8')).toBe(beforeManifest);
  expect(await readFile(path.join(setup.live, 'index.ts'), 'utf8')).toBe(beforeSource);
  expect(await readFile(path.join(setup.base, 'index.ts'), 'utf8')).toContain('speed = 1');
  expect(await readdir(path.join(setup.userData, 'Fork Updates'))).toEqual([]);
});

it('keeps the live extension and old base untouched when both sides change the same lines', async () => {
  const setup = await fixture();
  await writeFile(path.join(setup.current, 'index.ts'), 'export const size = 30;\n\nexport const speed = 1;\n');
  expect(await updateFork(setup)).toEqual({ kind: 'needs-agent', reason: 'conflict', conflicts: ['index.ts'] });
  expect(await readFile(path.join(setup.live, 'index.ts'), 'utf8')).toContain('size = 20');
  expect(await readFile(path.join(setup.live, 'manifest.json'), 'utf8')).toContain('tool@1.0.0');
  expect(await readdir(path.join(setup.userData, 'Agent Change History', 'fork-updates'))).toEqual([]);
});

it('compiles before publication and leaves the live copy untouched when the result fails validation', async () => {
  const setup = await fixture();
  await writeFile(path.join(setup.live, 'index.ts'), await readFile(path.join(setup.base, 'index.ts')));
  await writeFile(path.join(setup.current, 'index.ts'), 'export const broken = ;');
  expect(await updateFork(setup)).toEqual({ kind: 'needs-agent', reason: 'validation', conflicts: [] });
  expect(await readFile(path.join(setup.live, 'index.ts'), 'utf8')).toContain('speed = 1');
  expect(await readFile(path.join(setup.live, 'manifest.json'), 'utf8')).toContain('tool@1.0.0');
  expect(await readdir(path.join(setup.userData, 'Fork Updates'))).toEqual([]);
});

it('keeps unrelated extensions intact and handles upstream additions and removals', async () => {
  const setup = await fixture();
  const other = path.join(setup.userExtensionsDir, 'other-tool');
  await mkdir(other);
  await writeFile(path.join(other, 'manifest.json'), JSON.stringify({ id: 'other-tool', name: 'Other', version: '1.0.0', apiVersion: 1, entry: 'index.ts' }));
  await writeFile(path.join(other, 'index.ts'), 'export const untouched = true;');
  await Promise.all([
    writeFile(path.join(setup.base, 'obsolete.ts'), 'export const old = true;'),
    writeFile(path.join(setup.live, 'obsolete.ts'), 'export const old = true;'),
    writeFile(path.join(setup.current, 'new.ts'), 'export const added = true;'),
    writeFile(path.join(setup.live, 'custom.ts'), 'export const custom = true;')
  ]);
  expect(await updateFork(setup)).toMatchObject({ kind: 'updated' });
  await expect(access(path.join(setup.live, 'obsolete.ts'))).rejects.toMatchObject({ code: 'ENOENT' });
  expect(await readFile(path.join(setup.live, 'new.ts'), 'utf8')).toContain('added');
  expect(await readFile(path.join(setup.live, 'custom.ts'), 'utf8')).toContain('custom');
  expect(await readFile(path.join(other, 'index.ts'), 'utf8')).toBe('export const untouched = true;');
});
