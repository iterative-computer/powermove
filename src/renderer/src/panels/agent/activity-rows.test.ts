import { describe, expect, it } from 'vitest';

import { activityRows, joinActions, toolAction, type ActivityRow, type TraceStep } from './activity-rows';

/* Rows are a discriminated union and the array index is unchecked; tests reach
   in by position, so one loose accessor keeps the assertions readable. */
const at = (rows: ActivityRow[], index = 0): any => rows[index];

const tool = (over: Partial<Extract<TraceStep, { kind: 'tool' }>>): TraceStep => ({
  kind: 'tool',
  id: '1',
  toolName: 'bash',
  label: 'Ran a command',
  status: 'done',
  ...over
} as TraceStep);

describe('activityRows', () => {
  it('groups consecutive tool calls into one natural-language row', () => {
    const rows = activityRows([
      tool({ id: '1', toolName: 'bash' }),
      tool({ id: '2', toolName: 'edit_file' }),
      tool({ id: '3', toolName: 'web_search' })
    ]);

    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      kind: 'tools',
      id: 'tools-1',
      label: 'Ran commands, edited files, and searched the web',
      status: 'done',
      renderKey: 'tools-1-0',
      pulsing: false
    });
  });

  it('dedupes repeated actions instead of repeating the phrase', () => {
    const rows = activityRows([
      tool({ id: '1', toolName: 'bash' }),
      tool({ id: '2', toolName: 'shell' }),
      tool({ id: '3', toolName: 'write_file' })
    ]);

    expect(at(rows).label).toBe('Ran commands and edited files');
  });

  it('keeps individual labels as detail once a multi-step group settles', () => {
    const rows = activityRows([
      tool({ id: '1', toolName: 'bash', label: 'npm test' }),
      tool({ id: '2', toolName: 'edit', label: 'Timeline.svelte' })
    ]);

    expect(at(rows).detail).toEqual(['npm test', 'Timeline.svelte']);
  });

  it('omits detail while the group is still running, and for single steps', () => {
    const running = activityRows([
      tool({ id: '1', toolName: 'bash', label: 'npm test', status: 'running' }),
      tool({ id: '2', toolName: 'edit', label: 'Timeline.svelte' })
    ]);
    expect(at(running).detail).toBeUndefined();

    const single = activityRows([tool({ id: '1', toolName: 'bash', label: 'npm test' })]);
    expect(at(single).detail).toBeUndefined();
  });

  it('replayed provider ids cannot create duplicate keyed rows', () => {
    const rows = activityRows([
      tool({ id: 'replayed-call', toolName: 'read' }),
      { kind: 'thought', id: 'separator', label: 'Checking the result', live: false },
      tool({ id: 'replayed-call', toolName: 'bash', status: 'running' }),
      { kind: 'text', id: 'separator', text: 'The result is ready.' }
    ]);

    expect(new Set(rows.map((row) => row.renderKey)).size).toBe(rows.length);
    expect(rows.map((row) => row.renderKey)).toEqual([
      'tools-replayed-call-0',
      'separator-1',
      'tools-replayed-call-2',
      'separator-3'
    ]);
  });

  it('only the newest active row can pulse when stale live flags remain', () => {
    const rows = activityRows([
      { kind: 'thought', id: 'old-thought', label: 'Planning the first approach', live: true },
      tool({ id: 'old-tool', toolName: 'bash', status: 'running' }),
      { kind: 'thought', id: 'current-thought', label: 'Refining the result', live: true }
    ]);

    expect(rows.map((row) => row.pulsing)).toEqual([false, false, true]);
  });

  it('thinking separates tool groups and remains fully visible', () => {
    const thought: TraceStep = { kind: 'thought', id: 'thought', label: 'Checking the transition', live: true };
    const rows = activityRows([
      tool({ id: '1', toolName: 'read' }),
      thought,
      tool({ id: '2', toolName: 'bash', status: 'running' })
    ]);

    expect(at(rows).label).toBe('Read files');
    expect(rows[1]).toEqual({ ...thought, renderKey: 'thought-1', pulsing: false });
    expect(rows[2]).toMatchObject({ label: 'Ran commands', status: 'running', pulsing: true });
  });

  it('keeps model text in chronological order with thinking and tool calls', () => {
    const rows = activityRows([
      { kind: 'thought', id: 'thought-0', label: 'Inspecting the project', live: false },
      tool({ id: 'tool-1', toolName: 'read' }),
      { kind: 'text', id: 'text-2', text: 'I found the relevant file.' },
      { kind: 'thought', id: 'thought-3', label: 'Applying the change', live: false },
      tool({ id: 'tool-4', toolName: 'edit' }),
      { kind: 'text', id: 'text-5', text: 'The change is complete.' }
    ]);

    expect(rows.map((row) => `${row.kind}:${(row as any).text ?? (row as any).label}`)).toEqual([
      'thought:Inspecting the project',
      'tools:Read files',
      'text:I found the relevant file.',
      'thought:Applying the change',
      'tools:Edited files',
      'text:The change is complete.'
    ]);
  });

  it('applies running > error > done status precedence', () => {
    expect(at(activityRows([tool({ toolName: 'write', status: 'error' })])).status).toBe('error');
    expect(
      at(activityRows([tool({ id: 'a', status: 'error' }), tool({ id: 'b', status: 'running' })])).status
    ).toBe('running');
    expect(
      at(activityRows([tool({ id: 'a', status: 'done' }), tool({ id: 'b', status: 'error' })])).status
    ).toBe('error');
  });

  it('settles a continued tool call like a done one', () => {
    const rows = activityRows([tool({ toolName: 'bash', status: 'continued' })]);
    expect(at(rows).status).toBe('done');
    expect(at(rows).pulsing).toBe(false);
  });

  it('returns no rows for an empty or all-unknown trace', () => {
    expect(activityRows([])).toEqual([]);
    expect(activityRows(undefined as any)).toEqual([]);
  });
});

describe('toolAction', () => {
  it('maps tool families to sentence fragments', () => {
    expect(toolAction('bash')).toBe('ran commands');
    expect(toolAction('run_command')).toBe('ran commands');
    expect(toolAction('edit_file')).toBe('edited files');
    expect(toolAction('file_write')).toBe('edited files');
    expect(toolAction('web_search')).toBe('searched the web');
    expect(toolAction('read_url')).toBe('searched the web');
    expect(toolAction('image_generate')).toBe('generated images');
    expect(toolAction('computer_use')).toBe('operated the computer');
  });

  it('falls back to naming the tool, shortening mcp identifiers', () => {
    expect(toolAction('blender')).toBe('used blender');
    expect(toolAction('mcp__github__issues')).toBe('used issues');
    expect(toolAction('')).toBe('used a tool');
    expect(toolAction(undefined)).toBe('used a tool');
  });
});

describe('joinActions', () => {
  it('joins one, two, and three-plus actions as English', () => {
    expect(joinActions(['ran commands'])).toBe('ran commands');
    expect(joinActions(['a', 'b'])).toBe('a and b');
    expect(joinActions(['a', 'b', 'c'])).toBe('a, b, and c');
  });
});
