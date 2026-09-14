import path from 'node:path';

import { execFile } from 'node:child_process';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { bundledClaudeCandidates, discoverClaudeBinary, resetClaudeEnvironmentCacheForTests, DEVELOPMENT_CLAUDE_RELATIVE_PATH, PACKAGED_CLAUDE_RELATIVE_PATH } from './env';

vi.mock('node:child_process', () => ({ execFile: vi.fn() }));

afterEach(() => { vi.unstubAllEnvs(); vi.clearAllMocks(); resetClaudeEnvironmentCacheForTests(); });

describe('Claude runtime discovery', () => {
  it('does not start a login shell when an explicit executable is available', async () => {
    vi.stubEnv('CLAUDE_BINARY', '/bin/sh');
    // Return immediately in the old implementation so this proves the
    // unnecessary probe without invoking the user's shell startup files.
    vi.mocked(execFile).mockImplementation(((_file: string, _args: string[], _options: unknown, callback: Function) => {
      callback(null, '/bin/sh\n');
      return {};
    }) as any);
    await expect(discoverClaudeBinary()).resolves.toBe('/bin/sh');
    expect(execFile).not.toHaveBeenCalled();
  });

  it('uses the bundled runtime before the shell and only probes as a cached fallback', async () => {
    vi.stubEnv('CLAUDE_BINARY', undefined);
    vi.mocked(execFile).mockImplementation(((_file: string, _args: string[], _options: unknown, callback: Function) => {
      callback(null, '/bin/sh\n');
      return {};
    }) as any);
    await expect(discoverClaudeBinary(null, { bundledCandidates: ['/bin/sh'] })).resolves.toBe('/bin/sh');
    expect(execFile).not.toHaveBeenCalled();
    await expect(discoverClaudeBinary(null, { bundledCandidates: [] })).resolves.toBe('/bin/sh');
    await expect(discoverClaudeBinary(null, { bundledCandidates: [] })).resolves.toBe('/bin/sh');
    expect(execFile).toHaveBeenCalledTimes(1);
  });

  it('prefers the packaged binary and retains the official dependency in development', () => {
    expect(bundledClaudeCandidates('/app', '/resources')).toEqual([
      path.join('/resources', PACKAGED_CLAUDE_RELATIVE_PATH),
      path.join('/app', DEVELOPMENT_CLAUDE_RELATIVE_PATH)
    ]);
  });
});
