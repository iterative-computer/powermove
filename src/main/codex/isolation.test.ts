import { lstat, mkdtemp, mkdir, readdir, readlink, stat, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

import {
  ISOLATED_CODEX_HOME_NAME,
  discoverUserSkillFiles,
  isolatedCodexHome,
  prepareIsolatedCodexHome,
  userCodexHome
} from './isolation';

describe('isolated Codex home', () => {
  it('uses CODEX_HOME only as the credential source when configured', () => {
    expect(userCodexHome({ CODEX_HOME: ' /tmp/custom-codex ' })).toBe('/tmp/custom-codex');
  });

  it('creates a private app-owned home containing only an auth symlink', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'powermove-isolation-'));
    const userData = path.join(root, 'user-data');
    const sourceHome = path.join(root, 'real-codex');
    await mkdir(sourceHome, { recursive: true });
    await writeFile(path.join(sourceHome, 'auth.json'), '{"token":"test"}', { mode: 0o600 });

    const runtimeHome = await prepareIsolatedCodexHome(userData, sourceHome);
    expect(runtimeHome).toBe(path.join(userData, ISOLATED_CODEX_HOME_NAME));
    expect((await stat(runtimeHome)).mode & 0o777).toBe(0o700);
    expect(await readdir(runtimeHome)).toEqual(['auth.json']);
    expect((await lstat(path.join(runtimeHome, 'auth.json'))).isSymbolicLink()).toBe(true);
    expect(await readlink(path.join(runtimeHome, 'auth.json'))).toBe(path.join(sourceHome, 'auth.json'));
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
