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
      '--print', '--output-format', 'stream-json', '--safe-mode',
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
});
