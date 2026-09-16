import { mkdir, realpath, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';

import { collectArtifacts, mimeTypeForPath, readArtifact, reveal, validatedArtifactPath } from './artifacts';

const temporaryPaths: string[] = [];

async function makeFixture(): Promise<{ base: string; root: string; outside: string }> {
  const { mkdtemp } = await import('node:fs/promises');
  const base = await mkdtemp(path.join(tmpdir(), 'powermove-artifacts-'));
  temporaryPaths.push(base);
  const root = path.join(base, 'artifacts');
  const outside = path.join(base, 'outside');
  await mkdir(path.join(root, 'run-1'), { recursive: true });
  await mkdir(outside);
  await writeFile(path.join(root, 'run-1', 'frame.png'), new Uint8Array([1, 2, 3]));
  await writeFile(path.join(outside, 'secret.txt'), 'secret');
  return { base, root, outside };
}

afterEach(async () => {
  const { rm } = await import('node:fs/promises');
  await Promise.all(temporaryPaths.splice(0).map((entry) => rm(entry, { recursive: true, force: true })));
});

describe('validatedArtifactPath', () => {
  it('accepts a regular contained file', async () => {
    const { root } = await makeFixture();
    await expect(validatedArtifactPath(root, 'run-1/frame.png')).resolves.toBe(
      await realpath(path.join(root, 'run-1', 'frame.png'))
    );
  });

  it.each([
    '',
    '/etc/passwd',
    '../outside/secret.txt',
    'run-1/../outside.txt',
    'run-1\\frame.png',
    'run-1/%2fetc/passwd',
    'run-1/%2Fetc/passwd',
    'run-1/%5csecret',
    'run-1/evil\0name'
  ])('rejects unsafe path %j', async (relativePath) => {
    const { root } = await makeFixture();
    await expect(validatedArtifactPath(root, relativePath)).rejects.toThrow();
  });

  it('rejects a symlink whose target is outside the artifact root', async () => {
    const { root, outside } = await makeFixture();
    await symlink(path.join(outside, 'secret.txt'), path.join(root, 'run-1', 'escape.txt'));
    await expect(validatedArtifactPath(root, 'run-1/escape.txt')).rejects.toThrow(/escapes/);
  });

  it('rejects directories', async () => {
    const { root } = await makeFixture();
    await expect(validatedArtifactPath(root, 'run-1')).rejects.toThrow(/regular file/);
  });

  it('reads and reveals artifacts larger than 64 MB without losing their tail', async () => {
    const { root } = await makeFixture();
    const file = path.join(root, 'run-1', 'large.bin');
    const { open } = await import('node:fs/promises');
    const size = 64 * 1024 * 1024 + 1;
    const handle = await open(file, 'w');
    try { await handle.write(new Uint8Array([123]), 0, 1, size - 1); }
    finally { await handle.close(); }
    const realFile = await realpath(file);
    await expect(validatedArtifactPath(root, 'run-1/large.bin')).resolves.toBe(realFile);
    const showItemInFolder = vi.fn();
    await reveal(root, 'run-1/large.bin', { showItemInFolder });
    expect(showItemInFolder).toHaveBeenCalledWith(realFile);
    const result = await readArtifact(root, 'run-1/large.bin');
    expect(result.data.byteLength).toBe(size);
    expect(result.data.at(-1)).toBe(123);
  });

  it('collects every artifact beyond the former 80-file cutoff', async () => {
    const { root } = await makeFixture();
    const run = path.join(root, 'run-1');
    for (let i = 0; i < 85; i++) await writeFile(path.join(run, `output-${i}.txt`), 'output');
    const result = await collectArtifacts(run, 'run-1', [{ path: 'output-84.txt', importToTimeline: true }]);
    expect(result).toHaveLength(86);
    expect(result.find(item => item.name === 'output-84.txt')?.importToTimeline).toBe(true);
  });
});

describe('artifact operations', () => {
  it('reads bytes with a name and extension MIME type', async () => {
    const { root } = await makeFixture();
    const result = await readArtifact(root, 'run-1/frame.png');
    expect(result).toEqual({
      name: 'frame.png',
      mime: 'image/png',
      data: new Uint8Array([1, 2, 3])
    });
    expect(mimeTypeForPath('unknown.powermove')).toBe('application/octet-stream');
  });

  it('reveals the validated real file', async () => {
    const { root } = await makeFixture();
    const showItemInFolder = vi.fn();
    await reveal(root, 'run-1/frame.png', { showItemInFolder });
    expect(showItemInFolder).toHaveBeenCalledOnce();
    expect(showItemInFolder).toHaveBeenCalledWith(await realpath(path.join(root, 'run-1', 'frame.png')));
  });

  it('collects visible files, skips package descendants, and marks requested imports', async () => {
    const { root } = await makeFixture();
    const run = path.join(root, 'run-1');
    await writeFile(path.join(run, '.hidden.txt'), 'hidden');
    await mkdir(path.join(run, 'Example.app', 'Contents'), { recursive: true });
    await writeFile(path.join(run, 'Example.app', 'Contents', 'inside.txt'), 'package child');
    await mkdir(path.join(run, 'nested'));
    await writeFile(path.join(run, 'nested', 'sound.wav'), 'audio');

    await expect(
      collectArtifacts(run, 'run-1', [{ path: 'nested/sound.wav', importToTimeline: true }])
    ).resolves.toEqual([
      {
        path: 'run-1/frame.png',
        name: 'frame.png',
        size: 3,
        mime: 'image/png',
        importToTimeline: false
      },
      {
        path: 'run-1/nested/sound.wav',
        name: 'sound.wav',
        size: 5,
        mime: 'audio/wav',
        importToTimeline: true
      }
    ]);
  });
});
