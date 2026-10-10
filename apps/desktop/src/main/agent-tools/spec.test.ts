import { describe, expect, it } from 'vitest';

import {
  POWERMOVE_AGENT_TOOLS,
  POWERMOVE_APP_AGENT_TOOLS,
  POWERMOVE_LIVE_INSPECTION_TOOL_NAMES,
  POWERMOVE_MCP_TOOL_NAMES,
  POWERMOVE_STORE_READONLY_TOOL_NAMES,
  POWERMOVE_STORE_TOOL_NAMES
} from './spec';

describe('Powermove agent tool spec', () => {
  it('offers bounded extension inspection to app import runs without composition tools', () => {
    const tool = POWERMOVE_APP_AGENT_TOOLS.find(tool => tool.name === 'inspect_creative_extension');
    expect(tool?.inputSchema).toMatchObject({ additionalProperties: false, required: ['appId', 'toolId'], properties: {
      toolId: { pattern: '^[a-f0-9]{32}$' }, path: { minLength: 1, maxLength: 1024 }
    } });
    expect(POWERMOVE_MCP_TOOL_NAMES).toContain('mcp__powermove__inspect_creative_extension');
    expect(POWERMOVE_APP_AGENT_TOOLS.some(tool => tool.name === 'apply_commands')).toBe(false);
  });
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

  it('declares every store tool with a closed schema and MCP name', () => {
    for (const name of POWERMOVE_STORE_TOOL_NAMES) {
      const tool = POWERMOVE_AGENT_TOOLS.find((candidate) => candidate.name === name);
      expect(tool, name).toBeDefined();
      expect(tool?.inputSchema['additionalProperties']).toBe(false);
      expect(POWERMOVE_MCP_TOOL_NAMES).toContain(`mcp__powermove__${name}`);
    }
  });

  it('offers the store tools in app runs (no composition needed)', () => {
    const appNames = new Set(POWERMOVE_APP_AGENT_TOOLS.map((tool) => tool.name));
    for (const name of POWERMOVE_STORE_TOOL_NAMES) expect(appNames.has(name), name).toBe(true);
  });

  it('offers only the read-only store tools to editor inspection runs', () => {
    const inspection = new Set<string>(POWERMOVE_LIVE_INSPECTION_TOOL_NAMES);
    for (const name of POWERMOVE_STORE_READONLY_TOOL_NAMES) expect(inspection.has(name), name).toBe(true);
    for (const write of ['store_install', 'store_update', 'store_uninstall', 'store_publish', 'store_publish_prepare']) {
      expect(inspection.has(write), write).toBe(false);
    }
  });
});
