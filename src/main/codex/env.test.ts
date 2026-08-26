import { chmod, mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
  CODEX_NOT_FOUND_MESSAGE,
  describeCodex,
  discoverCodex,
  resetCodexEnvironmentCacheForTests
} from './env';

const originalCodexBinary = process.env.CODEX_BINARY;
const originalZdotdir = process.env.ZDOTDIR;

async function fakeCodex(directory: string, name: string, version: string): Promise<string> {
  const versionCount = path.join(directory, `${name}.version-count`);
  const binary = path.join(directory, name);
  await writeFile(
    binary,
    `#!/bin/sh\nif [ "$1" = "--version" ]; then\n  printf x >> '${versionCount}'\n  printf '%s\\n' '${version}'\n  exit 0\nfi\nexit 0\n`,
    'utf8'
  );
  await chmod(binary, 0o755);
  return binary;
}

beforeEach(() => {
  resetCodexEnvironmentCacheForTests();
  delete process.env.CODEX_BINARY;
});

afterEach(() => {
  resetCodexEnvironmentCacheForTests();
  if (originalCodexBinary === undefined) delete process.env.CODEX_BINARY;
  else process.env.CODEX_BINARY = originalCodexBinary;
  if (originalZdotdir === undefined) delete process.env.ZDOTDIR;
  else process.env.ZDOTDIR = originalZdotdir;
});

describe('Codex binary discovery', () => {
  it('prefers CODEX_BINARY over the caller preference', async () => {
    const directory = await mkdtemp(path.join(tmpdir(), 'powermove-env-'));
    const environmentBinary = await fakeCodex(directory, 'codex-environment', 'env 1.0');
    const preferredBinary = await fakeCodex(directory, 'codex-preference', 'pref 1.0');
    process.env.CODEX_BINARY = environmentBinary;

    await expect(discoverCodex(preferredBinary)).resolves.toBe(environmentBinary);
  });

  it('uses the executable caller preference when CODEX_BINARY is absent', async () => {
    const directory = await mkdtemp(path.join(tmpdir(), 'powermove-env-'));
    const preferredBinary = await fakeCodex(directory, 'codex-preference', 'pref 1.0');

    await expect(discoverCodex(preferredBinary)).resolves.toBe(preferredBinary);
  });

  it('falls back to the cached login-shell probe after invalid explicit candidates', async () => {
    const directory = await mkdtemp(path.join(tmpdir(), 'powermove-env-shell-'));
    const shellBinary = await fakeCodex(directory, 'codex', 'shell 1.0');
    await writeFile(path.join(directory, '.zprofile'), `export PATH='${directory}':$PATH\n`, 'utf8');
    process.env.ZDOTDIR = directory;
    process.env.CODEX_BINARY = '/definitely/missing/powermove-codex';

    await expect(discoverCodex('/also/missing/powermove-codex')).resolves.toBe(shellBinary);
    await expect(discoverCodex('/still/missing/powermove-codex')).resolves.toBe(shellBinary);
  });

  it('records codex --version once for the process-wide description', async () => {
    const directory = await mkdtemp(path.join(tmpdir(), 'powermove-env-'));
    const binary = await fakeCodex(directory, 'codex', 'codex-cli 0.147.0');
    process.env.CODEX_BINARY = binary;

    const first = await describeCodex(null);
    process.env.CODEX_BINARY = '/another/binary/that/must/not/be-used';
    const second = await describeCodex(null);

    expect(first).toEqual({ binary, version: 'codex-cli 0.147.0' });
    expect(second).toBe(first);
    const count = await import('node:fs/promises').then((fs) => fs.readFile(`${binary}.version-count`, 'utf8'));
    expect(count).toBe('x');
  });

  it('provides an actionable not-found message', () => {
    expect(CODEX_NOT_FOUND_MESSAGE).toContain('Install Codex');
    expect(CODEX_NOT_FOUND_MESSAGE).toContain('CODEX_BINARY');
    expect(CODEX_NOT_FOUND_MESSAGE).toContain('Settings');
  });
});
