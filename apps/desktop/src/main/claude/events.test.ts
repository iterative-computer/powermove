import { describe, expect, it, vi } from 'vitest';

import { LIMITS } from '../../shared/ipc';
import { ClaudeEventParser } from './events';

const SESSION_ID = '11111111-1111-4111-8111-111111111111';

function stream(event: Record<string, unknown>, sessionId = SESSION_ID): Record<string, unknown> {
  return { type: 'stream_event', event, session_id: sessionId };
}

function feed(parser: ClaudeEventParser, events: readonly Record<string, unknown>[], splitAt?: number): void {
  const bytes = new TextEncoder().encode(`${events.map((event) => JSON.stringify(event)).join('\n')}\n`);
  if (splitAt === undefined) parser.push(bytes);
  else {
    parser.push(bytes.subarray(0, splitAt));
    parser.push(bytes.subarray(splitAt));
  }
  parser.finish();
}

describe('Claude stream parser', () => {
  it('streams fragments and tool arguments once while retaining full-message progress', () => {
    const onSessionId = vi.fn();
    const onProgress = vi.fn();
    const onTrace = vi.fn();
    const parser = new ClaudeEventParser({ onSessionId, onProgress, onTrace, projectCwd: '/work/project' });
    const events = [
      { type: 'system', subtype: 'init', session_id: SESSION_ID },
      stream({ type: 'message_start', message: { id: 'message-tool' } }),
      stream({ type: 'content_block_delta', index: 0, delta: { type: 'thinking_delta', thinking: 'Inspecting  the\ncomposition' } }),
      stream({ type: 'content_block_start', index: 1, content_block: { type: 'tool_use', id: 'tool-1', name: 'Bash' } }),
      stream({ type: 'content_block_delta', index: 1, delta: { type: 'input_json_delta', partial_json: '{"command":"rg ' } }),
      stream({ type: 'content_block_delta', index: 1, delta: { type: 'input_json_delta', partial_json: 'foo\\nbar"}' } }),
      stream({ type: 'content_block_stop', index: 1 }),
      stream({ type: 'message_stop' }),
      { type: 'assistant', session_id: SESSION_ID, message: { id: 'message-tool', content: [
        { type: 'thinking', thinking: 'Inspecting  the\ncomposition' },
        { type: 'tool_use', id: 'tool-1', name: 'Bash', input: { command: 'rg foo\nbar' } }
      ] } },
      { type: 'user', message: { content: [{
        type: 'tool_result', tool_use_id: 'tool-1', is_error: false,
        content: [{ type: 'text', text: 'one\ntwo' }, { type: 'text', text: 'three\u0000four' }]
      }] } },
      stream({ type: 'message_start', message: { id: 'message-text' } }),
      stream({ type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'hello ' } }),
      stream({ type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: ' world\n' } }),
      stream({ type: 'message_stop' }),
      { type: 'assistant', message: { id: 'message-text', content: [{ type: 'text', text: 'hello  world\n' }] } },
      { type: 'result', subtype: 'success', is_error: false, result: '{"message":"done"}', structured_output: { message: 'done' }, session_id: SESSION_ID }
    ];

    feed(parser, events, 73);

    expect(onSessionId).toHaveBeenCalledExactlyOnceWith(SESSION_ID);
    expect(onTrace.mock.calls.map(([event]) => event)).toEqual([
      { kind: 'thought', text: 'Inspecting  the\ncomposition' },
      { kind: 'tool-start', itemId: 'tool-1', toolName: 'bash', label: 'Run' },
      { kind: 'tool-start', itemId: 'tool-1', toolName: 'bash', label: 'Run', detail: 'rg foo ⏎ bar' },
      { kind: 'tool-end', itemId: 'tool-1', isError: false, output: 'one\ntwo\nthreefour' },
      { kind: 'answer', text: 'hello ' },
      { kind: 'answer', text: ' world\n' }
    ]);
    expect(onProgress).toHaveBeenCalledWith('Using Run…');
    expect(onProgress).toHaveBeenCalledWith('hello world');
    expect(parser.output).toEqual({ message: 'done' });
    expect(parser.error).toBeNull();
  });

  it('falls back to complete assistant blocks with split labels and details', () => {
    const onTrace = vi.fn();
    const parser = new ClaudeEventParser({ onTrace, projectCwd: '/work/project' });
    feed(parser, [{
      type: 'assistant', message: { id: 'older-cli-message', content: [
        { type: 'thinking', thinking: 'Keep  spacing' },
        { type: 'tool_use', id: 'tool-read', name: 'Read', input: { file_path: '/work/project/src/main.ts' } },
        { type: 'text', text: 'Done  now' }
      ] }
    }]);
    expect(onTrace.mock.calls.map(([event]) => event)).toEqual([
      { kind: 'thought', text: 'Keep  spacing' },
      { kind: 'tool-start', itemId: 'tool-read', toolName: 'read', label: 'Read', detail: 'src/main.ts' },
      { kind: 'answer', text: 'Done  now' }
    ]);
  });

  it('does not replay blocks when Claude splits one streamed message into per-block assistant events', () => {
    const trace: unknown[] = [];
    const parser = new ClaudeEventParser({ onTrace: (step) => trace.push(step) });
    for (const event of [
      stream({ type: 'message_start', message: { id: 'msg-1' } }),
      stream({ type: 'content_block_start', index: 0, content_block: { type: 'text', text: '' } }),
      stream({ type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'Hello ' } }),
      stream({ type: 'content_block_stop', index: 0 }),
      { type: 'assistant', message: { id: 'msg-1', content: [{ type: 'text', text: 'Hello ' }] } },
      stream({ type: 'content_block_start', index: 1, content_block: { type: 'tool_use', id: 'tool-1', name: 'Bash', input: {} } }),
      stream({ type: 'content_block_delta', index: 1, delta: { type: 'input_json_delta', partial_json: '{"command":"ls"}' } }),
      stream({ type: 'content_block_stop', index: 1 }),
      stream({ type: 'message_stop' }),
      { type: 'assistant', message: { id: 'msg-1', content: [{ type: 'tool_use', id: 'tool-1', name: 'Bash', input: { command: 'ls' } }] } }
    ]) parser.push(Buffer.from(`${JSON.stringify(event)}\n`));
    parser.finish();
    expect(trace).toEqual([
      { kind: 'answer', text: 'Hello ' },
      { kind: 'tool-start', itemId: 'tool-1', toolName: 'bash', label: 'Run' },
      { kind: 'tool-start', itemId: 'tool-1', toolName: 'bash', label: 'Run', detail: 'ls' }
    ]);
  });

  it('bounds streamed JSON and marks an oversized tool detail unavailable', () => {
    const onTrace = vi.fn();
    const parser = new ClaudeEventParser({ onTrace });
    feed(parser, [
      stream({ type: 'message_start', message: { id: 'large-json' } }),
      stream({ type: 'content_block_start', index: 0, content_block: { type: 'tool_use', id: 'large-tool', name: 'Bash' } }),
      stream({ type: 'content_block_delta', index: 0, delta: { type: 'input_json_delta', partial_json: `{"command":"${'x'.repeat(64 * 1024)}"}` } }),
      stream({ type: 'content_block_stop', index: 0 })
    ]);
    expect(onTrace).toHaveBeenNthCalledWith(1, { kind: 'tool-start', itemId: 'large-tool', toolName: 'bash', label: 'Run' });
    expect(onTrace).toHaveBeenNthCalledWith(2, { kind: 'tool-start', itemId: 'large-tool', toolName: 'bash', label: 'Run' });
  });

  it('tracks at most 64 concurrent tool blocks', () => {
    const onTrace = vi.fn();
    const parser = new ClaudeEventParser({ onTrace });
    const starts = Array.from({ length: 65 }, (_, index) => stream({
      type: 'content_block_start', index,
      content_block: { type: 'tool_use', id: `tool-${index}`, name: 'Read' }
    }));
    feed(parser, [stream({ type: 'message_start', message: { id: 'many-tools' } }), ...starts]);
    expect(onTrace).toHaveBeenCalledTimes(64);
    expect(onTrace).not.toHaveBeenCalledWith(expect.objectContaining({ itemId: 'tool-64' }));
  });

  it('upserts malformed tool JSON without detail and keeps parsing later deltas', () => {
    const onTrace = vi.fn();
    const parser = new ClaudeEventParser({ onTrace });
    feed(parser, [
      stream({ type: 'message_start', message: { id: 'malformed' } }),
      stream({ type: 'content_block_start', index: 0, content_block: { type: 'tool_use', id: 'bad-tool', name: 'Grep' } }),
      stream({ type: 'content_block_delta', index: 0, delta: { type: 'input_json_delta', partial_json: '{' } }),
      stream({ type: 'content_block_stop', index: 0 }),
      stream({ type: 'content_block_delta', index: 1, delta: { type: 'text_delta', text: 'still streaming' } })
    ]);
    expect(onTrace.mock.calls.map(([event]) => event)).toEqual([
      { kind: 'tool-start', itemId: 'bad-tool', toolName: 'grep', label: 'Search' },
      { kind: 'tool-start', itemId: 'bad-tool', toolName: 'grep', label: 'Search' },
      { kind: 'answer', text: 'still streaming' }
    ]);
  });

  it('bounds each text fragment without trimming it', () => {
    const onTrace = vi.fn();
    const parser = new ClaudeEventParser({ onTrace });
    const text = ` ${'x'.repeat(LIMITS.codexTraceChars + 10)} `;
    feed(parser, [
      stream({ type: 'message_start', message: { id: 'bounded-text' } }),
      stream({ type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text } })
    ]);
    const fragment = onTrace.mock.calls[0]![0].text as string;
    expect(fragment).toHaveLength(LIMITS.codexTraceChars);
    expect(fragment.startsWith(' ')).toBe(true);
  });

  it('includes bounded error output from string tool results', () => {
    const onTrace = vi.fn();
    const parser = new ClaudeEventParser({ onTrace });
    const output = Array.from({ length: 12 }, (_, index) => `line ${index}`).join('\n');
    feed(parser, [{ type: 'user', message: { content: [{
      type: 'tool_result', tool_use_id: 'failed-tool', is_error: true, content: output
    }] } }]);
    expect(onTrace).toHaveBeenCalledWith({
      kind: 'tool-end', itemId: 'failed-tool', isError: true,
      output: Array.from({ length: 8 }, (_, index) => `line ${index}`).join('\n')
    });
  });

  it('keeps CLI failures out of successful structured output', () => {
    const parser = new ClaudeEventParser();
    feed(parser, [{ type: 'result', subtype: 'error', is_error: true, result: 'Authentication required' }]);
    expect(parser.error).toBe('Authentication required');
    expect(parser.output).toBeUndefined();
  });
});
