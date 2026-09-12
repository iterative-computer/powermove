import * as fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { MANIFEST_LIMITS, type ExtensionManifest } from '../../shared/extensions';
import { scanExtensionDirs } from './discovery';

const temporaryDirectories: string[] = [];

async function temporaryDirectory(): Promise<string> {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-discovery-'));
  temporaryDirectories.push(directory);
  return directory;
}

async function writeExtension(
  root: string,
  id: string,
  manifest: Partial<ExtensionManifest> = {}
): Promise<string> {
  const directory = path.join(root, id);
  await fs.mkdir(directory, { recursive: true });
  await fs.writeFile(
    path.join(directory, 'manifest.json'),
    JSON.stringify({ id, name: id, version: '1.0.0', apiVersion: 1, ...manifest })
  );
  return directory;
}

afterEach(async () => {
  await Promise.all(
    temporaryDirectories.splice(0).map((directory) => fs.rm(directory, { recursive: true }))
  );
});

describe('extension discovery', () => {
  it('discovers valid extensions in deterministic user-then-project order', async () => {
    const userDir = await temporaryDirectory();
    const projectDir = await temporaryDirectory();
    await writeExtension(userDir, 'zulu-ext');
    await writeExtension(userDir, 'alpha-ext', { entry: 'src/index.ts', author: 'user' });
    await writeExtension(projectDir, 'alpha-ext', { author: 'agent' });

    const result = await scanExtensionDirs([
      { dir: projectDir, scope: 'project' },
      { dir: userDir, scope: 'user' }
    ]);

    expect(result.map(({ id, scope }) => ({ id, scope }))).toEqual([
      { id: 'alpha-ext', scope: 'user' },
      { id: 'zulu-ext', scope: 'user' },
      { id: 'alpha-ext', scope: 'project' }
    ]);
    expect(result[0]?.manifest).toMatchObject({ id: 'alpha-ext', entry: 'src/index.ts' });
    expect(result.every((extension) => extension.error === undefined)).toBe(true);
  });

  it('records malformed JSON as a manifest error under the directory id', async () => {
    const root = await temporaryDirectory();
    const directory = path.join(root, 'broken-json');
    await fs.mkdir(directory);
    await fs.writeFile(path.join(directory, 'manifest.json'), '{"id":');

    const result = await scanExtensionDirs([{ dir: root, scope: 'user' }]);

    expect(result).toHaveLength(1);
    expect(result[0]).toMatchObject({ id: 'broken-json', manifest: null });
    expect(result[0]?.error).toBeTruthy();
  });

  it('records schema validation errors without accepting a partial manifest', async () => {
    const root = await temporaryDirectory();
    await writeExtension(root, 'bad-schema', { version: 'next' });

    const result = await scanExtensionDirs([{ dir: root, scope: 'user' }]);

    expect(result[0]).toMatchObject({ id: 'bad-schema', manifest: null });
    expect(result[0]?.error).toContain('invalid "version"');
  });

  it('rejects a valid manifest whose id differs from its directory name', async () => {
    const root = await temporaryDirectory();
    await writeExtension(root, 'folder-name', { id: 'different-id' });

    const result = await scanExtensionDirs([{ dir: root, scope: 'project' }]);

    expect(result[0]).toMatchObject({ id: 'folder-name', scope: 'project', manifest: null });
    expect(result[0]?.error).toContain('must match directory name');
  });

  it('skips dot directories, symlinked roots, symlinked manifests, and folders without manifests', async () => {
    const root = await temporaryDirectory();
    const target = await writeExtension(root, 'real-ext');
    await writeExtension(root, '.hidden-ext');
    await fs.symlink(target, path.join(root, 'linked-ext'), 'dir');
    await fs.mkdir(path.join(root, 'no-manifest'));
    const linkedManifest = path.join(root, 'linked-manifest');
    await fs.mkdir(linkedManifest);
    await fs.symlink(path.join(target, 'manifest.json'), path.join(linkedManifest, 'manifest.json'));

    const result = await scanExtensionDirs([{ dir: root, scope: 'user' }]);

    expect(result.map((extension) => extension.id)).toEqual(['real-ext']);
  });

  it('does not follow nested symlinks while counting source files', async () => {
    const root = await temporaryDirectory();
    const extension = await writeExtension(root, 'linked-sources');
    const outside = await temporaryDirectory();
    await Promise.all(
      Array.from({ length: MANIFEST_LIMITS.sourceFiles + 1 }, (_, index) =>
        fs.writeFile(path.join(outside, `${index}.ts`), '')
      )
    );
    await fs.symlink(outside, path.join(extension, 'external'), 'dir');

    const result = await scanExtensionDirs([{ dir: root, scope: 'user' }]);

    expect(result[0]?.manifest?.id).toBe('linked-sources');
    expect(result[0]?.error).toBeUndefined();
  });

  it('records an error when the recursive source-file limit is exceeded', async () => {
    const root = await temporaryDirectory();
    const extension = await writeExtension(root, 'too-many-files');
    const nested = path.join(extension, 'src', 'nested');
    await fs.mkdir(nested, { recursive: true });
    await Promise.all(
      Array.from({ length: MANIFEST_LIMITS.sourceFiles + 1 }, (_, index) =>
        fs.writeFile(path.join(nested, `${index}.ts`), '')
      )
    );

    const result = await scanExtensionDirs([{ dir: root, scope: 'user' }]);

    expect(result[0]).toMatchObject({ id: 'too-many-files', manifest: null });
    expect(result[0]?.error).toContain(`more than ${MANIFEST_LIMITS.sourceFiles}`);
  });

  it('tolerates missing scan roots', async () => {
    const root = await temporaryDirectory();

    await expect(
      scanExtensionDirs([{ dir: path.join(root, 'missing'), scope: 'user' }])
    ).resolves.toEqual([]);
  });
});
