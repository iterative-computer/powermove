import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import { LIMITS } from '../../shared/ipc';
import {
  CodexEventParser,
  MAX_CODEX_EVENT_LINE_BYTES,
  progressForCodexEvent,
  threadIdForCodexEvent
} from './events';

const bytes = (text: string): Uint8Array => new TextEncoder().encode(text);

describe('CodexEventParser', () => {
  it('parses the recorded transcript across a partial UTF-8 boundary', () => {
    const transcript = readFileSync(new URL('./__fixtures__/recorded-transcript.ndjson', import.meta.url));
    const split = transcript.indexOf(Buffer.from('café')) + Buffer.byteLength('caf') + 1;
    const progress: string[] = [];
    const ids: string[] = [];
    const parser = new CodexEventParser({
      onProgress: (text) => progress.push(text),
      onThreadId: (id) => ids.push(id)
    });

    parser.push(transcript.subarray(0, split));
    parser.push(transcript.subarray(split));
    parser.finish();

    expect(ids).toEqual(['thread-recorded-1']);
    expect(progress).toEqual([
      'Working with project files and shell tools…',
      'Reviewing the café timeline',
      'Preparing the final animation'
    ]);
  });

  it('decodes a multi-byte character split across chunks and flushes a final line without a newline', () => {
    const progress: string[] = [];
    const parser = new CodexEventParser({ onProgress: (text) => progress.push(text) });
    const transcript = bytes(
      JSON.stringify({ type: 'item.completed', item: { type: 'reasoning', text: 'Opening café files…' } })
    );
    const split = transcript.indexOf(0xc3) + 1;

    parser.push(transcript.slice(0, split));
    parser.push(transcript.slice(split));
    expect(progress).toEqual([]);
    parser.finish();

    expect(progress).toEqual(['Opening café files…']);
  });

  it('skips malformed JSON and continues with subsequent events', () => {
    const progress: string[] = [];
    const parser = new CodexEventParser({ onProgress: (text) => progress.push(text) });

    parser.push(
      bytes(
        'not-json\n' +
          JSON.stringify({ type: 'item.started', item: { type: 'web_search' } }) +
          '\n{still broken]\n'
      )
    );
    parser.finish();

    expect(progress).toEqual(['Researching on the web…']);
  });

  it('drops an over-limit line with one warning and resumes at the next newline', () => {
    const progress: string[] = [];
    const warnings: string[] = [];
    const parser = new CodexEventParser({
      onProgress: (text) => progress.push(text),
      onWarning: (warning) => warnings.push(warning)
    });

    parser.push(bytes('x'.repeat(MAX_CODEX_EVENT_LINE_BYTES)));
    parser.push(bytes('x'));
    parser.push(bytes('more ignored'));
    parser.push(
      bytes('\n' + JSON.stringify({ type: 'item.started', item: { type: 'file_change' } }) + '\n')
    );

    expect(warnings).toHaveLength(1);
    expect(progress).toEqual(['Preparing project files…']);
  });

  it('captures only the first thread.started id', () => {
    const ids: string[] = [];
    const parser = new CodexEventParser({ onThreadId: (id) => ids.push(id) });
    parser.push(
      bytes(
        [
          { type: 'thread.started', thread_id: 'thread-one' },
          { type: 'thread.started', thread_id: 'thread-two' }
        ]
          .map((event) => JSON.stringify(event))
          .join('\n') + '\n'
      )
    );

    expect(ids).toEqual(['thread-one']);
    expect(parser.capturedThreadId).toBe('thread-one');
  });
});

describe('Codex progress mapping', () => {
  it.each([
    ['command_execution', 'Working with project files and shell tools…'],
    ['web_search', 'Researching on the web…'],
    ['computer_use', 'Operating an application on this Mac…'],
    ['image_generation', 'Generating a visual deliverable…'],
    ['file_change', 'Preparing project files…']
  ])('maps an item.started %s event', (type, expected) => {
    expect(progressForCodexEvent({ type: 'item.started', item: { type } })).toBe(expected);
  });

  it('maps MCP calls, preferring tool and limiting its display length', () => {
    expect(
      progressForCodexEvent({
        type: 'item.started',
        item: { type: 'mcp_tool_call', tool: 't'.repeat(90), name: 'ignored' }
      })
    ).toBe(`Using the installed ${'t'.repeat(80)} integration…`);
    expect(progressForCodexEvent({ type: 'item.started', item: { type: 'mcp_tool_call' } })).toBe(
      'Using the installed integration integration…'
    );
  });

  it('normalizes and truncates reasoning progress', () => {
    const text = `  ${'a'.repeat(LIMITS.codexProgressChars + 20)}\n next  `;
    const result = progressForCodexEvent({ type: 'item.completed', item: { type: 'reasoning', text } });
    expect(result).toHaveLength(LIMITS.codexProgressChars);
    expect(result).toBe('a'.repeat(LIMITS.codexProgressChars));
  });

  it('extracts only approved fields from structured completion text', () => {
    expect(
      progressForCodexEvent({
        type: 'item.completed',
        item: {
          type: 'agent_message',
          text: JSON.stringify({ command: 'rm secret', message: 'A safe progress message' })
        }
      })
    ).toBe('A safe progress message');
    expect(
      progressForCodexEvent({
        type: 'item.completed',
        item: { type: 'reasoning', text: JSON.stringify({ command: 'never stream me' }) }
      })
    ).toBeNull();
  });

  it('ignores unsupported, empty, and short structured updates', () => {
    expect(progressForCodexEvent({ type: 'item.started', item: { type: 'unknown' } })).toBeNull();
    expect(
      progressForCodexEvent({ type: 'item.completed', item: { type: 'command_execution', text: 'secret' } })
    ).toBeNull();
    expect(
      progressForCodexEvent({ type: 'item.completed', item: { type: 'agent_message', text: '{"status":"short"}' } })
    ).toBeNull();
    expect(
      progressForCodexEvent({
        type: 'item.completed',
        item: { type: 'agent_message', text: '["command that must not stream"]' }
      })
    ).toBeNull();
  });
});

describe('threadIdForCodexEvent', () => {
  it('sniffs a valid thread.started event only', () => {
    expect(threadIdForCodexEvent({ type: 'thread.started', thread_id: 'abc-123' })).toBe('abc-123');
    expect(threadIdForCodexEvent({ type: 'thread.started', thread_id: '' })).toBeNull();
    expect(threadIdForCodexEvent({ type: 'item.started', thread_id: 'abc-123' })).toBeNull();
  });
});
