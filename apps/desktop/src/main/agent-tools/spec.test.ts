import { describe, expect, it } from 'vitest';

import { POWERMOVE_AGENT_TOOLS, POWERMOVE_MCP_TOOL_NAMES } from './spec';

describe('Powermove agent tool spec', () => {
  it('publishes the closed fork_builtin_extension schema to MCP clients', () => {
    const tool = POWERMOVE_AGENT_TOOLS.find((candidate) => candidate.name === 'fork_builtin_extension');

    expect(tool).toEqual({
      name: 'fork_builtin_extension',
      description: expect.stringContaining('extensions array with action created'),
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          id: { type: 'string', pattern: '^[a-z0-9][a-z0-9-]{1,63}$' },
          forkId: { type: 'string', pattern: '^[a-z0-9][a-z0-9-]{1,63}$' }
        },
        required: ['id']
      }
    });
    expect(POWERMOVE_MCP_TOOL_NAMES).toContain('mcp__powermove__fork_builtin_extension');
  });
});
