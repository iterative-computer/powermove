import { lstat, mkdtemp, mkdir, readFile, readdir, stat, symlink, unlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

import {
  ISOLATED_CODEX_HOME_NAME,
  POWERMOVE_AUTH_STORE_CONFIG,
  POWERMOVE_AUTH_OWNER_FILE,
  discoverUserSkillFiles,
  isolatedCodexHome,
  prepareIsolatedCodexHome,
  userCodexHome
} from './isolation';

describe('isolated Codex home', () => {
  it('uses CODEX_HOME only as the credential source when configured', () => {
    expect(userCodexHome({ CODEX_HOME: ' /tmp/custom-codex ' })).toBe('/tmp/custom-codex');
  });

  it('copies an existing Codex login into private app-owned storage once', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'powermove-isolation-'));
    const userData = path.join(root, 'user-data');
    const sourceHome = path.join(root, 'real-codex');
    await mkdir(sourceHome, { recursive: true });
    await writeFile(path.join(sourceHome, 'auth.json'), '{"token":"test"}', { mode: 0o600 });

    const runtimeHome = await prepareIsolatedCodexHome(userData, sourceHome);
    expect(runtimeHome).toBe(path.join(userData, ISOLATED_CODEX_HOME_NAME));
    expect((await stat(runtimeHome)).mode & 0o777).toBe(0o700);
    expect(await readdir(runtimeHome)).toEqual([POWERMOVE_AUTH_OWNER_FILE, 'auth.json', 'config.toml']);
    expect((await lstat(path.join(runtimeHome, 'auth.json'))).isSymbolicLink()).toBe(false);
    expect(await readFile(path.join(runtimeHome, 'auth.json'), 'utf8')).toBe('{"token":"test"}');
    expect((await stat(path.join(runtimeHome, 'auth.json'))).mode & 0o777).toBe(0o600);

    await unlink(path.join(runtimeHome, 'auth.json'));
    await prepareIsolatedCodexHome(userData, sourceHome);
    expect(await readdir(runtimeHome)).toEqual([POWERMOVE_AUTH_OWNER_FILE, 'config.toml']);
    expect(await readFile(path.join(runtimeHome, 'config.toml'), 'utf8')).toContain(POWERMOVE_AUTH_STORE_CONFIG);
  });

  it('forces file credentials without discarding private runtime config', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'powermove-isolation-config-'));
    const userData = path.join(root, 'user-data');
    const sourceHome = path.join(root, 'real-codex');
    const runtimeHome = isolatedCodexHome(userData, sourceHome);
    await mkdir(runtimeHome, { recursive: true });
    await writeFile(path.join(runtimeHome, 'config.toml'), [
      'cli_auth_credentials_store = "auto"',
      '[projects."/tmp/project"]',
      'trust_level = "trusted"',
      ''
    ].join('\n'));

    await prepareIsolatedCodexHome(userData, sourceHome);
    await prepareIsolatedCodexHome(userData, sourceHome);

    const config = await readFile(path.join(runtimeHome, 'config.toml'), 'utf8');
    expect(config.match(/cli_auth_credentials_store/gu)).toHaveLength(1);
    expect(config).toContain(POWERMOVE_AUTH_STORE_CONFIG);
    expect(config).toContain('trust_level = "trusted"');
  });

  it('migrates the old shared auth symlink to a private file', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'powermove-isolation-migration-'));
    const userData = path.join(root, 'user-data');
    const sourceHome = path.join(root, 'real-codex');
    const runtimeHome = isolatedCodexHome(userData, sourceHome);
    await mkdir(sourceHome, { recursive: true });
    await mkdir(runtimeHome, { recursive: true });
    await writeFile(path.join(sourceHome, 'auth.json'), '{"token":"source"}', { mode: 0o600 });
    await symlink(path.join(sourceHome, 'auth.json'), path.join(runtimeHome, 'auth.json'));

    await prepareIsolatedCodexHome(userData, sourceHome);

    expect((await lstat(path.join(runtimeHome, 'auth.json'))).isSymbolicLink()).toBe(false);
    expect(await readFile(path.join(runtimeHome, 'auth.json'), 'utf8')).toBe('{"token":"source"}');
    expect(await readFile(path.join(sourceHome, 'auth.json'), 'utf8')).toBe('{"token":"source"}');
  });

  it('avoids colliding with a configured source home inside user data', () => {
    const userData = '/tmp/powermove-user-data';
    const source = path.join(userData, ISOLATED_CODEX_HOME_NAME);
    expect(isolatedCodexHome(userData, source)).toBe(path.join(userData, `${ISOLATED_CODEX_HOME_NAME}-isolated`));
  });

  it('discovers nested and symlinked user skills for the session denylist', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'powermove-user-skills-'));
    const userHome = path.join(root, 'home');
    const externalSkill = path.join(root, 'external-skill');
    await mkdir(path.join(userHome, '.agents', 'skills', 'nested', 'local'), { recursive: true });
    await mkdir(externalSkill, { recursive: true });
    await writeFile(path.join(userHome, '.agents', 'skills', 'nested', 'local', 'SKILL.md'), '# Local');
    await writeFile(path.join(externalSkill, 'SKILL.md'), '# Linked');
    await symlink(externalSkill, path.join(userHome, '.agents', 'skills', 'linked'));

    expect(await discoverUserSkillFiles(userHome)).toEqual([
      path.join(userHome, '.agents', 'skills', 'linked', 'SKILL.md'),
      path.join(userHome, '.agents', 'skills', 'nested', 'local', 'SKILL.md')
    ]);
  });
});
