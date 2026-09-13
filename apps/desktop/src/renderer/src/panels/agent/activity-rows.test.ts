import { describe, expect, it } from 'vitest';

import { activityRows, durationLabel, joinActions, thoughtLead, toolAction, toolFamily, type ActivityRow, type TraceStep } from './activity-rows';

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
      label: '3 tool calls',
      summary: 'Ran commands, edited files, and searched the web',
      status: 'done',
      failedCount: 0,
      successCount: 3,
      toolCount: 3,
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

    expect(at(rows).summary).toBe('Ran commands and edited files');
  });

  it('keeps individual labels as detail once a multi-step group settles', () => {
    const rows = activityRows([
      tool({ id: '1', toolName: 'bash', label: 'npm test' }),
      tool({ id: '2', toolName: 'edit', label: 'Timeline.svelte' })
    ]);

    expect(at(rows).detail).toEqual(['Ran commands and edited files', 'npm test', 'Timeline.svelte']);
    expect(at(rows).details).toMatchObject([
      { id: '1', label: 'npm test', status: 'done', family: 'run' },
      { id: '2', label: 'Timeline.svelte', status: 'done', family: 'edit' }
    ]);
  });

  it('keeps real current activity visible while a group is running', () => {
    const running = activityRows([
      tool({ id: '1', toolName: 'bash', label: 'npm test', status: 'done' }),
      tool({ id: '2', toolName: 'edit', label: 'edit · Timeline.svelte', status: 'running' })
    ]);
    expect(at(running).detail).toBeUndefined();
    expect(at(running).currentLabel).toBe('edit · Timeline.svelte');
    expect(at(running).details).toMatchObject([
      { id: '1', label: 'npm test', status: 'done' },
      { id: '2', label: 'edit · Timeline.svelte', status: 'running' }
    ]);

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
    // The thought folds into the group; only text splits it.
    expect(rows.map((row) => row.renderKey)).toEqual([
      'tools-replayed-call-0',
      'separator-1'
    ]);
    expect(at(rows).details.map((detail: any) => detail.kind)).toEqual(['tool', 'thought', 'tool']);
  });

  it('only the newest active row can pulse when stale live flags remain', () => {
    const rows = activityRows([
      { kind: 'thought', id: 'old-thought', label: 'Planning the first approach', live: true },
      tool({ id: 'old-tool', toolName: 'bash', status: 'running' }),
      { kind: 'text', id: 'said', text: 'Halfway.' },
      { kind: 'thought', id: 'current-thought', label: 'Refining the result', live: true }
    ]);

    expect(rows.map((row) => row.pulsing)).toEqual([false, false, true]);
  });

  it('folds thinking into the work group as its own row', () => {
    const rows = activityRows([
      tool({ id: '1', toolName: 'read' }),
      { kind: 'thought', id: 'thought', label: 'Checking the transition. Then the easing.', live: true, startedAt: 5 },
      tool({ id: '2', toolName: 'bash', status: 'running' })
    ]);

    expect(rows).toHaveLength(1);
    expect(at(rows)).toMatchObject({ label: '2 tool calls', summary: 'Read files and ran commands', status: 'running', pulsing: true, thoughtCount: 1 });
    expect(at(rows).details[1]).toMatchObject({
      kind: 'thought', label: 'Thinking', status: 'running', family: 'think',
      detail: 'Checking the transition.', output: 'Checking the transition. Then the easing.'
    });
  });

  it('titles reasoning alone by its duration and lists edited files', () => {
    const alone = activityRows([{ kind: 'thought', id: 't', label: 'Weighing', live: false, startedAt: 0, endedAt: 4_000 }]);
    expect(at(alone)).toMatchObject({ label: 'Thought for 4s', toolCount: 0, status: 'done' });
    expect(at(alone).details[0]).toMatchObject({ label: 'Thought for 4s' });

    const edits = activityRows([
      tool({ id: 'a', toolName: 'edit', label: 'Edit', detail: 'src/a.ts' }),
      tool({ id: 'b', toolName: 'write', label: 'Write', detail: 'src/deep/b.css, c.ts' }),
      tool({ id: 'c', toolName: 'edit', label: 'Edit', detail: 'src/a.ts' }),
      tool({ id: 'd', toolName: 'edit', label: 'Edit', detail: 'broken.ts', status: 'error' }),
      tool({ id: 'e', toolName: 'read', label: 'Read', detail: 'notes.md' })
    ]);
    expect(at(edits).files).toEqual(['a.ts', 'b.css', 'c.ts']);
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

    expect(rows.map((row) => `${row.kind}:${(row as any).text ?? (row as any).summary}`)).toEqual([
      'tools:Read files',
      'text:I found the relevant file.',
      'tools:Edited files',
      'text:The change is complete.'
    ]);
    expect(at(rows, 0).details.map((detail: any) => detail.label)).toEqual(['Thought', 'Ran a command']);
  });

  it('distinguishes a partly failed batch from a wholly failed batch', () => {
    expect(at(activityRows([tool({ toolName: 'write', status: 'error' })])).status).toBe('error');
    expect(
      at(activityRows([tool({ id: 'a', status: 'error' }), tool({ id: 'b', status: 'running' })])).status
    ).toBe('running');
    expect(
      at(activityRows([tool({ id: 'a', status: 'done' }), tool({ id: 'b', status: 'error' })])).status
    ).toBe('partial');
    expect(
      at(activityRows([tool({ id: 'a', status: 'error' }), tool({ id: 'b', status: 'error' })])).status
    ).toBe('error');
    expect(
      at(activityRows([tool({ id: 'a', status: 'done' }), tool({ id: 'b', status: 'error' })])).detail
    ).toEqual(['Ran commands', 'Succeeded · Ran a command', 'Failed · Ran a command']);
    expect(
      at(activityRows([tool({ id: 'a', status: 'done' }), tool({ id: 'b', status: 'error' })])).successCount
    ).toBe(1);
  });

  it('settles a continued tool call like a done one', () => {
    const rows = activityRows([tool({ toolName: 'bash', status: 'continued' })]);
    expect(at(rows).status).toBe('done');
    expect(at(rows).pulsing).toBe(false);
  });

  it('carries the running call\'s argument and the group\'s wall-clock span', () => {
    const running = activityRows([
      tool({ id: '1', toolName: 'bash', label: 'Run', detail: 'npm test', status: 'running', startedAt: 1_000 })
    ]);
    expect(at(running).currentDetail).toBe('npm test');
    expect(at(running).startedAt).toBe(1_000);
    expect(at(running).endedAt).toBeUndefined();

    const settled = activityRows([
      tool({ id: '1', toolName: 'read', label: 'Read', detail: 'a.ts', startedAt: 1_000, endedAt: 2_000 }),
      tool({ id: '2', toolName: 'edit', label: 'Edit', detail: 'a.ts', output: '+3 -1', startedAt: 2_000, endedAt: 5_500 })
    ]);
    expect(at(settled).startedAt).toBe(1_000);
    expect(at(settled).endedAt).toBe(5_500);
    expect(at(settled).details[1]).toMatchObject({ detail: 'a.ts', output: '+3 -1', family: 'edit' });
  });

  it('returns no rows for an empty or all-unknown trace', () => {
    expect(activityRows([])).toEqual([]);
    expect(activityRows(undefined as any)).toEqual([]);
  });
});

