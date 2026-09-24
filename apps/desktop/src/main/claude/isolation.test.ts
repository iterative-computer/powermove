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
    const previous = process.env.CLAUDE_CODE_SAFE_MODE;
    try {
      process.env.CLAUDE_CODE_SAFE_MODE = '1';
      const environment = isolatedClaudeEnvironment(home);
      expect(environment.CLAUDE_CONFIG_DIR).toBe(home);
      expect(environment.CLAUDE_CODE_SAFE_MODE).toBeUndefined();
    } finally {
      if (previous === undefined) delete process.env.CLAUDE_CODE_SAFE_MODE;
      else process.env.CLAUDE_CODE_SAFE_MODE = previous;
    }
  });
});
