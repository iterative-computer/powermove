import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import { LIMITS } from '../../shared/ipc';
import type { CodexTraceEvent } from '../../shared/ipc';
import {
  CodexEventParser,
  MAX_CODEX_EVENT_LINE_BYTES,
  progressForCodexEvent,
  traceForCodexEvent,
  threadIdForCodexEvent
} from './events';

const bytes = (text: string): Uint8Array => new TextEncoder().encode(text);

describe('CodexEventParser', () => {
  it('parses the recorded transcript across a partial UTF-8 boundary', () => {
    const transcript = readFileSync(new URL('./__fixtures__/recorded-transcript.ndjson', import.meta.url));
    const split = transcript.indexOf(Buffer.from('café')) + Buffer.byteLength('caf') + 1;
    const progress: string[] = [];
    const ids: string[] = [];
    const trace: CodexTraceEvent[] = [];
    const parser = new CodexEventParser({
      onProgress: (text) => progress.push(text),
      onTrace: (step) => trace.push(step),
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
    expect(trace).toEqual([
      { kind: 'tool-start', itemId: '0', toolName: 'bash', label: 'bash · pwd' },
      { kind: 'tool-end', itemId: '0', isError: false },
      { kind: 'thought', text: 'Reviewing the café timeline' },
      { kind: 'answer', text: 'Preparing the final animation' }
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

describe('Codex trace mapping', () => {
  it.each([
    [
      { id: 'cmd-1', type: 'command_execution', command: 'npm run build' },
      { kind: 'tool-start', itemId: 'cmd-1', toolName: 'bash', label: 'bash · npm run build' }
    ],
    [
      { id: 'edit-1', type: 'file_change', changes: [{ path: '/tmp/src/main.ts' }, { path: 'styles/app.css' }, { path: 'README.md' }, { path: 'ignored.ts' }] },
      { kind: 'tool-start', itemId: 'edit-1', toolName: 'edit', label: 'edit · main.ts, app.css, README.md' }
    ],
    [
      { id: 'search-1', type: 'web_search', query: 'Codex structured event schema' },
      { kind: 'tool-start', itemId: 'search-1', toolName: 'search', label: 'search · Codex structured event schema' }
    ],
    [
      { id: 'image-1', type: 'image_generation' },
      { kind: 'tool-start', itemId: 'image-1', toolName: 'image', label: 'image' }
    ],
    [
      { id: 'computer-1', type: 'computer_use' },
      { kind: 'tool-start', itemId: 'computer-1', toolName: 'computer', label: 'computer' }
    ],
    [
      { id: 'mcp-1', type: 'mcp_tool_call', tool: 'github', name: 'create_issue' },
      { kind: 'tool-start', itemId: 'mcp-1', toolName: 'github', label: 'github · create_issue' }
    ]
  ])('maps a %# tool start with its useful payload', (item, expected) => {
    expect(traceForCodexEvent({ type: 'item.started', item })).toEqual(expected);
  });

  it.each([
    ['completed', false],
    ['failed', true],
    ['error', true],
    [undefined, false]
  ])('correlates a %s tool completion', (status, isError) => {
    expect(traceForCodexEvent({
      type: 'item.completed',
      item: { id: 'cmd-2', type: 'command_execution', status }
    })).toEqual({ kind: 'tool-end', itemId: 'cmd-2', isError });
  });

  it('preserves prose newlines and maps reasoning and agent messages separately', () => {
    expect(traceForCodexEvent({
      type: 'item.completed', item: { type: 'reasoning', text: '  First line\nSecond line  ' }
    })).toEqual({ kind: 'thought', text: 'First line\nSecond line' });
    expect(traceForCodexEvent({
      type: 'item.completed', item: { type: 'agent_message', text: 'Done.' }
    })).toEqual({ kind: 'answer', text: 'Done.' });
  });

  it('allows only display fields from structured thought and answer payloads', () => {
    expect(traceForCodexEvent({
      type: 'item.completed',
      item: { type: 'reasoning', text: JSON.stringify({ command: 'curl secret', critique: 'Checking the layout' }) }
    })).toEqual({ kind: 'thought', text: 'Checking the layout' });
    expect(traceForCodexEvent({
      type: 'item.completed',
      item: { type: 'agent_message', text: JSON.stringify({ command: 'never expose this', files: ['/private/key'] }) }
    })).toBeNull();
  });

  it('caps thought and answer strings at the shared trace limit', () => {
    for (const type of ['reasoning', 'agent_message'] as const) {
      const result = traceForCodexEvent({
        type: 'item.completed', item: { type, text: 'x'.repeat(LIMITS.codexTraceChars + 50) }
      });
      expect(result?.kind).toBe(type === 'reasoning' ? 'thought' : 'answer');
      if (result?.kind === 'thought' || result?.kind === 'answer') {
        expect(result.text).toHaveLength(LIMITS.codexTraceChars);
      }
    }
  });

  it('caps per-kind label details and strips label controls and newlines', () => {
    const command = traceForCodexEvent({
      type: 'item.started',
      item: { id: 'c'.repeat(240), type: 'command_execution', command: `${'x'.repeat(65)}\n\u0000tail` }
    });
    expect(command).toMatchObject({ kind: 'tool-start', toolName: 'bash' });
    if (command?.kind === 'tool-start') {
      expect(command.itemId).toHaveLength(120);
      expect(command.label).toHaveLength('bash · '.length + 68);
      expect(command.label).not.toMatch(/[\n\r\u0000]/);
    }

    const search = traceForCodexEvent({
      type: 'item.started', item: { id: 'search', type: 'web_search', query: 'q'.repeat(100) }
    });
    expect(search).toEqual({
      kind: 'tool-start', itemId: 'search', toolName: 'search', label: `search · ${'q'.repeat(60)}`
    });

    const mcp = traceForCodexEvent({
      type: 'item.started', item: { id: 'mcp', type: 'mcp_tool_call', tool: 't'.repeat(60), name: 'n'.repeat(60) }
    });
    expect(mcp).toEqual({
      kind: 'tool-start', itemId: 'mcp', toolName: 't'.repeat(40),
      label: `${'t'.repeat(40)} · ${'n'.repeat(40)}`
    });
  });

  it('ignores unknown, malformed, and non-item trace events', () => {
    expect(traceForCodexEvent({ type: 'item.started', item: { id: 'x', type: 'unknown', command: 'secret' } })).toBeNull();
    expect(traceForCodexEvent({ type: 'item.completed', item: { type: 'reasoning', text: '   ' } })).toBeNull();
    expect(traceForCodexEvent({ type: 'thread.started', thread_id: 'thread-1' })).toBeNull();
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
