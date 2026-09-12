import { mkdtemp, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { describe, expect, it } from 'vitest';

import { isolatedClaudeEnvironment, prepareIsolatedClaudeHome } from './isolation';

describe('Claude runtime isolation', () => {
  it('uses an app-owned config directory without exposing credentials to Powermove', async () => {
    const userData = await mkdtemp(path.join(tmpdir(), 'powermove-claude-isolation-'));
    const home = await prepareIsolatedClaudeHome(userData);
    expect(home).toBe(path.join(userData, 'claude-runtime'));
    expect((await stat(home)).isDirectory()).toBe(true);
    expect(isolatedClaudeEnvironment(home)).toMatchObject({
      CLAUDE_CONFIG_DIR: home,
      CLAUDE_CODE_SAFE_MODE: '1'
    });
  });
});
