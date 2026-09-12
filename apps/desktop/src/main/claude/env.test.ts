import path from 'node:path';

import { describe, expect, it } from 'vitest';

import { bundledClaudeCandidates, DEVELOPMENT_CLAUDE_RELATIVE_PATH, PACKAGED_CLAUDE_RELATIVE_PATH } from './env';

describe('Claude runtime discovery', () => {
  it('prefers the packaged binary and retains the official dependency in development', () => {
    expect(bundledClaudeCandidates('/app', '/resources')).toEqual([
      path.join('/resources', PACKAGED_CLAUDE_RELATIVE_PATH),
      path.join('/app', DEVELOPMENT_CLAUDE_RELATIVE_PATH)
    ]);
  });
});
