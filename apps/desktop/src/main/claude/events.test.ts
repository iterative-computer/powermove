import { describe, expect, it, vi } from 'vitest';

import { ClaudeEventParser } from './events';

describe('Claude stream parser', () => {
  it('captures sessions, structured output, and renderer-safe activity', () => {
    const onSessionId = vi.fn();
    const onProgress = vi.fn();
    const onTrace = vi.fn();
    const parser = new ClaudeEventParser({ onSessionId, onProgress, onTrace });
    const lines = [
      { type: 'system', subtype: 'init', session_id: '11111111-1111-4111-8111-111111111111' },
      { type: 'assistant', session_id: '11111111-1111-4111-8111-111111111111', message: { content: [
        { type: 'thinking', thinking: 'Inspecting the composition' },
        { type: 'tool_use', id: 'tool-1', name: 'Read', input: { file_path: 'project.json' } },
        { type: 'text', text: 'Preparing the editable result' }
      ] } },
      { type: 'user', message: { content: [{ type: 'tool_result', tool_use_id: 'tool-1', is_error: false }] } },
      { type: 'result', subtype: 'success', is_error: false, result: '{"message":"done"}', structured_output: { message: 'done' }, session_id: '11111111-1111-4111-8111-111111111111' }
    ].map((line) => JSON.stringify(line)).join('\n') + '\n';
    const bytes = new TextEncoder().encode(lines);
    parser.push(bytes.subarray(0, 73));
    parser.push(bytes.subarray(73));
    parser.finish();

    expect(onSessionId).toHaveBeenCalledExactlyOnceWith('11111111-1111-4111-8111-111111111111');
    expect(onTrace).toHaveBeenCalledWith({ kind: 'thought', text: 'Inspecting the composition' });
    expect(onTrace).toHaveBeenCalledWith({ kind: 'tool-start', itemId: 'tool-1', toolName: 'read', label: 'Read' });
    expect(onTrace).toHaveBeenCalledWith({ kind: 'tool-end', itemId: 'tool-1', isError: false });
    expect(onProgress).toHaveBeenCalledWith('Using Read…');
    expect(parser.output).toEqual({ message: 'done' });
    expect(parser.error).toBeNull();
  });

  it('keeps CLI failures out of successful structured output', () => {
    const parser = new ClaudeEventParser();
    parser.push(new TextEncoder().encode(`${JSON.stringify({
      type: 'result', subtype: 'error', is_error: true, result: 'Authentication required'
    })}\n`));
    parser.finish();
    expect(parser.error).toBe('Authentication required');
    expect(parser.output).toBeUndefined();
  });
});
