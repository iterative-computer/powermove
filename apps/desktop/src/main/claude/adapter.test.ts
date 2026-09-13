import { describe, expect, it } from 'vitest';

import { buildClaudeArgv, CLAUDE_PROJECT_SANDBOX_SETTINGS } from './adapter';

const schema = { type: 'object', required: ['message'], properties: { message: { type: 'string' } } };

describe('Claude CLI adapter', () => {
  it('uses structured streaming, safe mode, isolated settings, and strict project sandboxing', () => {
    const argv = buildClaudeArgv({
      schema,
      prompt: 'Make the title bounce',
      imagePaths: ['/tmp/reference.png'],
      model: 'sonnet',
      reasoningEffort: 'high',
      sessionId: '11111111-1111-4111-8111-111111111111',
      access: 'project',
      extensionsDir: '/tmp/extensions',
      instructions: 'Work inside Powermove.'
    });

    expect(argv).toEqual(expect.arrayContaining([
      '--print', '--output-format', 'stream-json', '--include-partial-messages', '--safe-mode',
      '--setting-sources', '', '--strict-mcp-config', '--mcp-config', '{"mcpServers":{}}',
      '--json-schema', JSON.stringify(schema), '--model', 'sonnet', '--effort', 'high',
      '--resume', '11111111-1111-4111-8111-111111111111',
      '--permission-mode', 'acceptEdits', '--settings', CLAUDE_PROJECT_SANDBOX_SETTINGS,
      '--add-dir', '/tmp/extensions'
    ]));
    expect(argv).not.toContain('--dangerously-skip-permissions');
    expect(argv.at(-1)).toContain('/tmp/reference.png');
    expect(JSON.parse(CLAUDE_PROJECT_SANDBOX_SETTINGS)).toMatchObject({
      sandbox: { enabled: true, allowUnsandboxedCommands: false, failIfUnavailable: true }
    });
  });

  it('only enables unrestricted CLI permissions after computer consent is handled by main', () => {
    const argv = buildClaudeArgv({
      schema,
      prompt: 'Use the Mac',
      imagePaths: [],
      model: null,
      reasoningEffort: null,
      sessionId: null,
      access: 'computer'
    });
    expect(argv).toContain('--dangerously-skip-permissions');
    expect(argv).not.toContain('--settings');
  });

  it('gives the native Claude harness only the run-scoped Powermove MCP tools', () => {
    const nativeTools = {
      command: '/Applications/Powermove.app/Contents/MacOS/Powermove',
      args: ['/Applications/Powermove.app/Contents/Resources/agent-tools/mcp-server.mjs'],
      env: { ELECTRON_RUN_AS_NODE: '1', POWERMOVE_AGENT_TOOL_TOKEN: 'secret' }
    };
    const argv = buildClaudeArgv({
      schema,
      prompt: 'Edit the live composition',
      imagePaths: [],
      model: 'sonnet',
      reasoningEffort: 'high',
      sessionId: null,
      access: 'project',
      extensionsDir: '/tmp/extensions',
      instructions: 'Use Powermove tools.',
      nativeTools
    });
    const config = JSON.parse(argv[argv.indexOf('--mcp-config') + 1]!);
    expect(config).toEqual({ mcpServers: { powermove: { type: 'stdio', ...nativeTools } } });
    const allowed = argv[argv.indexOf('--allowedTools') + 1]!;
    expect(allowed).toContain('mcp__powermove__get_project_state');
    expect(allowed).toContain('mcp__powermove__apply_commands');
  });

  it('limits editor runs to live Powermove inspection and capture tools', () => {
    const nativeTools = {
      command: '/Applications/Powermove.app/Contents/MacOS/Powermove',
      args: ['/Applications/Powermove.app/Contents/Resources/agent-tools/mcp-server.mjs'],
      env: { ELECTRON_RUN_AS_NODE: '1', POWERMOVE_AGENT_TOOL_TOKEN: 'secret' }
    };
    const argv = buildClaudeArgv({
      schema,
      prompt: 'Inspect the built-in inspector',
      imagePaths: [],
      model: 'sonnet',
      reasoningEffort: 'high',
      sessionId: null,
      access: 'editor',
      nativeTools
    });
    const allowed = argv[argv.indexOf('--allowedTools') + 1]!;
    expect(allowed).toContain('mcp__powermove__capture_panel');
    expect(allowed).toContain('mcp__powermove__render_frames');
    expect(allowed).not.toContain('mcp__powermove__apply_commands');
    expect(allowed).not.toContain('mcp__powermove__computer_use_panel');
  });
});