describe('toolAction', () => {
  it('describes the screenshot tool group in plain English', () => {
    const rows = activityRows(['bash', 'get_panel_layout', 'get_project_state', 'get_panel_state']
      .map((toolName, index) => tool({ id: String(index), toolName })));
    expect(at(rows).summary).toBe('Ran commands, checked the panel layout, inspected the project, and checked panel controls');
  });

  it.each([
    ['get_project_state', 'inspected the project'],
    ['get_panel_layout', 'checked the panel layout'],
    ['open_panel', 'opened a panel'],
    ['get_panel_state', 'checked panel controls'],
    ['interact_panel', 'used panel controls'],
    ['render_frames', 'previewed composition frames'],
    ['apply_commands', 'edited the composition'],
    ['edit_video', 'edited video clips'],
    ['rollback_changes', 'undid agent changes']
  ])('describes %s with and without its provider prefix', (name, action) => {
    expect(toolAction(name)).toBe(action);
    expect(toolAction(`mcp__powermove__${name}`)).toBe(action);
  });

  it('normalizes provider prefixes before matching families and humanizes unknown names', () => {
    expect(toolAction('mcp__filesystem__read_file')).toBe('read files');
    expect(toolAction('mcp__custom__special_action')).toBe('used special action');
  });

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

describe('toolFamily', () => {
  it.each([
    ['bash', 'run'], ['Bash', 'run'], ['run_command', 'run'],
    ['edit', 'edit'], ['write_file', 'edit'], ['multiedit', 'edit'], ['file_change', 'edit'],
    ['read', 'read'], ['view', 'read'],
    ['grep', 'search'], ['glob', 'search'], ['web_search', 'search'], ['webfetch', 'search'],
    ['image_generation', 'image'], ['computer_use', 'computer'],
    ['mcp__powermove__get_panel_state', 'panel'], ['render_frames', 'panel'],
    ['agent', 'think'], ['todowrite', 'think'],
    ['blender', 'tool'], ['', 'tool'], [undefined, 'tool']
  ])('maps %s to the %s glyph', (name, family) => {
    expect(toolFamily(name as string | undefined)).toBe(family);
  });
});

describe('thoughtLead', () => {
  it('takes the opening sentence, flattens markdown, and bounds the length', () => {
    expect(thoughtLead('The plan is **simple**. Then details follow.')).toBe('The plan is simple.');
    expect(thoughtLead('No terminal punctuation here')).toBe('No terminal punctuation here');
    expect(thoughtLead('Ok. Short first sentence should not stop early.')).toBe('Ok. Short first sentence should not stop early.');
    expect(thoughtLead('x'.repeat(200)).length).toBe(96);
  });
});

describe('durationLabel', () => {
  it('formats spans the way the settled header reads them', () => {
    expect(durationLabel(300)).toBe('<1s');
    expect(durationLabel(4_200)).toBe('4s');
    expect(durationLabel(72_000)).toBe('1m 12s');
    expect(durationLabel(3_600_000 * 2 + 60_000 * 5)).toBe('2h 5m');
    expect(durationLabel(-1)).toBe('');
    expect(durationLabel(Number.NaN)).toBe('');
  });
});

describe('joinActions', () => {
  it('joins one, two, and three-plus actions as English', () => {
    expect(joinActions(['ran commands'])).toBe('ran commands');
    expect(joinActions(['a', 'b'])).toBe('a and b');
    expect(joinActions(['a', 'b', 'c'])).toBe('a, b, and c');
  });
});
