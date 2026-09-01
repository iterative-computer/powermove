import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';
import { resolvePackagedEsbuildBinary } from './esbuild-binary';

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

describe('resolvePackagedEsbuildBinary', () => {
  it('resolves the executable electron-builder unpacked outside app.asar', async () => {
    const resourcesPath = await mkdtemp(path.join(os.tmpdir(), 'powermove-esbuild-test-'));
    temporaryDirectories.push(resourcesPath);
    const binaryPath = path.join(resourcesPath, 'app.asar.unpacked', 'node_modules', '@esbuild', 'darwin-arm64', 'bin', 'esbuild');
    await mkdir(path.dirname(binaryPath), { recursive: true });
    await writeFile(binaryPath, 'fixture');

    expect(resolvePackagedEsbuildBinary(resourcesPath, 'darwin', 'arm64')).toBe(binaryPath);
  });

  it('leaves development and incomplete packages on esbuild default resolution', () => {
    expect(resolvePackagedEsbuildBinary(undefined, 'darwin', 'arm64')).toBeUndefined();
    expect(resolvePackagedEsbuildBinary('/missing/resources', 'darwin', 'arm64')).toBeUndefined();
  });
});
