import path from 'node:path';

import { describe, expect, it } from 'vitest';

import { LIMITS } from '../../shared/ipc';
import { fragmentText, humanLabel, outputExcerpt, toolDetail } from './trace-format';

describe('agent trace formatting', () => {
  it('uses concise labels for built-in, MCP, snake-case and camel-case tools', () => {
    expect([
      'Read', 'Edit', 'MultiEdit', 'Write', 'Bash', 'Grep', 'Glob', 'LS', 'WebSearch',
      'WebFetch', 'Agent', 'Task', 'TodoWrite', 'NotebookEdit'
    ].map(humanLabel)).toEqual([
      'Read', 'Edit', 'Edit', 'Write', 'Run', 'Search', 'Search', 'List', 'Search web',
      'Fetch', 'Agent', 'Agent', 'Plan', 'Edit notebook'
    ]);
    expect(humanLabel('mcp__powermove__get_panel_state')).toBe('Get panel state');
    expect(humanLabel('capturePanel')).toBe('Capture panel');
    expect(humanLabel('a_very_long_tool_name_that_cannot_fit_inside_the_label')).toHaveLength(40);
  });

  it('extracts tool-specific details and safely makes project paths relative', () => {
    const cwd = path.resolve('/workspace/project');
    expect(toolDetail('Bash', { command: 'bun test' }, cwd)).toBe('bun test');
    expect(toolDetail('Read', { file_path: path.join(cwd, 'src/main.ts') }, cwd)).toBe(path.join('src', 'main.ts'));
    expect(toolDetail('Write', { file_path: '/workspace/project-copy/main.ts' }, cwd)).toBe('/workspace/project-copy/main.ts');
    expect(toolDetail('Grep', { pattern: 'hello', path: path.join(cwd, 'src') }, cwd)).toBe(`hello in src`);
    expect(toolDetail('Glob', { pattern: '**/*.ts' })).toBe('**/*.ts');
    expect(toolDetail('WebSearch', { query: 'streaming APIs' })).toBe('streaming APIs');
    expect(toolDetail('WebFetch', { url: 'https://example.com/docs' })).toBe('https://example.com/docs');
    expect(toolDetail('Task', { description: 'Inspect the code' })).toBe('Inspect the code');
    expect(toolDetail('custom_tool', { count: 2, panelId: 'inspector' })).toBe('inspector');
    expect(toolDetail('custom_tool', null)).toBe('');
  });

  it('keeps details single-line, control-free and bounded', () => {
    const detail = toolDetail('Bash', { command: `first\r\nsecond\u0000${'x'.repeat(300)}` });
    expect(detail).toContain('first ⏎ second');
    expect(detail).not.toMatch(/[\r\n\u0000]/u);
    expect(detail.length).toBe(LIMITS.codexToolDetailChars);
  });

  it('preserves fragment whitespace while stripping controls and bounding each fragment', () => {
    expect(fragmentText('  hello\t \n world\u0000\u0007  ')).toBe('  hello\t \n world  ');
    expect(fragmentText('x'.repeat(LIMITS.codexTraceChars + 4))).toHaveLength(LIMITS.codexTraceChars);
    expect(fragmentText({ text: 'no' })).toBe('');
  });

  it('extracts useful output, preserving lines while limiting line and character counts', () => {
    expect(outputExcerpt([{ type: 'text', text: 'one\tline' }, { type: 'text', text: 'two\u0000' }])).toBe('one line\ntwo');
    expect(outputExcerpt({ error: 'failed\nbecause' })).toBe('failed\nbecause');
    expect(outputExcerpt(Array.from({ length: 10 }, (_, index) => `line ${index + 1}`)).split('\n')).toHaveLength(8);
    expect(outputExcerpt('x'.repeat(LIMITS.codexToolOutputChars + 20))).toHaveLength(LIMITS.codexToolOutputChars);
  });
});
